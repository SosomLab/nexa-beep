//! 대화 화면 위젯 — 말풍선 스레드 + 멀티라인 입력(M3 · 08-10 전면 개편).
//!
//! 메시지는 [`nbeep_core::safetext`]를 **통과한 것만** 담긴다(타입이 `SafeText` — 무해화 우회
//! 불가). 입력은 [`crate::edit::EditState`](캐럿·선택·char 단위) + **IME 프리에딧**(조합 중 밑줄
//! 표시 — M3-3). 확정 문자는 `Char`로, 조합 중 텍스트는 [`ChatViewWidget::set_preedit`]로 온다.
//!
//! ## 스레드(사용자 확정 08-10)
//! - **수신 = 좌측 풍선 · 발신 = 우측 풍선**. 풍선은 컨트롤이 아니라 **렌더 프리미티브**다 —
//!   포커스·자체 이벤트가 없고 상태는 [`ChatLine`]이 소유하므로 `controls`(포커스 링·도움말
//!   상속)로 만들면 과잉이다. 그리기 함수로만 분리한다.
//! - **개별 풍선 옆에 시각**(분 단위 · 같은 발신자·같은 분 묶음의 마지막에만) ·
//!   **날짜가 바뀌면 중앙 알약**(구분선 없음). 원본 시각은 밀리초까지 보관([`ChatLine::at_ms`]).
//! - 텍스트는 풍선 최대 폭에서 **자동 줄바꿈**, 스크롤은 픽셀 단위 + 오버레이 스크롤바.
//!
//! ## 입력(사용자 확정 08-10)
//! Enter = 전송 · **Shift+Enter = 줄바꿈**(최대 4줄 표시 · 초과는 캐럿 추종 스크롤) ·
//! 드래그 선택 · 라인 단위 Home/End · 복사/잘라내기/붙여넣기는 호스트가 OS 클립보드와 잇는다.
//!
//! 발신·복귀는 폴링(`take_outgoing`/`take_back`) — 위젯은 부모를 모른다.

use crate::controls::ScrollBars;
use crate::draw::{DrawCtx, FontSlot};
use crate::event::{InputEvent, Key};
use crate::geom::{Point, Rect};
use crate::theme::Theme;
use crate::widget::{Invalidations, Widget};
use nbeep_core::safetext::{sanitize_message, SafeText};
use std::cell::{Cell, RefCell};

/// 스레드 항목의 벽시계 시각(지역 · 분까지 — 표시용).
///
/// ui는 OS 시간대를 모른다(DR-21) — 호스트가 plat에서 변환해 채운다.
/// **원본 정밀도(밀리초)는 [`ChatLine::at_ms`]가 보관**하고 표시는 분에서 끊는다(사용자 확정 08-10).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct WallTime {
    /// 연(예: 2026).
    pub y: u32,
    /// 월 1~12.
    pub mo: u32,
    /// 일 1~31.
    pub d: u32,
    /// 0~23.
    pub h: u32,
    /// 0~59.
    pub m: u32,
}

impl WallTime {
    fn same_day(self, o: Self) -> bool {
        (self.y, self.mo, self.d) == (o.y, o.mo, o.d)
    }
    fn same_minute(self, o: Self) -> bool {
        self.same_day(o) && (self.h, self.m) == (o.h, o.m)
    }
    /// 요일(0=일 … 6=토) — 그레고리력(Zeller 변형).
    fn weekday(self) -> u32 {
        let (mut y, mut m, d) = (i64::from(self.y), i64::from(self.mo), i64::from(self.d));
        if m < 3 {
            y -= 1;
            m += 12;
        }
        let k = y % 100;
        let j = y / 100;
        let h = (d + (13 * (m + 1)) / 5 + k + k / 4 + j / 4 + 5 * j) % 7;
        // Zeller: 0=토 → 0=일 기준으로 회전.
        ((h + 6) % 7) as u32
    }
}

/// `19:02` 또는 `PM 7:02`(24시간 표시 설정 — 사용자 확정 08-10).
/// 목록의 "마지막 확인 시각"(③ 08-13)도 같은 표기를 쓴다 — 시각 표기는 한 벌.
#[must_use]
pub fn fmt_hm(w: WallTime, h24: bool) -> String {
    if h24 {
        format!("{:02}:{:02}", w.h, w.m)
    } else {
        let (ap, h) = if w.h < 12 {
            ("AM", if w.h == 0 { 12 } else { w.h })
        } else {
            ("PM", if w.h == 12 { 12 } else { w.h - 12 })
        };
        format!("{ap} {h}:{:02}", w.m)
    }
}

/// 날짜 알약 라벨 — `2026-08-10 (월)`(기본) 또는 `8/10 (월)`(설정 선택).
fn fmt_date_pill(w: WallTime, short: bool) -> String {
    const DOW: [&str; 7] = ["일", "월", "화", "수", "목", "금", "토"];
    let dow = DOW[w.weekday() as usize % 7];
    if short {
        format!("{}/{} ({dow})", w.mo, w.d)
    } else {
        format!("{:04}-{:02}-{:02} ({dow})", w.y, w.mo, w.d)
    }
}

/// 스레드 한 줄.
#[derive(Clone, Debug)]
pub struct ChatLine {
    /// 내가 보낸 것인가(우측 풍선·색 구분).
    pub mine: bool,
    /// 본문 — 텍스트 또는 파일 전송 기록.
    pub body: ChatBody,
    /// 기록 시각(Unix 밀리초 — **전체 정밀도 보관**, 표시는 분 단위).
    pub at_ms: u64,
    /// 표시용 지역 벽시계(호스트가 plat에서 변환).
    pub wall: WallTime,
    /// 송신자 표시 이름 — **단체 대화**에서 수신 풍선 위에 표시(08-10).
    /// 1:1은 헤더가 상대를 식별하므로 `None`(생략).
    pub from: Option<String>,
}

impl ChatLine {
    /// 텍스트 줄(이미 무해화된 타입만).
    #[must_use]
    pub fn text(mine: bool, text: SafeText, at_ms: u64, wall: WallTime) -> Self {
        Self {
            mine,
            body: ChatBody::Text(text),
            at_ms,
            wall,
            from: None,
        }
    }

    /// 송신자 이름을 붙인다(단체 대화 수신 풍선 식별 — 빌더).
    #[must_use]
    pub fn with_from(mut self, name: impl Into<String>) -> Self {
        self.from = Some(name.into());
        self
    }

    /// 파일 전송 항목 — `승인 대기` 상태로 시작한다.
    #[must_use]
    pub fn xfer(mine: bool, name: SafeText, size: u64, at_ms: u64, wall: WallTime) -> Self {
        Self {
            mine,
            body: ChatBody::Xfer(XferLine {
                thumb: None,
                name,
                size,
                state: XferLineState::Waiting,
            }),
            at_ms,
            wall,
            from: None,
        }
    }
}

/// 스레드 줄의 본문 종류.
#[derive(Clone, Debug)]
pub enum ChatBody {
    /// 무해화된 텍스트.
    Text(SafeText),
    /// 파일 전송 기록 — 진행 중엔 상태가 갱신되고, **완료 후에도 스레드에 남는다**
    /// (송신·수신 이력을 대화에서 본다 — 사용자 요청 08-10).
    Xfer(XferLine),
}

/// 파일 전송 스레드 항목.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct XferLine {
    /// 수신 이미지 소형 미리보기(M4-5ⓑ — imgdec 격리 디코드 · 인라인 18px).
    /// 확대 미리보기는 격리함 몫(후속).
    pub thumb: Option<std::rc::Rc<crate::theme::IconImage>>,
    /// 무해화된 파일명(원격 제공 이름 — 표시 전 무해화 필수).
    pub name: SafeText,
    /// 전체 크기(바이트).
    pub size: u64,
    /// 현재 상태.
    pub state: XferLineState,
}

/// 파일 전송 항목의 상태 — `Waiting`/`Active`만 [`update_xfer_in`] 갱신 대상.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum XferLineState {
    /// 승인 대기(발신 = 상대 승인 · 수신 = 내 승인).
    Waiting,
    /// 전송 중(누적 바이트).
    Active {
        /// 지금까지 오간 바이트.
        done: u64,
    },
    /// **발신 후 수신 종단 확인 대기**(M4-9) — 청크·Done을 다 보냈지만 상대의 `Received`
    /// ack가 아직 안 왔다. "보냈다"≠"닿았다"라 아직 완료가 아니다([`update_xfer_ack`] 대상).
    AwaitingAck,
    /// 완료(부가 설명 — 예: 격리·위험 등급).
    Done {
        /// 상태 뒤에 붙는 설명(비면 "완료"만).
        note: String,
    },
    /// 실패·거절·취소(사유).
    Failed {
        /// 사람이 읽는 사유.
        why: String,
    },
}

/// `lines`에서 **가장 오래된 진행 항목**(`Waiting`/`Active` · 방향 일치)의 상태를 갱신한다.
///
/// 앞에서부터 찾는 이유 — 오퍼 큐가 FIFO라 결정·진행도 오래된 것부터 처리된다
/// (뒤에서 찾으면 대기 2건일 때 앞 건의 거절이 뒤 항목에 붙는다).
/// **`AwaitingAck`·종결(`Done`/`Failed`)은 건드리지 않는다** — 확인 대기 항목은
/// [`update_xfer_ack`]가 따로 닫는다(그래야 다음 파일의 진행이 앞 파일의 확인 대기를
/// 덮지 않는다 · M4-9). 갱신했으면 `true`.
pub fn update_xfer_in(lines: &mut [ChatLine], mine: bool, state: XferLineState) -> bool {
    for line in lines.iter_mut() {
        if line.mine != mine {
            continue;
        }
        if let ChatBody::Xfer(x) = &mut line.body {
            if matches!(
                x.state,
                XferLineState::Waiting | XferLineState::Active { .. }
            ) {
                x.state = state;
                return true;
            }
        }
    }
    false
}

/// `lines`에서 **가장 오래된 확인 대기 항목**(`AwaitingAck` · 방향 일치)을 종결 상태로
/// 닫는다(M4-9 — 수신 ack `Received`→완료 · `Failed`→실패). 갱신했으면 `true`.
pub fn update_xfer_ack(lines: &mut [ChatLine], mine: bool, terminal: XferLineState) -> bool {
    for line in lines.iter_mut() {
        if line.mine != mine {
            continue;
        }
        if let ChatBody::Xfer(x) = &mut line.body {
            if x.state == XferLineState::AwaitingAck {
                x.state = terminal;
                return true;
            }
        }
    }
    false
}

/// 수신 이미지 소형 미리보기 부착(M4-5ⓑ) — **가장 최근의 종결(Done)·미부착** 항목에.
/// 격리 완료 직후 호출되므로 방금 닫힌 항목이 대상이다(FIFO 갱신과 같은 계약).
pub fn attach_xfer_thumb(
    lines: &mut [ChatLine],
    mine: bool,
    thumb: std::rc::Rc<crate::theme::IconImage>,
) -> bool {
    for line in lines.iter_mut().rev() {
        if line.mine != mine {
            continue;
        }
        if let ChatBody::Xfer(x) = &mut line.body {
            if matches!(x.state, XferLineState::Done { .. }) && x.thumb.is_none() {
                x.thumb = Some(thumb);
                return true;
            }
        }
    }
    false
}

/// 사람이 읽는 크기 표기.
fn human_bytes(b: u64) -> String {
    const K: u64 = 1024;
    match b {
        v if v >= K * K => format!("{:.1}MiB", v as f64 / (K * K) as f64),
        v if v >= K => format!("{:.1}KiB", v as f64 / K as f64),
        v => format!("{v}B"),
    }
}

