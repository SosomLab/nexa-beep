//! **개발자 상세 로그**(docs/52 · 10-19 사용자 요청 "nexa-sql을 참고해 로그 기능 구성 — 개발자 레벨에서
//! 기능·전달인자·판단 기준/값·실패 여부/원인") — nexa-sql `nsql-log`(docs/48) 이식.
//!
//! ## 원칙(docs/48 §0 그대로)
//!
//! 1. **끄면 비용 = 원자 정수 load 1회 + 예측되는 분기 1개** — [`wants`]는 `#[inline(always)]`·`Relaxed`.
//!    문자열·시각·할당은 게이트 **뒤**에서만 일어난다(앱의 `dlog!` 매크로가 보장).
//! 2. **한 벌의 상태 = 원자 마스크 `u64`**(층 10 × 수준 4 = 40비트) — 설정 `log.dev_mode`×`log.dev_layers`
//!    또는 환경변수 `NEXA_DEVLOG`가 마스크를 만든다.
//! 3. **생산자는 절대 막히지 않는다** — 싱크는 기존 M3-22 상태 로그(`statuslog` · bounded 큐 · 드롭 계상)를
//!    그대로 쓴다(새 스레드 0 · 새 파일 0 — 상태바 줄과 같은 파일에 시각순으로 섞인다).
//! 4. **봉투 원리**: 레코드는 지문 short·길이·판정값만. 메시지 본문·경로·키는 넣지 않는다
//!    (넣어야 하면 [`crate::redact`]로 가린 길이만).
//!
//! ## 레코드 모양([`Rec`]) — 한 줄 · grep 가능한 `k=v`
//!
//! ```text
//! conn/decide start_connect peer=5016194e auto=false psk=true manual=false relay=true | rule latch=free → dial
//! xfer/decide offer peer=b7147214 name_len=9 size=1048576 trust=Pinned mutual=true policy=Ask | → Ask
//! conn/decide connect_failed peer=b7147214 why="NotFound" reachable=false stage=0 | FAIL auto retry stopped
//! ```
//! `층/수준 기능 인자… | rule 기준=값 … | → 결과` 또는 `| FAIL 원인`.

use core::fmt::{Display, Write as _};
use std::sync::atomic::{AtomicU64, Ordering};

/// 층 — 무엇의 로그인가(beep의 수직 축 · nexa-sql의 net/exec/fetch… 대응).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum Layer {
    /// 앱 수명·부팅·종료·설정 적용.
    App = 0,
    /// 발견(S1~S6 · 등장·이탈·힌트).
    Disc = 1,
    /// 연결 사다리(직접·수동·서버 · Noise · 경로 등급 · 재연결 백오프).
    Conn = 2,
    /// 세션 수명(성립·TOFU 판정·종료·대피).
    Sess = 3,
    /// 대화(발신·수신·dedup·ack·대기 큐).
    Chat = 4,
    /// 파일 전송(오퍼 판정·게이트·재개·격리).
    Xfer = 5,
    /// 사용자·형제 기기(ADR-0015 · 인증·따라잡기).
    User = 6,
    /// 릴레이 서버(attach·핀·roster).
    Relay = 7,
    /// 영속(기록·설정·대기 큐 · 봉인).
    Store = 8,
    /// UI(창·뷰 전환·IME 게이트).
    Ui = 9,
}

impl Layer {
    /// 전체(설정 문법·메뉴 순서).
    pub const ALL: [Layer; 10] = [
        Layer::App,
        Layer::Disc,
        Layer::Conn,
        Layer::Sess,
        Layer::Chat,
        Layer::Xfer,
        Layer::User,
        Layer::Relay,
        Layer::Store,
        Layer::Ui,
    ];

