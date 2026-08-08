//! `MuxSession` — 한 세션 위의 **논리 스트림 다중화**([docs/08] §3 · M2-3).
//!
//! 대화 메시지와 제어 트래픽(수신 확인·프로필 조회·공유 목록 — ADR-0008/0009/0010)이
//! **같은 보안 세션 하나**를 나눠 쓴다. 세션을 스트림별로 따로 열면 핸드셰이크·소켓이 배로 늘고
//! 경로 전환 시 상태가 갈라진다 — 다중화가 [docs/09] "세션 = 상대당 하나"를 지킨다.
//!
//! ## 와이어 형식 (봉투)
//!
//! `[stream_id: 1B][payload]` — 다중화 계층의 봉투는 **스트림 번호뿐**이다. 내용은 보지 않는다.
//! **미지 스트림 번호는 조용히 버린다** — v2가 스트림을 추가해도 v1이 죽지 않는다
//! (발견 패킷의 "미지 버전 무시"와 같은 전방 호환 규약 — [docs/08] §2).
//!
//! ## 백프레셔 (fail-closed)
//!
//! `recv(Chat)` 중 도착한 Control 프레임은 큐에 쌓인다. 큐가 [`MAX_QUEUED`]를 넘으면
//! [`SessionError::Backpressure`]로 **세션을 끊는 쪽을 택한다** — 무한 버퍼는 원격이 내
//! 메모리를 키우는 공격면이다(NFR-B). 정상 트래픽에서 한 스트림만 수십 프레임 밀리는 상황은
//! 프로토콜 위반에 가깝다.

use crate::identity::{PeerId, TrustLevel};
use crate::session::{Session, SessionError};
use std::collections::VecDeque;

/// 논리 스트림 번호. v1은 2종 — 추가는 뒤에 append(번호 불변).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StreamId {
    /// 제어 — 수신 확인(ack)·프로필 조회·공유 목록 등 프로토콜 내부 트래픽.
    Control,
    /// 대화 — 사용자 메시지.
    Chat,
}

impl StreamId {
    /// 와이어 바이트.
    #[must_use]
    pub fn to_byte(self) -> u8 {
        match self {
            StreamId::Control => 0,
            StreamId::Chat => 1,
        }
    }

    /// 와이어 바이트 해석 — 미지 값은 `None`(수신 측이 조용히 버린다 — 전방 호환).
    #[must_use]
    pub fn from_byte(b: u8) -> Option<Self> {
        match b {
            0 => Some(StreamId::Control),
            1 => Some(StreamId::Chat),
            _ => None,
        }
    }

    fn index(self) -> usize {
        self.to_byte() as usize
    }
}

/// 스트림 수(내부 큐 배열 크기).
const STREAMS: usize = 2;

/// 다중화 페이로드 상한 — Noise 메시지 상한(65535) − AEAD 태그(16) − 스트림 바이트(1).
/// 넘는 메시지는 상위 계층이 쪼갠다(파일 전송 청킹은 M4).
pub const MAX_PAYLOAD: usize = 65_518;

/// 스트림당 대기 큐 상한 — 넘으면 [`SessionError::Backpressure`](fail-closed).
pub const MAX_QUEUED: usize = 64;

/// [`Session`] 하나를 논리 스트림 여럿으로 나눠 쓰는 다중화 계층.
#[derive(Debug)]
pub struct MuxSession<S: Session> {
    inner: S,
    queues: [VecDeque<Vec<u8>>; STREAMS],
}

impl<S: Session> MuxSession<S> {
    /// 수립된 세션을 감싼다.
    #[must_use]
    pub fn new(inner: S) -> Self {
        Self {
            inner,
            queues: [VecDeque::new(), VecDeque::new()],
        }
    }

    /// 인증된 상대(안쪽 세션 위임).
    #[must_use]
    pub fn peer(&self) -> PeerId {
        self.inner.peer()
    }