// ── 레이아웃 상수(논리 px) ──
/// 풍선 내부 줄 높이.
const TH_LINE_H: i32 = 22;
/// 풍선 내부 가로 여백.
const BUB_PAD_H: i32 = 10;
/// 풍선 내부 세로 여백.
const BUB_PAD_V: i32 = 5;
/// 풍선 블록 사이 간격.
const ENTRY_GAP: i32 = 6;
/// 날짜 알약 높이·상하 간격.
const PILL_H: i32 = 24;
const PILL_GAP: i32 = 8;
/// 수신 풍선 위 송신자 이름 행 높이(단체 대화).
const NAME_H: i32 = 18;
/// 스레드 좌우 안쪽 여백.
const TH_INSET: i32 = 12;
/// 입력창 표시 줄 상한 — 초과분은 캐럿을 따라 스크롤(사용자 확정 08-10).
const INPUT_MAX_LINES: usize = 4;
/// 입력 한 줄 논리 높이(1줄일 때 40px 유지: 10+20+10).
const INPUT_LINE_H: i32 = 20;
/// 입력창 상하 여백(동일 — 세로 중앙 정렬 · 사용자 지적 08-10).
const INPUT_PAD_V: i32 = 10;

/// paint가 남기는 입력 히트테스트 지오메트리(이벤트 시점에 폰트를 잴 수 없어 캐시).
#[derive(Debug, Default)]
struct InputGeom {
    /// 텍스트 시작 x(물리).
    tx: i32,
    /// 첫 표시 줄 상단 y(물리).
    top: i32,
    /// 줄 높이(물리).
    line_h: i32,
    /// 캐시 시점의 `input_scroll`.
    scroll: usize,
    /// 줄별 문자 경계 x(누적 폭 · len+1개) — **전체 줄**(표시 여부 무관).
    lines: Vec<Vec<i32>>,
}

/// 대화 화면 위젯.
#[derive(Debug)]
pub struct ChatViewWidget {
    bounds: Rect,
    /// 상대 표시 이름(헤더).
    title: String,
    lines: Vec<ChatLine>,
    input: crate::edit::EditState,
    /// IME 조합 중 텍스트(확정 전 — 밑줄 표시. 확정은 input에 삽입).
    preedit: String,
    scale: f32,
    outgoing: Option<SafeText>,
    back: bool,
    /// 스레드 스크롤 — **하단에서 위로 밀어올린 물리 px**(0 = 최신 붙어 봄).
    scroll: i32,
    /// 마지막 커서 위치 — 휠 대상(스레드/입력창) 판정용.
    cursor: (i32, i32),
    /// 진행 중 파일 전송(헤더 아래 진척 줄 · 사용자 요청 08-09).
    xfer: Option<crate::peer_list::XferProgress>,
    /// 입력창 세로 스크롤 — 맨 위 표시 줄(멀티라인 · 표시 상한 [`INPUT_MAX_LINES`]).
    input_scroll: usize,
    /// 입력창 **가로** 스크롤(px) — 줄바꿈 없는 긴 문장을 따라간다(사용자 요청 08-10).
    input_hscroll: i32,
    /// 입력창 드래그 선택 중.
    dragging: bool,
    /// paint가 캐시하는 입력 지오메트리 — 마우스 히트테스트용.
    input_geom: RefCell<InputGeom>,
    /// paint가 캐시하는 스레드 콘텐츠 총 높이(물리 px) — 스크롤 클램프 근거.
    content_h: Cell<i32>,
    /// 24시간 표시(`false` = `PM 7:02`) — 설정 `chat.time_24h`.
    time_24h: bool,
    /// 날짜 축약(`8/10`) — 설정 `chat.date_format`.
    date_short: bool,
    /// 스레드 오버레이 스크롤바(설정·갤러리와 같은 문법 — 사용자 요청 08-10).
    thread_bars: ScrollBars,
    /// 입력창 오버레이 스크롤바(4줄 초과 시).
    input_bars: ScrollBars,
    /// paint가 캐시하는 풍선 히트 rect(→ 항목 인덱스) — 우클릭 복사용.
    hit_rects: RefCell<Vec<(Rect, usize)>>,
    /// 우클릭 복사 요청(1회성) — 호스트가 OS 클립보드에 쓴다.
    copy_out: Option<String>,
    /// 우클릭 컨텍스트 메뉴(입력란 · 말풍선).
    ctx_menu: crate::controls::ContextMenu,
    /// 메뉴를 연 시점의 말풍선 인덱스(있으면 "메시지 복사" 대상).
    ctx_bubble: Option<usize>,
    /// 붙여넣기 요청(1회성) — 클립보드는 호스트만 읽을 수 있다.
    paste_req: bool,
    /// 클립보드에 붙여넣을 것이 있는가 — 호스트가 우클릭 직전에 알려준다.
    /// (UI 크레이트는 OS를 모른다. 모르면 "붙여넣기"를 항상 활성으로 두게 되는데,
    ///  눌러도 아무 일이 없는 항목은 고장으로 읽힌다.)
    clip_has_text: bool,
}

impl ChatViewWidget {
    /// 상대 이름으로 빈 대화 화면을 연다.
    #[must_use]
    pub fn new(title: String) -> Self {
        Self {
            bounds: Rect::default(),
            title,
            lines: Vec::new(),
            input: crate::edit::EditState::new(),
            preedit: String::new(),
            scale: 1.0,
            outgoing: None,
            back: false,
            scroll: 0,
            cursor: (0, 0),
            xfer: None,
            input_scroll: 0,
            input_hscroll: 0,
            dragging: false,
            input_geom: RefCell::new(InputGeom::default()),
            content_h: Cell::new(0),
            time_24h: true,
            date_short: false,
            thread_bars: ScrollBars::new(),
            input_bars: ScrollBars::new(),
            hit_rects: RefCell::new(Vec::new()),
            copy_out: None,
            ctx_menu: crate::controls::ContextMenu::new(),
            ctx_bubble: None,
            paste_req: false,
            clip_has_text: false,
        }
    }

    /// 우클릭으로 복사 요청된 메시지 본문(1회성 — 사용자 요청 08-10).
    pub fn take_copy_text(&mut self) -> Option<String> {
        self.copy_out.take()
    }

    /// 진행 중 전송 상태 지정(`None` = 진행 없음 — 줄이 사라진다).
    pub fn set_xfer(
        &mut self,
        xfer: Option<crate::peer_list::XferProgress>,
        inv: &mut Invalidations,
    ) {
        if self.xfer != xfer {
            self.xfer = xfer;
            inv.push(self.bounds);
        }
    }

    /// 시각 표시 형식 지정(설정 `chat.time_24h`·`chat.date_format`).
    pub fn set_time_format(&mut self, h24: bool, date_short: bool, inv: &mut Invalidations) {
        if (self.time_24h, self.date_short) != (h24, date_short) {
            self.time_24h = h24;
            self.date_short = date_short;
            inv.push(self.bounds);
        }
    }

    /// 스크롤바 자동숨김 틱 — 표시가 바뀌면 `true`. `now_ms`는 호스트 시계(단조).
    pub fn tick(&mut self, now_ms: u64) -> bool {
        self.thread_bars.tick(now_ms) | self.input_bars.tick(now_ms)
    }

    /// 배율 지정(고DPI).
    pub fn set_scale(&mut self, scale: f32, inv: &mut Invalidations) {
        let scale = scale.max(0.5);
        if (scale - self.scale).abs() > f32::EPSILON {
            self.scale = scale;
            inv.push(self.bounds);
        }
    }

    /// 스레드에 줄 추가(수신·발신 확정분 — 이미 무해화된 타입만 받는다).
    pub fn push_line(&mut self, line: ChatLine, inv: &mut Invalidations) {
        self.lines.push(line);
        self.scroll = 0; // 새 메시지 = 최신으로 스냅(표준 채팅 동작)
        inv.push(self.bounds);
    }

    /// 가장 오래된 미종결 전송 항목(방향 일치)의 상태를 갱신한다([`update_xfer_in`]).
    pub fn update_xfer_line(
        &mut self,
        mine: bool,
        state: XferLineState,
        inv: &mut Invalidations,
    ) -> bool {
        let hit = update_xfer_in(&mut self.lines, mine, state);
        if hit {
            inv.push(self.bounds);
        }
        hit
    }

    /// 가장 오래된 확인 대기 항목을 종결 상태로 닫는다([`update_xfer_ack`] · M4-9).
    /// 수신 이미지 소형 미리보기 부착(M4-5ⓑ — 종결 항목에 · 뷰 열려 있을 때 즉시 반영).
    pub fn attach_xfer_thumb(
        &mut self,
        mine: bool,
        thumb: std::rc::Rc<crate::theme::IconImage>,
        inv: &mut Invalidations,
    ) -> bool {
        let hit = attach_xfer_thumb(&mut self.lines, mine, thumb);
        if hit {
            inv.push(self.bounds);
        }
        hit
    }

    pub fn ack_xfer_line(
        &mut self,
        mine: bool,
        terminal: XferLineState,
        inv: &mut Invalidations,
    ) -> bool {
        let hit = update_xfer_ack(&mut self.lines, mine, terminal);
        if hit {
            inv.push(self.bounds);
        }
        hit
    }

    /// 스레드 스크롤 오프셋(물리 px · 테스트).
    #[must_use]
    pub fn scroll(&self) -> i32 {
        self.scroll
    }

    /// Enter로 확정된 발신 본문(1회성) — 조립 지점이 시퀀서·팬아웃에 넘긴다.
    pub fn take_outgoing(&mut self) -> Option<SafeText> {
        self.outgoing.take()
    }

    /// Esc 복귀 요청(1회성).
    pub fn take_back(&mut self) -> bool {
        std::mem::take(&mut self.back)
    }

    /// 현재 입력 중 텍스트(테스트·HUD용).
    #[must_use]
    pub fn input(&self) -> String {
        self.input.text()
    }

    /// 입력 캐럿 위치(문자 인덱스) — 커서 렌더용.
    #[must_use]
    pub fn input_caret(&self) -> usize {
        self.input.caret()
    }

    /// IME 조합 중 텍스트 설정(빈 문자열 = 조합 종료·취소). 확정은 `on_event`의 Char로.
    pub fn set_preedit(&mut self, text: String, inv: &mut Invalidations) {
        self.preedit = text;
        inv.push(self.input_bar());
    }

    /// 조합 중 텍스트(테스트·렌더).
    #[must_use]
    pub fn preedit(&self) -> &str {
        &self.preedit
    }

    // ── 클립보드(호스트가 OS와 잇는다 — ui는 OS를 모른다) ──

    /// 선택 텍스트(복사용) — 없으면 `None`.
    #[must_use]
    pub fn copy_selection(&self) -> Option<String> {
        self.input.selected_text()
    }

    /// 선택 텍스트를 잘라낸다(반환 = 잘라낸 텍스트).
    pub fn cut_selection(&mut self, inv: &mut Invalidations) -> Option<String> {
        let t = self.input.cut();
        if t.is_some() {
            self.after_input_edit(inv);
        }
        t
    }

