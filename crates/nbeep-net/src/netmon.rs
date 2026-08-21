//! **네트워크 계측 모드(netmon)** — 과도한 패킷 송수신의 관측·기록(사용자 요청 08-21).
//!
//! 목적 = **주기 점검**: 발견 광고 폭주·재연결 폭풍·자기 에코 과다 같은 "조용히
//! 과한" 트래픽을 로그로 남겨 분석한다(08-20 \[무시\] 로그 홍수류를 숫자로 조기
//! 발견). 완성도 도구이지 상시 기능이 아니다 — **기록은 의도적으로 켠 경우에만**
//! (`netmon.enabled` 기본 off · 켜는 쪽은 호스트 앱).
//!
//! ## 구조 — 세는 것과 쓰는 것을 나눈다
//!
//! - **카운터는 상시**(이 모듈의 `on_*` — relaxed 원자 덧셈 하나, ns 수준·할당 0).
//!   이음새(발견 소켓·TCP 링크·연결 시도)가 지나갈 때 올린다. 꺼져 있으면 아무도
//!   안 읽을 뿐이다 — 켜는 순간부터 델타가 성립한다.
//! - **기록은 옵트인** — 호스트가 주기(`netmon.interval_s`)마다 [`snapshot`]을 떠
//!   [`report_line`]으로 한 줄을 만들어 파일에 쓴다(로그 파이프라인은 호스트 몫 —
//!   M3-22 StatusLog 재사용).
//!
//! **봉투 원리**: 여기 있는 것은 **횟수와 바이트 수뿐**이다 — 주소·신원·내용은
//! 세지도 남기지도 않는다(계측은 횟수만 · [docs/13]).
//!
//! 전역 정적 카운터는 이 저장소가 피하는 싱글톤이지만, 여기는 예외로 둔 근거가
//! 있다: 계측점이 3개 크레이트 이음새에 흩어져 있어 값 소유자를 스레딩하면 **모든
//! 포트 시그니처가 계측 타입을 알게 된다**(DR-21 위반이 더 크다). 쓰기 전용
//! 텔레메트리 + 읽기는 스냅숏 한 곳이라 상태 결합이 없다.

use std::sync::atomic::{AtomicU64, Ordering::Relaxed};

static DISCO_TX: AtomicU64 = AtomicU64::new(0);
static DISCO_RX: AtomicU64 = AtomicU64::new(0);
static DISCO_SELF: AtomicU64 = AtomicU64::new(0);
static SESS_TX_BYTES: AtomicU64 = AtomicU64::new(0);
static SESS_RX_BYTES: AtomicU64 = AtomicU64::new(0);
static SESS_TX_FRAMES: AtomicU64 = AtomicU64::new(0);
static SESS_RX_FRAMES: AtomicU64 = AtomicU64::new(0);
static CONN_OUT: AtomicU64 = AtomicU64::new(0);
static CONN_OK: AtomicU64 = AtomicU64::new(0);
static CONN_IN: AtomicU64 = AtomicU64::new(0);

/// 발견 패킷 발신(멀티캐스트·브로드캐스트·S4 유니캐스트·HELLO 응답 — 데이터그램 단위).
#[inline]
pub fn on_disco_tx(pkts: u64) {
    DISCO_TX.fetch_add(pkts, Relaxed);
}
/// 발견 패킷 수신(디코드 전 — 소켓에 도달한 데이터그램 단위).
#[inline]
pub fn on_disco_rx() {
    DISCO_RX.fetch_add(1, Relaxed);
}
/// 수신 중 자기 패킷(멀티캐스트 루프백 에코) — 과다면 인터페이스 중복 발신 신호.
#[inline]
pub fn on_disco_rx_self() {
    DISCO_SELF.fetch_add(1, Relaxed);
}
/// 세션 링크 발신(프레임 1 · 길이 프리픽스 포함 바이트).
#[inline]
pub fn on_sess_tx(bytes: u64) {
    SESS_TX_FRAMES.fetch_add(1, Relaxed);
    SESS_TX_BYTES.fetch_add(bytes, Relaxed);
}
/// 세션 링크 수신(프레임 1 · 길이 프리픽스 포함 바이트).
#[inline]
pub fn on_sess_rx(bytes: u64) {
    SESS_RX_FRAMES.fetch_add(1, Relaxed);
    SESS_RX_BYTES.fetch_add(bytes, Relaxed);
}
/// 아웃바운드 TCP 연결 **시도**(후보 주소당 1) · `ok` = 성립.
#[inline]
pub fn on_conn_out(ok: bool) {
    CONN_OUT.fetch_add(1, Relaxed);
    if ok {
        CONN_OK.fetch_add(1, Relaxed);
    }
}
/// 인바운드 TCP 수락 1건.
#[inline]
pub fn on_conn_in() {
    CONN_IN.fetch_add(1, Relaxed);
}

