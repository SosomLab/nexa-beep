//! TCP 링크 — 길이 접두 프레이밍(M1-4 · [docs/08 §3] "TCP 위 프레임").
//!
//! `[len u32 BE][payload]`. 세션(Noise)은 프레임 단위로 주고받으므로 스트림 경계를 여기서 만든다.
//!
//! **수신 타임아웃 모드**: 핸드셰이크 동안은 블로킹(순차 왕복), 수립 후
//! [`TcpLink::set_poll_mode`]로 짧은 타임아웃을 걸어 **수신 펌프**(송신 채널 폴링과 교대)가
//! 가능해진다 — 타임아웃은 [`LinkError::TimedOut`]("지금은 없음")으로 전파된다.

use nbeep_core::link::{Link, LinkError};
use nbeep_core::PeerId;
use std::io::{Read as _, Write as _};
use std::net::TcpStream;
use std::time::Duration;

/// 프레임 상한 — Noise 메시지 상한과 정합([docs/08 §3]).
const MAX_FRAME: usize = 65_535;

/// 길이 접두 프레이밍 TCP 링크.
#[derive(Debug)]
pub struct TcpLink {
    stream: TcpStream,
    /// 호출자가 건 수신 타임아웃(RL-12 · 08-18) — 본문 읽기의 블로킹 복원 뒤
    /// **이 값으로 되건다**. 종전엔 고정 200ms로 되걸어, 액터의 1ms(08-16 전송
    /// 페이싱)가 첫 프레임 수신 즉시 무효화됐다(액터는 값 동일로 오인해 재설정
    /// 안 함 = 영구 200ms 고착 — 성능 회귀).
    recv_timeout: Option<Duration>,
}

impl TcpLink {
    /// 연결된 스트림을 감싼다(NODELAY — 소량 프레임 지연 제거).
    ///
    /// **TCP keepalive**(08-13 실기 — FIN 없는 죽음 감지): 상대가 전원 차단·강제 종료·
    /// 망 단절로 사라지면 FIN/RST가 없어 세션이 **영원히 산 것처럼 보였다**(목록 점이
    /// 초록으로 잔존). keepalive가 유휴 10초 후 5초 간격으로 찔러 죽은 연결을
    /// 커널이 닫게 한다(read가 Err로 깨어나 세션 액터가 `Closed`를 낸다 — unix 약 25초,
    /// Windows는 OS 기본 재시도 횟수라 약 1분). 진짜 생존 신호(하트비트)는 M2-4b.
    ///
    /// # Errors
    /// 소켓 옵션 설정 실패 시 `io::Error`.
    pub fn new(stream: TcpStream) -> std::io::Result<Self> {
        stream.set_nodelay(true)?;
        let ka = socket2::TcpKeepalive::new()
            .with_time(Duration::from_secs(10))
            .with_interval(Duration::from_secs(5));
        #[cfg(unix)]
        let ka = ka.with_retries(3);
        socket2::SockRef::from(&stream).set_tcp_keepalive(&ka)?;
        // ★ 쓰기 타임아웃 30초(08-16 실기 — 쓰기-쓰기 교착): 대용량 청크 홍수와
        //   양방향 대형 Control 프레임이 겹치면 **양쪽 소켓 버퍼가 동시에 가득**
        //   차 서로 write에서 영구 블록될 수 있다(액터가 굳어 취소·채팅 전부
        //   정지 — 전송 86%/80% 동결 실측). 30초면 정상 배압은 절대 안 걸리고,
        //   진짜 교착만 Err → Closed → 백오프 재연결로 **자기 치유**된다.
        stream.set_write_timeout(Some(Duration::from_secs(30)))?;
        Ok(Self {
            stream,
            recv_timeout: None,
        })
    }

    /// 수신 펌프 모드 — `timeout`마다 [`LinkError::TimedOut`]으로 돌아온다(송신 폴링과 교대).
    ///
    /// # Errors
    /// 소켓 옵션 설정 실패 시 `io::Error`.
    pub fn set_poll_mode(&mut self, timeout: Duration) -> std::io::Result<()> {
        self.recv_timeout = Some(timeout); // recv 복원 원천과 동기(RL-12)
        self.stream.set_read_timeout(Some(timeout))
    }
}

impl Link for TcpLink {
    fn peer(&self) -> PeerId {
        // TCP 수준에서 상대 신원은 알 수 없다 — 신원은 세션 핸드셰이크가 확정한다([docs/08]).
        // (이 값은 표시·테스트 편의였고 실경로는 세션 peer()를 쓴다.)
        PeerId::from_bytes([0u8; 32])
    }