    /// 우클릭 위치에 맞는 컨텍스트 메뉴를 연다.
    ///
    /// 위치로 대상이 갈린다 — **입력란**이면 편집 3종, **말풍선**이면 메시지 복사.
    /// 둘 다 아니면 열지 않는다(빈 곳 우클릭에 메뉴가 뜨면 무엇에 대한 메뉴인지 모른다).
    fn open_ctx_menu(&mut self, x: i32, y: i32, inv: &mut Invalidations) {
        use crate::controls::CtxItem;
        use nbeep_core::i18n::{t, Msg};

        let p = Point { x, y };
        let mut items = Vec::new();
        self.ctx_bubble = None;

        if self.input_bar().contains(p) {
            let has_sel = self.input.selected_text().is_some();
            let has_text = !self.input.text().is_empty();
            items.push(CtxItem::maybe("copy", t(Msg::CtxCopy), has_sel));
            items.push(CtxItem::maybe("cut", t(Msg::CtxCut), has_sel));
            items.push(CtxItem::maybe(
                "paste",
                t(Msg::CtxPaste),
                self.clip_has_text,
            ));
            items.push(CtxItem::Separator);
            items.push(CtxItem::maybe("select_all", t(Msg::CtxSelectAll), has_text));
        } else if let Some(i) = self
            .hit_rects
            .borrow()
            .iter()
            .find(|(r, _)| r.contains(p))
            .map(|&(_, i)| i)
        {
            // 풍선 우클릭 — 예전에는 여기서 곧바로 클립보드를 덮어썼다(08-10 지적).
            self.ctx_bubble = Some(i);
            items.push(CtxItem::item("copy_message", t(Msg::CtxCopyMessage)));
        }

        if items.is_empty() {
            return;
        }
        // 폭 측정은 paint 밖에서 못 한다 — 가장 긴 라벨의 글자 수로 근사한다
        // (한글은 대략 배 폭이라 그만큼 더 잡는다. 모자라면 잘리므로 넉넉히).
        let widest = items
            .iter()
            .map(|it| match it {
                CtxItem::Item { label, .. } => label
                    .chars()
                    .map(|c| if c.is_ascii() { 8 } else { 15 })
                    .sum::<i32>(),
                CtxItem::Separator => 0,
            })
            .max()
            .unwrap_or(0);
        self.ctx_menu.set_scale(self.scale);
        self.ctx_menu.open_at(x, y, items, self.bounds, widest);
        inv.push(self.ctx_menu.bounds());
    }

    /// 메뉴에서 고른 항목을 실행한다.
    fn run_ctx_action(&mut self, id: &str, inv: &mut Invalidations) {
        match id {
            "copy" => self.copy_out = self.input.selected_text(),
            "cut" => self.copy_out = self.cut_selection(inv),
            // 클립보드는 호스트만 읽는다 — 요청만 남기고 돌아간다.
            "paste" => self.paste_req = true,
            "select_all" => {
                self.input.key(crate::edit::EditKey::SelectAll, false);
                inv.push(self.input_bar());
            }
            "copy_message" => {
                if let Some(i) = self.ctx_bubble.take() {
                    if let Some(line) = self.lines.get(i) {
                        self.copy_out = Some(self.body_text(line));
                    }
                }
            }
            _ => {}
        }
    }

    /// 컨텍스트 메뉴가 요청한 붙여넣기를 가져간다(1회성) — 호스트가 클립보드를 읽어
    /// [`ChatViewWidget::paste`]를 부른다.
    pub fn take_paste_request(&mut self) -> bool {
        core::mem::take(&mut self.paste_req)
    }

    /// 클립보드에 붙여넣을 텍스트가 있는지 알려 준다(우클릭 직전에 호스트가 갱신).
    pub fn set_clipboard_has_text(&mut self, yes: bool) {
        self.clip_has_text = yes;
    }

    /// 붙여넣기 — CRLF는 LF로, 개행·탭 외 제어문자는 버린다(무해화는 전송 시 한 번 더).
    pub fn paste(&mut self, text: &str, inv: &mut Invalidations) {
        let mut clean = String::with_capacity(text.len());
        let mut chars = text.chars().peekable();
        while let Some(c) = chars.next() {
            match c {
                '\r' => {
                    if chars.peek() == Some(&'\n') {
                        chars.next();
                    }
                    clean.push('\n');
                }
                '\t' => clean.push('\t'),
                c if c.is_control() && c != '\n' => {}
                c => clean.push(c),
            }
        }
        if !clean.is_empty() {
            self.input.insert_str(&clean);
            self.after_input_edit(inv);
        }
    }

    // ── 멀티라인 입력 유틸 ──

    fn s(&self, logical: i32) -> i32 {
        (logical as f32 * self.scale).round() as i32
    }

    /// 입력 텍스트의 줄 수(빈 텍스트 = 1).
    fn input_line_count(&self) -> usize {
        self.input.text().split('\n').count().max(1)
    }

    /// 입력창 rect — 줄 수에 따라 위로 자란다(표시 상한 [`INPUT_MAX_LINES`]).
    fn input_bar(&self) -> Rect {
        let visible = self.input_line_count().min(INPUT_MAX_LINES) as i32;
        let h = self.s(INPUT_PAD_V) * 2 + self.s(INPUT_LINE_H) * visible;
        Rect::new(self.bounds.x, self.bounds.bottom() - h, self.bounds.w, h)
    }

    /// 캐럿의 (줄, 줄 내 문자) 위치.
    fn caret_line_col(&self) -> (usize, usize) {
        let text: Vec<char> = self.input.text().chars().collect();
        let caret = self.input.caret().min(text.len());
        let mut line = 0usize;
        let mut col = 0usize;
        for &c in &text[..caret] {
            if c == '\n' {
                line += 1;
                col = 0;
            } else {
                col += 1;
            }
        }
        (line, col)
    }

    /// (줄, 열) → 전역 문자 인덱스.
    fn index_at(&self, line: usize, col: usize) -> usize {
        let text = self.input.text();
        let mut idx = 0usize;
        for (li, l) in text.split('\n').enumerate() {
            let len = l.chars().count();
            if li == line {
                return idx + col.min(len);
            }
            idx += len + 1; // '\n' 포함
        }
        text.chars().count()
    }

    /// 입력창에서 실제로 글자가 보이는 가로 폭(px).
    fn input_view_w(&self) -> i32 {
        (self.input_bar().w - self.s(20)).max(1)
    }

    /// 현재 입력 내용의 최대 줄 폭(px · paint 캐시 기준 · 캐시 전엔 0).
    fn input_content_w(&self) -> i32 {
        self.input_geom
            .borrow()
            .lines
            .iter()
            .filter_map(|xs| xs.last().copied())
            .max()
            .unwrap_or(0)
    }

    /// 입력창 가로 썸의 현재 rect(테스트·검증용) — 없으면 `None`.
    #[cfg(test)]
    fn input_h_thumb_for_test(&self) -> Option<Rect> {
        let input = self.input_bar();
        let content_w = (self.input_content_w() + self.s(20)).max(input.w);
        crate::controls::ScrollBars::h_thumb_for_test(
            input,
            content_w,
            self.input_hscroll,
            self.scale,
        )
    }

    /// **가로** 스크롤을 캐럿이 보이도록 맞춘다(줄바꿈 없는 긴 문장 대응).
    /// paint가 남긴 문자 경계 실측을 쓴다 — 폭 계산을 두 벌로 만들지 않는다.
    fn ensure_caret_visible_h(&mut self) {
        let before = self.input_hscroll;
        let (cline, ccol) = self.caret_line_col();
        let cx = {
            let g = self.input_geom.borrow();
            match g.lines.get(cline) {
                Some(xs) => xs[ccol.min(xs.len().saturating_sub(1))],
                None => 0,
            }
        };
        let view = self.input_view_w();
        let margin = self.s(24); // 캐럿이 가장자리에 붙지 않게 여유를 둔다
        if cx - self.input_hscroll > view - margin {
            self.input_hscroll = cx - view + margin;
        }
        if cx - self.input_hscroll < 0 {
            self.input_hscroll = (cx - margin).max(0);
        }
        let max_h = (self.input_content_w() - view + margin).max(0);
        let next = self.input_hscroll.clamp(0, max_h);
        if next != before {
            // 코드가 옮긴 스크롤도 **막대를 보여 준다** — 안 그러면 사용자 눈에는
            // "가로 스크롤이 생기지 않는다"로 보인다(08-10 지적).
            self.input_bars.show();
        }
        self.input_hscroll = next;
    }

    /// 편집 후 공통 — 캐럿이 보이도록 입력 스크롤을 따라붙인다.
    fn after_input_edit(&mut self, inv: &mut Invalidations) {
        let (line, _) = self.caret_line_col();
        let count = self.input_line_count();
        let max_top = count.saturating_sub(INPUT_MAX_LINES);
        if line < self.input_scroll {
            self.input_scroll = line;
        } else if line >= self.input_scroll + INPUT_MAX_LINES {
            self.input_scroll = line + 1 - INPUT_MAX_LINES;
        }
        self.input_scroll = self.input_scroll.min(max_top);
        self.ensure_caret_visible_h();
        inv.push(self.bounds); // 줄 수 변화 = 입력창 높이 변화(전체 재배치)
    }

    /// 마우스 좌표 → 입력 문자 인덱스(paint 캐시 기반 — 캐시 전엔 `None`).
    fn input_hit(&self, x: i32, y: i32) -> Option<usize> {
        let g = self.input_geom.borrow();
        if g.line_h <= 0 || g.lines.is_empty() {
            return None;
        }
        let rel = (y - g.top).max(0) / g.line_h.max(1);
        let line = (g.scroll + rel as usize).min(g.lines.len() - 1);
        let xs = &g.lines[line];
        let mut col = xs.len().saturating_sub(1);
        for (i, &bx) in xs.iter().enumerate() {
            if x < g.tx + bx {
                col = i.saturating_sub(if i > 0 { 1 } else { 0 });
                // 문자 중간 이후 클릭은 다음 경계로 — 가장 가까운 경계 선택.
                if i > 0 {
                    let prev = g.tx + xs[i - 1];
                    let cur = g.tx + bx;
                    col = if x - prev > cur - x { i } else { i - 1 };
                }
                break;
            }
        }
        Some(self.index_at(line, col))
    }

    /// 스레드 뷰포트(헤더·진척 줄·입력창 제외).
    fn thread_viewport(&self) -> Rect {
        let head_h = self.s(34);
        let xfer_h = if self.xfer.is_some() { self.s(22) } else { 0 };
        let input = self.input_bar();
        let top = self.bounds.y + head_h + xfer_h;
        Rect::new(
            self.bounds.x,
            top,
            self.bounds.w,
            (input.y - self.s(4) - top).max(0),
        )
    }

    /// 스레드 콘텐츠 높이 추정(개행만 반영 — 자동 줄바꿈 전 하한).
    /// paint가 실측을 [`Self::content_h`]에 캐시하면 그 값을 우선한다.
    fn content_h_estimate(&self) -> i32 {
        let mut h = 0;
        let mut prev: Option<WallTime> = None;
        for l in &self.lines {
            if prev.is_none_or(|p| !p.same_day(l.wall)) {
                h += self.s(PILL_H) + self.s(PILL_GAP) * 2;
            }
            let subs = match &l.body {
                ChatBody::Text(t) => t.as_str().split('\n').count().max(1),
                ChatBody::Xfer(_) => 1,
            } as i32;
            if !l.mine && l.from.is_some() {
                h += self.s(NAME_H); // 상한 추정(연속 묶음 생략은 paint가 정확히)
            }
            h += subs * self.s(TH_LINE_H) + self.s(BUB_PAD_V) * 2 + self.s(ENTRY_GAP);
            prev = Some(l.wall);
        }
        h
    }

    fn max_scroll(&self) -> i32 {
        let content = self.content_h.get().max(self.content_h_estimate());
        (content - self.thread_viewport().h).max(0)
    }

    /// 이 항목 옆에 시각을 표시하는가 — 같은 발신자·같은 분 묶음의 **마지막**에만.
    fn shows_time(&self, i: usize) -> bool {
        match self.lines.get(i + 1) {
            None => true,
            Some(next) => {
                next.mine != self.lines[i].mine || !next.wall.same_minute(self.lines[i].wall)
            }
        }
    }