/// 누적 카운터의 한 시점 값 — 델타는 두 스냅숏의 차로 구한다(리셋 없음 · 경합 무해).
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct NetSnapshot {
    /// 발견 발신 데이터그램 누적.
    pub disco_tx: u64,
    /// 발견 수신 데이터그램 누적.
    pub disco_rx: u64,
    /// 수신 중 자기 에코 누적.
    pub disco_self: u64,
    /// 세션 발신 바이트 누적.
    pub sess_tx_bytes: u64,
    /// 세션 수신 바이트 누적.
    pub sess_rx_bytes: u64,
    /// 세션 발신 프레임 누적.
    pub sess_tx_frames: u64,
    /// 세션 수신 프레임 누적.
    pub sess_rx_frames: u64,
    /// 아웃바운드 연결 시도 누적.
    pub conn_out: u64,
    /// 아웃바운드 연결 성립 누적.
    pub conn_ok: u64,
    /// 인바운드 수락 누적.
    pub conn_in: u64,
}

/// 지금까지의 누적을 뜬다.
#[must_use]
pub fn snapshot() -> NetSnapshot {
    NetSnapshot {
        disco_tx: DISCO_TX.load(Relaxed),
        disco_rx: DISCO_RX.load(Relaxed),
        disco_self: DISCO_SELF.load(Relaxed),
        sess_tx_bytes: SESS_TX_BYTES.load(Relaxed),
        sess_rx_bytes: SESS_RX_BYTES.load(Relaxed),
        sess_tx_frames: SESS_TX_FRAMES.load(Relaxed),
        sess_rx_frames: SESS_RX_FRAMES.load(Relaxed),
        conn_out: CONN_OUT.load(Relaxed),
        conn_ok: CONN_OK.load(Relaxed),
        conn_in: CONN_IN.load(Relaxed),
    }
}

/// 과다 판정 임계 — **발견 발신** 초당 패킷. 정상 = 주기 광고(기본 0.8s)에
/// 인터페이스 수·S4 라운드를 곱해도 한 자릿수라, 지속 25pps는 폭주다.
pub const WARN_DISCO_TX_PPS: u64 = 25;
/// 과다 판정 임계 — **발견 수신** 초당 패킷(이웃 수에 비례하므로 더 관대).
pub const WARN_DISCO_RX_PPS: u64 = 100;
/// 과다 판정 임계 — 분당 연결 시도(재연결 폭풍 — 08-13 백오프 계열 회귀 감시).
pub const WARN_CONN_PER_MIN: u64 = 30;

