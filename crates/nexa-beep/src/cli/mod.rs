//! **CLI 모드** — 헤드리스 검증·수동 연결·인터랙티브 채팅·무해화 실측.
//!
//! GUI(`--window`)를 뺀 나머지 실행 모드. 각 모드는 같은 도메인 스택(발견→Noise→TOFU→
//! 다중화)을 쓰고 **프레젠테이션만 stdin/stdout**이다 — 그래서 GUI와 상호 운용된다
//! (예: `--chat-live`가 GUI 목록에 뜬다). 실행 절차는 [docs/26].

pub(crate) mod chat;
pub(crate) mod manual;
pub(crate) mod probe;
pub(crate) mod quarantine;
