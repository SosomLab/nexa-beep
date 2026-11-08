//! 계측 — **횟수만**(봉투 원리 · 내용·키·주소 없음). 운영자가 "무엇이 돌고 있나"를 상태로 본다.

use std::sync::atomic::{AtomicU64, Ordering};

/// 누적 카운터(프로세스 수명).
#[derive(Debug, Default)]
pub struct Metrics {
    /// 수락된 TCP 접속(프리앰블 전 포함).
    pub accepted: AtomicU64,
    /// 프리앰블 없음 = v1 클라 또는 비프로토콜(끊음).
    pub rejected_v1: AtomicU64,
    /// 프리앰블 major 불일치(끊음).
    pub rejected_version: AtomicU64,
    /// Noise 실패(키·prologue).
    pub handshake_failed: AtomicU64,
    /// Hello 거절(Reject).
    pub rejected_hello: AtomicU64,
    /// 헬스 프로브(Hello{app="probe"}).
    pub probes: AtomicU64,
    /// Welcome까지 간 세션(누적).
    pub sessions_total: AtomicU64,
    /// 지금 살아 있는 세션.
    pub sessions_live: AtomicU64,
    /// 성립(Accept)된 채널 누적.
    pub channels_total: AtomicU64,
    /// 중계한 Data 바이트(본문 길이 합).
    pub bytes_relayed: AtomicU64,
    /// 보낸 Error 봉투.
    pub errors_sent: AtomicU64,
    /// 보낸 Goaway(우아한 종료).
    pub goaway_sent: AtomicU64,
    /// 느린 소비자 절단.
    pub killed_slow: AtomicU64,
}

impl Metrics {
    pub(crate) fn inc(&self, c: &AtomicU64) {
        c.fetch_add(1, Ordering::Relaxed);
    }
    pub(crate) fn add(&self, c: &AtomicU64, n: u64) {
        c.fetch_add(n, Ordering::Relaxed);
    }
    /// 순간 스냅숏.
    #[must_use]
    pub fn snapshot(&self) -> Snapshot {
        let g = |c: &AtomicU64| c.load(Ordering::Relaxed);
        Snapshot {
            accepted: g(&self.accepted),
            rejected_v1: g(&self.rejected_v1),
            rejected_version: g(&self.rejected_version),
            handshake_failed: g(&self.handshake_failed),
            rejected_hello: g(&self.rejected_hello),
            probes: g(&self.probes),
            sessions_total: g(&self.sessions_total),
            sessions_live: g(&self.sessions_live),
            channels_total: g(&self.channels_total),
            bytes_relayed: g(&self.bytes_relayed),
            errors_sent: g(&self.errors_sent),
            goaway_sent: g(&self.goaway_sent),
            killed_slow: g(&self.killed_slow),
        }
    }
}

/// 계측 스냅숏(값 복사 · 로그 한 줄로 찍힌다).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[allow(missing_docs)]
pub struct Snapshot {
    pub accepted: u64,
    pub rejected_v1: u64,
    pub rejected_version: u64,
    pub handshake_failed: u64,
    pub rejected_hello: u64,
    pub probes: u64,
    pub sessions_total: u64,
    pub sessions_live: u64,
    pub channels_total: u64,
    pub bytes_relayed: u64,
    pub errors_sent: u64,
    pub goaway_sent: u64,
    pub killed_slow: u64,
}

impl core::fmt::Display for Snapshot {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(
            f,
            "accepted={} v1={} ver={} hs_fail={} hello_rej={} probes={} sessions={}/{} chans={} bytes={} errors={} goaway={} killed={}",
            self.accepted,
            self.rejected_v1,
            self.rejected_version,
            self.handshake_failed,
            self.rejected_hello,
            self.probes,
            self.sessions_live,
            self.sessions_total,
            self.channels_total,
            self.bytes_relayed,
            self.errors_sent,
            self.goaway_sent,
            self.killed_slow
        )
    }
}
