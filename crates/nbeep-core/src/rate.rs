//! **전송 속도 제어** — 자동 측정(최고 속도의 50%) · 상한 설정 · **쌍방 협상**.
//!
//! 사용자 확정(08-09): *"네트워크 대역폭을 점유하지 않도록 최고 속도의 50% 수준을 자동
//! 설정"* · 송·수신 각각 상한 지정(자동/100KB/1MB/10MB/100MB/1GB/사용자 지정) ·
//! **쌍방 간 최고 속도를 창(window) 크기로 제약**.
//!
//! ## 세 조각
//!
//! | 조각 | 역할 |
//! |---|---|
//! | [`RateMeter`] | 실제 처리량을 재서 **관측 최고치**를 남긴다 → 자동값의 근거 |
//! | [`Pacer`] | 목표 속도에 맞춰 **다음 청크까지 기다릴 시간**을 계산(토큰 버킷) |
//! | [`negotiate`] | 내 상한과 상대 상한 중 **낮은 쪽**을 택한다(둘 다 무제한이면 무제한) |
//!
//! 시계를 직접 읽지 않는다 — 호스트가 `now_ms`를 주입한다(테스트 결정성 · 이 저장소 공통 규약).
//!
//! ⚠️ **자동값의 정직한 한계**: 관측 최고치는 "지금까지 실제로 낸 속도"이지 링크 용량이
//! 아니다. 상대가 느리거나 파일이 작으면 과소 추정되고, 그만큼 보수적으로 (느리게) 잡힌다.
//! 그래서 자동값에는 [`AUTO_FLOOR_BPS`] 하한을 둔다 — 한 번 느렸다고 영영 기어가지 않게.

/// 자동 모드가 목표로 삼는 비율(관측 최고치의 50% — 사용자 확정).
pub const AUTO_SHARE: f64 = 0.5;

/// 자동 모드 하한(256 KiB/s) — 초기 관측이 작을 때 과소 추정 방지.
pub const AUTO_FLOOR_BPS: u64 = 256 * 1024;

/// 속도 상한 설정.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum RateLimit {
    /// 자동 — 관측 최고치의 [`AUTO_SHARE`]배(하한 [`AUTO_FLOOR_BPS`]).
    #[default]
    Auto,
    /// 초당 바이트 고정 상한(0 = 무제한).
    PerSec(u64),
}

impl RateLimit {
    /// 설정 값 코드 → 상한. 미지 코드는 숫자(바이트/초)로 해석하고, 실패하면 자동.
    #[must_use]
    pub fn from_code(s: &str) -> Self {
        match s {
            "auto" => Self::Auto,
            "100k" => Self::PerSec(100 * 1024),
            "1m" => Self::PerSec(1024 * 1024),
            "10m" => Self::PerSec(10 * 1024 * 1024),
            "100m" => Self::PerSec(100 * 1024 * 1024),
            "1g" => Self::PerSec(1024 * 1024 * 1024),
            other => other.parse::<u64>().map_or(Self::Auto, Self::PerSec),
        }
    }

    /// 실제 목표 속도(B/s) — 자동이면 계측기에서 구한다. 0 = 무제한.
    #[must_use]
    pub fn target_bps(self, meter: &RateMeter) -> u64 {
        match self {
            Self::Auto => meter.auto_target(),
            Self::PerSec(v) => v,
        }
    }

    /// **상대에게 공지할 상한**(M4-11 · 08-16) — [`Self::target_bps`]와 다르다:
    /// 그건 *내 발신 페이싱 목표*고, 이건 *상대에게 주장하는 내 수신 상한*이다.
    /// Auto = **0(상한 무주장)** — 종전엔 관측 없는 수신측이 하한(256KiB/s)을
    /// 공지해 쌍방 negotiate가 하한에 고착됐다(로컬 0.25MB/s 실기). 자동 모드의
    /// 뜻은 "내가 수신 대역을 조절하지 않는다 — 발신자가 자기 관측의 절반으로
    /// 알아서 양보한다"이므로 상한을 주장하지 않는 게 맞다. 명시(PerSec)만 주장.
    #[must_use]
    pub fn advertised_cap(self) -> u64 {
        match self {
            Self::Auto => 0,
            Self::PerSec(v) => v,
        }
    }
}

