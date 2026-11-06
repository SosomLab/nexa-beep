//! 전송 경계 — [`Transport`] 트레이트와 그 주변 타입([docs/09] ADR-0003 §3).
//!
//! `Transport`는 **발견하고 [`Link`]를 만들어 주는 것**이다. 상위 계층은 이 경계 위에서만 동작하므로,
//! LocalDirect(v1)·InMemory(테스트)·Relay(X-2 · `nbeep-relay`의 `RelayTransport`/`LadderTransport`)를 갈아 끼워도 App·Session 코드는 바뀌지 않는다.

use nbeep_core::link::Link;
use nbeep_core::{DisplayName, PeerId};
use std::sync::mpsc::Receiver;

/// 인바운드가 들어온 통로(BL-2 · 11-05) — 필터가 통로별로 다르게 셀 수 있게(accept 폭주 계수는 TCP만).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum InboundKind {
    /// 세션 TCP accept(핸드셰이크 전).
    Accept,
    /// 발견 UDP 관측(패킷 파싱 뒤 · 반영 전).
    Discovery,
}

/// 인바운드 IP 판정자([docs/38] B-2) — `true` = **받지 않는다**(TCP는 즉시 close · 발견은 폐기). 소켓 스레드에서 불린다.
pub type InboundFilter =
    std::sync::Arc<dyn Fn(std::net::IpAddr, InboundKind) -> bool + Send + Sync>;

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
    /// 세션 세대(M1-13 ⓒ · [`nbeep_core::SESSION_GEN`]) — 0 = 구버전 광고 · [`nbeep_core::FOREIGN_GEN`] = 미래 와이어(비호환 표시).
    pub gen: u8,
    /// 출처(X-2f · ADR-0003 §9 B) — Local = LAN 광고 · Rendezvous = 서버 명부. **Locator가 아니다**(주소·포트 없음 · 표시·수명 판정 전용).
    pub origin: nbeep_core::HintOrigin,
    /// 수명 방식(X-2f · §9 D) — PeerTable 만료 규칙 선택(Advertised = TTL · Eventful = Withdrawn으로만).
    pub liveness: nbeep_core::Liveness,
    /// 관측 경로 토큰(불투명 · PeerTable `SourceId`) — LAN = 0 · 랑데부 = 세대별 1+(서버 재접속마다 다른 값).
    pub source: u32,
    /// 공개 카드(랑데부 전용 · 경계에서 정제 완료 · ADR-0003 §9 A) — LAN 광고는 None.
    pub profile: Option<nbeep_core::PublicCard>,
}

/// 발견 이벤트 — 등장·이탈([docs/06] FR-D-8).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DiscoveryEvent {
    /// 새 상대가 목록에 등장.
    Appeared(PeerHint),
    /// 상대가 목록에서 사라짐(goodbye·타임아웃) — LAN 경로(`source` 0) 의미 그대로.
    Vanished(PeerId),
    /// **경로 하나**의 이탈(X-2f · §9 C) — 랑데부 명부 Down · 공개 해제 · 서버 접속 해제(일괄). 호스트는 `goodbye(peer, SourceId(source))`로
    /// 접는다 — 다른 경로(LAN)가 살아 있으면 행은 남고 출처만 바뀐다.
    Withdrawn { peer: PeerId, source: u32 },
    /// ★ 사용자 힌트(ADR-0015) — 상대가 낸 LAN_tag. 내 태그 셋과 같으면 "형제 후보"(미검증 ·
    /// 판정은 XXpsk3). 태그가 다르면 무시.
    UserHint { peer: PeerId, tag: [u8; 16] },
}

/// `connect` 실패.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ConnectError {
    /// 대상 `PeerId`를 아는 경로가 없다(발견 안 됨·이탈 · 랑데부 서버에도 등록 안 됨).
    Unreachable,
    /// 랑데부 경로가 **상한·거절**로 열리지 않았다(X-2 · 11-06) — 재시도 가치 있음(백오프).
    Busy,
    /// 랑데부 경로의 서버 세션이 **죽었다/시간 초과**(X-2) — 호스트가 서버 재접속을 수렴한 뒤 재시도.
    Lost,
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

/// 발견하고 [`Link`]를 만들어 주는 것(ADR-0003 §3). 구현: LocalDirect(v1) · InMemory(테스트) · Relay(X-2 ✅ 11-06 —
/// `nbeep-relay`의 `RelayTransport` + 셋을 묶는 `LadderTransport` · 사다리 = 로컬 > 수동 > 릴레이 · 핸드셰이크는 전부 호출자).
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

    /// **수동 주소를 `peer`의 경로 후보로 기억**(DR-19 ④ · ADR-0003 §4 "로컬 > 수동 > 릴레이" · X-2 11-06) — 호스트가
    /// 수동 등록으로 성립해 **신원을 확정한 뒤** 부른다. 이후 [`Self::connect`]가 발견 경로 다음 단으로 이 주소를 시도한다.
    /// 기본 no-op(사다리가 없는 전송 — 호출자는 종전처럼 `add_endpoint` 폴백을 직접 해도 된다).
    fn note_endpoint(&self, _peer: PeerId, _addr: &str) {}

    /// 기억한 수동 주소 제거(R-20 — 그 주소에서 **다른 키**가 성립하면 즉시 무효). 기본 no-op.
    fn forget_endpoint(&self, _peer: PeerId) {}

    /// L1 링크 변화 통지(M1-2) — 호스트가 OS 구독(디바운스 후)으로 부른다.
    /// 전송은 재발견(그룹 재조인·즉시 HELLO 등)으로 반응한다. 기본 = no-op
    /// (InMemory 등 링크 개념이 없는 전송).
    fn link_changed(&self) {}

    /// 이 전송이 광고하는 능력.
    fn caps(&self) -> Caps;

    /// 표시 이름 교체(M1-10 · FR-S-50) — 발견 광고가 있는 전송은 **즉시 재공지**해
    /// 상대 목록이 갱신되게 한다. 기본 = 아무 것도 안 함(발견 없는 전송·데모).
    fn set_display_name(&self, _name: DisplayName) {}

    /// ★ 사용자 힌트 태그 발신(ADR-0015 §3-4) — `Some` = 광고마다 `UserHint` 패킷을 같이 낸다 ·
    /// `None` = 내지 않는다(사용자 기능 꺼짐·미인증). 기본 no-op(인메모리 등).
    fn set_user_tag(&self, _tag: Option<[u8; 16]>) {}

    /// ★ 조용한 모드(내 상태 = 오프라인 · 10-19) — `true` = 발견 광고·응답·관측 중단(상대에게 보이지
    /// 않고 상대도 보지 않는다) · `false` = 재개 + 즉시 재발견. 세션 수신 소켓은 열어 두되 수락은 호스트가
    /// 거른다. 기본 no-op(인메모리 등).
    fn set_quiet(&self, _quiet: bool) {}

    /// ★ 인바운드 IP 필터([docs/38] B-2 · BL-2 · 11-05) — 호스트가 넣은 판정자를 **소켓 직후 · 파싱/핸드셰이크 전**에
    /// 적용한다: TCP accept 즉시 close(Noise 비용 자체를 차단 · 오류 회신 없음) · 발견 관측은 반영 전 폐기(그림자 —
    /// 내 광고 응답은 평소대로). 기본 no-op(인메모리 등).
    fn set_inbound_filter(&self, _filter: InboundFilter) {}
}
