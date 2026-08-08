//! `ActionKind` — **모든 유의미한 행위를 표현하는 단일 통로**([docs/13] §3).
//!
//! 로그·검증·계측·정책을 각 기능에 흩뿌리면 반드시 빠뜨리고, 빠뜨린 곳이 곧 감사 구멍이 된다.
//! 그래서 의미 있는 행위를 전부 이 열거로 표현하고 **하나의 파이프라인**([`crate::pipeline`])을 통과시킨다.
//!
//! **새 기능 추가의 첫 단계 = 여기에 항목을 추가하는 것.** 핸들러 안에 `log!`/`meter!`를 쓰고 싶어지면
//! `ActionKind`가 부족한 것이다. `u16` 안정 코드 매핑(`stable_code`)은 M0-1c에서 추가한다.

use crate::identity::TrustLevel;

/// 발견 단계([docs/06] S1~S6). 계측·로그의 문맥.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum DiscoveryTier {
    /// S1 IPv6 링크로컬 멀티캐스트.
    Ipv6Multicast,
    /// S2 IPv4 멀티캐스트.
    Ipv4Multicast,
    /// S3 UDP 브로드캐스트.
    Broadcast,
    /// S4 이웃 테이블(ARP/NDP) 유니캐스트 프로브.
    NeighborProbe,
    /// S5 서브넷 스캔(기본 꺼짐).
    SubnetScan,
    /// S6 수동 엔드포인트 등록([docs/19] ADR-0006).
    ManualEndpoint,
}

/// 수신 파일 위험 등급([docs/11] ADR-0004 · [docs/04]).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum RiskLevel {
    /// 🔴 실행형(exe·bat·ps1·lnk·app·sh …) — 최고 마찰.
    Executable,
    /// 🟠 능동 콘텐츠 문서(매크로 Office·PDF·iso/vhd).
    ActiveDocument,
    /// 🟡 아카이브(zip·7z …) — 자동 해제 금지.
    Archive,
    /// 🟢 데이터(이미지·텍스트·일반 문서).
    Data,
}

/// 검사 결과([docs/11] §6). **"검사 통과 = 안전"이라고 표기하지 않는다** — 사실만.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ScanOutcome {
    /// 검사 못 함(백신 미설치·미지원) — 마찰을 한 단계 올린다.
    Unavailable,
    /// 검사됨 · 탐지 없음.
    Clean,
    /// 검사됨 · 탐지됨.
    Detected,
}

/// 이미지 격리 디코드 거부 사유([docs/04] §5).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum RejectReason {
    /// 지원하지 않는 형식(SVG 등).
    UnsupportedFormat,
    /// 픽셀·메모리·시간 상한 초과.
    LimitExceeded,
    /// 디코드 실패(손상·악성).
    DecodeFailed,
}

/// **사용자가 인지할 수 있는 단위 행위 하나.**
///
/// 항목이 늘 때 `stable_code` 매핑도 함께 늘린다(부여 후 불변 — M0-1c에서 추가).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum ActionKind {
    // ── 발견 ─────────────────────────────
    /// 발견 라운드 시작(단계별).
    DiscoveryRoundStarted(DiscoveryTier),
    /// 피어가 목록에 등장.
    PeerAppeared,
    /// 피어가 목록에서 사라짐(goodbye·타임아웃).
    PeerVanished,

    // ── 세션 ─────────────────────────────
    /// Noise 핸드셰이크 시도.
    HandshakeAttempted,
    /// TOFU로 신뢰 고정(최초 접촉).
    TrustPinned,
    /// 지문 불일치로 차단(롤백·사칭 의심).
    TrustMismatchBlocked,
    /// SAS 지문 대조 완료.
    FingerprintVerified,

    // ── 대화 ─────────────────────────────
    /// 메시지 송신.
    MessageSent,
    /// 메시지 수신.
    MessageReceived,
    /// 그룹 팬아웃(대상 수는 [`crate::ports::Quantity::peers`]).
    GroupMessageFannedOut,
    /// 전체 브로드캐스트 공지.
    BroadcastSent,

    // ── 전송 ─────────────────────────────
    /// 파일 전송 제안(메타만).
    TransferOffered,
    /// 수신자가 전송 수락.
    TransferAccepted,
    /// 전송 완료.
    TransferCompleted,

    // ── 안전(수신 무해화) ────────────────
    /// 수신물 격리(`.beepq`).
    FileQuarantined(RiskLevel),
    /// 검사 완료.
    FileInspected(ScanOutcome),
    /// 사용자 승인.
    FileApproved(RiskLevel),
    /// 실체화(승인 후 원본 복원).
    FileMaterialized,
    /// 거부.
    FileRejected,
    /// 보존 기간 만료 삭제.
    FileExpired,
    /// 이미지 디코드 성공(재인코딩본).
    ImageDecoded,
    /// 이미지 디코드 거부.
    ImageDecodeRejected(RejectReason),

    // ── 정책 ─────────────────────────────
    /// 속도 제한 발동.
    RateLimited,
    /// 발신자 차단.
    SenderBlocked,
}

