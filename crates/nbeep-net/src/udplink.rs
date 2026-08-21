//! `UdpLink` — [`ArqCore`](crate::arq) 위의 실소켓 셸(X-UDP-b).
//!
//! [`Link`] 계약(프레임 보존·순서·신뢰·수신 타임아웃)을 UDP 위에서 만족한다.
//! 성공 기준(TODO X-UDP-b): **기존 mux·Noise·파일 전송이 코드 무변경으로 이 위에서 돈다** —
//! 상태기계는 [`crate::arq`]에 있고 여기는 소켓·시계 왕복만 한다(sans-io 셸).
//!
//! **동시 열기 = 홀펀칭의 전제** — [`UdpLink::punch`]는 **주어진 소켓**(서버가 관측한
//! 매핑과 같은 로컬 포트)으로 양쪽이 동시에 SYN을 쏜다. NAT 매핑은 (로컬 포트, 목적지)
//! 쌍으로 열리므로 소켓을 바꾸면 관측이 무효가 된다.
//!
//! 소켓은 **connect하지 않는다** — 펀칭 소켓엔 서버 관측 에코 등 다른 발신원의
//! 데이터그램이 섞여 들어올 수 있어, `recv_from` + 발신원 주소 필터로 거른다.

use crate::arq::{ArqCore, ArqError};
use nbeep_core::link::{Link, LinkError};
use nbeep_core::PeerId;
use std::net::{SocketAddr, UdpSocket};
use std::time::{Duration, Instant};

/// 소켓 폴 한 사이클의 대기 상한 — ARQ 타이머 해상도와 셸 반응성의 절충.
const IO_SLICE: Duration = Duration::from_millis(20);

/// 송신 배압 해소 대기 상한 — TCP 링크의 쓰기 타임아웃 30초와 같은 논리
/// (정상 배압은 절대 안 걸리고, 진짜 교착·죽은 경로만 Closed로 끊는다).
const SEND_DEADLINE: Duration = Duration::from_secs(30);

/// UDP 위 신뢰성 링크 — [`ArqCore`] + `UdpSocket`.
#[derive(Debug)]
pub struct UdpLink {
    sock: UdpSocket,
    remote: SocketAddr,
    core: ArqCore,
    epoch: Instant,
    recv_timeout: Option<Duration>,
}

impl UdpLink {
    /// 새 임시 소켓으로 `remote`에 연결한다(성립까지 블로킹 · `timeout` 상한).
    ///
    /// # Errors
    /// 바인드·소켓 옵션 실패, 또는 `timeout` 내 성립 실패 시 `io::Error`.
    pub fn connect(remote: SocketAddr, timeout: Duration) -> std::io::Result<Self> {
        let bind_addr: SocketAddr = if remote.is_ipv4() {
            "0.0.0.0:0".parse().expect("리터럴 주소")
        } else {
            "[::]:0".parse().expect("리터럴 주소")
        };
        let sock = UdpSocket::bind(bind_addr)?;
        Self::punch(sock, remote, timeout)
    }