    /// 풍선 본문 텍스트(전송 항목은 상태 라벨 포함 한 줄).
    fn body_text(&self, l: &ChatLine) -> String {
        match &l.body {
            ChatBody::Text(t) => t.as_str().to_string(),
            ChatBody::Xfer(x) => {
                let dir = if l.mine { "전송" } else { "수신" };
                let state = match &x.state {
                    XferLineState::Waiting => "승인 대기".to_string(),
                    XferLineState::Active { done } => {
                        let pct = if x.size > 0 {
                            (*done as f64 / x.size as f64 * 100.0).round() as u32
                        } else {
                            0
                        };
                        format!("{dir} {pct}% · {}", human_bytes(*done))
                    }
                    XferLineState::AwaitingAck => "전달됨 · 확인 대기".to_string(),
                    XferLineState::Done { note } if note.is_empty() => "완료".to_string(),
                    XferLineState::Done { note } => format!("완료 — {note}"),
                    XferLineState::Failed { why } => format!("실패 — {why}"),
                };
                format!(
                    "[파일] {} ({}) · {state}",
                    x.name.as_str(),
                    human_bytes(x.size)
                )
            }
        }
    }
}

/// 그리기용 — 문자 단위 그리디 줄바꿈(공백 우선 분리 · 한 단어가 폭을 넘으면 문자에서 자른다).
fn wrap_text(ctx: &mut dyn DrawCtx, text: &str, max_w: i32) -> Vec<String> {
    let mut out = Vec::new();
    for raw in text.split('\n') {
        if raw.is_empty() {
            out.push(String::new());
            continue;
        }
        let mut line = String::new();
        let mut line_w = 0;
        let mut last_space: Option<(usize, i32)> = None; // (byte idx, width까지)
        for c in raw.chars() {
            let cw = ctx.text_width(&c.to_string());
            if line_w + cw > max_w && !line.is_empty() {
                if let Some((bi, _)) = last_space {
                    // 공백에서 자른다 — 다음 줄은 공백 뒤부터.
                    let rest = line.split_off(bi);
                    out.push(std::mem::take(&mut line));
                    line = rest.trim_start().to_string();
                    line_w = ctx.text_width(&line);
                    last_space = None;
                } else {
                    out.push(std::mem::take(&mut line));
                    line_w = 0;
                }
            }
            if c == ' ' {
                last_space = Some((line.len(), line_w));
            }
            line.push(c);
            line_w += cw;
        }
        out.push(line);
    }
    if out.is_empty() {
        out.push(String::new());
    }
    out
}

impl Widget for ChatViewWidget {
    fn bounds(&self) -> Rect {
        self.bounds
    }

    fn set_bounds(&mut self, bounds: Rect, inv: &mut Invalidations) {
        self.bounds = bounds;
        inv.push(bounds);
    }

    fn on_event(&mut self, ev: &InputEvent, inv: &mut Invalidations) {
        use crate::edit::EditKey;

        // ── 컨텍스트 메뉴가 열려 있으면 **가장 먼저** 먹는다(팝업은 최상위 레이어) ──
        if self.ctx_menu.is_open() {
            let menu_rect = self.ctx_menu.bounds();
            if self.ctx_menu.on_event(ev) {
                inv.push(menu_rect);
                inv.push(self.bounds);
                if let Some(id) = self.ctx_menu.take_picked() {
                    self.run_ctx_action(&id, inv);
                }
                return;
            }
        }

        // 커서 추적 — 휠의 대상(스레드 vs 입력창)을 위치로 가른다(사용자 확정 08-10).
        if let InputEvent::MouseMove { x, y } = *ev {
            self.cursor = (x, y);
        }
        let is_wheel = matches!(ev, InputEvent::Wheel { .. } | InputEvent::HWheel { .. });
        let over_input = self.input_bar().contains(Point {
            x: self.cursor.0,
            y: self.cursor.1,
        });

        // ── 입력창 스크롤(멀티라인) — 입력창 위의 휠은 **입력 내용만** 스크롤한다 ──
        let count = self.input_line_count();
        // 가로 콘텐츠 폭을 함께 넘겨야 **가로 막대 드래그·가로 휠**이 산다.
        let content_w = self.input_content_w() + self.s(20);
        let input_rect = self.input_bar();
        // ★ 세로 조건만 보면 안 된다 — 줄바꿈 없는 긴 한 줄은 **세로로는 1줄**이라
        //   여기서 걸러졌고, 그 결과 가로 막대는 그려지는데 클릭·드래그·가로 휠이
        //   전혀 닿지 않았다(08-11 지적: "좌우 스크롤을 클릭해서 드래그할 수 없다",
        //   "트랙패드 두 손가락 좌우 이동에도 막대가 뜨지 않는다").
        let needs_v = count > INPUT_MAX_LINES;
        let needs_h = content_w > input_rect.w;
        if (needs_v || needs_h) && (!is_wheel || over_input) {
            let input = input_rect;
            let line_h = self.s(INPUT_LINE_H);
            // 콘텐츠 높이에 상하 여백 포함 — 뷰포트(input.h)에도 여백이 들어 있어,
            // 빼먹으면 최대 스크롤이 정확히 1줄 모자란다(사용자 재현 08-10).
            let content_h = count as i32 * line_h + self.s(INPUT_PAD_V) * 2;
            let (nx, ny, consumed) = self.input_bars.on_event(
                ev,
                input,
                content_w.max(input.w),
                content_h,
                self.input_hscroll,
                self.input_scroll as i32 * line_h,
                self.scale,
            );
            if nx != self.input_hscroll {
                self.input_hscroll = nx.max(0);
                inv.push(self.bounds);
            }
            // 반올림 — 내림이면 경계에서 마지막 줄에 못 닿는다.
            let nl = ((ny + line_h / 2) / line_h.max(1)).max(0) as usize;
            if nl != self.input_scroll {
                self.input_scroll = nl.min(count.saturating_sub(INPUT_MAX_LINES));
                inv.push(self.bounds);
            }
            if consumed {
                return;
            }
        }
        // ── 스레드 스크롤바 — 휠 포함(활동 감지·페이드는 컴포넌트 몫 · 갤러리와 동일 문법).
        //    단 입력창 위의 휠은 스레드로 새지 않는다(위에서 입력창이 대상).
        if !is_wheel || !over_input {
            let vp = self.thread_viewport();
            let content = self.content_h.get().max(self.content_h_estimate());
            let top_off = (content - vp.h - self.scroll).max(0);
            let (_, ny, consumed) = self
                .thread_bars
                .on_event(ev, vp, vp.w, content, 0, top_off, self.scale);
            if ny != top_off {
                self.scroll = (content - vp.h - ny).clamp(0, self.max_scroll());
                inv.push(self.bounds);
            }
            if consumed {
                return;
            }
        }

        match *ev {
            InputEvent::Key {
                key: Key::Enter,
                shift,
                ..
            } => {
                if shift {
                    // Shift+Enter = 줄바꿈(사용자 확정 08-10).
                    self.input.insert('\n');
                    self.after_input_edit(inv);
                } else {
                    let text = self.input.text();
                    let trimmed = text.trim();
                    if !trimmed.is_empty() {
                        // 발신 확정 — 무해화를 여기서 통과시킨다(개행은 보존된다).
                        self.outgoing = Some(sanitize_message(trimmed));
                        self.input.set_text("");
                        self.input_scroll = 0;
                        inv.push(self.bounds);
                    }
                }
            }
            InputEvent::Key {
                key: Key::Escape, ..
            } => {
                self.back = true;
            }
            InputEvent::Key {
                key: Key::PageUp, ..
            } => {
                let vp = self.thread_viewport();
                self.scroll = (self.scroll + vp.h).min(self.max_scroll());
                inv.push(self.bounds);
            }
            InputEvent::Key {
                key: Key::PageDown, ..
            } => {
                let vp = self.thread_viewport();
                self.scroll = (self.scroll - vp.h).max(0);
                inv.push(self.bounds);
            }
            InputEvent::Key {
                key: Key::Home,
                shift,
                ..
            } => {
                // 줄 단위 Home(멀티라인 — 사용자 요청 08-10).
                let (line, _) = self.caret_line_col();
                self.input.set_caret(self.index_at(line, 0), shift);
                inv.push(self.input_bar());
            }
            InputEvent::Key {
                key: Key::End,
                shift,
                ..
            } => {
                let (line, _) = self.caret_line_col();
                self.input.set_caret(self.index_at(line, usize::MAX), shift);
                inv.push(self.input_bar());
            }
            InputEvent::Key {
                key: Key::Up,
                shift,
                ..
            } if self.input_line_count() > 1 => {
                let (line, col) = self.caret_line_col();
                if line > 0 {
                    self.input.set_caret(self.index_at(line - 1, col), shift);
                    self.after_input_edit(inv);
                }
            }
            InputEvent::Key {
                key: Key::Down,
                shift,
                ..
            } if self.input_line_count() > 1 => {
                let (line, col) = self.caret_line_col();
                if line + 1 < self.input_line_count() {
                    self.input.set_caret(self.index_at(line + 1, col), shift);
                    self.after_input_edit(inv);
                }
            }
            InputEvent::Key {
                key,
                shift,
                primary,
            } => {
                // ⌘/Ctrl+←/→ = 줄 처음/끝(mac 관례 · DR-16 — 08-13 전수 검사.
                // 멀티라인이라 Home/End 팔과 같은 줄 단위 이동).
                if primary && matches!(key, Key::Left | Key::Right) {
                    let (line, _) = self.caret_line_col();
                    let col = if matches!(key, Key::Left) {
                        0
                    } else {
                        usize::MAX
                    };
                    self.input.set_caret(self.index_at(line, col), shift);
                    inv.push(self.input_bar());
                    return;
                }
                let ek = match key {
                    Key::Left => Some(EditKey::Left),
                    Key::Right => Some(EditKey::Right),
                    Key::Delete => Some(EditKey::DeleteForward),
                    _ => None,
                };
                if let Some(ek) = ek {
                    self.input.key(ek, shift);
                    self.after_input_edit(inv);
                }
            }
            InputEvent::MouseDown { x, y, shift, .. } => {
                let input = self.input_bar();
                if input.contains(Point { x, y }) {
                    if let Some(idx) = self.input_hit(x, y) {
                        self.input.set_caret(idx, shift);
                        self.dragging = true;
                        inv.push(input);
                    }
                }
            }
            InputEvent::MouseMove { x, y } => {
                if self.dragging {
                    // 영역 밖 드래그 = 자동 스크롤 — 가려진 윗줄/아랫줄까지 선택이
                    // 이어진다(사용자 보완 요청 08-10).
                    let input = self.input_bar();
                    if y < input.y + self.s(INPUT_PAD_V) {
                        self.input_scroll = self.input_scroll.saturating_sub(1);
                    } else if y > input.bottom() - self.s(INPUT_PAD_V) {
                        let max_top = self.input_line_count().saturating_sub(INPUT_MAX_LINES);
                        self.input_scroll = (self.input_scroll + 1).min(max_top);
                    }
                    // **좌우** 자동 스크롤 — 창 밖으로 끌면 가려진 글자까지 선택이 이어진다
                    // (08-10 지적: 드래그로 좌우 선택이 불가능했다).
                    let edge = self.s(12);
                    let step = self.s(24);
                    let max_h = (self.input_content_w() - self.input_view_w() + self.s(24)).max(0);
                    if x < input.x + edge {
                        self.input_hscroll = (self.input_hscroll - step).max(0);
                        self.input_bars.show();
                    } else if x > input.right() - edge {
                        self.input_hscroll = (self.input_hscroll + step).min(max_h);
                        self.input_bars.show();
                    }
                    // 자동 스크롤로 창 밖까지 끌 때도 캐럿이 끝까지 따라가게 좌표를 가둔다.
                    let hx = x.clamp(input.x + self.s(1), input.right() - self.s(1));
                    let hy = y.clamp(input.y + self.s(1), input.bottom() - self.s(1));
                    if let Some(idx) = self.input_hit(hx, hy) {
                        self.input.set_caret(idx, true);
                    }
                    inv.push(self.bounds);
                }
            }
            InputEvent::MouseUp { .. } => {
                self.dragging = false;
            }
            InputEvent::RightDown { x, y } => {
                self.open_ctx_menu(x, y, inv);
            }
            InputEvent::SelectAll => {
                self.input.key(EditKey::SelectAll, false);
                inv.push(self.input_bar());
            }
            InputEvent::Char { c, .. } => {
                self.preedit.clear(); // 확정 문자 도착 = 조합 종료
                if c == '\u{8}' {
                    self.input.backspace();
                } else if !c.is_control() {
                    self.input.insert(c);
                }
                self.after_input_edit(inv);
            }
            _ => {}
        }
    }