/// 행위 거부 코드(정책 계층) — 계측·사용자 표시에 쓰이므로 **부여 후 불변**.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum RejectCode {
    /// 속도 제한.
    RateLimited,
    /// 차단 목록.
    Blocked,
    /// 신뢰 불일치(TOFU).
    TrustMismatch,
    /// 검증 실패(무해화·불변식).
    ValidationFailed,
    /// 미확인 발신자(승인 대기).
    Unapproved,
}

/// 행위 실패 코드(인프라 계층) — 예상 밖 오류. **부여 후 불변**.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum FailCode {
    /// I/O 오류.
    Io,
    /// 디코드·파싱 오류.
    Decode,
    /// 시간 초과.
    Timeout,
    /// 자원 상한 초과.
    ResourceLimit,
}

/// 행위 결과 — 파이프라인·계측이 공유([docs/13] §6-2 "결과").
///
/// **거부(Rejected)와 실패(Failed)를 구분한다** — 거부는 정책이 막은 것, 실패는 예상 밖 오류.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Outcome {
    /// 정상 수행.
    Ok,
    /// 정책이 막음(속도 제한·차단·신뢰 불일치 …).
    Rejected(RejectCode),
    /// 예상 밖 오류(I/O·디코드 …).
    Failed(FailCode),
}

impl Outcome {
    /// 정상 수행 여부.
    #[must_use]
    pub fn is_ok(&self) -> bool {
        matches!(self, Outcome::Ok)
    }
}

/// 신뢰 등급이 위험 등급과 만날 때의 마찰 판단 보조.
///
/// 미검증 발신자의 실행형 파일이 가장 강한 경고를 받아야 한다([docs/11] §7).
#[must_use]
pub fn requires_extra_friction(trust: TrustLevel, risk: RiskLevel) -> bool {
    matches!(trust, TrustLevel::Unverified)
        && matches!(risk, RiskLevel::Executable | RiskLevel::ActiveDocument)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn outcome_ok_only_for_ok() {
        assert!(Outcome::Ok.is_ok());
        assert!(!Outcome::Rejected(RejectCode::RateLimited).is_ok());
        assert!(!Outcome::Failed(FailCode::Io).is_ok());
    }

    #[test]
    fn unverified_executable_needs_friction() {
        assert!(requires_extra_friction(
            TrustLevel::Unverified,
            RiskLevel::Executable
        ));
        assert!(requires_extra_friction(
            TrustLevel::Unverified,
            RiskLevel::ActiveDocument
        ));
        // 검증된 상대의 데이터 파일은 추가 마찰 없음.
        assert!(!requires_extra_friction(
            TrustLevel::FingerprintVerified,
            RiskLevel::Data
        ));
        // 미검증이어도 데이터는 최고 마찰 대상이 아님.
        assert!(!requires_extra_friction(
            TrustLevel::Unverified,
            RiskLevel::Data
        ));
    }
}