    /// 고정 라벨(번역하지 않는다 — 파일·grep 대상).
    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            Layer::App => "app",
            Layer::Disc => "disc",
            Layer::Conn => "conn",
            Layer::Sess => "sess",
            Layer::Chat => "chat",
            Layer::Xfer => "xfer",
            Layer::User => "user",
            Layer::Relay => "relay",
            Layer::Store => "store",
            Layer::Ui => "ui",
        }
    }

    /// 라벨 → 층(대소문자 무관).
    #[must_use]
    pub fn parse(s: &str) -> Option<Layer> {
        let l = s.trim().to_ascii_lowercase();
        Self::ALL.into_iter().find(|k| k.label() == l)
    }
}

/// 수준 — 얼마나 상세한가. `basic`은 상태바 줄(개발자 모드와 무관하게 늘 기록)이라 마스크 대상이 아니다.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum Level {
    /// 상태바 문구(기존 M3-22 — 게이트 밖).
    Basic = 0,
    /// **판단**: 기능 + 전달인자 + 판단 기준·값 + 결과/실패 원인(기능당 1~2줄).
    Decide = 1,
    /// **상세**: 중간값·개수·바이트(단계마다).
    Detail = 2,
    /// **추적**: 호출 단위(수신 프레임마다 — 많음).
    Trace = 3,
}

impl Level {
    /// 마스크 대상 수준(basic 제외).
    pub const DETAIL: [Level; 3] = [Level::Decide, Level::Detail, Level::Trace];

    /// 고정 라벨.
    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            Level::Basic => "basic",
            Level::Decide => "decide",
            Level::Detail => "detail",
            Level::Trace => "trace",
        }
    }

    /// 라벨 → 수준(basic은 설정에 쓰지 않는다).
    #[must_use]
    pub fn parse(s: &str) -> Option<Level> {
        let l = s.trim().to_ascii_lowercase();
        Self::DETAIL.into_iter().find(|k| k.label() == l)
    }
}

/// 층×수준 비트 마스크 — 0이면 상세 로그 코드는 load 1 + 분기 1만 남는다. 쓰기는 설정 변경·부팅 때만.
static MASK: AtomicU64 = AtomicU64::new(0);

#[inline(always)]
const fn bit(layer: Layer, level: Level) -> u64 {
    1u64 << ((layer as u64) * 4 + (level as u64))
}

/// 이 층·수준의 상세 로그를 만들어야 하는가 — **항상 인라인**(호출 0 · 분기 1).
#[inline(always)]
#[must_use]
pub fn wants(layer: Layer, level: Level) -> bool {
    MASK.load(Ordering::Relaxed) & bit(layer, level) != 0
}

/// 마스크 교체(설정·환경변수 → 부팅·변경 시).
pub fn set_mask(mask: u64) {
    MASK.store(mask, Ordering::Relaxed);
}

/// 현재 마스크.
#[must_use]
pub fn mask() -> u64 {
    MASK.load(Ordering::Relaxed)
}

/// 전 층 전 수준.
#[must_use]
pub fn mask_all() -> u64 {
    parse_layers("*")
}

/// 설정 `log.dev_layers` → 마스크. 문법: `layer[:level+level]`을 쉼표로 · 수준 생략 = decide+detail+trace ·
/// `*`/`all` = 전 층 전 수준 · `1`/`on`/`true` = 전부 · 빈 문자열·미지 층 = 0.
/// 예 `conn,xfer:decide,user:decide+detail`.
#[must_use]
pub fn parse_layers(spec: &str) -> u64 {
    let spec = spec.trim();
    if matches!(
        spec.to_ascii_lowercase().as_str(),
        "1" | "on" | "true" | "all"
    ) {
        return parse_layers("*");
    }
    let mut m = 0u64;
    for part in spec.split(',').map(str::trim).filter(|p| !p.is_empty()) {
        let (layer_s, levels_s) = match part.split_once(':') {
            Some((a, b)) => (a, Some(b)),
            None => (part, None),
        };
        let layers: Vec<Layer> = if layer_s.trim() == "*" {
            Layer::ALL.to_vec()
        } else {
            Layer::parse(layer_s).into_iter().collect()
        };
        let levels: Vec<Level> = match levels_s {
            Some(ls) => ls.split('+').filter_map(Level::parse).collect(),
            None => Level::DETAIL.to_vec(),
        };
        for l in &layers {
            for v in &levels {
                m |= bit(*l, *v);
            }
        }
    }
    m
}