/// 처리량 계측 — 관측 최고치(B/s)를 남긴다.
///
/// 짧은 구간(기본 500ms)마다 속도를 재고 최고치를 갱신한다. 구간을 짧게 잡는 이유는
/// **전체 평균이 아니라 "낼 수 있었던 속도"** 를 알고 싶기 때문이다(대기 시간이 섞이면
/// 평균은 링크 능력을 과소 평가한다).
#[derive(Clone, Copy, Debug)]
pub struct RateMeter {
    window_ms: u64,
    win_start_ms: u64,
    win_bytes: u64,
    peak_bps: u64,
    total_bytes: u64,
}

impl Default for RateMeter {
    fn default() -> Self {
        Self::new(500)
    }
}

impl RateMeter {
    /// 구간 길이(ms)로 만든다.
    #[must_use]
    pub fn new(window_ms: u64) -> Self {
        Self {
            window_ms: window_ms.max(1),
            win_start_ms: 0,
            win_bytes: 0,
            peak_bps: 0,
            total_bytes: 0,
        }
    }

    /// 구간 실측을 peak에 **직접** 반영(M4-11 후속 · 08-16) — 무페이싱 프로브가
    /// 관측 창(500ms)보다 빨리 끝나면(localhost 2MiB ≈ 수십 ms) 창이 한 번도 안
    /// 닫힌 채 재산출이 일어나 peak=0 → 하한 조임이 재발한다(실기: 1차 전송
    /// 316KiB/s · 2차만 2.4MiB/s — 1차의 peak가 500ms 뒤에야 기록된 탓).
    /// 경계 시점에 프로브 구간(bytes/dur)을 즉시 peak 후보로 넣는다.
    pub fn note_burst(&mut self, bytes: u64, dur_ms: u64) {
        let bps = bytes.saturating_mul(1000) / dur_ms.max(1);
        self.peak_bps = self.peak_bps.max(bps);
    }

    /// 이미 계산된 속도(B/s)를 peak 후보로 반영(08-16) — 액터 세션의 실측을
    /// GUI 집계기로 전달할 때 쓴다(집계기는 바이트 흐름을 직접 보지 못한다).
    pub fn note_peak(&mut self, bps: u64) {
        self.peak_bps = self.peak_bps.max(bps);
    }

    /// 전송/수신한 바이트를 기록한다.
    pub fn observe(&mut self, bytes: u64, now_ms: u64) {
        if self.total_bytes == 0 && self.win_bytes == 0 {
            self.win_start_ms = now_ms;
        }
        self.win_bytes += bytes;
        self.total_bytes += bytes;
        let elapsed = now_ms.saturating_sub(self.win_start_ms);
        if elapsed >= self.window_ms {
            let bps = self.win_bytes.saturating_mul(1000) / elapsed.max(1);
            self.peak_bps = self.peak_bps.max(bps);
            self.win_start_ms = now_ms;
            self.win_bytes = 0;
        }
    }

    /// 관측 최고 속도(B/s · 아직 없으면 0).
    #[must_use]
    pub fn peak_bps(&self) -> u64 {
        self.peak_bps
    }

    /// 누적 바이트.
    #[must_use]
    pub fn total(&self) -> u64 {
        self.total_bytes
    }

    /// 자동 목표(B/s) — 관측 최고치의 절반, 하한 [`AUTO_FLOOR_BPS`].
    /// 관측이 아직 없으면 하한값으로 시작한다(무제한으로 내달리지 않는다).
    #[must_use]
    pub fn auto_target(&self) -> u64 {
        #[allow(clippy::cast_precision_loss, clippy::cast_sign_loss)]
        let half = (self.peak_bps as f64 * AUTO_SHARE) as u64;
        half.max(AUTO_FLOOR_BPS)
    }
}

/// **쌍방 협상** — 내 상한과 상대 상한 중 낮은 쪽. 0은 "무제한"을 뜻한다.
///
/// 한쪽만 무제한이면 다른 쪽 값이 이긴다(무제한이 0이라 단순 min을 쓰면 안 된다).
#[must_use]
pub fn negotiate(local_bps: u64, remote_bps: u64) -> u64 {
    match (local_bps, remote_bps) {
        (0, r) => r,
        (l, 0) => l,
        (l, r) => l.min(r),
    }
}

