//! 전송 경계 — [`Transport`] 트레이트와 그 주변 타입([docs/09] ADR-0003 §3).
//!
//! `Transport`는 **발견하고 [`Link`]를 만들어 주는 것**이다. 상위 계층은 이 경계 위에서만 동작하므로,
//! LocalDirect(v1)·InMemory(테스트)·Relay(v2)를 갈아 끼워도 App·Session 코드는 바뀌지 않는다.

use nbeep_core::link::Link;
use nbeep_core::{DisplayName, PeerId};
use std::sync::mpsc::Receiver;

/// 전송별 도달 경로. **이 크레이트 밖으로 나가지 않는다**(ADR-0003 규칙 1).
///
/// 상위 계층은 `PeerId`로만 상대를 가리키고, `PeerId → Locator` 해석은 전송 내부에서만 한다.
#[derive(Clone, Debug, PartialEq, Eq)]
#[allow(dead_code)] // LocalDirect/Manual/Relay 변형은 M1-2+/M5/v2에서 채운다.
pub(crate) enum Locator {
    /// 인메모리 버스상의 논리 주소(테스트).
    InMemory(u64),
    // LocalDirect { ip, port, if_index } — M1-2~4
    // Manual { host, port }             — M5 · ADR-0006
    // Relay { server, peer }            — v2 · nexa-beepd
}

/// 전송이 광고하는 능력 — App이 기능 노출을 조정하는 근거(ADR-0003 §3 `capabilities()`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Caps {
    /// 오프라인 큐(상대 부재 시 서버 보관)를 전송이 제공하는가. 로컬 직접·인메모리는 `false`.
    pub offline_queue: bool,
}

/// 발견 결과 — **미검증**([docs/08]). `PeerId`가 곧 상대 식별자지만, 신원은 세션 성립 전까지 주장되지 않는다.
///
/// ⚠️ `Locator`는 담지 않는다 — 경로는 전송 내부에만 있다(규칙 1). App은 `peer`로 `connect`를 요청할 뿐이다.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PeerHint {
    /// 미검증 상대 식별자.
    pub peer: PeerId,
    /// 무해화된 표시 이름([`DisplayName`]).
    pub name: DisplayName,
    /// 상대가 광고한 능력 플래그.
    pub caps: Caps,
}

/// 발견 이벤트 — 등장·이탈([docs/06] FR-D-8).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DiscoveryEvent {
    /// 새 상대가 목록에 등장.
    Appeared(PeerHint),
    /// 상대가 목록에서 사라짐(goodbye·타임아웃).
    Vanished(PeerId),
}

/// `connect` 실패.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ConnectError {
    /// 대상 `PeerId`를 아는 경로가 없다(발견 안 됨·이탈).
    Unreachable,
}

/// 수동 엔드포인트 등록 실패(DR-19 · ADR-0006 §3).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AddError {
    /// 주소 형식 오류(host:port 아님·해석 불가).
    BadAddress,
    /// 지금 도달 불가(오프라인일 수 있음 — 등록은 남겨도 된다).
    Unreachable,
    /// 이 전송은 수동 등록을 지원하지 않는다(InMemory 등).
    Unsupported,
}

/// 발견하고 [`Link`]를 만들어 주는 것(ADR-0003 §3). 구현: LocalDirect(v1) · InMemory(테스트) · Relay(v2).
pub trait Transport: Send + Sync {
    /// 발견 이벤트 스트림(등장·이탈). 호출자는 이 수신단을 소유해 폴링한다.
    fn discovery(&self) -> Receiver<DiscoveryEvent>;

    /// 수신 연결 스트림 — 상대가 우리에게 연결해 올 때의 `Link`.
    fn incoming(&self) -> Receiver<Box<dyn Link>>;

    /// `peer`로 연결을 시도해 `Link`를 반환한다. **어떤 경로를 썼는지는 밖으로 알리지 않는다**.
    ///
    /// # Errors
    /// 도달 경로가 없으면 [`ConnectError::Unreachable`].
    fn connect(&self, peer: PeerId) -> Result<Box<dyn Link>, ConnectError>;

    /// **수동 엔드포인트 등록**(DR-19 · ADR-0006) — 발견 없이 주소로 직접 연결한다.
    /// 연결·핸드셰이크로 **`PeerId`를 확정**해 돌려준다(주소는 힌트, 신원은 지문 — DR-8).
    /// 이미 아는 노드면 경로만 병합된다. 기본 = 미지원([`AddError::Unsupported`]).
    ///
    /// # Errors
    /// 주소 오류·도달 불가·미지원 시 [`AddError`].
    fn add_endpoint(&self, _addr: &str) -> Result<Box<dyn Link>, AddError> {
        Err(AddError::Unsupported)
    }

    /// 이 전송이 광고하는 능력.
    fn caps(&self) -> Caps;
}