    #[allow(clippy::too_many_lines)]
    fn paint(&self, ctx: &mut dyn DrawCtx, theme: &Theme) {
        ctx.fill_rect(self.bounds, theme.panel_bg);
        // ⚠️ 그리기 순서 = 스레드 먼저, **헤더는 마지막**(사용자 버그 지적 08-10 —
        // 스크롤로 잘린 풍선이 타이틀 위로 올라와 보였다. 헤더가 위 레이어가 되어 덮는다.
        // 그룹 대화의 참여자 목록/커스텀 타이틀도 같은 헤더 영역이고, 스크롤 영역은
        // 그 아래부터다 — thread_viewport가 이미 그렇게 자른다).

        // ── 스레드(말풍선) — 전체 레이아웃을 계산해 총 높이를 캐시하고, 보이는 것만 그린다 ──
        let vp = self.thread_viewport();
        let line_h = self.s(TH_LINE_H);
        let pad_h = self.s(BUB_PAD_H);
        let pad_v = self.s(BUB_PAD_V);
        let inset = self.s(TH_INSET);
        let max_text_w = (vp.w * 2 / 3 - pad_h * 2).max(self.s(40));

        ctx.select_font(FontSlot::Message, false);
        let th = ctx.text_height();

        // 1패스 — 블록 레이아웃(알약·풍선 줄들·높이).
        struct Block {
            pill: Option<String>,
            /// 수신 풍선 위 송신자 이름(단체 대화 — 같은 송신자 연속 묶음은 첫 풍선에만).
            name: Option<String>,
            entry: usize,
            lines: Vec<String>,
            h: i32,
        }
        let name_h = self.s(NAME_H);
        let mut blocks: Vec<Block> = Vec::with_capacity(self.lines.len());
        let mut content = 0;
        let mut prev: Option<WallTime> = None;
        for (i, l) in self.lines.iter().enumerate() {
            let pill = if prev.is_none_or(|p| !p.same_day(l.wall)) {
                content += self.s(PILL_H) + self.s(PILL_GAP) * 2;
                Some(fmt_date_pill(l.wall, self.date_short))
            } else {
                None
            };
            let name = if l.mine {
                None
            } else {
                match &l.from {
                    Some(n)
                        if pill.is_some()
                            || i == 0
                            || self.lines[i - 1].mine
                            || self.lines[i - 1].from.as_deref() != Some(n.as_str()) =>
                    {
                        Some(n.clone())
                    }
                    _ => None,
                }
            };
            let wrapped = wrap_text(ctx, &self.body_text(l), max_text_w);
            let h =
                wrapped.len() as i32 * line_h + pad_v * 2 + if name.is_some() { name_h } else { 0 };
            content += h + self.s(ENTRY_GAP);
            blocks.push(Block {
                pill,
                name,
                entry: i,
                lines: wrapped,
                h,
            });
            prev = Some(l.wall);
        }
        self.content_h.set(content);
        let scroll = self.scroll.clamp(0, (content - vp.h).max(0));

        // 2패스 — 그리기(top-down · 콘텐츠가 뷰포트보다 작으면 하단 정렬).
        self.hit_rects.borrow_mut().clear();
        let mut y = if content >= vp.h {
            vp.y - (content - vp.h - scroll)
        } else {
            vp.y + (vp.h - content)
        };
        for b in &blocks {
            if let Some(pill) = &b.pill {
                y += self.s(PILL_GAP);
                let ph = self.s(PILL_H);
                if y + ph >= vp.y && y < vp.bottom() {
                    ctx.select_font(FontSlot::Status, false);
                    let sh = ctx.text_height();
                    let pw = ctx.text_width(pill) + self.s(24);
                    let px = vp.x + (vp.w - pw) / 2;
                    let pr = Rect::new(px, y, pw, ph);
                    ctx.fill_round_rect(pr, ph / 2, theme.panel_bg_alt);
                    ctx.text(px + self.s(12), y + (ph - sh) / 2, pr, pill, theme.text_dim);
                    ctx.select_font(FontSlot::Message, false);
                }
                y += ph + self.s(PILL_GAP);
            }
            let l = &self.lines[b.entry];
            if y + b.h >= vp.y && y < vp.bottom() {
                // 송신자 이름(단체 대화 · 수신 풍선 위 — 같은 송신자 연속 묶음은 첫 풍선에만).
                let mut by0 = y;
                if let Some(n) = &b.name {
                    ctx.select_font(FontSlot::Status, false);
                    let sh = ctx.text_height();
                    ctx.text(
                        vp.x + inset + self.s(2),
                        by0 + (name_h - sh) / 2,
                        Rect::new(vp.x, by0, vp.w, name_h),
                        n,
                        theme.text_dim,
                    );
                    ctx.select_font(FontSlot::Message, false);
                    by0 += name_h;
                }
                // 풍선 — 수신 좌측 / 발신 우측(사용자 확정 08-10).
                // 수신 이미지 소형 미리보기(M4-5ⓑ) — 있으면 첫 줄 앞 18px 자리.
                let xthumb = match &l.body {
                    ChatBody::Xfer(x) => x.thumb.clone(),
                    ChatBody::Text(_) => None,
                };
                let thumb_pad = if xthumb.is_some() { self.s(24) } else { 0 };
                let widest = b.lines.iter().map(|s| ctx.text_width(s)).max().unwrap_or(0);
                let bw = widest + pad_h * 2 + thumb_pad;
                let bx = if l.mine {
                    vp.right() - inset - bw
                } else {
                    vp.x + inset
                };
                let bub_h = b.h - if b.name.is_some() { name_h } else { 0 };
                let bub = Rect::new(bx, by0, bw, bub_h);
                let (bg, fg) = if l.mine {
                    (theme.accent, theme.chrome_bg)
                } else {
                    // 수신 풍선 — 다크는 패널보다 밝게, 라이트는 어둡게(대비 확보 · 08-10).
                    (theme.bubble_peer, theme.text)
                };
                ctx.fill_round_rect(bub, self.s(9), bg);
                // 꼬리 — 발신은 오른쪽, 수신은 왼쪽을 가리켜 방향을 구별(사용자 요청 08-10).
                let tail_y = by0 + self.s(6);
                if l.mine {
                    ctx.fill_triangle(
                        (bub.right() - 1, tail_y),
                        (bub.right() - 1, tail_y + self.s(10)),
                        (bub.right() + self.s(6), tail_y + self.s(2)),
                        bg,
                    );
                } else {
                    ctx.fill_triangle(
                        (bub.x + 1, tail_y),
                        (bub.x + 1, tail_y + self.s(10)),
                        (bub.x - self.s(6), tail_y + self.s(2)),
                        bg,
                    );
                }
                self.hit_rects.borrow_mut().push((bub, b.entry)); // 우클릭 복사 히트
                if let Some(img) = &xthumb {
                    let d = self.s(18);
                    let ir = Rect::new(bub.x + pad_h, by0 + pad_v + (line_h - d) / 2, d, d);
                    ctx.image_scaled(ir, img, bub);
                }
                for (si, s) in b.lines.iter().enumerate() {
                    let ly = by0 + pad_v + si as i32 * line_h;
                    let clip = Rect::new(bub.x, ly, bub.w, line_h);
                    ctx.text(
                        bub.x + pad_h + thumb_pad,
                        ly + (line_h - th) / 2,
                        clip,
                        s,
                        fg,
                    );
                }
                // 진행 중 전송 풍선 — 하단에 소형 막대.
                if let ChatBody::Xfer(x) = &l.body {
                    if let XferLineState::Active { done } = x.state {
                        let ratio = if x.size > 0 {
                            (done as f32 / x.size as f32).clamp(0.0, 1.0)
                        } else {
                            0.0
                        };
                        let bar_h = self.s(4);
                        let bar = Rect::new(
                            bub.x + pad_h,
                            bub.bottom() - pad_v - bar_h,
                            bub.w - pad_h * 2,
                            bar_h,
                        );
                        ctx.fill_round_rect(bar, bar_h / 2, theme.panel_bg);
                        let fw = (bar.w as f32 * ratio).round() as i32;
                        if fw > 0 {
                            ctx.fill_round_rect(
                                Rect::new(bar.x, bar.y, fw, bar_h),
                                bar_h / 2,
                                theme.ok,
                            );
                        }
                    }
                }
                // 시각 — 풍선 바깥쪽(발신=왼쪽 · 수신=오른쪽), 같은 분 묶음의 마지막에만.
                if self.shows_time(b.entry) {
                    ctx.select_font(FontSlot::Status, false);
                    let sh = ctx.text_height();
                    let label = fmt_hm(l.wall, self.time_24h);
                    let lw = ctx.text_width(&label);
                    let lx = if l.mine {
                        bub.x - self.s(6) - lw
                    } else {
                        bub.right() + self.s(6)
                    };
                    ctx.text(
                        lx,
                        bub.bottom() - sh - self.s(2),
                        Rect::new(vp.x, bub.y, vp.w, b.h + self.s(4)),
                        &label,
                        theme.text_dim,
                    );
                    ctx.select_font(FontSlot::Message, false);
                }
            }
            y += b.h + self.s(ENTRY_GAP);
        }
        // 스레드 오버레이 스크롤바.
        let top_off = (content - vp.h - scroll).max(0);
        self.thread_bars
            .paint(ctx, theme, vp, vp.w, content, 0, top_off, self.scale);

        // ── 헤더(맨 위 레이어 — 스크롤된 풍선을 덮는다 · 그룹 참여자 목록/타이틀 자리) ──
        let head_h = self.s(34);
        let head = Rect::new(self.bounds.x, self.bounds.y, self.bounds.w, head_h);
        ctx.select_font(FontSlot::Base, false);
        ctx.text_opaque(
            head.x + self.s(12),
            head.y + self.s(7),
            head,
            &self.title,
            theme.text,
            theme.chrome_bg,
        );

        // 전송 진척 줄 — 헤더 바로 아래(배치 합계 — 항목별 진행은 스레드 풍선에).
        if let Some(xp) = self.xfer {
            let row = Rect::new(head.x, head.bottom(), head.w, self.s(22));
            ctx.fill_rect(row, theme.chrome_bg);
            ctx.select_font(FontSlot::Status, false);
            let sh = ctx.text_height();
            let pct = (xp.ratio() * 100.0).round() as u32;
            let label = format!(
                "{} {pct}% · {} / {} ({}/{} 파일)",
                if xp.sending { "전송" } else { "수신" },
                human_bytes(xp.done_bytes),
                human_bytes(xp.total_bytes),
                xp.done_files,
                xp.total_files
            );
            ctx.text(
                row.x + self.s(12),
                row.y + (row.h - sh) / 2,
                row,
                &label,
                theme.text_dim,
            );
            let bar_w = self.s(90);
            let bar_h = self.s(5);
            let bx = row.right() - bar_w - self.s(12);
            let by = row.y + (row.h - bar_h) / 2;
            ctx.fill_round_rect(
                Rect::new(bx, by, bar_w, bar_h),
                bar_h / 2,
                theme.panel_bg_alt,
            );
            let fw = (bar_w as f32 * xp.ratio()).round() as i32;
            if fw > 0 {
                ctx.fill_round_rect(
                    Rect::new(bx, by, fw, bar_h),
                    bar_h / 2,
                    if xp.sending { theme.accent } else { theme.ok },
                );
            }
        }

        // ── 입력창(멀티라인 · 상하 여백 동일 — 세로 중앙 정렬) ──
        let input = self.input_bar();
        ctx.fill_rect(input, theme.field_bg);
        ctx.fill_rect(
            Rect::new(input.x, input.y, input.w, self.s(1)),
            theme.border,
        );
        ctx.select_font(FontSlot::Message, false);
        let ith = ctx.text_height();
        let iline_h = self.s(INPUT_LINE_H);
        let pad_v_in = self.s(INPUT_PAD_V);
        // 가로 스크롤 반영 — 줄바꿈 없는 긴 문장이 캐럿을 따라 흐른다.
        let tx = input.x + self.s(10) - self.input_hscroll;
        let text = self.input.text();
        let all_lines: Vec<&str> = text.split('\n').collect();
        let count = all_lines.len().max(1);
        let top_line = self.input_scroll.min(count.saturating_sub(1));

        if text.is_empty() && self.preedit.is_empty() {
            let ly = input.y + pad_v_in;
            ctx.text(
                tx,
                ly + (iline_h - ith) / 2,
                input,
                "메시지 입력… (Enter 전송 · Shift+Enter 줄바꿈 · Esc 목록)",
                theme.text_dim,
            );
            // 빈 입력창에도 **Beam 커서** — 높이는 글자 실측(캐럿 과대 표시 수정 08-10).
            ctx.fill_rect(
                Rect::new(tx, ly + (iline_h - ith) / 2, self.s(2).max(1), ith),
                theme.accent,
            );
            // 지오메트리 캐시(빈 텍스트).
            *self.input_geom.borrow_mut() = InputGeom {
                tx,
                top: input.y + pad_v_in,
                line_h: iline_h,
                scroll: 0,
                lines: vec![vec![0]],
            };
        } else {
            // 줄별 문자 경계 x 캐시(전체 줄 — 히트테스트·캐럿·선택 공용).
            let mut geom_lines: Vec<Vec<i32>> = Vec::with_capacity(count);
            for l in &all_lines {
                let mut xs = Vec::with_capacity(l.chars().count() + 1);
                xs.push(0);
                let mut acc = String::new();
                for c in l.chars() {
                    acc.push(c);
                    xs.push(ctx.text_width(&acc));
                }
                geom_lines.push(xs);
            }
            let sel = self.input.selection();
            let (cline, ccol) = self.caret_line_col();
            // 줄 시작 전역 인덱스 누적.
            let mut line_start = Vec::with_capacity(count);
            let mut acc_idx = 0usize;
            for l in &all_lines {
                line_start.push(acc_idx);
                acc_idx += l.chars().count() + 1;
            }
            let visible = count.min(INPUT_MAX_LINES);
            for vi in 0..visible {
                let li = top_line + vi;
                if li >= count {
                    break;
                }
                let ly = input.y + pad_v_in + vi as i32 * iline_h;
                let ty = ly + (iline_h - ith) / 2;
                let xs = &geom_lines[li];
                // 선택 하이라이트(줄 구간 겹침).
                if let Some((a, b)) = sel {
                    let ls = line_start[li];
                    let le = ls + all_lines[li].chars().count();
                    let sa = a.max(ls);
                    let sb = b.min(le + 1); // 개행까지 선택되면 줄 끝 +약간
                    if sa < sb {
                        // 가로 스크롤로 밀린 만큼 입력창 밖으로 나갈 수 있다 — 잘라 그린다
                        // (fill_rect는 표면 기준으로만 자르므로 여기서 창 기준으로 한 번 더).
                        let x0 = (tx + xs[(sa - ls).min(xs.len() - 1)]).max(input.x);
                        let x1 = if sb > le {
                            tx + xs[xs.len() - 1] + self.s(4)
                        } else {
                            tx + xs[sb - ls]
                        }
                        .min(input.right());
                        if x1 > x0 {
                            ctx.fill_rect(
                                Rect::new(x0, ly + self.s(1), x1 - x0, iline_h - self.s(2)),
                                theme.sel_bg,
                            );
                        }
                    }
                }
                ctx.text(tx, ty, input, all_lines[li], theme.text);
                // 캐럿(글자 실측 높이 — 세로 중앙).
                if li == cline && (top_line..top_line + visible).contains(&cline) {
                    let cx = tx + xs[ccol.min(xs.len() - 1)];
                    if self.preedit.is_empty() {
                        // 창 안에 있을 때만 — 밖이면 그리지 않는다(가로 스크롤 경계).
                        if cx >= input.x && cx < input.right() {
                            ctx.fill_rect(Rect::new(cx, ty, self.s(2).max(1), ith), theme.accent);
                        }
                    } else {
                        // IME 조합 중 — 캐럿 위치에 프리에딧을 accent 색 + 밑줄로(확정 전).
                        ctx.text(cx, ty, input, &self.preedit, theme.accent);
                        let pw = ctx.text_width(&self.preedit);
                        ctx.fill_rect(
                            Rect::new(cx, ty + ith, pw.max(1), self.s(2).max(1)),
                            theme.accent,
                        );
                    }
                }
            }
            // 콘텐츠 폭은 캐시로 넘기기 **전에** 잰다(넘기면 소유가 옮겨간다).
            let content_w = geom_lines
                .iter()
                .filter_map(|xs| xs.last().copied())
                .max()
                .unwrap_or(0)
                + self.s(20);
            *self.input_geom.borrow_mut() = InputGeom {
                tx,
                top: input.y + pad_v_in,
                line_h: iline_h,
                scroll: top_line,
                lines: geom_lines,
            };
            // 입력창 오버레이 스크롤바 — 세로(4줄 초과)와 **가로**(줄이 창보다 길 때).
            let needs_v = count > INPUT_MAX_LINES;
            let needs_h = content_w > input.w;
            if needs_v || needs_h {
                self.input_bars.paint(
                    ctx,
                    theme,
                    input,
                    content_w.max(input.w),
                    count as i32 * iline_h + pad_v_in * 2,
                    self.input_hscroll,
                    top_line as i32 * iline_h,
                    self.scale,
                );
            }
        }
        // ── 컨텍스트 메뉴는 **맨 마지막** — 팝업은 무엇보다 위에 떠야 한다 ──
        self.ctx_menu.paint(ctx, theme);
    }
}