/// 토큰 버킷 페이서 — 목표 속도를 넘지 않도록 **다음 청크까지 기다릴 시간**을 준다.
///
/// 창(window) 개념: 한 번에 흘려보낼 수 있는 양이 [`Pacer::burst_bytes`]로 제한되고,
/// 그 창이 시간에 비례해 다시 열린다. 결과적으로 평균 처리량이 목표치에 수렴한다.
#[derive(Clone, Copy, Debug)]
pub struct Pacer {
    target_bps: u64,
    /// 버킷 잔량(바이트).
    tokens: f64,
    /// 최대 축적(= 창 크기).
    burst: f64,
    last_ms: u64,
    started: bool,
}

impl Pacer {
    /// 목표 속도(B/s)로 만든다. 0 = 무제한(항상 대기 0).
    /// 창 크기는 목표의 0.25초분(최소 32 KiB) — 너무 크면 순간 폭주, 너무 작으면 잘게 끊긴다.
    #[must_use]
    pub fn new(target_bps: u64) -> Self {
        #[allow(clippy::cast_precision_loss)]
        let burst = ((target_bps / 4).max(32 * 1024)) as f64;
        Self {
            target_bps,
            tokens: burst,
            burst,
            last_ms: 0,
            started: false,
        }
    }

    /// 목표 속도(B/s).
    #[must_use]
    pub fn target_bps(&self) -> u64 {
        self.target_bps
    }

    /// 창 크기(바이트) — 한 번에 흘려보낼 수 있는 최대량.
    #[must_use]
    pub fn burst_bytes(&self) -> u64 {
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        let b = self.burst as u64;
        b
    }

