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

    /// 수신 폴 타임아웃 설정 — `Some(d)`면 `d`마다 [`LinkError::TimedOut`]("지금은 없음")으로
    /// 돌아온다(수신 펌프가 송신과 교대할 수 있게). 기본 no-op(블로킹 유지 — fake·InMemory).
    ///
    /// # Errors
    /// 소켓 옵션 실패 시 [`LinkError::Closed`].
    fn set_recv_timeout(&mut self, _dur: Option<core::time::Duration>) -> Result<(), LinkError> {
        Ok(())
    }

    /// 실소켓 상대 IP — **경로 등급([`crate::path::PathClass`]) 판정 전용**(ADR-0006 §5-1-5:
    /// 등급은 광고가 아니라 성립한 세션의 실주소가 정한다). 소켓이 아닌 링크(인메모리·fake)는
    /// `None` = 로컬 취급. 표시·라우팅 등 다른 용도로 쓰지 않는다(주소는 전송 계층 내부 사정).
    fn remote_ip(&self) -> Option<std::net::IpAddr> {
        None
    }

    /// **랑데부 표식**(X-2 Transport 승격 · 11-06 · **표시 전용** — 헤더·목록 배지 `via_server` · DR-28 · M3-27) —
    /// `true` = 이 링크는 서버 랑데부 사다리로 성립했다(홀펀칭 UDP 직결·릴레이 중계 모두). 경로 **등급**은 여전히
    /// [`Self::remote_ip`]가 정한다(이 값은 등급이 아니라 "어느 사다리로 왔나"의 표시 근거). 기본 `false`(LAN·수동·인메모리).
    fn rendezvous(&self) -> bool {
        false
    }

    /// **송신 핸들 분리**(LAT-1 · [docs/55] §2-3) — 같은 연결의 두 번째 핸들을 돌려준다. 그 뒤
    /// `self`는 **수신 전용**, 돌려준 것은 **송신 전용**으로 쓴다(둘 다 `Link`지만 역할을 나눈다 ·
    /// 수신 타임아웃은 `self`에만 건다). 핸들을 복제할 수 없는 링크는 `None` = 호출자가 종전
    /// 단일 루프로 폴백한다(기본 — UDP·릴레이·fake).
    fn split_send(&mut self) -> Option<Box<dyn Link>> {
        None
    }
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
    fn set_recv_timeout(&mut self, dur: Option<core::time::Duration>) -> Result<(), LinkError> {
        (**self).set_recv_timeout(dur)
    }
    fn remote_ip(&self) -> Option<std::net::IpAddr> {
        (**self).remote_ip()
    }
    fn rendezvous(&self) -> bool {
        (**self).rendezvous()
    }
    fn split_send(&mut self) -> Option<Box<dyn Link>> {
        (**self).split_send()
    }
}