/// 두 스냅숏의 델타로 **분석용 한 줄**을 만든다(안정 key=value 포맷 — 파서 대상).
/// 반환 = (줄, 경고 태그들 — 비면 정상 구간).
#[must_use]
pub fn report_line(
    prev: &NetSnapshot,
    cur: &NetSnapshot,
    dt_ms: u64,
) -> (String, Vec<&'static str>) {
    let d = |a: u64, b: u64| b.saturating_sub(a);
    let dt = dt_ms.max(1);
    let dtx = d(prev.disco_tx, cur.disco_tx);
    let drx = d(prev.disco_rx, cur.disco_rx);
    let dself = d(prev.disco_self, cur.disco_self);
    let co = d(prev.conn_out, cur.conn_out);
    let cok = d(prev.conn_ok, cur.conn_ok);
    let ci = d(prev.conn_in, cur.conn_in);
    let stxb = d(prev.sess_tx_bytes, cur.sess_tx_bytes);
    let srxb = d(prev.sess_rx_bytes, cur.sess_rx_bytes);
    let stxf = d(prev.sess_tx_frames, cur.sess_tx_frames);
    let srxf = d(prev.sess_rx_frames, cur.sess_rx_frames);

    let mut warns = Vec::new();
    if dtx.saturating_mul(1000) / dt > WARN_DISCO_TX_PPS {
        warns.push("disco_tx_pps");
    }
    if drx.saturating_mul(1000) / dt > WARN_DISCO_RX_PPS {
        warns.push("disco_rx_pps");
    }
    if (co + ci).saturating_mul(60_000) / dt > WARN_CONN_PER_MIN {
        warns.push("conn_rate");
    }

    let mut line = format!(
        "netmon dt_ms={dt_ms} dtx={dtx} drx={drx} dself={dself} \
         co={co} cok={cok} ci={ci} stxf={stxf} srxf={srxf} stxb={stxb} srxb={srxb}"
    );
    if !warns.is_empty() {
        line.push_str(" warn=");
        line.push_str(&warns.join(","));
    }
    (line, warns)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 카운터 → 스냅숏 델타 반영(다른 테스트도 올릴 수 있어 ≥로 단언).
    #[test]
    fn counters_flow_into_snapshot() {
        let a = snapshot();
        on_disco_tx(3);
        on_disco_rx();
        on_disco_rx_self();
        on_sess_tx(100);
        on_sess_rx(50);
        on_conn_out(true);
        on_conn_out(false);
        on_conn_in();
        let b = snapshot();
        assert!(b.disco_tx - a.disco_tx >= 3);
        assert!(b.disco_rx - a.disco_rx >= 1);
        assert!(b.disco_self - a.disco_self >= 1);
        assert!(b.sess_tx_bytes - a.sess_tx_bytes >= 100);
        assert!(b.sess_rx_bytes - a.sess_rx_bytes >= 50);
        assert!(b.sess_tx_frames - a.sess_tx_frames >= 1);
        assert!(b.conn_out - a.conn_out >= 2);
        assert!(b.conn_ok - a.conn_ok >= 1);
        assert!(b.conn_in - a.conn_in >= 1);
    }

    /// 정상 구간 = 경고 없음 · 포맷은 안정 key=value(분석 파서 계약).
    #[test]
    fn quiet_interval_has_no_warn_and_stable_format() {
        let prev = NetSnapshot::default();
        let cur = NetSnapshot {
            disco_tx: 40, // 10초에 40 = 4pps — 정상
            disco_rx: 80,
            sess_tx_bytes: 1_000,
            sess_rx_bytes: 2_000,
            sess_tx_frames: 3,
            sess_rx_frames: 4,
            conn_out: 2, // 10초에 2+1 = 분당 18 — 임계 안
            conn_ok: 2,
            conn_in: 1,
            disco_self: 20,
        };
        let (line, warns) = report_line(&prev, &cur, 10_000);
        assert!(warns.is_empty(), "{warns:?}");
        assert_eq!(
            line,
            "netmon dt_ms=10000 dtx=40 drx=80 dself=20 co=2 cok=2 ci=1 \
             stxf=3 srxf=4 stxb=1000 srxb=2000"
        );
    }

    /// 폭주 구간 = 태그가 붙는다(발견 발신 폭주 · 연결 폭풍).
    #[test]
    fn floods_are_tagged() {
        let prev = NetSnapshot::default();
        let cur = NetSnapshot {
            disco_tx: 400, // 10초에 400 = 40pps > 25
            conn_out: 20,  // 10초에 20+0 = 분당 120 > 30
            ..NetSnapshot::default()
        };
        let (line, warns) = report_line(&prev, &cur, 10_000);
        assert_eq!(warns, vec!["disco_tx_pps", "conn_rate"]);
        assert!(line.ends_with("warn=disco_tx_pps,conn_rate"), "{line}");
    }

    /// 카운터가 뒤로 간 것처럼 보여도(스냅숏 경합) 포화 뺄셈 — 패닉·언더플로 없음.
    #[test]
    fn regressed_snapshot_is_safe() {
        let prev = NetSnapshot {
            disco_tx: 100,
            ..NetSnapshot::default()
        };
        let cur = NetSnapshot::default();
        let (line, warns) = report_line(&prev, &cur, 1_000);
        assert!(warns.is_empty());
        assert!(line.contains("dtx=0"));
    }
}