impl ChatViewWidget {
    /// 팝업(우클릭 메뉴)만 다시 그린다 — 호스트가 상태 바 등 **자기 크롬을 그린 뒤**
    /// 호출해 z-순서를 복구한다(08-13 실기: 메뉴가 하단 정보 텍스트에 덮였다).
    pub fn paint_popup(&self, ctx: &mut dyn DrawCtx, theme: &Theme) {
        if self.ctx_menu.is_open() {
            self.ctx_menu.paint(ctx, theme);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(i: u64) -> (u64, WallTime) {
        // 분 단위로 증가하는 가짜 시각(2026-08-10 12:00 + i분).
        (
            i * 60_000,
            WallTime {
                y: 2026,
                mo: 8,
                d: 10,
                h: 12,
                m: (i % 60) as u32,
            },
        )
    }
    fn tline(mine: bool, s: &str, i: u64) -> ChatLine {
        let (ms, w) = at(i);
        ChatLine::text(mine, nbeep_core::sanitize_message(s), ms, w)
    }

    fn widget() -> (ChatViewWidget, Invalidations) {
        let mut w = ChatViewWidget::new("김철수".into());
        let mut inv = Invalidations::default();
        w.set_bounds(Rect::new(0, 0, 400, 300), &mut inv);
        (w, inv)
    }
    /// 페인트를 한 번 태워 문자 경계 실측 캐시를 채운다(가로 스크롤 계산의 전제).
    fn paint_once(w: &ChatViewWidget) {
        let mut probe = crate::controls::ProbeCtx;
        let theme = crate::theme::Theme::dark();
        w.paint(&mut probe, &theme);
    }
    fn rclick(w: &mut ChatViewWidget, x: i32, y: i32, inv: &mut Invalidations) {
        w.on_event(&InputEvent::RightDown { x, y }, inv);
    }
    fn lclick(w: &mut ChatViewWidget, x: i32, y: i32, inv: &mut Invalidations) {
        w.on_event(
            &InputEvent::MouseDown {
                x,
                y,
                shift: false,
                primary: true,
            },
            inv,
        );
    }

    #[test]
    fn single_long_line_can_be_scrolled_horizontally_by_wheel_and_drag() {
        // ★ 회귀(08-11): 입력창 스크롤바가 "세로 4줄 초과"일 때만 이벤트를 받아,
        //   줄바꿈 없는 긴 한 줄(=세로 1줄)에서는 막대가 **보이는데 잡히지 않았다**.
        let (mut w, mut inv) = widget();
        for _ in 0..200 {
            ch(&mut w, 'x', &mut inv);
        }
        paint_once(&w); // 문자 경계 실측 캐시(가로 폭의 근거)
        assert_eq!(w.input_line_count(), 1, "줄바꿈이 없으니 한 줄");
        assert!(
            w.input_content_w() > w.input_view_w(),
            "가로로 넘쳐야 의미 있는 검증"
        );

        // ① 가로 휠(트랙패드 두 손가락 좌우) — 오프셋이 움직이고 막대가 떠야 한다.
        w.input_hscroll = 0;
        let input = w.input_bar();
        w.on_event(
            &InputEvent::MouseMove {
                x: input.x + 10,
                y: input.y + 4,
            },
            &mut inv,
        );
        w.on_event(&InputEvent::HWheel { delta: 300 }, &mut inv);
        assert!(w.input_hscroll > 0, "가로 휠로 스크롤되어야 한다");
        w.input_bars.tick(0);
        assert!(w.input_bars.is_visible(), "스크롤하면 막대가 뜬다");

        // ② 가로 썸 클릭 → 드래그로 오프셋이 따라와야 한다.
        w.input_hscroll = 0;
        paint_once(&w);
        let thumb = w.input_h_thumb_for_test().expect("가로 썸이 있어야 한다");
        w.on_event(
            &InputEvent::MouseDown {
                x: thumb.x + thumb.w / 2,
                y: thumb.y + thumb.h / 2,
                shift: false,
                primary: true,
            },
            &mut inv,
        );
        w.on_event(
            &InputEvent::MouseMove {
                x: thumb.x + thumb.w / 2 + 60,
                y: thumb.y + thumb.h / 2,
            },
            &mut inv,
        );
        assert!(w.input_hscroll > 0, "썸 드래그로 가로 스크롤되어야 한다");
    }

    #[test]
    fn bubble_right_click_shows_a_menu_instead_of_copying_immediately() {
        // 예전엔 우클릭 = 즉시 클립보드 덮어쓰기였다(08-10 지적). 이제 메뉴가 먼저 뜬다.
        let (mut w, mut inv) = widget();
        w.push_line(tline(false, "안녕하세요", 0), &mut inv);
        paint_once(&w); // 풍선 히트 rect 캐시를 채운다
        let (r, _) = w.hit_rects.borrow()[0];
        rclick(&mut w, r.x + 2, r.y + 2, &mut inv);
        assert!(w.ctx_menu.is_open(), "메뉴가 떠야 한다");
        assert_eq!(w.take_copy_text(), None, "아직 복사되면 안 된다");
        // 첫 항목("메시지 복사")을 고르면 그때 복사된다.
        let row = w.ctx_menu.row_rect_of(0).unwrap();
        lclick(&mut w, row.x + 4, row.y + 2, &mut inv);
        assert!(!w.ctx_menu.is_open(), "고르면 닫힌다");
        assert_eq!(w.take_copy_text().as_deref(), Some("안녕하세요"));
    }

    #[test]
    fn input_right_click_offers_edit_actions_gated_by_state() {
        let (mut w, mut inv) = widget();
        w.set_clipboard_has_text(false);
        let input = w.input_bar();
        rclick(&mut w, input.x + 20, input.y + 4, &mut inv);
        assert!(w.ctx_menu.is_open());
        // 선택도 없고 클립보드도 비었으면 셋 다 비활성 — 눌러도 안 되는 걸 활성으로 보이면 거짓말이다.
        let enabled: Vec<bool> = w
            .ctx_menu
            .items_for_test()
            .iter()
            .filter_map(|it| match it {
                crate::controls::CtxItem::Item { id, enabled, .. } => Some((id.clone(), *enabled)),
                crate::controls::CtxItem::Separator => None,
            })
            .map(|(_, e)| e)
            .collect();
        assert_eq!(enabled, vec![false, false, false, false], "빈 상태");
    }

    #[test]
    fn input_right_click_enables_paste_when_clipboard_has_text() {
        let (mut w, mut inv) = widget();
        ch(&mut w, '가', &mut inv);
        w.set_clipboard_has_text(true);
        let input = w.input_bar();
        rclick(&mut w, input.x + 20, input.y + 4, &mut inv);
        let items = w.ctx_menu.items_for_test();
        let by = |want: &str| {
            items.iter().find_map(|it| match it {
                crate::controls::CtxItem::Item { id, enabled, .. } if id == want => Some(*enabled),
                _ => None,
            })
        };
        assert_eq!(
            by("paste"),
            Some(true),
            "클립보드에 글이 있으면 붙여넣기 가능"
        );
        assert_eq!(by("copy"), Some(false), "선택이 없으면 복사 불가");
        assert_eq!(
            by("select_all"),
            Some(true),
            "입력된 글이 있으면 전체 선택 가능"
        );
        // 붙여넣기를 고르면 **요청만** 남는다(클립보드 읽기는 호스트 몫).
        let row = w.ctx_menu.row_rect_of(2).unwrap();
        lclick(&mut w, row.x + 4, row.y + 2, &mut inv);
        assert!(w.take_paste_request(), "붙여넣기 요청이 올라온다");
        assert!(!w.take_paste_request(), "요청은 1회성");
    }

    #[test]
    fn right_click_on_empty_area_opens_nothing() {
        // 무엇에 대한 메뉴인지 모를 자리에는 띄우지 않는다.
        let (mut w, mut inv) = widget();
        rclick(&mut w, 200, 120, &mut inv);
        assert!(!w.ctx_menu.is_open());
    }

    #[test]
    fn open_menu_swallows_the_next_click_and_does_not_move_the_caret() {
        let (mut w, mut inv) = widget();
        w.set_clipboard_has_text(true);
        let input = w.input_bar();
        rclick(&mut w, input.x + 20, input.y + 4, &mut inv);
        assert!(w.ctx_menu.is_open());
        // 메뉴 바깥 클릭 = 닫기만. 뒤 콘텐츠로 새면 캐럿이 엉뚱하게 옮겨간다.
        lclick(&mut w, 5, 5, &mut inv);
        assert!(!w.ctx_menu.is_open());
        assert!(!w.dragging, "닫기 클릭이 드래그 선택을 시작하면 안 된다");
    }

    fn ch(w: &mut ChatViewWidget, c: char, inv: &mut Invalidations) {
        w.on_event(&InputEvent::Char { c, now_ms: 0 }, inv);
    }
    fn enter(w: &mut ChatViewWidget, inv: &mut Invalidations) {
        key_s(w, Key::Enter, false, inv);
    }
    fn key(w: &mut ChatViewWidget, k: Key, inv: &mut Invalidations) {
        key_s(w, k, false, inv);
    }
    fn key_s(w: &mut ChatViewWidget, k: Key, shift: bool, inv: &mut Invalidations) {
        w.on_event(
            &InputEvent::Key {
                key: k,
                shift,
                primary: false,
            },
            inv,
        );
    }
    fn wheel_up(w: &mut ChatViewWidget, inv: &mut Invalidations) {
        w.on_event(&InputEvent::Wheel { delta: 120 }, inv); // 1노치 위(과거)
    }

    #[test]
    fn scroll_reveals_history_and_new_msg_snaps_to_latest() {
        let (mut w, mut inv) = widget();
        for i in 0..30 {
            w.push_line(tline(true, &format!("m{i}"), i), &mut inv);
        }
        assert_eq!(w.scroll(), 0, "push 시 최신 스냅");
        for _ in 0..5 {
            wheel_up(&mut w, &mut inv);
        }
        assert!(w.scroll() > 0, "위로 스크롤됨: {}", w.scroll());
        let before = w.scroll();
        key(&mut w, Key::PageUp, &mut inv);
        assert!(w.scroll() > before);
        w.push_line(tline(false, "새 메시지", 31), &mut inv);
        assert_eq!(w.scroll(), 0, "새 메시지 = 하단 스냅");
    }

    #[test]
    fn scroll_clamped_to_history() {
        let (mut w, mut inv) = widget();
        for i in 0..3 {
            w.push_line(tline(true, &format!("m{i}"), i), &mut inv);
        }
        for _ in 0..50 {
            wheel_up(&mut w, &mut inv);
        }
        assert_eq!(w.scroll(), 0, "짧은 대화는 스크롤 상한 0");
    }

    #[test]
    fn preedit_shown_then_commit_inserts() {
        let (mut w, mut inv) = widget();
        w.set_preedit("한".into(), &mut inv);
        assert_eq!(w.preedit(), "한");
        assert_eq!(w.input(), "", "조합 중엔 확정 텍스트 없음");
        ch(&mut w, '한', &mut inv);
        assert_eq!(w.preedit(), "", "확정 시 조합 종료");
        assert_eq!(w.input(), "한");
    }

    #[test]
    fn empty_preedit_ends_composition() {
        let (mut w, mut inv) = widget();
        w.set_preedit("ㅎ".into(), &mut inv);
        w.set_preedit(String::new(), &mut inv);
        assert_eq!(w.preedit(), "");
    }

    #[test]
    fn caret_moves_and_inserts_mid_string() {
        let (mut w, mut inv) = widget();
        for c in "helo".chars() {
            ch(&mut w, c, &mut inv);
        }
        key(&mut w, Key::Left, &mut inv);
        ch(&mut w, 'l', &mut inv);
        assert_eq!(w.input(), "hello");
        assert_eq!(w.input_caret(), 4);
    }

    #[test]
    fn typing_and_enter_produces_sanitized_outgoing() {
        let (mut w, mut inv) = widget();
        for c in "안녕 bob".chars() {
            ch(&mut w, c, &mut inv);
        }
        assert_eq!(w.input(), "안녕 bob");
        enter(&mut w, &mut inv);
        let out = w.take_outgoing().expect("발신 확정");
        assert_eq!(out.as_str(), "안녕 bob");
        assert_eq!(w.input(), "", "전송 후 입력창 비움");
        assert!(w.take_outgoing().is_none(), "1회성");
    }

    #[test]
    fn outgoing_is_sanitized_rlo_stripped() {
        let (mut w, mut inv) = widget();
        for c in "a\u{202E}b".chars() {
            ch(&mut w, c, &mut inv);
        }
        enter(&mut w, &mut inv);
        assert_eq!(w.take_outgoing().unwrap().as_str(), "ab");
    }

    #[test]
    fn empty_or_whitespace_input_does_not_send() {
        let (mut w, mut inv) = widget();
        enter(&mut w, &mut inv);
        assert!(w.take_outgoing().is_none());
        ch(&mut w, ' ', &mut inv);
        enter(&mut w, &mut inv);
        assert!(w.take_outgoing().is_none(), "공백만 = 미전송");
    }

    #[test]
    fn backspace_edits_and_escape_requests_back() {
        let (mut w, mut inv) = widget();
        ch(&mut w, 'h', &mut inv);
        ch(&mut w, 'i', &mut inv);
        ch(&mut w, '\u{8}', &mut inv);
        assert_eq!(w.input(), "h");
        assert!(!w.take_back());
        key(&mut w, Key::Escape, &mut inv);
        assert!(w.take_back(), "Esc = 복귀 요청");
        assert!(!w.take_back(), "1회성");
    }

    // ── 멀티라인 입력(Shift+Enter · 사용자 확정 08-10) ──

    #[test]
    fn shift_enter_inserts_newline_and_enter_sends_multiline() {
        let (mut w, mut inv) = widget();
        for c in "안녕".chars() {
            ch(&mut w, c, &mut inv);
        }
        key_s(&mut w, Key::Enter, true, &mut inv); // Shift+Enter = 줄바꿈
        assert!(w.take_outgoing().is_none(), "줄바꿈은 전송이 아니다");
        for c in "잘 지내?".chars() {
            ch(&mut w, c, &mut inv);
        }
        assert_eq!(w.input(), "안녕\n잘 지내?");
        enter(&mut w, &mut inv);
        let out = w.take_outgoing().expect("멀티라인 전송");
        assert_eq!(out.as_str(), "안녕\n잘 지내?", "개행이 보존된다");
        assert_eq!(w.input(), "");
    }

    #[test]
    fn input_bar_grows_to_four_lines_then_scrolls() {
        let (mut w, mut inv) = widget();
        let h1 = w.input_bar().h;
        for i in 0..6 {
            for c in format!("줄{i}").chars() {
                ch(&mut w, c, &mut inv);
            }
            if i < 5 {
                key_s(&mut w, Key::Enter, true, &mut inv);
            }
        }
        assert_eq!(w.input_line_count(), 6);
        let h6 = w.input_bar().h;
        assert!(h6 > h1, "여러 줄 = 입력창이 자란다");
        // 표시 상한 4줄 — 높이는 4줄분에서 멈춘다.
        let expect = w.s(INPUT_PAD_V) * 2 + w.s(INPUT_LINE_H) * 4;
        assert_eq!(h6, expect, "4줄 초과는 더 안 자란다(스크롤)");
        // 캐럿(마지막 줄)이 보이도록 스크롤됐다.
        assert_eq!(w.input_scroll, 2, "6줄 중 캐럿 줄(5)이 보이려면 top=2");
    }

    #[test]
    fn line_home_end_work_per_line() {
        let (mut w, mut inv) = widget();
        for c in "ab".chars() {
            ch(&mut w, c, &mut inv);
        }
        key_s(&mut w, Key::Enter, true, &mut inv);
        for c in "cd".chars() {
            ch(&mut w, c, &mut inv);
        }
        // 캐럿 = 끝(5). Home = 둘째 줄 시작(3).
        key(&mut w, Key::Home, &mut inv);
        assert_eq!(w.input_caret(), 3, "라인 Home");
        key(&mut w, Key::End, &mut inv);
        assert_eq!(w.input_caret(), 5, "라인 End");
        // Up = 윗줄 같은 열.
        key(&mut w, Key::Up, &mut inv);
        assert_eq!(w.input_caret(), 2, "윗줄 열 유지(끝으로 클램프)");
    }

    #[test]
    fn paste_normalizes_crlf_and_strips_controls() {
        let (mut w, mut inv) = widget();
        w.paste("a\r\nb\u{7}c", &mut inv);
        assert_eq!(w.input(), "a\nbc", "CRLF→LF · 제어문자 제거");
        assert_eq!(w.input_line_count(), 2);
    }

    #[test]
    fn copy_cut_selection_roundtrip() {
        let (mut w, mut inv) = widget();
        for c in "hello".chars() {
            ch(&mut w, c, &mut inv);
        }
        w.on_event(&InputEvent::SelectAll, &mut inv);
        assert_eq!(w.copy_selection().as_deref(), Some("hello"));
        assert_eq!(w.cut_selection(&mut inv).as_deref(), Some("hello"));
        assert_eq!(w.input(), "", "잘라내기 후 비움");
        assert!(w.copy_selection().is_none(), "선택 없음 = None");
    }

    // ── 파일 전송 스레드 항목(08-10 — 송수신 이력·진행률·완료 잔존) ──

    fn xline(mine: bool, name: &str, size: u64, i: u64) -> ChatLine {
        let (ms, w) = at(i);
        ChatLine::xfer(mine, nbeep_core::sanitize_message(name), size, ms, w)
    }
    fn xfer_state(line: &ChatLine) -> &XferLineState {
        match &line.body {
            ChatBody::Xfer(x) => &x.state,
            ChatBody::Text(_) => panic!("전송 항목이어야 한다"),
        }
    }

    #[test]
    fn xfer_line_lifecycle_waiting_active_done() {
        let (mut w, mut inv) = widget();
        w.push_line(xline(true, "a.bin", 100, 0), &mut inv);
        assert_eq!(*xfer_state(&w.lines[0]), XferLineState::Waiting);
        assert!(w.update_xfer_line(true, XferLineState::Active { done: 40 }, &mut inv));
        assert_eq!(*xfer_state(&w.lines[0]), XferLineState::Active { done: 40 });
        assert!(w.update_xfer_line(
            true,
            XferLineState::Done {
                note: String::new()
            },
            &mut inv
        ));
        assert!(
            !w.update_xfer_line(true, XferLineState::Active { done: 99 }, &mut inv),
            "종결 항목은 갱신 불가"
        );
    }

    #[test]
    fn xfer_ack_closes_awaiting_and_progress_skips_it() {
        // M4-9 — 발신 2건: A는 확인 대기(AwaitingAck), B는 아직 진행 중.
        // B의 진행 갱신이 A의 확인 대기를 덮으면 안 되고(update_xfer_in은 AwaitingAck 제외),
        // ack는 A만 닫아야 한다(update_xfer_ack는 AwaitingAck만).
        let (mut w, mut inv) = widget();
        w.push_line(xline(true, "a.bin", 100, 0), &mut inv); // A
        w.push_line(xline(true, "b.bin", 200, 1), &mut inv); // B
                                                             // A 전송 끝 → 확인 대기(가장 오래된 진행 항목이 A).
        assert!(w.update_xfer_line(true, XferLineState::AwaitingAck, &mut inv));
        assert_eq!(*xfer_state(&w.lines[0]), XferLineState::AwaitingAck);
        // B 진행 → update_xfer_in은 AwaitingAck을 건너뛰고 B(다음 진행 항목)를 잡는다.
        assert!(w.update_xfer_line(true, XferLineState::Active { done: 50 }, &mut inv));
        assert_eq!(
            *xfer_state(&w.lines[0]),
            XferLineState::AwaitingAck,
            "A 확인 대기 유지"
        );
        assert_eq!(*xfer_state(&w.lines[1]), XferLineState::Active { done: 50 });
        // A의 ack 도착 → 확인 대기(A)만 완료로 닫는다.
        assert!(w.ack_xfer_line(
            true,
            XferLineState::Done {
                note: String::new()
            },
            &mut inv
        ));
        assert_eq!(
            *xfer_state(&w.lines[0]),
            XferLineState::Done {
                note: String::new()
            }
        );
        assert_eq!(
            *xfer_state(&w.lines[1]),
            XferLineState::Active { done: 50 },
            "B는 그대로"
        );
        // 더 닫을 확인 대기 없음.
        assert!(!w.ack_xfer_line(
            true,
            XferLineState::Done {
                note: String::new()
            },
            &mut inv
        ));
    }

    #[test]
    fn xfer_update_matches_direction_only() {
        let (mut w, mut inv) = widget();
        w.push_line(xline(false, "recv.bin", 10, 0), &mut inv);
        assert!(!w.update_xfer_line(true, XferLineState::Active { done: 1 }, &mut inv));
        assert_eq!(*xfer_state(&w.lines[0]), XferLineState::Waiting);
    }

    #[test]
    fn xfer_update_fifo_two_open_offers() {
        let (mut w, mut inv) = widget();
        w.push_line(xline(false, "first.bin", 10, 0), &mut inv);
        w.push_line(xline(false, "second.bin", 20, 1), &mut inv);
        assert!(w.update_xfer_line(
            false,
            XferLineState::Failed {
                why: "거절함".into()
            },
            &mut inv
        ));
        assert_eq!(
            *xfer_state(&w.lines[0]),
            XferLineState::Failed {
                why: "거절함".into()
            },
            "앞(오래된) 오퍼가 먼저 갱신"
        );
        assert_eq!(*xfer_state(&w.lines[1]), XferLineState::Waiting);
    }

    #[test]
    fn xfer_update_skips_text_and_terminal_hits_open() {
        let (mut w, mut inv) = widget();
        w.push_line(xline(true, "old.bin", 10, 0), &mut inv);
        assert!(w.update_xfer_line(
            true,
            XferLineState::Failed {
                why: "취소".into()
            },
            &mut inv
        ));
        w.push_line(tline(true, "중간 메시지", 1), &mut inv);
        w.push_line(xline(true, "new.bin", 20, 2), &mut inv);
        assert!(w.update_xfer_line(true, XferLineState::Active { done: 5 }, &mut inv));
        assert_eq!(
            *xfer_state(&w.lines[0]),
            XferLineState::Failed {
                why: "취소".into()
            },
            "이전 종결 기록은 그대로"
        );
        assert_eq!(*xfer_state(&w.lines[2]), XferLineState::Active { done: 5 });
    }

    // ── 시각 표시(08-10 — 풍선 옆 분 단위 · 날짜 알약) ──

    #[test]
    fn time_shown_on_last_of_minute_group_only() {
        let (mut w, mut inv) = widget();
        w.push_line(tline(true, "a", 0), &mut inv); // 12:00 나
        w.push_line(tline(true, "b", 0), &mut inv); // 12:00 나(같은 분)
        w.push_line(tline(false, "c", 0), &mut inv); // 12:00 상대(발신자 바뀜)
        w.push_line(tline(true, "d", 1), &mut inv); // 12:01 나
        assert!(!w.shows_time(0), "같은 분·같은 발신자 묶음의 처음은 생략");
        assert!(w.shows_time(1), "묶음 마지막(다음은 발신자 다름)");
        assert!(w.shows_time(2), "다음은 분이 다르다");
        assert!(w.shows_time(3), "마지막 항목은 항상 표시");
    }

    #[test]
    fn hm_format_follows_24h_setting() {
        let w = WallTime {
            y: 2026,
            mo: 8,
            d: 10,
            h: 19,
            m: 2,
        };
        assert_eq!(fmt_hm(w, true), "19:02");
        assert_eq!(fmt_hm(w, false), "PM 7:02");
        let am = WallTime { h: 0, m: 5, ..w };
        assert_eq!(fmt_hm(am, false), "AM 12:05");
        let noon = WallTime { h: 12, m: 0, ..w };
        assert_eq!(fmt_hm(noon, false), "PM 12:00");
    }

    #[test]
    fn date_pill_format_and_weekday() {
        // 2026-08-10은 월요일.
        let w = WallTime {
            y: 2026,
            mo: 8,
            d: 10,
            h: 0,
            m: 0,
        };
        assert_eq!(fmt_date_pill(w, false), "2026-08-10 (월)");
        assert_eq!(fmt_date_pill(w, true), "8/10 (월)");
    }

    #[test]
    fn group_sender_name_adds_row_height() {
        // 단체 대화 — 수신 항목에 송신자 이름이 붙으면 이름 행만큼 커진다(발신엔 무시).
        let (mut w, mut inv) = widget();
        w.push_line(tline(false, "a", 0), &mut inv);
        let base = w.content_h_estimate();
        let (mut w2, mut inv2) = widget();
        w2.push_line(tline(false, "a", 0).with_from("라이언"), &mut inv2);
        assert_eq!(
            w2.content_h_estimate(),
            base + w2.s(NAME_H),
            "수신 + 이름 = 이름 행 추가"
        );
        let (mut w3, mut inv3) = widget();
        w3.push_line(tline(true, "a", 0).with_from("나"), &mut inv3);
        assert_eq!(w3.content_h_estimate(), base, "발신 풍선엔 이름 행 없음");
    }

    #[test]
    fn content_estimate_adds_pill_on_date_change() {
        let (mut w, mut inv) = widget();
        w.push_line(tline(true, "a", 0), &mut inv);
        let one_day = w.content_h_estimate();
        // 다음 날 메시지 → 알약 1개 추가만큼 커진다.
        let (ms, mut wall) = at(2);
        wall.d = 11;
        w.push_line(
            ChatLine::text(false, nbeep_core::sanitize_message("b"), ms, wall),
            &mut inv,
        );
        let two_days = w.content_h_estimate();
        assert!(
            two_days > one_day + w.s(TH_LINE_H),
            "날짜 변경 = 알약 행 추가: {one_day} → {two_days}"
        );
    }

    /// 줄바꿈 없는 긴 문장 — **가로 스크롤이 캐럿을 따라간다**(사용자 요청 08-10).
    /// 예전에는 텍스트가 오른쪽으로 그냥 흘러 나가 `fill_rect` 역전 슬라이스로 앱이 죽었다.
    #[test]
    fn long_single_line_scrolls_horizontally() {
        let (mut w, mut inv) = widget();
        w.set_bounds(Rect::new(0, 0, 300, 200), &mut inv);
        // 창보다 훨씬 긴 한 줄(개행 없음).
        for _ in 0..200 {
            ch(&mut w, 'x', &mut inv);
        }
        // paint가 실측 폭을 남기고, 그걸로 가로 스크롤이 잡힌다.
        paint_once(&w);
        w.ensure_caret_visible_h();
        assert!(
            w.input_hscroll > 0,
            "긴 줄인데 가로 스크롤이 0이다(캐럿이 화면 밖)"
        );
        // 캐럿이 보이는 범위 안에 들어와야 한다.
        let (line, col) = w.caret_line_col();
        let cx = w.input_geom.borrow().lines[line][col];
        let visible = cx - w.input_hscroll;
        assert!(
            visible >= 0 && visible <= w.input_view_w(),
            "캐럿이 보이지 않는다: cx={cx} hscroll={} view={}",
            w.input_hscroll,
            w.input_view_w()
        );
    }

    /// 짧은 줄로 되돌아오면 가로 스크롤도 0으로 풀린다.
    #[test]
    fn hscroll_resets_when_text_shrinks() {
        let (mut w, mut inv) = widget();
        w.set_bounds(Rect::new(0, 0, 300, 200), &mut inv);
        for _ in 0..200 {
            ch(&mut w, 'x', &mut inv);
        }
        paint_once(&w);
        w.ensure_caret_visible_h();
        assert!(w.input_hscroll > 0);
        // 전부 지운다.
        for _ in 0..200 {
            ch(&mut w, '\u{8}', &mut inv);
        }
        paint_once(&w);
        w.ensure_caret_visible_h();
        assert_eq!(w.input_hscroll, 0, "짧아졌는데 스크롤이 남아 있다");
    }

    /// 창 밖으로 끌면 **좌우로 자동 스크롤**되어 가려진 글자까지 선택된다(08-10 지적).
    #[test]
    fn drag_past_right_edge_scrolls_horizontally() {
        let (mut w, mut inv) = widget();
        w.set_bounds(Rect::new(0, 0, 300, 200), &mut inv);
        for _ in 0..200 {
            ch(&mut w, 'x', &mut inv);
        }
        paint_once(&w);
        // 왼쪽 끝으로 스크롤을 되돌린 뒤 드래그를 시작한다.
        w.input_hscroll = 0;
        let input = w.input_bar();
        w.on_event(
            &InputEvent::MouseDown {
                x: input.x + 20,
                y: input.y + 10,
                shift: false,
                primary: false,
            },
            &mut inv,
        );
        // 오른쪽 경계 밖으로 끈다 — 여러 번 움직이면 계속 밀려야 한다.
        for _ in 0..3 {
            w.on_event(
                &InputEvent::MouseMove {
                    x: input.right() + 30,
                    y: input.y + 10,
                },
                &mut inv,
            );
        }
        assert!(
            w.input_hscroll > 0,
            "오른쪽 밖으로 끌었는데 가로 스크롤이 그대로다"
        );
        assert!(
            w.input.selection().is_some(),
            "자동 스크롤 중에도 선택이 이어져야 한다"
        );
    }
}
