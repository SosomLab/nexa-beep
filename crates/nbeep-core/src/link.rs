//! `Link` — 신뢰·순서 있는 양방향 바이트 스트림(핵심 추상화).
//!
//! **암호화를 모른다** — 그냥 바이트 관이다. [`crate::session::Session`](crate::session)이 이 위에
//! Noise 핸드셰이크·다중화를 얹고, `nbeep-net`의 전송이 이걸 만든다(TCP·인메모리 …).
//!
//! ⚠️ **왜 core에 있나** — Session은 *"Link만 알고 Transport는 모른다"*([docs/09] ADR-0003).
//! `Link`를 core에 두면 `nbeep-crypto`(Session)가 `nbeep-net`(Transport)에 **의존하지 않아도** 된다.
//! 규칙을 크레이트 경계로 강제하는 것이다.

use crate::identity::PeerId;

/// [`Link`] 송수신 오류.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LinkError {
    /// 상대가 링크를 닫음(정상 종료 포함).
    Closed,
    /// 수신 대기 시간 초과 — 오류가 아니라 "지금은 없음"(수신 펌프의 폴링 경로).
    TimedOut,
}

/// 신뢰·순서 있는 양방향 바이트 스트림. 프레임 단위로 주고받는다.
pub trait Link: Send {
    /// 이 링크가 향하는 **(미검증)** 상대. 신원은 세션 핸드셰이크로만 확정된다([docs/08]).
    fn peer(&self) -> PeerId;

    /// 프레임 하나 송신.
    ///
    /// # Errors
    /// 상대가 닫았으면 [`LinkError::Closed`].
    fn send(&mut self, frame: &[u8]) -> Result<(), LinkError>;

    /// 프레임 하나 수신(블로킹).
    ///
    /// # Errors
    /// 상대가 닫았으면 [`LinkError::Closed`].
    fn recv(&mut self) -> Result<Vec<u8>, LinkError>;
}

impl Link for Box<dyn Link> {
    fn peer(&self) -> PeerId {
        (**self).peer()
    }
    fn send(&mut self, frame: &[u8]) -> Result<(), LinkError> {
        (**self).send(frame)
    }
    fn recv(&mut self) -> Result<Vec<u8>, LinkError> {
        (**self).recv()
    }
}
