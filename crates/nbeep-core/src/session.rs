//! `Session` — 인증된 보안 세션([docs/09] ADR-0003 · [docs/08] ADR-0002).
//!
//! [`Link`](crate::link::Link)(바이트 관) 위에 핸드셰이크를 얹어 **인증된 상대([`PeerId`])** 와
//! **논리 메시지**를 주고받는다. 구현:
//! - `NoiseSession`(실물 · Noise_XX · `nbeep-crypto` M2-1b) — AEAD 암호화.
//! - `PlainSession`(스텁 · `nbeep-crypto` testkit) — 암호화 없이 통과. 상위 계층을 먼저 검증한다.
//!
//! **암호화는 이 계층에 있다** — [`Link`](crate::link::Link)는 평문·암호문을 구분하지 않는다.
//! 전송(TCP·인메모리·릴레이)이 무엇이든 세션 구현은 하나다(ADR-0003).

use crate::identity::{PeerId, TrustLevel};
use crate::link::LinkError;

/// 세션 송수신·수립 오류.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SessionError {
    /// 링크가 닫힘.
    Closed,
    /// 핸드셰이크 실패(프로토콜 불일치·조작).
    Handshake,
    /// 프레임이 상한을 초과.
    TooLarge,
    /// 차단된 상대 — 수립 금지([`crate::trust`] · fail-closed).
    Blocked,
    /// 수신 큐 초과 — 무한 버퍼 대신 세션 종료([`crate::mux`] · fail-closed).
    Backpressure,
    /// 상대가 **내 신원과 같다** — 자기 연결 또는 **키 파일 복제**(D-22 U-P2 · [docs/21 §5]).
    SelfPeer,
    /// 수신 대기 시간 초과 — 오류가 아니라 "지금은 없음"(수신 펌프의 정상 경로).
    TimedOut,
}

impl From<LinkError> for SessionError {
    fn from(e: LinkError) -> Self {
        match e {
            LinkError::Closed => SessionError::Closed,
            LinkError::TimedOut => SessionError::TimedOut, // 재시도 대상(수신 펌프)
        }
    }
}

impl core::fmt::Display for SessionError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            SessionError::Closed => f.write_str("세션 링크가 닫힘"),
            SessionError::Handshake => f.write_str("핸드셰이크 실패"),
            SessionError::TooLarge => f.write_str("프레임 상한 초과"),
            SessionError::Blocked => f.write_str("차단된 상대"),
            SessionError::Backpressure => f.write_str("수신 큐 초과"),
            SessionError::SelfPeer => f.write_str("자기 신원과의 세션(키 복제 의심)"),
            SessionError::TimedOut => f.write_str("수신 타임아웃(재시도 대상)"),
        }
    }
}

impl std::error::Error for SessionError {}

/// 인증된 보안 세션 — 상위(대화·그룹·전송)가 보는 유일한 통신 경계.
pub trait Session: Send {
    /// **인증된** 상대. `Link`의 미검증 peer와 달리, 여기 값은 핸드셰이크로 확정된 것이다.
    fn peer(&self) -> PeerId;

    /// 상대 신뢰 상태(TOFU·SAS — [docs/08]).
    fn trust(&self) -> TrustLevel;

    /// 논리 메시지 하나 송신(구현이 암호화).
    ///
    /// # Errors
    /// 링크가 닫혔으면 [`SessionError::Closed`], 상한 초과면 [`SessionError::TooLarge`].
    fn send(&mut self, message: &[u8]) -> Result<(), SessionError>;

    /// 논리 메시지 하나 수신(블로킹·구현이 복호).
    ///
    /// # Errors
    /// 링크가 닫혔으면 [`SessionError::Closed`].
    fn recv(&mut self) -> Result<Vec<u8>, SessionError>;

    /// 수신 폴 타임아웃 — `Some(d)`면 `recv`가 `d`마다 [`SessionError::TimedOut`]으로 돌아온다
    /// (비동기 수신 펌프가 송신과 교대 — M2-7). 기본 no-op(블로킹 유지).
    fn set_recv_timeout(&mut self, _dur: Option<core::time::Duration>) {}

    /// 핸드셰이크 바인딩 — `(핸드셰이크 해시, 내가 개시자인가)`. 세션 뒤에 오가는
    /// 증명(ADR-0015 형제 증명 — 세션 안에서 PSK 소유를 HMAC으로)을 **이 세션에 묶는**
    /// 재료다. 해시가 없는 구현(테스트 더블·릴레이 원시)은 `None` = 증명 불가.
    fn handshake_binding(&self) -> Option<([u8; 32], bool)> {
        None
    }
}

/// 세션의 **수신 반쪽**(LAT-1 · [docs/55] §2) — 읽기 루프 하나만 들고 있다(잠금 없음 · recv nonce는
/// 단일 소비자).
pub trait RecvHalf: Send {
    /// 논리 메시지 하나 수신(블로킹 · 구현이 복호).
    ///
    /// # Errors
    /// 링크가 닫혔으면 [`SessionError::Closed`] · 폴 타임아웃이면 [`SessionError::TimedOut`].
    fn recv(&mut self) -> Result<Vec<u8>, SessionError>;
    /// 수신 폴 타임아웃 — [`Session::set_recv_timeout`]과 같다. 기본 no-op.
    fn set_recv_timeout(&mut self, _dur: Option<core::time::Duration>) {}
}

/// 세션의 **송신 반쪽** — `&self`라 여러 스레드가 나눠 쓴다(쓰기 스레드 + 파일 펌프). 구현은
/// **암호화와 링크 송신을 한 잠금 안에서** 해 와이어 순서 = nonce 순서를 지킨다(§2-1 불변식 —
/// 깨지면 상대가 복호 실패로 세션을 끊는다).
pub trait SendHalf: Send + Sync {
    /// 논리 메시지 하나 송신(구현이 암호화).
    ///
    /// # Errors
    /// 링크가 닫혔으면 [`SessionError::Closed`], 상한 초과면 [`SessionError::TooLarge`].
    fn send(&self, message: &[u8]) -> Result<(), SessionError>;
}

/// 분리된 두 반쪽(수신 · 송신).
pub type Halves = (Box<dyn RecvHalf>, Box<dyn SendHalf>);

/// 송수신을 두 반쪽으로 나눌 수 있는 세션. 링크가 핸들 복제를 못 하면 `Err(self)`로 **그대로 돌려줘**
/// 호출자가 단일 루프로 폴백한다(분리는 최적화 — 기능은 어느 쪽이든 같다).
pub trait Splittable: Session + Sized {
    /// 수신 반쪽·송신 반쪽으로 분리.
    ///
    /// # Errors
    /// 분리 불가면 `Err(self)`(세션은 손상 없이 그대로).
    #[allow(clippy::result_large_err)] // Err = 세션 자체(손상 없이 되돌려 줌)가 계약이다
    fn split(self) -> Result<Halves, Self>;
}