/// 마스크에 층이 하나라도 켜져 있는가(표시·메뉴 체크).
#[must_use]
pub fn layer_in_mask(mask: u64, layer: Layer) -> bool {
    Level::DETAIL.iter().any(|v| mask & bit(layer, *v) != 0)
}

/// 결과.
#[derive(Clone, Debug, PartialEq, Eq)]
enum Outcome {
    /// 아직 없음(관측만).
    None,
    /// 성공·결정(짧은 결과 문구).
    Ok(String),
    /// 실패(원인).
    Fail(String),
}

/// 상세 로그 한 건 — 게이트 뒤에서만 만들어진다(생성 = 할당 발생). 한 줄로 렌더한다([`Rec::render`]).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Rec {
    layer: Layer,
    level: Level,
    func: &'static str,
    args: Vec<(&'static str, String)>,
    rules: Vec<(&'static str, String)>,
    outcome: Outcome,
}

impl Rec {
    /// 기능 이름(함수·이벤트 이름 — 번역하지 않는다)으로 시작.
    #[must_use]
    pub fn new(layer: Layer, level: Level, func: &'static str) -> Self {
        Self {
            layer,
            level,
            func,
            args: Vec::new(),
            rules: Vec::new(),
            outcome: Outcome::None,
        }
    }

    /// 전달인자(`k=v`). 값에 공백·`=`가 있으면 따옴표로 감싼다. **비밀·본문·경로는 넣지 않는다**(봉투 원리).
    #[must_use]
    pub fn arg(mut self, key: &'static str, value: impl Display) -> Self {
        self.args.push((key, value.to_string()));
        self
    }

    /// 판단 기준과 그 값(`rule k=v` — "무엇을 보고 그렇게 판단했나").
    #[must_use]
    pub fn rule(mut self, name: &'static str, value: impl Display) -> Self {
        self.rules.push((name, value.to_string()));
        self
    }

    /// 결과(성공·결정).
    #[must_use]
    pub fn ok(mut self, what: impl Display) -> Self {
        self.outcome = Outcome::Ok(what.to_string());
        self
    }

    /// 실패와 원인.
    #[must_use]
    pub fn fail(mut self, why: impl Display) -> Self {
        self.outcome = Outcome::Fail(why.to_string());
        self
    }

    /// 층.
    #[must_use]
    pub fn layer(&self) -> Layer {
        self.layer
    }

    /// 수준.
    #[must_use]
    pub fn level(&self) -> Level {
        self.level
    }

    /// 실패인가.
    #[must_use]
    pub fn is_fail(&self) -> bool {
        matches!(self.outcome, Outcome::Fail(_))
    }

    /// 한 줄 렌더 — `층/수준 기능 k=v … | rule k=v … | → 결과` / `| FAIL 원인`.
    #[must_use]
    pub fn render(&self) -> String {
        let mut s = String::with_capacity(96);
        let _ = write!(
            s,
            "{}/{} {}",
            self.layer.label(),
            self.level.label(),
            self.func
        );
        for (k, v) in &self.args {
            s.push(' ');
            s.push_str(k);
            s.push('=');
            push_value(&mut s, v);
        }
        if !self.rules.is_empty() {
            s.push_str(" | rule");
            for (k, v) in &self.rules {
                s.push(' ');
                s.push_str(k);
                s.push('=');
                push_value(&mut s, v);
            }
        }
        match &self.outcome {
            Outcome::None => {}
            Outcome::Ok(w) => {
                s.push_str(" | → ");
                s.push_str(w);
            }
            Outcome::Fail(w) => {
                s.push_str(" | FAIL ");
                s.push_str(w);
            }
        }
        s
    }
}

/// 값 쓰기 — 공백·`=`·`|`·따옴표가 있으면 `"…"`로(줄 하나 = 레코드 하나 불변 · 개행은 `␤`).
fn push_value(s: &mut String, v: &str) {
    let needs = v.is_empty()
        || v.chars()
            .any(|c| c.is_whitespace() || matches!(c, '=' | '|' | '"'));
    if !needs {
        s.push_str(v);
        return;
    }
    s.push('"');
    for c in v.chars() {
        match c {
            '"' => s.push_str("\\\""),
            '\n' => s.push('␤'),
            '\r' => {}
            c => s.push(c),
        }
    }
    s.push('"');
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    use super::*;

    #[test]
    fn mask_parse_and_gate_both_branches() {
        set_mask(0);
        assert!(!wants(Layer::Conn, Level::Decide), "마스크 0 = 전부 끔");
        assert_eq!(parse_layers(""), 0);
        assert_eq!(parse_layers("nope"), 0, "미지 층 = 무시");
        let m = parse_layers("conn,xfer:decide,user:decide+trace");
        assert!(m & bit(Layer::Conn, Level::Decide) != 0);
        assert!(
            m & bit(Layer::Conn, Level::Trace) != 0,
            "수준 생략 = 세 수준"
        );
        assert!(m & bit(Layer::Xfer, Level::Decide) != 0);
        assert!(m & bit(Layer::Xfer, Level::Detail) == 0);
        assert!(m & bit(Layer::User, Level::Trace) != 0);
        assert!(m & bit(Layer::User, Level::Detail) == 0);
        assert!(layer_in_mask(m, Layer::User) && !layer_in_mask(m, Layer::Disc));
        set_mask(m);
        assert!(wants(Layer::Conn, Level::Decide));
        assert!(!wants(Layer::Disc, Level::Decide));
        assert_eq!(mask(), m);
        // 전부.
        let all = parse_layers("*");
        assert_eq!(all, mask_all());
        assert_eq!(parse_layers("on"), all);
        assert_eq!(parse_layers("ALL"), all);
        assert_eq!(all.count_ones(), 30, "층 10 × 수준 3");
        for l in Layer::ALL {
            assert!(layer_in_mask(all, l));
            assert_eq!(Layer::parse(l.label()), Some(l));
        }
        for v in Level::DETAIL {
            assert_eq!(Level::parse(v.label()), Some(v));
        }
        assert_eq!(Level::parse("basic"), None, "basic은 마스크 대상이 아니다");
        set_mask(0);
    }

    #[test]
    fn rec_renders_one_line_with_rules_and_outcome() {
        let r = Rec::new(Layer::Conn, Level::Decide, "start_connect")
            .arg("peer", "5016194e")
            .arg("auto", false)
            .arg("why", "Not Found")
            .rule("latch", "free")
            .ok("dial");
        assert_eq!(
            r.render(),
            "conn/decide start_connect peer=5016194e auto=false why=\"Not Found\" | rule latch=free | → dial"
        );
        assert!(!r.is_fail());
        let f = Rec::new(Layer::Xfer, Level::Decide, "offer")
            .arg("size", 1_048_576u64)
            .fail("NotPinned");
        assert_eq!(
            f.render(),
            "xfer/decide offer size=1048576 | FAIL NotPinned"
        );
        assert!(f.is_fail());
        // 결과 없음 · 빈 값·개행·따옴표 = 한 줄 유지.
        let n = Rec::new(Layer::Ui, Level::Trace, "paint")
            .arg("empty", "")
            .arg("q", "a\"b\nc");
        assert_eq!(n.render(), "ui/trace paint empty=\"\" q=\"a\\\"b␤c\"");
        assert!(!n.render().contains('\n'));
        assert_eq!((n.layer(), n.level()), (Layer::Ui, Level::Trace));
    }
}
