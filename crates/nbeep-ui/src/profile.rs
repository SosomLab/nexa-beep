//! **프로필 변경 화면**(M3-17 · 사용자 요청 08-11) — 프로필 이미지·표시 이름·이메일·
//! 전화번호 편집 + 항목별 공개 토글(DR-22 옵트인 · ADR-0008).
//!
//! 위젯은 저장·전송을 모른다 — 바뀐 (키, 값)만 [`ProfileWidget::take_changes`]로 내놓고,
//! 영속(SettingsState/nexa-conf)·재공지·프로필 교환은 호스트 몫이다. 이미지 선택도
//! [`ProfileWidget::take_pick_image`] 요청만 올리고 실제 파일 선택은 호스트 피커가 한다.

use crate::controls::{Button, Carousel, ColorPicker, Control as _, LabelSide, Switch, TextBox};
use crate::draw::{DrawCtx, FontSlot};
use crate::event::{InputEvent, Key};
use crate::geom::{Point, Rect};
use crate::theme::Theme;
use crate::widget::{Invalidations, Widget};
use nbeep_core::avatar::AvatarChoice;
use nbeep_core::i18n::{t, Msg};

/// 최근 프로필 이미지 보관 상한(08-14 사용자 확정 — 최대 10).
const RECENT_MAX: usize = 10;

/// 프로필 화면 초기값(호스트가 설정에서 읽어 넘긴다).
#[derive(Debug, Default, Clone)]
pub struct ProfileValues {
    /// 표시 이름("auto" = 정제된 호스트명 자동).
    pub display_name: String,
    /// 이메일(빈 값 = 미설정).
    pub email: String,
    /// 전화번호(빈 값 = 미설정).
    pub phone: String,
    /// 소개글(08-17 · 여러 줄 가능 — 빈 값 = 미설정).
    pub bio: String,
    /// 프로필 이미지 경로(빈 값 = 없음).
    pub image_path: String,
    /// 공개 여부 — 기본정보(사진·이름)/이메일/전화.
    pub share_basic: bool,
    /// 이메일 공개.
    pub share_email: bool,
    /// 전화번호 공개.
    pub share_phone: bool,
    /// 해석된 표시 이름(이니셜 아바타 원료 — "auto"의 실제 값은 호스트만 안다).
    pub resolved_name: String,
    /// 아바타 색 시드(내 키 지문).
    pub seed: Vec<u8>,
    /// 내 키 지문 짧은 표기(08-17 — 상대 카드의 "키 지문"과 같은 값 · 대조 기준).
    pub fingerprint: String,
    /// 내 사진(M4-5 — 호스트가 imgdec로 디코드·원형 마스크해 넘긴다). 없으면 이니셜.
    pub avatar: Option<std::rc::Rc<crate::theme::IconImage>>,
    /// 아바타 선택 원문(`profile.avatar` — [`AvatarChoice`] 직렬). 사진(`image_path`)이
    /// 있으면 사진이 우선한다(선택 UI에서 스와치를 고르면 사진을 비운다 — 상호 배타).
    pub avatar_choice: String,
    /// 아바타 보더 색 `"#RRGGBB"`(`profile.avatar_border` — 부팅 시 시드 기본값 저장이
    /// 보장돼 비어 오지 않는다 · 08-14).
    pub avatar_border: String,
    /// 최근 등록한 프로필 이미지 경로(`profile.image_recent` — 최신 먼저 · 최대 10 ·
    /// 08-14 사용자 확정). 썸네일은 호스트가 격리 디코드해 `set_recent_thumb`로 채운다.
    pub recent: Vec<String>,
    /// 툴팁 표시 대기(ms · `ui.tooltip_ms` — 기본 2000 · 08-14). 200 미만(Default 0
    /// 포함)은 기본값으로 본다(ADR-0011 관용 파싱과 같은 자세).
    pub tooltip_ms: u64,
}