    fn send(&mut self, frame: &[u8]) -> Result<(), LinkError> {
        if frame.len() > MAX_FRAME {
            return Err(LinkError::Closed); // 상한 위반 = 프로토콜 오류
        }
        let len = u32::try_from(frame.len()).map_err(|_| LinkError::Closed)?;
        let mut buf = Vec::with_capacity(4 + frame.len());
        buf.extend_from_slice(&len.to_be_bytes());
        buf.extend_from_slice(frame);
        self.stream.write_all(&buf).map_err(|_| LinkError::Closed)
    }

    fn set_recv_timeout(&mut self, dur: Option<Duration>) -> Result<(), LinkError> {
        self.recv_timeout = dur; // recv 본문 복원의 원천(RL-12)
        self.stream
            .set_read_timeout(dur)
            .map_err(|_| LinkError::Closed)
    }

    fn recv(&mut self) -> Result<Vec<u8>, LinkError> {
        let mut len4 = [0u8; 4];
        if let Err(e) = self.stream.read_exact(&mut len4) {
            return Err(match e.kind() {
                std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut => {
                    LinkError::TimedOut
                }
                _ => LinkError::Closed,
            });
        }
        let len = u32::from_be_bytes(len4) as usize;
        if len > MAX_FRAME {
            return Err(LinkError::Closed);
        }
        let mut buf = vec![0u8; len];
        // 길이를 읽은 뒤의 본문은 끝까지 읽는다(타임아웃이 프레임을 찢지 않게 블로킹 복원).
        self.stream
            .set_read_timeout(None)
            .map_err(|_| LinkError::Closed)?;
        let r = self
            .stream
            .read_exact(&mut buf)
            .map_err(|_| LinkError::Closed);
        // 호출자가 건 타임아웃으로 복원(RL-12) — 고정 200ms로 되걸면 액터의
        // 1ms 페이싱 설정이 첫 프레임 수신 즉시 무효화된다(영구 200ms 고착).
        let _ = self.stream.set_read_timeout(self.recv_timeout);
        r?;
        Ok(buf)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::TcpListener;

    #[test]
    fn framed_roundtrip_over_localhost() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let t = std::thread::spawn(move || {
            let (s, _) = listener.accept().unwrap();
            let mut link = TcpLink::new(s).unwrap();
            let m = link.recv().unwrap();
            link.send(&m).unwrap(); // 에코
            let big = link.recv().unwrap();
            assert_eq!(big.len(), 60_000, "큰 프레임 보존");
        });
        let mut a = TcpLink::new(TcpStream::connect(addr).unwrap()).unwrap();
        a.send(b"framed hello").unwrap();
        assert_eq!(a.recv().unwrap(), b"framed hello");
        a.send(&vec![7u8; 60_000]).unwrap();
        t.join().unwrap();
    }

    /// RL-12(08-18) — 본문 읽기 후 복원이 **호출자 타임아웃**이어야 한다.
    /// 고정 200ms로 되걸면 액터의 짧은 페이싱 설정(1ms)이 첫 프레임 수신
    /// 즉시 무효화된다(값 동일 오인으로 재설정도 안 됨 = 영구 고착).
    #[test]
    fn recv_restores_caller_timeout_not_fixed_200ms() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let t = std::thread::spawn(move || {
            let (s, _) = listener.accept().unwrap();
            let mut link = TcpLink::new(s).unwrap();
            link.send(b"hi").unwrap();
            std::thread::sleep(Duration::from_millis(400)); // 판정 동안 연결 유지
        });
        let mut a = TcpLink::new(TcpStream::connect(addr).unwrap()).unwrap();
        a.set_recv_timeout(Some(Duration::from_millis(20))).unwrap();
        assert_eq!(a.recv().unwrap(), b"hi");
        // min-of-3(08-18) — 부하 중 1회 측정은 스케줄러 지연이 낀다(병렬 빌드
        // 중 플레이크 실측). 셋 중 최솟값이면 진짜 타임아웃 값이 드러난다.
        let mut best = Duration::MAX;
        for _ in 0..3 {
            let t0 = std::time::Instant::now();
            assert_eq!(a.recv().err(), Some(LinkError::TimedOut));
            best = best.min(t0.elapsed());
        }
        assert!(
            best < Duration::from_millis(150),
            "20ms 설정이 200ms 고정 복원으로 덮이면 실패(최속 {best:?})"
        );
        t.join().unwrap();
    }

    #[test]
    fn poll_mode_times_out_without_data() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let _keep = std::thread::spawn(move || listener.accept());
        let mut a = TcpLink::new(TcpStream::connect(addr).unwrap()).unwrap();
        a.set_poll_mode(Duration::from_millis(100)).unwrap();
        assert_eq!(
            a.recv().err(),
            Some(LinkError::TimedOut),
            "데이터 없음 = 재시도 신호"
        );
    }
}