    /// **주어진 소켓**으로 `remote`와 동시 열기(홀펀칭 경로 — 서버가 관측한 로컬 포트 유지).
    /// 양쪽이 서로의 관측 엔드포인트로 같은 시점에 부르면 NAT 매핑이 양방향으로 열린다.
    ///
    /// # Errors
    /// 소켓 옵션 실패 또는 `timeout` 내 성립 실패 시 `io::Error`(`TimedOut`).
    pub fn punch(sock: UdpSocket, remote: SocketAddr, timeout: Duration) -> std::io::Result<Self> {
        // nonce: OS 난수 없이도 충분 — 예측 불가가 요구가 아니다(보안은 위층 Noise).
        // 시각+주소 해시로 양쪽이 다른 값을 얻는다(같으면 성립이 안 될 뿐 = 재시도).
        let seed = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0x9e37_79b9, |d| d.subsec_nanos());
        let nonce =
            seed ^ (std::process::id().rotate_left(16)) ^ u32::from(sock.local_addr()?.port());
        let mut link = Self {
            sock,
            remote,
            core: ArqCore::new(nonce, 0),
            epoch: Instant::now(),
            recv_timeout: None,
        };
        let deadline = Instant::now() + timeout;
        while !link.core.is_established() {
            if Instant::now() >= deadline || link.core.is_terminated() {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::TimedOut,
                    "UDP 링크 성립 실패(상대 무응답·경로 없음)",
                ));
            }
            link.drive(IO_SLICE).map_err(|_| {
                std::io::Error::new(std::io::ErrorKind::ConnectionAborted, "UDP 링크 종료")
            })?;
        }
        Ok(link)
    }

    fn now_ms(&self) -> u64 {
        u64::try_from(self.epoch.elapsed().as_millis()).unwrap_or(u64::MAX)
    }

    /// IO 한 사이클 — 타이머 구동 → 송신 배출 → `wait` 상한으로 수신 → 다시 배출.
    fn drive(&mut self, wait: Duration) -> Result<(), LinkError> {
        let now = self.now_ms();
        self.core.poll(now);
        self.flush_out()?;
        let wake = Duration::from_millis(self.core.next_wake_in(now).max(1));
        let slice = wait.min(wake).min(IO_SLICE);
        self.sock
            .set_read_timeout(Some(slice.max(Duration::from_millis(1))))
            .map_err(|_| LinkError::Closed)?;
        let mut buf = [0u8; 2048];
        match self.sock.recv_from(&mut buf) {
            Ok((n, from)) => {
                // 발신원 필터 — 펀칭 소켓엔 서버 에코 등 다른 데이터그램이 섞인다.
                if from == self.remote {
                    crate::netmon::on_sess_rx(n as u64); // 계측은 횟수·바이트만(netmon)
                    self.core.handle_datagram(&buf[..n], self.now_ms());
                }
            }
            Err(e)
                if matches!(
                    e.kind(),
                    std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
                ) => {}
            // ⚠ Windows: 이전 send_to가 ICMP Port Unreachable로 돌아오면 recv가
            // ConnReset을 낸다 — 상대가 아직 안 떴을 뿐이므로 죽음이 아니다(재시도).
            Err(e) if e.kind() == std::io::ErrorKind::ConnectionReset => {}
            Err(_) => return Err(LinkError::Closed),
        }
        let now = self.now_ms();
        self.core.poll(now);
        self.flush_out()?;
        Ok(())
    }

    fn flush_out(&mut self) -> Result<(), LinkError> {
        while let Some(pkt) = self.core.take_outgoing() {
            crate::netmon::on_sess_tx(pkt.len() as u64);
            // 일시 오류(버퍼 가득 등)는 버린다 — 유실은 ARQ 재전송이 흡수한다.
            let _ = self.sock.send_to(&pkt, self.remote);
        }
        Ok(())
    }
}

impl Link for UdpLink {
    fn peer(&self) -> PeerId {
        // 전송 수준에서 상대 신원은 알 수 없다 — 신원은 세션 핸드셰이크가 확정한다(TcpLink와 동일).
        PeerId::from_bytes([0u8; 32])
    }

    fn send(&mut self, frame: &[u8]) -> Result<(), LinkError> {
        let deadline = Instant::now() + SEND_DEADLINE;
        loop {
            match self.core.push_frame(frame, self.now_ms()) {
                Ok(()) => {
                    // 이 프레임의 세그먼트가 전부 **1차 송신**될 때까지 구동한다 —
                    // send만 하고 recv를 안 부르는 호출자여도 창 밖 큐가 썩지 않게
                    // (창 전진에는 상대 ACK가 필요하고, ACK 소비는 drive가 한다).
                    self.flush_out()?;
                    while self.core.unpumped() > 0 {
                        if Instant::now() >= deadline || self.core.is_terminated() {
                            return Err(LinkError::Closed);
                        }
                        self.drive(IO_SLICE)?;
                    }
                    return Ok(());
                }
                Err(ArqError::TooLarge | ArqError::Closed) => return Err(LinkError::Closed),
                Err(ArqError::WouldBlock) => {
                    // 배압 — ACK가 창을 열 때까지 IO를 돌린다(상한 = SEND_DEADLINE).
                    if Instant::now() >= deadline || self.core.is_terminated() {
                        return Err(LinkError::Closed);
                    }
                    self.drive(IO_SLICE)?;
                }
            }
        }
    }