    /// 신뢰 등급(안쪽 세션 위임).
    #[must_use]
    pub fn trust(&self) -> TrustLevel {
        self.inner.trust()
    }

    /// `stream`으로 논리 메시지 하나 송신.
    ///
    /// # Errors
    /// [`MAX_PAYLOAD`] 초과면 [`SessionError::TooLarge`], 링크 종료면 [`SessionError::Closed`].
    pub fn send(&mut self, stream: StreamId, message: &[u8]) -> Result<(), SessionError> {
        if message.len() > MAX_PAYLOAD {
            return Err(SessionError::TooLarge);
        }
        let mut frame = Vec::with_capacity(1 + message.len());
        frame.push(stream.to_byte());
        frame.extend_from_slice(message);
        self.inner.send(&frame)
    }

    /// `stream`의 다음 메시지 수신(블로킹). 그 사이 도착한 **다른 스트림** 프레임은 큐에 쌓인다.
    ///
    /// # Errors
    /// 링크 종료 [`SessionError::Closed`] · 빈 프레임(스트림 바이트 없음)은 프로토콜 위반
    /// [`SessionError::Handshake`] · 반대편 큐 초과 [`SessionError::Backpressure`].
    pub fn recv(&mut self, stream: StreamId) -> Result<Vec<u8>, SessionError> {
        if let Some(m) = self.queues[stream.index()].pop_front() {
            return Ok(m);
        }
        loop {
            let mut frame = self.inner.recv()?;
            let Some(&id_byte) = frame.first() else {
                return Err(SessionError::Handshake); // 빈 프레임 = 위반
            };
            frame.drain(..1);
            let Some(id) = StreamId::from_byte(id_byte) else {
                continue; // 미지 스트림 — 조용히 버림(전방 호환)
            };
            if id == stream {
                return Ok(frame);
            }
            let q = &mut self.queues[id.index()];
            if q.len() >= MAX_QUEUED {
                return Err(SessionError::Backpressure);
            }
            q.push_back(frame);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc::{channel, Receiver, Sender};

    /// 링크 없이 mux만 검증하는 세션 fake(채널 페어).
    #[derive(Debug)]
    struct PairSession {
        peer: PeerId,
        tx: Sender<Vec<u8>>,
        rx: Receiver<Vec<u8>>,
    }
    impl Session for PairSession {
        fn peer(&self) -> PeerId {
            self.peer
        }
        fn trust(&self) -> TrustLevel {
            TrustLevel::Pinned
        }
        fn send(&mut self, m: &[u8]) -> Result<(), SessionError> {
            self.tx.send(m.to_vec()).map_err(|_| SessionError::Closed)
        }
        fn recv(&mut self) -> Result<Vec<u8>, SessionError> {
            self.rx.recv().map_err(|_| SessionError::Closed)
        }
    }

    fn pid(b: u8) -> PeerId {
        let mut a = [0u8; 32];
        a[0] = b;
        PeerId::from_bytes(a)
    }

    fn pair() -> (MuxSession<PairSession>, MuxSession<PairSession>) {
        let (atx, brx) = channel();
        let (btx, arx) = channel();
        (
            MuxSession::new(PairSession {
                peer: pid(2),
                tx: atx,
                rx: arx,
            }),
            MuxSession::new(PairSession {
                peer: pid(1),
                tx: btx,
                rx: brx,
            }),
        )
    }

    #[test]
    fn streams_are_independent() {
        let (mut a, mut b) = pair();
        a.send(StreamId::Chat, b"hello").unwrap();
        a.send(StreamId::Control, b"ack:1").unwrap();
        // 순서를 바꿔 받아도 각 스트림의 내용이 섞이지 않는다.
        assert_eq!(b.recv(StreamId::Control).unwrap(), b"ack:1");
        assert_eq!(b.recv(StreamId::Chat).unwrap(), b"hello");
    }

    #[test]
    fn interleaved_frames_queue_up_in_order() {
        let (mut a, mut b) = pair();
        a.send(StreamId::Control, b"c1").unwrap();
        a.send(StreamId::Control, b"c2").unwrap();
        a.send(StreamId::Chat, b"m1").unwrap();
        // Chat을 먼저 읽으면 Control 2건이 순서대로 큐에 남는다.
        assert_eq!(b.recv(StreamId::Chat).unwrap(), b"m1");
        assert_eq!(b.recv(StreamId::Control).unwrap(), b"c1");
        assert_eq!(b.recv(StreamId::Control).unwrap(), b"c2");
    }

    /// 원시 프레임을 직접 주입할 수 있는 수신 전용 mux(프로토콜 위반·전방 호환 테스트용).
    fn raw_rx() -> (Sender<Vec<u8>>, MuxSession<PairSession>) {
        let (raw_tx, rx) = channel();
        let (tx, _sink) = channel();
        std::mem::forget(_sink); // 송신은 검증 대상 아님 — 채널만 살려둔다
        (
            raw_tx,
            MuxSession::new(PairSession {
                peer: pid(1),
                tx,
                rx,
            }),
        )
    }

    #[test]
    fn unknown_stream_is_silently_dropped() {
        // v2가 스트림 7을 추가해도 v1 수신자는 죽지 않는다(전방 호환).
        let (raw, mut b) = raw_rx();
        raw.send(b"\x07from-the-future".to_vec()).unwrap();
        raw.send(b"\x01after".to_vec()).unwrap(); // Chat=1
        assert_eq!(b.recv(StreamId::Chat).unwrap(), b"after");
    }

    #[test]
    fn empty_frame_is_protocol_violation() {
        let (raw, mut b) = raw_rx();
        raw.send(Vec::new()).unwrap();
        assert_eq!(b.recv(StreamId::Chat).err(), Some(SessionError::Handshake));
    }

    #[test]
    fn oversized_payload_is_rejected_before_send() {
        let (mut a, _b) = pair();
        let big = vec![0u8; MAX_PAYLOAD + 1];
        assert_eq!(
            a.send(StreamId::Chat, &big).err(),
            Some(SessionError::TooLarge)
        );
        assert!(
            a.send(StreamId::Chat, &vec![0u8; MAX_PAYLOAD]).is_ok(),
            "상한 정확히는 허용"
        );
    }

    #[test]
    fn backpressure_kills_session_instead_of_buffering_forever() {
        let (mut a, mut b) = pair();
        // Control만 MAX_QUEUED+1건 보내고 Chat을 기다리면 큐 초과.
        for i in 0..=MAX_QUEUED {
            a.send(StreamId::Control, format!("c{i}").as_bytes())
                .unwrap();
        }
        a.send(StreamId::Chat, b"never-reached").unwrap();
        assert_eq!(
            b.recv(StreamId::Chat).err(),
            Some(SessionError::Backpressure),
            "무한 버퍼 대신 fail-closed"
        );
    }

    #[test]
    fn queue_boundary_is_exact() {
        let (mut a, mut b) = pair();
        // 정확히 MAX_QUEUED건은 허용된다.
        for i in 0..MAX_QUEUED {
            a.send(StreamId::Control, format!("c{i}").as_bytes())
                .unwrap();
        }
        a.send(StreamId::Chat, b"ok").unwrap();
        assert_eq!(b.recv(StreamId::Chat).unwrap(), b"ok");
        assert_eq!(b.recv(StreamId::Control).unwrap(), b"c0");
    }

    #[test]
    fn stream_id_roundtrip_and_unknown() {
        assert_eq!(
            StreamId::from_byte(StreamId::Control.to_byte()),
            Some(StreamId::Control)
        );
        assert_eq!(
            StreamId::from_byte(StreamId::Chat.to_byte()),
            Some(StreamId::Chat)
        );
        assert_eq!(StreamId::from_byte(200), None);
    }
}