    /// `bytes`를 보내기 전에 **기다려야 할 시간(ms)**. 0이면 바로 보내도 된다.
    /// 호출자는 반환값만큼 쉰 뒤 다시 호출하지 않고 그대로 보내면 된다(토큰은 이미 차감).
    pub fn take(&mut self, bytes: u64, now_ms: u64) -> u64 {
        if self.target_bps == 0 {
            return 0; // 무제한
        }
        if !self.started {
            self.started = true;
            self.last_ms = now_ms;
        }
        // 경과 시간만큼 토큰 보충.
        let dt = now_ms.saturating_sub(self.last_ms);
        self.last_ms = now_ms;
        #[allow(clippy::cast_precision_loss)]
        let refill = (dt as f64) * (self.target_bps as f64) / 1000.0;
        self.tokens = (self.tokens + refill).min(self.burst);

        #[allow(clippy::cast_precision_loss)]
        let need = bytes as f64;
        if self.tokens >= need {
            self.tokens -= need;
            return 0;
        }
        // 모자란 만큼 기다린다(기다린 뒤 보낼 것이므로 토큰은 지금 차감).
        let deficit = need - self.tokens;
        self.tokens = 0.0;
        #[allow(
            clippy::cast_precision_loss,
            clippy::cast_possible_truncation,
            clippy::cast_sign_loss
        )]
        let wait = (deficit * 1000.0 / self.target_bps as f64).ceil() as u64;
        // 기다린 시간만큼 시계도 앞당겨 둔다(다음 호출에서 이중 보충 방지).
        self.last_ms = self.last_ms.saturating_add(wait);
        wait
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `note_peak` = 최고치 후보 주입 — 더 큰 값만 남고 작은 값은 깎지 못한다.
    #[test]
    fn note_peak_keeps_maximum_only() {
        let mut m = RateMeter::default();
        assert_eq!(m.peak_bps(), 0);
        m.note_peak(2_000_000);
        m.note_peak(500_000); // 나중의 느린 실측이 peak를 되돌리면 안 된다
        assert_eq!(m.peak_bps(), 2_000_000);
        assert_eq!(m.auto_target(), 1_000_000); // 50%가 하한(256KiB)보다 크다
    }

    #[test]
    fn codes_map_to_expected_limits() {
        assert_eq!(RateLimit::from_code("auto"), RateLimit::Auto);
        assert_eq!(RateLimit::from_code("100k"), RateLimit::PerSec(102_400));
        assert_eq!(RateLimit::from_code("1m"), RateLimit::PerSec(1_048_576));
        assert_eq!(RateLimit::from_code("1g"), RateLimit::PerSec(1_073_741_824));
        // 사용자 지정 = 숫자(바이트/초).
        assert_eq!(RateLimit::from_code("50000"), RateLimit::PerSec(50_000));
        assert_eq!(RateLimit::from_code("이상한값"), RateLimit::Auto);
    }

    #[test]
    fn meter_tracks_peak_not_average() {
        let mut m = RateMeter::new(100);
        // 첫 구간: 100ms에 100KB → 1MB/s.
        m.observe(100_000, 0);
        m.observe(0, 100);
        let fast = m.peak_bps();
        assert!(fast >= 900_000, "관측 최고 {fast}");
        // 이후 한참 쉬어도(평균은 뚝 떨어져도) **최고치는 유지**된다.
        m.observe(1_000, 5_000);
        m.observe(0, 5_100);
        assert_eq!(m.peak_bps(), fast, "평균이 아니라 최고치");
    }

    #[test]
    fn note_burst_updates_peak_without_window() {
        // 프로브(수십 ms)가 500ms 창보다 빨리 끝나도 peak가 즉시 선다(M4-11).
        let mut m = RateMeter::new(500);
        m.note_burst(2 * 1024 * 1024, 40); // 2MiB / 40ms = 50MiB/s
        assert!(m.peak_bps() >= 50 * 1024 * 1024);
        assert!(m.auto_target() >= 25 * 1024 * 1024, "절반 목표가 즉시 선다");
    }

    #[test]
    fn advertised_cap_claims_nothing_on_auto() {
        // Auto = 상한 무주장(0) — 관측 유무와 무관. 종전 target_bps를 공지에
        // 쓰면 관측 없는 수신측이 하한을 주장해 협상이 하한에 고착된다(M4-11).
        assert_eq!(RateLimit::Auto.advertised_cap(), 0);
        assert_eq!(RateLimit::PerSec(1_000).advertised_cap(), 1_000);
    }

    #[test]
    fn auto_target_is_half_of_peak_with_floor() {
        let mut m = RateMeter::new(100);
        // 관측 없음 → 하한.
        assert_eq!(m.auto_target(), AUTO_FLOOR_BPS);
        // 10MB/s 관측 → 5MB/s 목표.
        m.observe(1_000_000, 0);
        m.observe(0, 100);
        let peak = m.peak_bps();
        assert_eq!(m.auto_target(), (peak / 2).max(AUTO_FLOOR_BPS));
        assert!(m.auto_target() < peak, "절반이어야 대역을 남긴다");
    }

    #[test]
    fn negotiate_picks_lower_and_treats_zero_as_unlimited() {
        assert_eq!(negotiate(1_000, 500), 500, "낮은 쪽");
        assert_eq!(negotiate(500, 1_000), 500);
        assert_eq!(negotiate(0, 700), 700, "내가 무제한이면 상대 값");
        assert_eq!(negotiate(700, 0), 700, "상대가 무제한이면 내 값");
        assert_eq!(negotiate(0, 0), 0, "둘 다 무제한");
    }

    #[test]
    fn pacer_unlimited_never_waits() {
        let mut p = Pacer::new(0);
        assert_eq!(p.take(10_000_000, 0), 0);
        assert_eq!(p.take(10_000_000, 0), 0);
    }

    #[test]
    fn pacer_enforces_average_rate() {
        // 목표 100KB/s · 32KiB 청크를 연달아 보낼 때 총 대기 시간으로 속도를 확인한다.
        let target = 100_000u64;
        let mut p = Pacer::new(target);
        let chunk = 32 * 1024u64;
        let mut now = 0u64;
        let mut sent = 0u64;
        for _ in 0..20 {
            let wait = p.take(chunk, now);
            now += wait; // 기다린 만큼 시간이 흐른다
            sent += chunk;
        }
        // 첫 창(burst)만큼은 즉시 나가므로 총 시간은 (보낸양 - 창)/목표 근처.
        let expect_ms = (sent.saturating_sub(p.burst_bytes())) * 1000 / target;
        let diff = now.abs_diff(expect_ms);
        assert!(
            diff <= expect_ms / 10 + 50,
            "실측 {now}ms vs 기대 {expect_ms}ms"
        );
    }

    #[test]
    fn pacer_window_allows_initial_burst_then_throttles() {
        let mut p = Pacer::new(1_000_000); // 1MB/s → 창 250KB
        assert!(p.burst_bytes() >= 250_000);
        // 창 안쪽은 대기 없음.
        assert_eq!(p.take(200_000, 0), 0);
        // 창을 넘기면 기다린다.
        let wait = p.take(200_000, 0);
        assert!(wait > 0, "창 초과인데 대기가 0");
    }
}