    fn recv(&mut self) -> Result<Vec<u8>, LinkError> {
        let start = Instant::now();
        loop {
            if let Some(f) = self.core.take_frame() {
                return Ok(f);
            }
            if self.core.is_terminated() {
                return Err(LinkError::Closed);
            }
            if let Some(t) = self.recv_timeout {
                let elapsed = start.elapsed();
                if elapsed >= t {
                    return Err(LinkError::TimedOut);
                }
                self.drive(t - elapsed)?;
            } else {
                self.drive(IO_SLICE)?;
            }
        }
    }

    fn set_recv_timeout(&mut self, dur: Option<Duration>) -> Result<(), LinkError> {
        self.recv_timeout = dur;
        Ok(())
    }

    fn remote_ip(&self) -> Option<std::net::IpAddr> {
        // 경로 등급(PathClass) 판정 전용 — 커널이 보는 상대 주소(ADR-0006 §5-1-5).
        Some(self.remote.ip())
    }
}

impl Drop for UdpLink {
    fn drop(&mut self) {
        // 우아한 FIN 시도(1회 배출 — 유실돼도 상대의 죽은 경로 판정이 정리한다).
        let now = self.now_ms();
        self.core.close(now);
        self.core.poll(now);
        let _ = self.flush_out();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pair() -> (UdpLink, UdpLink) {
        let sa = UdpSocket::bind("127.0.0.1:0").unwrap();
        let sb = UdpSocket::bind("127.0.0.1:0").unwrap();
        let aa = sa.local_addr().unwrap();
        let ab = sb.local_addr().unwrap();
        // 동시 열기 — 양쪽이 서로를 부른다(홀펀칭과 같은 문법).
        let tb =
            std::thread::spawn(move || UdpLink::punch(sb, aa, Duration::from_secs(5)).unwrap());
        let a = UdpLink::punch(sa, ab, Duration::from_secs(5)).unwrap();
        let b = tb.join().unwrap();
        (a, b)
    }

    /// TcpLink 계약 테스트의 재사용(X-UDP-b 회귀 기준) — 프레임 왕복 + 큰 프레임 보존.
    #[test]
    fn framed_roundtrip_over_localhost() {
        let (mut a, mut b) = pair();
        let t = std::thread::spawn(move || {
            let m = b.recv().unwrap();
            b.send(&m).unwrap(); // 에코
            let big = b.recv().unwrap();
            assert_eq!(big.len(), 60_000, "큰 프레임 보존");
            b.send(b"done").unwrap();
            b
        });
        a.send(b"framed hello").unwrap();
        assert_eq!(a.recv().unwrap(), b"framed hello");
        a.send(&vec![7u8; 60_000]).unwrap();
        assert_eq!(a.recv().unwrap(), b"done");
        drop(t.join().unwrap());
    }

    #[test]
    fn poll_mode_times_out_without_data() {
        let (mut a, _b) = pair();
        a.set_recv_timeout(Some(Duration::from_millis(100)))
            .unwrap();
        let t0 = Instant::now();
        assert_eq!(
            a.recv().err(),
            Some(LinkError::TimedOut),
            "데이터 없음 = 재시도 신호"
        );
        assert!(t0.elapsed() < Duration::from_secs(2));
    }

    #[test]
    fn connect_times_out_against_silence() {
        // 아무도 안 듣는 포트 — 성립 실패가 조용한 무한 대기가 아니라 TimedOut이어야 한다.
        let dead = UdpSocket::bind("127.0.0.1:0").unwrap();
        let addr = dead.local_addr().unwrap();
        drop(dead); // 포트를 비운다(응답 없음 보장)
        let r = UdpLink::connect(addr, Duration::from_millis(600));
        assert!(r.is_err());
    }

    /// 기존 Noise 세션이 **코드 무변경**으로 UdpLink 위에서 도는가 — X-UDP-b 성공 기준.
    /// (nbeep-crypto를 dev-dep으로 끌 수 없어(순환) 여기서는 mux 프레임 다중 왕복으로 대신하고,
    /// Noise/mux 통합은 nexa-beepd의 릴레이 e2e 테스트가 UdpLink 펀치 경로와 함께 덮는다.)
    #[test]
    fn many_frames_in_order() {
        let (mut a, mut b) = pair();
        let t = std::thread::spawn(move || {
            for i in 0..200u32 {
                let f = b.recv().unwrap();
                assert_eq!(f, i.to_be_bytes().to_vec(), "순서 보존");
            }
            b
        });
        for i in 0..200u32 {
            a.send(&i.to_be_bytes()).unwrap();
        }
        drop(t.join().unwrap());
    }
}