/// 프로필 변경 화면 위젯.
#[derive(Debug)]
pub struct ProfileWidget {
    bounds: Rect,
    scale: f32,
    name: TextBox,
    email: TextBox,
    phone: TextBox,
    /// 소개글(08-17 · 멀티라인 — Enter가 개행). 목록은 줄바꿈을 접어 한 줄로 본다.
    bio: TextBox,
    choose_img: Button,
    sw_basic: Switch,
    sw_email: Switch,
    sw_phone: Switch,
    image_path: String,
    resolved_name: String,
    seed: Vec<u8>,
    fingerprint: String,
    avatar: Option<std::rc::Rc<crate::theme::IconImage>>,
    /// 아바타 선택 원문(스와치 클릭으로 바뀐다 · 사진이 있으면 사진 우선).
    avatar_choice: String,
    /// 아바타 보더 색 선택(08-14 — 큰 프리뷰 3px·소형 2px는 그리는 쪽 몫).
    border: ColorPicker,
    /// 내장 12간지(키, 그림) — 스와치·프리뷰 공용(new에서 1회 해석).
    builtins: Vec<(String, std::rc::Rc<crate::theme::IconImage>)>,
    /// 스와치 캐러셀(08-14 사용자 확정 — 32px 아이템 · 넘칠 때만 좌/우 이동 버튼).
    swatches: Carousel,
    /// 최근 프로필 이미지(경로, 썸네일 — 호스트가 채움 · 최신 먼저 · 최대 10).
    recent: Vec<(String, Option<std::rc::Rc<crate::theme::IconImage>>)>,
    /// 최근 이미지 캐러셀(Profile image 라벨과 Choose… 사이 · 28px · × 삭제 오버레이).
    recent_car: Carousel,
    /// 커서가 올라간 최근 아이템(08-14 — ×는 이 아이템에만 보인다).
    recent_hover: Option<usize>,
    /// 호버 시작 시각(ms · 틱이 채움) — 3초 경과 = 파일명 툴팁.
    recent_hover_since: Option<u64>,
    /// 툴팁 표시 중(틱이 켜고 호버 이동이 끈다).
    recent_tip: bool,
    /// 툴팁 표시 대기(ms — 설정 `ui.tooltip_ms` · 즉시 적용).
    tip_delay_ms: u64,
    changes: Vec<(&'static str, String)>,
    /// 적용 확정분(M3-18) — [`Self::take_changes`]는 이것만 내놓는다: 편집은
    /// `changes`에 **보류**되고, 적용 버튼이 눌릴 때 일괄 이동한다(원자적 저장 —
    /// 사용자 확정 08-13 · 설정 화면의 "즉시 적용"과 의도적으로 다른 화면 규약).
    ready: Vec<(&'static str, String)>,
    /// 적용/취소 버튼(M3-18).
    apply_btn: Button,
    cancel_btn: Button,
    /// 미저장 닫기 2단계 확인(M3-18 ④ — 격리함 confirming 문법): Esc 1차 = 경고
    /// 표시, 2차 = 버리고 닫기. 다른 입력이 오면 해제.
    discard_arm: bool,
    /// 열림 시점의 텍스트 3종 원값(정규화 후 · M3-18 후속 08-16) — 적용 시
    /// **Enter 없이 타이핑만 한 값**도 수확하되, 원값과 같으면 보고하지 않는다
    /// (같은 값 재보고 = 불필요한 이름 재공지·프로필 재전파).
    orig_name: String,
    orig_email: String,
    orig_phone: String,
    orig_bio: String,
    /// 소개글 표시 줄 수(08-18 — 고정 [`BIO_MAX_LINES`]줄 · 초과분은 필드 내부
    /// 스크롤. 창 크기는 지금 고정이라 성장시키지 않는다).
    bio_lines: usize,
    /// 공유 안내문 y 오프셋(relayout이 bio 높이에 맞춰 계산 · paint가 읽는다).
    note_dy: i32,
    /// 마지막 커서 위치(08-18 — 휠은 좌표가 없어 hover 판정에 쓴다: bio 위 휠 =
    /// bio 내부 스크롤).
    last_cursor: Point,
    pick_image: bool,
    closed: bool,
}

/// 소개글 표시 줄 수(08-18 사용자 확정 — **고정 3줄** · 초과분은 필드 내부
/// 스크롤(키보드 이동 + 마우스 휠). 창 크기 조정·스크롤 영역 재설계는 취소).
const BIO_MAX_LINES: usize = 3;
/// 소개글 한 줄 높이(멀티라인 TextBox의 line_h와 같은 값 · 배율 전).
const BIO_LINE_H: i32 = 20;

impl ProfileWidget {
    /// 현재 값으로 화면을 만든다.
    #[must_use]
    pub fn new(v: &ProfileValues) -> Self {
        let mut name = TextBox::new(t(Msg::NameAuto));
        if v.display_name != "auto" {
            name = TextBox::new(t(Msg::NameAuto)).with_text(&v.display_name);
        }
        name.set_focused(true);
        let builtins: Vec<(String, std::rc::Rc<crate::theme::IconImage>)> =
            crate::avatar_assets::builtins()
                .into_iter()
                .map(|b| (b.key, std::rc::Rc::new(b.image)))
                .collect();
        let mut swatches = Carousel::new(32, 4);
        swatches.set_count(2 + builtins.len()); // [이니셜][없음] + 내장
        let recent: Vec<(String, Option<std::rc::Rc<crate::theme::IconImage>>)> = v
            .recent
            .iter()
            .filter(|p| !p.is_empty())
            .take(RECENT_MAX)
            .map(|p| (p.clone(), None))
            .collect();
        let mut recent_car = Carousel::new(28, 4);
        recent_car.set_count(recent.len());
        Self {
            bounds: Rect::default(),
            scale: 1.0,
            name,
            email: TextBox::new("name@example.com").with_text(&v.email),
            phone: TextBox::new("010-0000-0000").with_text(&v.phone),
            bio: TextBox::new(t(Msg::BioPlaceholder))
                .with_multiline()
                .with_text(&v.bio),
            choose_img: Button::new(t(Msg::ActChoose)),
            sw_basic: Switch::new(t(Msg::ShareBasic), v.share_basic)
                .with_label_side(LabelSide::Left),
            sw_email: Switch::new(t(Msg::ShareEmail), v.share_email)
                .with_label_side(LabelSide::Left),
            sw_phone: Switch::new(t(Msg::SharePhone), v.share_phone)
                .with_label_side(LabelSide::Left),
            image_path: v.image_path.clone(),
            resolved_name: v.resolved_name.clone(),
            seed: v.seed.clone(),
            fingerprint: v.fingerprint.clone(),
            avatar: v.avatar.clone(),
            avatar_choice: v.avatar_choice.clone(),
            border: ColorPicker::new(&v.avatar_border),
            builtins,
            swatches,
            recent,
            recent_car,
            recent_hover: None,
            recent_hover_since: None,
            recent_tip: false,
            tip_delay_ms: if v.tooltip_ms < 200 {
                2000
            } else {
                v.tooltip_ms
            },
            changes: Vec::new(),
            ready: Vec::new(),
            apply_btn: Button::new(t(Msg::ActApply)),
            cancel_btn: Button::new(t(Msg::OfferCancel)),
            discard_arm: false,
            orig_name: v.display_name.clone(),
            orig_email: v.email.clone(),
            orig_phone: v.phone.clone(),
            orig_bio: v.bio.clone(),
            bio_lines: BIO_MAX_LINES, // 고정 3줄(초과는 내부 스크롤 · 창 성장 안 함)
            note_dy: 0,
            last_cursor: Point { x: 0, y: 0 },
            pick_image: false,
            closed: false,
        }
    }

    /// 텍스트 3종의 현재 값을 수확한다(M3-18 후속 — 실기: Enter를 안 누르고
    /// 적용을 누르면 타이핑한 값이 사라졌다). Enter 확정 경로와 **같은 정규화**
    /// (trim · 빈 이름 = "auto")를 거치고, 마지막 보고값(없으면 원값)과 다를
    /// 때만 changes에 넣는다.
    fn harvest_texts(&mut self) {
        let items = [
            (
                self.name.text(),
                "profile.display_name",
                true,
                self.orig_name.clone(),
            ),
            (
                self.email.text(),
                "profile.email",
                false,
                self.orig_email.clone(),
            ),
            (
                self.phone.text(),
                "profile.phone",
                false,
                self.orig_phone.clone(),
            ),
            (self.bio.text(), "profile.bio", false, self.orig_bio.clone()),
        ];
        for (raw, key, auto_empty, orig) in items {
            let v = raw.trim().to_string();
            let out = if v.is_empty() && auto_empty {
                "auto".to_string()
            } else {
                v
            };
            let last = self
                .changes
                .iter()
                .rev()
                .find(|(k, _)| *k == key)
                .map_or(orig, |(_, v)| v.clone());
            if out != last {
                self.changes.push((key, out));
            }
        }
    }

    /// 미확정 타이핑까지 포함한 편집 여부(M3-18 — Esc 2단계 판정도 이걸 본다).
    fn texts_dirty(&self) -> bool {
        let norm = |raw: String, auto_empty: bool| {
            let v = raw.trim().to_string();
            if v.is_empty() && auto_empty {
                "auto".to_string()
            } else {
                v
            }
        };
        norm(self.name.text(), true) != self.orig_name
            || norm(self.email.text(), false) != self.orig_email
            || norm(self.phone.text(), false) != self.orig_phone
            || norm(self.bio.text(), false) != self.orig_bio
    }

    /// 저장 안 된 편집이 있는가(M3-18 — 호스트의 닫기 판단·상태 표시용).
    /// Enter 안 누른 타이핑도 포함한다(수확은 적용 시점 · 08-16 실기).
    #[must_use]
    pub fn dirty(&self) -> bool {
        !self.changes.is_empty() || self.texts_dirty()
    }

    /// 스와치 목록 — [이니셜, 없음] + 내장 12간지(설정 저장값과 1:1).
    fn swatch_values(&self) -> Vec<String> {
        let mut v = vec!["initials".to_string(), "none".to_string()];
        v.extend(self.builtins.iter().map(|(k, _)| format!("b:{k}")));
        v
    }

    /// 스와치 선택 적용(캐러셀 클릭 → 전역 인덱스 · 08-14) — 사진과 상호 배타.
    fn apply_swatch(&mut self, i: usize, inv: &mut Invalidations) {
        let Some(val) = self.swatch_values().get(i).cloned() else {
            return;
        };
        if self.image_path.is_empty() && self.avatar_choice == val {
            return; // 이미 선택된 스와치 재클릭 = 무변경 — push 없음(RL-2ⓐ)
        }
        self.avatar_choice = val.clone();
        self.avatar = None; // 사진 미리보기 해제(호스트도 image_path "" 반영)
        if !self.image_path.is_empty() {
            self.image_path.clear();
            self.changes.push(("profile.image_path", String::new()));
        }
        self.changes.push(("profile.avatar", val));
        inv.push(self.bounds);
    }

    /// IME 조합 중 문자열(08-13) — 초점 필드만 받는다(TextBox가 스스로 거른다 ·
    /// 빈 문자열은 소거라 전 필드에 전달).
    pub fn set_preedit(&mut self, text: &str, inv: &mut Invalidations) {
        self.name.set_preedit(text, inv);
        self.email.set_preedit(text, inv);
        self.phone.set_preedit(text, inv);
        self.bio.set_preedit(text, inv);
    }

    /// 캐러셀 스크롤 방향(설정 `ui.carousel_scroll` — 호스트가 OS 기본 해석 · 08-14).
    pub fn set_carousel_inverted(&mut self, invert: bool) {
        self.swatches.set_scroll_inverted(invert);
        self.recent_car.set_scroll_inverted(invert);
    }

    /// 우클릭 편집 메뉴 행동(1회성 — 08-13 전수 검사).
    pub fn take_edit_ctx(&mut self) -> Option<crate::controls::EditCtxAction> {
        self.name
            .take_edit_ctx()
            .or_else(|| self.email.take_edit_ctx())
            .or_else(|| self.phone.take_edit_ctx())
            .or_else(|| self.bio.take_edit_ctx())
    }

    /// 클립보드 텍스트 유무 주입(우클릭 시점 — 붙여넣기 항목 활성 근거).
    pub fn set_clipboard_has_text(&mut self, yes: bool) {
        self.name.set_clipboard_has_text(yes);
        self.email.set_clipboard_has_text(yes);
        self.phone.set_clipboard_has_text(yes);
        self.bio.set_clipboard_has_text(yes);
    }

    /// 사진 미리보기 교체(호스트 — 이미지 선택 직후 imgdec 결과 반영).
    pub fn set_avatar(
        &mut self,
        img: Option<std::rc::Rc<crate::theme::IconImage>>,
        inv: &mut Invalidations,
    ) {
        self.avatar = img;
        inv.push(self.bounds);
    }

    /// 바뀐 (설정 키, 값) 회수(1회성) — 호스트가 설정 적용 깔때기에 넘긴다.
    pub fn take_changes(&mut self) -> Vec<(&'static str, String)> {
        // M3-18: 편집은 보류(`changes`) — 적용 버튼이 옮긴 `ready`만 나간다.
        // 같은 키 중복은 그대로 둔다(apply_settings가 순차 적용 = 마지막 값이
        // 이기고, image_path↔avatar 배타의 push 순서도 보존된다).
        std::mem::take(&mut self.ready)
    }

    /// 이미지 선택 요청(1회성 · 선택… 버튼) — 호스트가 피커를 연다.
    /// 선택 복사(① 08-13) — 포커스된 입력 필드에서만 나온다(비포커스는 None).
    #[must_use]
    pub fn clipboard_copy(&self) -> Option<String> {
        self.name
            .copy_selection()
            .or_else(|| self.email.copy_selection())
            .or_else(|| self.phone.copy_selection())
            .or_else(|| self.bio.copy_selection())
    }

    /// 선택 잘라내기(①) — 포커스된 필드에서만.
    pub fn clipboard_cut(&mut self, inv: &mut Invalidations) -> Option<String> {
        self.name
            .cut_selection(inv)
            .or_else(|| self.email.cut_selection(inv))
            .or_else(|| self.phone.cut_selection(inv))
            .or_else(|| self.bio.cut_selection(inv))
    }

    /// 붙여넣기(①) — 포커스된 필드만 받는다(TextBox가 비포커스면 무시).
    pub fn clipboard_paste(&mut self, text: &str, inv: &mut Invalidations) {
        self.name.paste(text, inv);
        self.email.paste(text, inv);
        self.phone.paste(text, inv);
        self.bio.paste(text, inv);
    }

    pub fn take_pick_image(&mut self) -> bool {
        std::mem::take(&mut self.pick_image)
    }

    /// 닫기 요청(1회성 · Esc).
    pub fn take_closed(&mut self) -> bool {
        std::mem::take(&mut self.closed)
    }

    /// 피커가 고른 이미지 경로 반영(호스트 호출) — 변경으로도 보고하고 **최근 목록에
    /// 편입**한다(최신 먼저 · 중복 승격 · 최대 10(`RECENT_MAX`) · 08-14 사용자 확정).
    pub fn set_image_path(&mut self, path: &str, inv: &mut Invalidations) {
        self.image_path = path.to_string();
        self.changes.push(("profile.image_path", path.to_string()));
        if !path.is_empty() {
            self.recent.retain(|(p, _)| p != path);
            self.recent.insert(0, (path.to_string(), None));
            self.recent.truncate(RECENT_MAX);
            self.recent_car.set_count(self.recent.len());
            self.changes
                .push(("profile.image_recent", self.encode_recent()));
        }
        inv.push(self.bounds);
    }

    /// 최근 목록 직렬화(`profile.image_recent` — 탭 구분 · 경로에 탭은 없다고 본다.
    /// 개행은 nexa-conf가 공백으로 무해화하므로 못 쓴다).
    fn encode_recent(&self) -> String {
        self.recent
            .iter()
            .map(|(p, _)| p.as_str())
            .collect::<Vec<_>>()
            .join("\t")
    }

    /// 최근 이미지 썸네일 도착(호스트 — 격리 디코드 완료). 모르는 경로는 무시.
    pub fn set_recent_thumb(
        &mut self,
        path: &str,
        img: Option<std::rc::Rc<crate::theme::IconImage>>,
        inv: &mut Invalidations,
    ) {
        if let Some(slot) = self.recent.iter_mut().find(|(p, _)| p == path) {
            slot.1 = img;
            inv.push(self.bounds);
        }
    }

    /// i번째 최근 이미지의 × 삭제 오버레이 영역(우상단 — 표시 중일 때만).
    fn recent_x_rect(&self, i: usize) -> Option<Rect> {
        let r = self.recent_car.item_rect(i)?;
        let d = self.s(12);
        Some(Rect::new(r.right() - d + self.s(2), r.y - self.s(2), d, d))
    }

    /// 툴팁 표시 대기 변경(설정 `ui.tooltip_ms` — hot-swap · 08-14). 무효는 기본 2000.
    pub fn set_tooltip_ms(&mut self, ms: u64) {
        self.tip_delay_ms = if ms < 200 { 2000 } else { ms };
    }

    /// 시간 틱(호스트 ~5Hz · 08-14) — 최근 아이템을 설정 시간(기본 2000ms)만큼
    /// 호버하면 파일명 툴팁.
    /// `true` = 다시 그려야 한다.
    pub fn tick(&mut self, now_ms: u64) -> bool {
        // bio 스크롤바 자동 숨김 틱(08-18) — 항상 돌린다(툴팁 로직과 독립).
        let bars = self.bio.tick(now_ms);
        let tip = if self.recent_hover.is_none() {
            let was = self.recent_tip;
            self.recent_hover_since = None;
            self.recent_tip = false;
            was
        } else {
            let since = *self.recent_hover_since.get_or_insert(now_ms);
            let show = now_ms.saturating_sub(since) >= self.tip_delay_ms;
            if show != self.recent_tip {
                self.recent_tip = show;
                true
            } else {
                false
            }
        };
        bars || tip
    }

    /// 배율 지정 — 내부 컨트롤 전파.
    pub fn set_scale(&mut self, scale: f32, inv: &mut Invalidations) {
        self.scale = scale.max(0.5);
        self.name.set_scale(self.scale);
        self.email.set_scale(self.scale);
        self.phone.set_scale(self.scale);
        self.bio.set_scale(self.scale);
        self.swatches.set_scale(self.scale);
        self.recent_car.set_scale(self.scale);
        self.border.set_scale(self.scale);
        self.choose_img.set_scale(self.scale);
        self.apply_btn.set_scale(self.scale);
        self.cancel_btn.set_scale(self.scale);
        self.sw_basic.set_scale(self.scale);
        self.sw_email.set_scale(self.scale);
        self.sw_phone.set_scale(self.scale);
        self.relayout(inv);
    }

    fn s(&self, v: i32) -> i32 {
        #[allow(clippy::cast_possible_truncation, clippy::cast_precision_loss)]
        let r = (v as f32 * self.scale).round() as i32;
        r
    }

    fn relayout(&mut self, inv: &mut Invalidations) {
        let b = self.bounds;
        let pad = self.s(16);
        let field_h = self.s(28);
        let bw = self.s(80);
        // 아바타 스와치 캐러셀(08-14 — 32px 아이템 · 넘치면 좌/우 이동 버튼).
        self.swatches.set_bounds(
            Rect::new(b.x + pad, b.y + self.s(164), b.w - pad * 2, self.s(36)),
            inv,
        );
        // 최근 프로필 이미지 캐러셀(08-14) — "Profile image" 라벨과 Choose… 사이.
        let rx = b.x + pad + self.s(110);
        let rw = (b.right() - pad - self.s(80) - self.s(8) - rx).max(0);
        self.recent_car
            .set_bounds(Rect::new(rx, b.y + self.s(240), rw, self.s(36)), inv);
        // 보더 색 행(08-14) — 라벨은 paint · ColorPick 우측 정렬.
        let pick_w = self.border.preferred_width();
        self.border.set_bounds(
            Rect::new(b.right() - pad - pick_w, b.y + self.s(204), pick_w, field_h),
            inv,
        );
        // 이미지 행(라벨은 paint) — 버튼 우측.
        self.choose_img.set_bounds(
            Rect::new(b.right() - pad - bw, b.y + self.s(244), bw, field_h),
            inv,
        );
        // 입력 3종 — 라벨 아래 필드.
        let fw = b.w - pad * 2;
        self.name
            .set_bounds(Rect::new(b.x + pad, b.y + self.s(304), fw, field_h), inv);
        self.email
            .set_bounds(Rect::new(b.x + pad, b.y + self.s(362), fw, field_h), inv);
        self.phone
            .set_bounds(Rect::new(b.x + pad, b.y + self.s(420), fw, field_h), inv);
        // 소개글(08-18 사용자 확정) — 라벨 y456 아래 멀티라인 필드. **고정 3줄**:
        // 높이 = 3 × line_h + 상하 여백. 초과분은 필드 내부 스크롤(키보드 + 마우스
        // 휠). 창 크기 조정·중간영역 스크롤 재설계는 취소(후속 TODO).
        let bio_y = b.y + self.s(474);
        let bio_h = self.bio_lines as i32 * self.s(BIO_LINE_H) + self.s(16);
        self.bio
            .set_bounds(Rect::new(b.x + pad, bio_y, fw, bio_h), inv);
        // 이하 흐름 배치 — bio 아래에서 이어진다.
        let sw_h = self.s(26);
        let sw1 = bio_y + bio_h + self.s(26);
        self.sw_basic
            .set_bounds(Rect::new(b.x + pad, sw1, fw, sw_h), inv);
        let sw2 = sw1 + self.s(32);
        self.sw_email
            .set_bounds(Rect::new(b.x + pad, sw2, fw, sw_h), inv);
        let sw3 = sw2 + self.s(32);
        self.sw_phone
            .set_bounds(Rect::new(b.x + pad, sw3, fw, sw_h), inv);
        // 공유 안내문(2줄 · 각 s(16)) — 토글 아래. paint가 note_dy(오프셋)로 그린다.
        self.note_dy = (sw3 + sw_h + self.s(12)) - b.y;
        // 적용/취소(M3-18) — 안내문(2줄) 아래 우측 정렬.
        let by = b.y + self.note_dy + self.s(32) + self.s(14);
        self.cancel_btn
            .set_bounds(Rect::new(b.right() - pad - bw, by, bw, field_h), inv);
        self.apply_btn.set_bounds(
            Rect::new(b.right() - pad - bw * 2 - self.s(8), by, bw, field_h),
            inv,
        );
    }
}

impl Widget for ProfileWidget {
    fn bounds(&self) -> Rect {
        self.bounds
    }

    fn set_bounds(&mut self, bounds: Rect, inv: &mut Invalidations) {
        self.bounds = bounds;
        self.relayout(inv);
        inv.push(bounds);
    }

    fn on_event(&mut self, ev: &InputEvent, inv: &mut Invalidations) {
        // 커서 추적(08-18) — 휠 hover 판정용(휠 이벤트엔 좌표가 없다).
        if let InputEvent::MouseMove { x, y } = *ev {
            self.last_cursor = Point { x, y };
        }
        // 마우스 휠(세로/가로) — bio 위면 bio로 넘겨 스크롤바가 처리(3줄 초과분 ·
        // 상하+좌우). 그 외 컨트롤 위는 지금 스크롤 대상 없음(중간영역은 후속 TODO).
        if matches!(*ev, InputEvent::Wheel { .. } | InputEvent::HWheel { .. }) {
            if self.bio.bounds().contains(self.last_cursor) {
                self.bio.on_event(ev, inv);
            }
            return;
        }
        if matches!(
            *ev,
            InputEvent::Key {
                key: Key::Escape,
                ..
            }
        ) {
            // 우클릭 메뉴가 열려 있으면 Esc는 메뉴 몫(메뉴만 닫는다) — 창 닫기가
            // 가로채면 메뉴를 키보드로 못 닫는다(08-13 실기).
            let popup =
                self.name.popup_open() || self.email.popup_open() || self.phone.popup_open();
            if !popup {
                // M3-18 ④ — 미저장 변경이 있으면 1차 Esc = 경고, 2차 = 버리고 닫기.
                if self.dirty() && !self.discard_arm {
                    self.discard_arm = true;
                    inv.push(self.bounds);
                    return;
                }
                self.changes.clear(); // 버리기(취소와 동일 — 적용분만 ready로 나간다)
                self.closed = true;
                return;
            }
        }
        // 아바타 스와치 캐러셀(08-14) — 이동 버튼·아이템 클릭 판정은 컨트롤 몫,
        // 선택 적용(사진과 배타)은 여기서. 클릭이 잡히면 아래 포커스 처리로 안 흘린다.
        self.swatches.on_event(ev, inv);
        if let Some(i) = self.swatches.take_clicked() {
            self.apply_swatch(i, inv);
            return;
        }
        // 최근 이미지 호버 추적(08-14) — ×는 커서가 올라간 아이템에만 보인다.
        if let InputEvent::MouseMove { x, y } = *ev {
            let p = Point { x, y };
            let over = (0..self.recent.len())
                .find(|&i| self.recent_car.item_rect(i).is_some_and(|r| r.contains(p)));
            if over != self.recent_hover {
                self.recent_hover = over;
                self.recent_hover_since = None; // 다음 틱이 시작 시각을 다시 잰다
                self.recent_tip = false;
                inv.push(self.bounds);
            }
        }
        // × 오버레이 클릭(호버 중 = 보이는 것만) — 아이템 클릭보다 **먼저**(겹친다).
        if let InputEvent::MouseDown { x, y, .. } = *ev {
            let p = Point { x, y };
            if let Some(i) = self.recent_hover {
                if self.recent_x_rect(i).is_some_and(|r| r.contains(p)) {
                    let (path, _) = self.recent.remove(i);
                    self.recent_car.set_count(self.recent.len());
                    self.changes
                        .push(("profile.image_recent", self.encode_recent()));
                    // 사용 중인 사진을 지웠으면 = 사진 없음 → **이니셜로 폴백**
                    // (08-14 사용자 확정 — 빈 상태로 두지 않고 다시 고를 수 있게).
                    if !self.image_path.is_empty() && path == self.image_path {
                        self.image_path.clear();
                        self.avatar = None;
                        self.avatar_choice = "initials".into();
                        self.changes.push(("profile.image_path", String::new()));
                        self.changes
                            .push(("profile.avatar", "initials".to_string()));
                    }
                    self.recent_hover = None;
                    self.recent_tip = false;
                    inv.push(self.bounds);
                    return;
                }
            }
        }
        self.recent_car.on_event(ev, inv);
        if let Some(i) = self.recent_car.take_clicked() {
            if let Some((path, thumb)) = self.recent.get(i).cloned() {
                // 최근 이미지 선택 = 그 사진으로 전환(사진 우선 — 스와치 링 해제).
                self.image_path = path.clone();
                self.avatar = thumb; // 큰 미리보기는 호스트 디코드(256)가 곧 교체
                self.changes.push(("profile.image_path", path));
            }
            return;
        }
        if let InputEvent::MouseDown { x, y, .. } = *ev {
            let p = Point { x, y };
            // 포커스는 **배타** — 클릭된 컨트롤만 갖는다(08-14 실기: Choose… 버튼이
            // 묵은 포커스를 쥔 채라 텍스트박스 Enter가 **버튼 클릭으로도** 처리되고
            // 포커스 링도 남았다. 브로드캐스트 전달 구조에선 포커스 단일성이 이중
            // 처리를 막는 유일한 문이다 — 갤러리는 처음부터 전 컨트롤 배타였다).
            self.name.set_focused(self.name.bounds().contains(p));
            self.email.set_focused(self.email.bounds().contains(p));
            self.phone.set_focused(self.phone.bounds().contains(p));
            self.bio.set_focused(self.bio.bounds().contains(p));
            self.choose_img
                .set_focused(self.choose_img.bounds().contains(p));
            self.sw_basic
                .set_focused(self.sw_basic.bounds().contains(p));
            self.sw_email
                .set_focused(self.sw_email.bounds().contains(p));
            self.sw_phone
                .set_focused(self.sw_phone.bounds().contains(p));
            if !self.border.bounds().contains(p) {
                self.border.set_focused(false); // ColorPick 내부(hex) 포커스도 배타
            }
        }
        self.apply_btn.on_event(ev, inv);
        if self.apply_btn.take_clicked() {
            // 적용(M3-18) — **Enter 없이 타이핑한 값까지 수확** 후(08-16 실기:
            // 엔터를 안 누르면 적용이 안 됐다) 보류분 일괄 확정 + 닫기.
            self.harvest_texts();
            let held = std::mem::take(&mut self.changes);
            self.ready.extend(held);
            self.closed = true;
            return;
        }
        self.cancel_btn.on_event(ev, inv);
        if self.cancel_btn.take_clicked() {
            self.changes.clear(); // 무반영 폐기 — 다음 열림 = 저장값(복원과 동등)
            self.closed = true;
            return;
        }
        if self.discard_arm && !matches!(*ev, InputEvent::MouseMove { .. }) {
            self.discard_arm = false; // 다른 조작 = 경고 해제(격리함 문법)
            inv.push(self.bounds);
        }
        self.choose_img.on_event(ev, inv);
        if self.choose_img.take_clicked() {
            self.pick_image = true;
            return;
        }
        for (sw, key) in [
            (&mut self.sw_basic, "profile.share.basic"),
            (&mut self.sw_email, "profile.share.email"),
            (&mut self.sw_phone, "profile.share.phone"),
        ] {
            sw.on_event(ev, inv);
            if let Some(on) = sw.take_toggled() {
                self.changes
                    .push((key, if on { "on" } else { "off" }.to_string()));
            }
        }
        // 보더 색(08-14) — ColorPick 확정 시 보고(hex 검증은 컨트롤 몫).
        self.border.on_event(ev, inv);
        if let Some(hex) = self.border.take_changed() {
            self.changes.push(("profile.avatar_border", hex));
            inv.push(self.bounds);
        }
        // 텍스트 필드 — Enter 확정 시에만 보고(설정 Face와 같은 규약).
        for (tbx, key, auto_when_empty) in [
            (&mut self.name, "profile.display_name", true),
            (&mut self.email, "profile.email", false),
            (&mut self.phone, "profile.phone", false),
        ] {
            tbx.on_event(ev, inv);
            if let Some(v) = tbx.take_committed() {
                let v = v.trim().to_string();
                let out = if v.is_empty() && auto_when_empty {
                    "auto".to_string()
                } else {
                    v
                };
                // 무변경 Enter = push 없음(RL-2ⓐ · harvest_texts와 같은 판정 —
                // 여기만 비교가 없어 Enter 연타가 재공지를 냈다).
                let orig = match key {
                    "profile.display_name" => self.orig_name.as_str(),
                    "profile.email" => self.orig_email.as_str(),
                    _ => self.orig_phone.as_str(),
                };
                let last = self
                    .changes
                    .iter()
                    .rev()
                    .find(|(k, _)| *k == key)
                    .map_or(orig, |(_, v)| v.as_str());
                if out != last {
                    self.changes.push((key, out));
                }
            }
            let _ = tbx.take_changed();
        }
        // 소개글(08-17) — 멀티라인이라 Enter=개행(확정 없음). 값은 적용 시
        // harvest_texts가 .text()로 수확한다(여기선 편집 이벤트만 흘린다).
        // 08-18: 창 크기 조정은 하지 않는다(고정 4줄 · 초과는 필드 내부 스크롤).
        self.bio.on_event(ev, inv);
        let _ = self.bio.take_changed();
    }

    fn paint(&self, ctx: &mut dyn DrawCtx, theme: &Theme) {
        let b = self.bounds;
        ctx.fill_rect(b, theme.panel_bg);
        let pad = self.s(16);
        ctx.select_font(FontSlot::Base, true);
        ctx.text(
            b.x + pad,
            b.y + self.s(14),
            b,
            t(Msg::ProfileTitle),
            theme.text,
        );
        // 내 키 지문(08-17 — 제목 줄 우측 · 상대 카드의 "키 지문"과 같은 값을 나도
        // 볼 수 있게. 신원은 이름이 아니라 이 지문이다).
        if !self.fingerprint.is_empty() {
            ctx.select_font(FontSlot::Status, false);
            let fp = format!("{}  ·  {}", t(Msg::FingerprintLabel), self.fingerprint);
            let tw = ctx.text_width(&fp);
            ctx.text(
                b.right() - pad - tw,
                b.y + self.s(16),
                b,
                &fp,
                theme.text_dim,
            );
        }
        // 큰 원형 아바타(목록 40의 3배 = 120 · 사용자 요청 08-11) — 이니셜 가상 이미지.
        // 사진을 골라도 픽셀 렌더는 M4-5(imgdec) 후 — 그때까지 이니셜 + 파일명 표기.
        let d = self.s(120);
        let av = Rect::new(b.x + (b.w - d) / 2, b.y + self.s(40), d, d);
        // 프리뷰 우선순위(08-14): 사진 > 내장 12간지 > 없음(빈 원) > 이니셜.
        let choice = AvatarChoice::parse(&self.avatar_choice);
        let builtin_img = match &choice {
            AvatarChoice::Builtin(k) => self
                .builtins
                .iter()
                .find(|(bk, _)| bk == k)
                .map(|(_, i)| i.clone()),
            _ => None,
        };
        if let Some(img) = &self.avatar {
            // 선택된 사진(M4-5 imgdec — 원형 마스크 완료본) — 동그랗고 큰 미리보기.
            ctx.image_scaled(av, img, b);
        } else if let Some(img) = builtin_img {
            crate::avatar::draw_builtin(ctx, av, &img, &self.seed);
        } else if matches!(choice, AvatarChoice::None) {
            crate::avatar::draw_avatar(ctx, av, "", &self.seed, 34.0); // 빈 원
        } else {
            let ini_src = {
                // 조합 중 글자까지 반영(display_text) — 필드와 아바타가 같은 것을
                // 보여야 "입력이 안 됐다"로 오독하지 않는다(08-13 실기).
                let typed = self.name.display_text();
                if typed.trim().is_empty() {
                    self.resolved_name.clone()
                } else {
                    typed
                }
            };
            crate::avatar::draw_avatar(ctx, av, &ini_src, &self.seed, 34.0);
        }
        // ── 아바타 스와치 행(08-14) — [이니셜][없음][12간지 …] · 현재 선택은 링 ──
        let cur = if self.avatar.is_some() {
            None // 사진 사용 중 — 스와치 어느 것도 "현재"가 아니다
        } else {
            Some(choice.to_setting())
        };
        for (i, val) in self.swatch_values().into_iter().enumerate() {
            let Some(r) = self.swatches.item_rect(i) else {
                continue; // 캐러셀 창 밖 — 이동 버튼으로 넘겨 본다
            };
            match val.as_str() {
                "initials" => {
                    let ini_src = {
                        let typed = self.name.display_text();
                        if typed.trim().is_empty() {
                            self.resolved_name.clone()
                        } else {
                            typed
                        }
                    };
                    crate::avatar::draw_avatar(ctx, r, &ini_src, &self.seed, 0.0);
                }
                "none" => crate::avatar::draw_avatar(ctx, r, "", &self.seed, 0.0),
                v => {
                    if let Some((_, img)) =
                        self.builtins.iter().find(|(k, _)| format!("b:{k}") == v)
                    {
                        crate::avatar::draw_builtin(ctx, r, img, &self.seed);
                    }
                }
            }
            if cur.as_deref() == Some(val.as_str()) {
                // 선택 링 — 위치 신호(색만으로 가르지 않는다 규약과 정합).
                let ring = Rect::new(r.x - 2, r.y - 2, r.w + 4, r.h + 4);
                ctx.stroke_round_rect(ring, ring.w / 2, theme.accent, 2.0);
            }
        }
        // 캐러셀 이동 버튼(넘칠 때만 · 08-14) — 아이템 위에 얹는다.
        self.swatches.paint(ctx, theme);
        // 아바타 보더(08-14 사용자 요청) — **큰 프리뷰는 3px**(소형 2px는 목록 몫).
        if let Some((br, bg, bb)) = nbeep_core::avatar::parse_border(&self.border.value_hex()) {
            let c =
                crate::theme::Color((u32::from(br) << 16) | (u32::from(bg) << 8) | u32::from(bb));
            ctx.stroke_ellipse(av, c, self.s(3).max(3) as f32);
        }
        // 보더 색 행(08-14) — 라벨 + ColorPick(우측).
        ctx.select_font(FontSlot::Base, false);
        ctx.text(
            b.x + pad,
            b.y + self.s(208),
            b,
            t(Msg::AvatarBorderLabel),
            theme.text,
        );
        self.border.paint(ctx, theme);
        // 이미지 행 — 라벨 + 파일명(버튼 왼쪽까지 클립).
        ctx.select_font(FontSlot::Base, false);
        ctx.text(
            b.x + pad,
            b.y + self.s(248),
            b,
            t(Msg::ProfileImage),
            theme.text,
        );
        // 최근 이미지 캐러셀(08-14) — 파일명 텍스트는 없앴다(사용자 확정 — 정보는
        // 3초 호버 툴팁으로). 비어 있으면 아무것도 안 그린다.
        for (i, (path, thumb)) in self.recent.iter().enumerate() {
            let Some(r) = self.recent_car.item_rect(i) else {
                continue;
            };
            if let Some(img) = thumb {
                let fit = crate::controls::image_fit_contain(r, img.w as i32, img.h as i32);
                ctx.image_scaled(fit, img, r);
            } else {
                // 디코드 전/실패 — 빈 원 자리표시.
                ctx.fill_round_rect(r, r.w / 2, theme.field_bg);
            }
            // 현재 사용 중인 사진 = 링(스와치와 같은 문법).
            if !self.image_path.is_empty() && *path == self.image_path {
                let ring = Rect::new(r.x - 2, r.y - 2, r.w + 4, r.h + 4);
                ctx.stroke_round_rect(ring, ring.w / 2, theme.accent, 2.0);
            }
            // × 삭제 오버레이 — **커서가 올라간 아이템에만**(08-14 사용자 확정 ·
            // 렌더러에 반투명 채움이 없어 danger 원 + 흰 ×로 대비를 만든다).
            if self.recent_hover == Some(i) {
                if let Some(xr) = self.recent_x_rect(i) {
                    ctx.fill_round_rect(xr, xr.w / 2, theme.danger);
                    let m = self.s(3);
                    let (x0, y0, x1, y1) = (xr.x + m, xr.y + m, xr.right() - m, xr.bottom() - m);
                    let w = self.s(1).max(1) as f32 + 0.5;
                    ctx.polyline(&[(x0, y0), (x1, y1)], crate::theme::Color(0x00FF_FFFF), w);
                    ctx.polyline(&[(x0, y1), (x1, y0)], crate::theme::Color(0x00FF_FFFF), w);
                }
            }
        }
        self.recent_car.paint(ctx, theme);
        // 3초 호버 툴팁 — 원래 파일명(경로 말고 이름만). 최상위로 마지막에 그린다.
        if self.recent_tip {
            if let Some(i) = self.recent_hover {
                if let (Some((path, _)), Some(r)) =
                    (self.recent.get(i), self.recent_car.item_rect(i))
                {
                    let name = std::path::Path::new(path)
                        .file_name()
                        .map_or_else(|| path.clone(), |n| n.to_string_lossy().into_owned());
                    ctx.select_font(FontSlot::Status, false);
                    let tw = ctx.text_width(&name);
                    let th = ctx.text_height();
                    let (px, py) = (self.s(8), self.s(4));
                    let mut tip = Rect::new(
                        r.x + r.w / 2 - tw / 2 - px,
                        r.bottom() + self.s(6),
                        tw + px * 2,
                        th + py * 2,
                    );
                    // 창 밖으로 안 나가게 좌우만 접는다(아래 공간은 항상 있다).
                    tip.x = tip
                        .x
                        .clamp(b.x + self.s(4), (b.right() - tip.w - self.s(4)).max(b.x));
                    ctx.fill_round_rect(tip, self.s(4), theme.panel_bg);
                    ctx.stroke_round_rect(tip, self.s(4), theme.border, 1.0);
                    ctx.text(tip.x + px, tip.y + py, tip, &name, theme.text);
                }
            }
        }
        // 필드 라벨.
        let labels = [
            (Msg::DisplayNameLabel, self.s(286)),
            (Msg::FieldEmail, self.s(344)),
            (Msg::FieldPhone, self.s(402)),
            (Msg::BioLabel, self.s(458)),
        ];
        for (m, dy) in labels {
            ctx.select_font(FontSlot::Status, false);
            ctx.text(b.x + pad, b.y + dy, b, t(m), theme.text_dim);
        }
        self.name.paint(ctx, theme);
        self.email.paint(ctx, theme);
        self.phone.paint(ctx, theme);
        self.bio.paint(ctx, theme);
        self.choose_img.paint(ctx, theme);
        // 적용/취소(M3-18) + 미저장 경고줄(Esc 2단계 — 격리함 confirming 문법).
        self.apply_btn.paint(ctx, theme);
        self.cancel_btn.paint(ctx, theme);
        if self.discard_arm {
            ctx.select_font(FontSlot::Status, false);
            let ay = self.apply_btn.bounds().y;
            let ah = self.apply_btn.bounds().h;
            let sh = ctx.text_height();
            ctx.text(
                b.x + pad,
                ay + (ah - sh) / 2,
                b,
                t(Msg::ProfileUnsavedHint),
                theme.warn,
            );
        }
        self.sw_basic.paint(ctx, theme);
        self.sw_email.paint(ctx, theme);
        self.sw_phone.paint(ctx, theme);
        // 안내 — 브로드캐스트 미포함·연결 상대 한정(워드랩 2줄).
        ctx.select_font(FontSlot::Status, false);
        let avail = b.w - pad * 2;
        let lines = crate::settings::wrap_text(ctx, t(Msg::ProfileShareNote), avail, 2);
        for (i, line) in lines.iter().enumerate() {
            #[allow(clippy::cast_possible_truncation, clippy::cast_possible_wrap)]
            let dy = self.note_dy + i as i32 * self.s(16);
            ctx.text(b.x + pad, b.y + dy, b, line, theme.text_dim);
        }
        // 우클릭 편집 메뉴 — 모든 자식 뒤에 재도색(08-13 실기: Email 필드가
        // Display name 메뉴의 가운데를 덮었다).
        self.name.paint_popup(ctx, theme);
        self.email.paint_popup(ctx, theme);
        self.phone.paint_popup(ctx, theme);
        self.bio.paint_popup(ctx, theme);
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    use super::*;

    /// 적용 버튼 클릭(M3-18) — 편집은 보류되므로 테스트는 이걸 눌러 회수한다.
    fn click_apply(w: &mut ProfileWidget, inv: &mut Invalidations) {
        let r = w.apply_btn.bounds();
        w.on_event(
            &InputEvent::MouseDown {
                x: r.x + 2,
                y: r.y + 2,
                shift: false,
                primary: false,
            },
            inv,
        );
        w.on_event(
            &InputEvent::MouseUp {
                x: r.x + 2,
                y: r.y + 2,
            },
            inv,
        );
    }

    /// ★ M3-18 후속(08-16 실기) — **Enter 없이 타이핑만 한 값도 적용이 수확**한다.
    /// 종전엔 Enter 확정분만 나가서 "엔터를 안 누르면 적용이 안 되는" 함정이었다.
    #[test]
    fn apply_harvests_uncommitted_typing() {
        let (mut w, mut inv) = widget();
        let r = w.email.bounds();
        w.on_event(
            &InputEvent::MouseDown {
                x: r.x + 4,
                y: r.y + 4,
                shift: false,
                primary: false,
            },
            &mut inv,
        );
        for c in "a@b.c".chars() {
            w.on_event(&InputEvent::Char { c, now_ms: 0 }, &mut inv);
        }
        // Enter 없이 바로 적용.
        click_apply(&mut w, &mut inv);
        let ch = w.take_changes();
        assert!(
            ch.contains(&("profile.email", "a@b.c".to_string())),
            "미확정 타이핑 수확: {ch:?}"
        );
        // 원값과 같으면 재보고하지 않는다(불필요한 재공지 방지).
        let (mut w2, mut inv2) = widget();
        click_apply(&mut w2, &mut inv2);
        assert!(w2.take_changes().is_empty(), "무편집 적용 = 보고 0");
    }

    /// ★ M3-18 계약 — 편집은 보류: 적용 전 take_changes는 비고, 취소는 폐기한다.
    #[test]
    fn edits_are_held_until_apply_and_cancel_discards() {
        let (mut w, mut inv) = widget();
        let r = w.sw_basic.bounds();
        let click = |w: &mut ProfileWidget, inv: &mut Invalidations, r: Rect| {
            w.on_event(
                &InputEvent::MouseDown {
                    x: r.x + 2,
                    y: r.y + 2,
                    shift: false,
                    primary: false,
                },
                inv,
            );
            w.on_event(
                &InputEvent::MouseUp {
                    x: r.x + 2,
                    y: r.y + 2,
                },
                inv,
            );
        };
        click(&mut w, &mut inv, r);
        assert!(w.dirty(), "편집됨");
        assert!(w.take_changes().is_empty(), "적용 전엔 나가지 않는다");
        click_apply(&mut w, &mut inv);
        assert!(!w.take_changes().is_empty(), "적용 = 일괄 확정");
        assert!(w.take_closed(), "적용 = 닫기(대화상자 관례)");
        // 취소 — 편집 후 폐기.
        let (mut w, mut inv) = widget();
        let r = w.sw_basic.bounds();
        click(&mut w, &mut inv, r);
        let c = w.cancel_btn.bounds();
        click(&mut w, &mut inv, c);
        assert!(w.take_changes().is_empty(), "취소 = 무반영 폐기");
        assert!(w.take_closed(), "취소 = 닫기");
        assert!(!w.dirty(), "폐기 후 깨끗");
    }

    fn widget() -> (ProfileWidget, Invalidations) {
        let mut w = ProfileWidget::new(&ProfileValues {
            display_name: "auto".into(),
            ..ProfileValues::default()
        });
        let mut inv = Invalidations::default();
        w.set_bounds(Rect::new(0, 0, 440, 430), &mut inv);
        (w, inv)
    }

    /// 토글 클릭 → (키, on/off) 변경 보고.
    #[test]
    fn share_toggle_reports_change() {
        let (mut w, mut inv) = widget();
        let r = w.sw_basic.bounds();
        w.on_event(
            &InputEvent::MouseDown {
                x: r.x + 2,
                y: r.y + 2,
                shift: false,
                primary: true,
            },
            &mut inv,
        );
        click_apply(&mut w, &mut inv);
        let ch = w.take_changes();
        assert_eq!(ch, vec![("profile.share.basic", "on".to_string())]);
        assert!(w.take_changes().is_empty(), "1회성");
    }

    /// 표시 이름 Enter 확정 — 빈 값은 "auto"로 정규화하되, **무변경 Enter는
    /// push하지 않는다**(RL-2ⓐ 08-18 — 원값이 이미 auto인데 빈 필드에서 Enter
    /// 연타 = 재공지 낭비였다).
    #[test]
    fn empty_name_commits_auto() {
        let (mut w, mut inv) = widget();
        w.on_event(
            &InputEvent::Key {
                key: Key::Enter,
                shift: false,
                primary: false,
            },
            &mut inv,
        );
        click_apply(&mut w, &mut inv);
        assert!(
            w.take_changes().is_empty(),
            "원값 auto + 빈 필드 Enter = 무변경 — push 없음(RL-2ⓐ)"
        );
        // 원값이 실명("kim")인 위젯 — 필드를 비우고 확정하면 "auto"로 정규화되고,
        // 이는 원값과 다르므로 **변경으로 보고**된다(정규화 축은 그대로 산다).
        let mut w = ProfileWidget::new(&ProfileValues {
            display_name: "kim".into(),
            ..ProfileValues::default()
        });
        w.set_bounds(Rect::new(0, 0, 440, 430), &mut inv);
        let n = w.name.bounds();
        w.on_event(
            &InputEvent::MouseDown {
                x: n.x + 4,
                y: n.y + 4,
                shift: false,
                primary: false,
            },
            &mut inv,
        );
        w.on_event(
            &InputEvent::Key {
                key: Key::End,
                shift: false,
                primary: false,
            },
            &mut inv,
        );
        for _ in 0..3 {
            // Backspace는 Char('\u{8}')로 온다(event.rs 규약).
            w.on_event(
                &InputEvent::Char {
                    c: '\u{8}',
                    now_ms: 0,
                },
                &mut inv,
            );
        }
        w.on_event(
            &InputEvent::Key {
                key: Key::Enter,
                shift: false,
                primary: false,
            },
            &mut inv,
        );
        click_apply(&mut w, &mut inv);
        let ch = w.take_changes();
        assert_eq!(
            ch.last(),
            Some(&("profile.display_name", "auto".to_string())),
            "빈 값 확정 = auto 정규화(원값과 다르므로 보고)"
        );
    }

    /// 08-14 실기 — Choose… 클릭 후 이름 필드를 편집하고 Enter를 치면 **버튼이
    /// 묵은 포커스로 또 클릭**됐다(피커 재오픈). 포커스는 배타여야 한다.
    #[test]
    fn enter_in_textbox_does_not_click_stale_focused_button() {
        let (mut w, mut inv) = widget();
        // Choose… 클릭 — 버튼이 포커스를 얻고 피커 요청 1회.
        let r = w.choose_img.bounds();
        w.on_event(
            &InputEvent::MouseDown {
                x: r.x + 2,
                y: r.y + 2,
                shift: false,
                primary: true,
            },
            &mut inv,
        );
        w.on_event(
            &InputEvent::MouseUp {
                x: r.x + 2,
                y: r.y + 2,
            },
            &mut inv,
        );
        assert!(w.take_pick_image(), "버튼 클릭 = 피커 요청");
        // 이름 필드 클릭 — 버튼 포커스는 **해제**되어야 한다(배타).
        let n = w.name.bounds();
        w.on_event(
            &InputEvent::MouseDown {
                x: n.x + 2,
                y: n.y + 2,
                shift: false,
                primary: false,
            },
            &mut inv,
        );
        assert!(
            !w.choose_img.is_focused(),
            "텍스트박스 클릭 후 버튼 포커스 잔존(포커스 링·Enter 이중 처리의 원인)"
        );
        // Enter = 이름 확정만 — 버튼 클릭(피커 재요청)이 되면 안 된다.
        // 실제 변경을 만들고 확정한다(RL-2ⓐ — 무변경 Enter는 push가 없다).
        w.on_event(&InputEvent::Char { c: 'k', now_ms: 0 }, &mut inv);
        w.on_event(
            &InputEvent::Key {
                key: Key::Enter,
                shift: false,
                primary: false,
            },
            &mut inv,
        );
        assert!(
            !w.take_pick_image(),
            "Enter가 묵은 포커스 버튼을 누르면 안 된다"
        );
        click_apply(&mut w, &mut inv);
        assert!(
            w.take_changes()
                .iter()
                .any(|(k, _)| *k == "profile.display_name"),
            "Enter = 표시 이름 확정 보고"
        );
    }

    /// 아바타 스와치(08-14) — 클릭 = 선택 보고 · **사진과 상호 배타**(image_path 소거).
    #[test]
    fn swatch_click_reports_choice_and_clears_photo() {
        let mut w = ProfileWidget::new(&ProfileValues {
            display_name: "auto".into(),
            image_path: "C:/pics/me.png".into(),
            avatar_choice: "initials".into(),
            ..ProfileValues::default()
        });
        let mut inv = Invalidations::default();
        w.set_bounds(Rect::new(0, 0, 440, 610), &mut inv);
        let r = w.swatches.item_rect(2).expect("표시 중"); // [이니셜][없음] 다음 = 첫 12간지(rat)
        w.on_event(
            &InputEvent::MouseDown {
                x: r.x + 2,
                y: r.y + 2,
                shift: false,
                primary: true,
            },
            &mut inv,
        );
        click_apply(&mut w, &mut inv);
        let ch = w.take_changes();
        assert!(
            ch.contains(&("profile.avatar", "b:rat".to_string())),
            "스와치 클릭 = 선택 보고: {ch:?}"
        );
        assert!(
            ch.contains(&("profile.image_path", String::new())),
            "사진과 배타 — image_path 소거: {ch:?}"
        );
    }

    /// 이미지 선택 요청 · 경로 반영이 변경으로 보고된다.
    #[test]
    fn image_pick_roundtrip() {
        let (mut w, mut inv) = widget();
        let r = w.choose_img.bounds();
        let (cx, cy) = (r.x + 2, r.y + 2);
        w.on_event(
            &InputEvent::MouseDown {
                x: cx,
                y: cy,
                shift: false,
                primary: true,
            },
            &mut inv,
        );
        w.on_event(&InputEvent::MouseUp { x: cx, y: cy }, &mut inv);
        assert!(w.take_pick_image(), "선택 요청");
        w.set_image_path("C:/pics/me.png", &mut inv);
        click_apply(&mut w, &mut inv);
        // 경로 저장 + **최근 목록 편입**(08-14 — 캐러셀 재선택 재료)이 함께 보고된다.
        assert_eq!(
            w.take_changes(),
            vec![
                ("profile.image_path", "C:/pics/me.png".to_string()),
                ("profile.image_recent", "C:/pics/me.png".to_string()),
            ]
        );
    }

    #[test]
    fn recent_click_selects_and_x_deletes() {
        let mut w = ProfileWidget::new(&ProfileValues {
            display_name: "auto".into(),
            recent: vec!["/a.png".into(), "/b.png".into()],
            ..ProfileValues::default()
        });
        let mut inv = Invalidations::default();
        w.set_bounds(Rect::new(0, 0, 440, 610), &mut inv);
        // 최근 아이템 클릭 = 그 사진으로 전환.
        let r = w.recent_car.item_rect(1).expect("표시 중");
        w.on_event(
            &InputEvent::MouseDown {
                x: r.x + r.w / 2,
                y: r.y + r.h / 2,
                shift: false,
                primary: false,
            },
            &mut inv,
        );
        click_apply(&mut w, &mut inv);
        let ch = w.take_changes();
        assert!(
            ch.contains(&("profile.image_path", "/b.png".to_string())),
            "최근 클릭 = 선택: {ch:?}"
        );
        // × 오버레이는 **호버 중에만** 보이고 눌린다(08-14) — 먼저 커서를 올린다.
        let ir = w.recent_car.item_rect(0).expect("표시 중");
        w.on_event(
            &InputEvent::MouseMove {
                x: ir.x + ir.w / 2,
                y: ir.y + ir.h / 2,
            },
            &mut inv,
        );
        let xr = w.recent_x_rect(0).expect("표시 중");
        w.on_event(
            &InputEvent::MouseDown {
                x: xr.x + xr.w / 2,
                y: xr.y + xr.h / 2,
                shift: false,
                primary: false,
            },
            &mut inv,
        );
        click_apply(&mut w, &mut inv);
        let ch = w.take_changes();
        assert!(
            ch.contains(&("profile.image_recent", "/b.png".to_string())),
            "× = 삭제 후 목록 재보고: {ch:?}"
        );
        assert_eq!(w.recent.len(), 1, "한 개 삭제됨");
    }

    #[test]
    fn deleting_current_image_falls_back_to_initials() {
        let mut w = ProfileWidget::new(&ProfileValues {
            display_name: "auto".into(),
            image_path: "/a.png".into(),
            recent: vec!["/a.png".into()],
            ..ProfileValues::default()
        });
        let mut inv = Invalidations::default();
        w.set_bounds(Rect::new(0, 0, 440, 610), &mut inv);
        let ir = w.recent_car.item_rect(0).expect("표시 중");
        w.on_event(
            &InputEvent::MouseMove {
                x: ir.x + ir.w / 2,
                y: ir.y + ir.h / 2,
            },
            &mut inv,
        );
        let xr = w.recent_x_rect(0).expect("표시 중");
        w.on_event(
            &InputEvent::MouseDown {
                x: xr.x + xr.w / 2,
                y: xr.y + xr.h / 2,
                shift: false,
                primary: false,
            },
            &mut inv,
        );
        click_apply(&mut w, &mut inv);
        let ch = w.take_changes();
        assert!(
            ch.contains(&("profile.image_path", String::new())),
            "사용 중 사진 삭제 = 사진 해제: {ch:?}"
        );
        assert!(
            ch.contains(&("profile.avatar", "initials".to_string())),
            "이니셜 폴백(08-14 사용자 확정): {ch:?}"
        );
    }
}
