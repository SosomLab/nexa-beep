//! 대화 메시지 — **봉투·논리 시퀀스·중복 제거·팬아웃**(M2-4 · FR-M-9 · FR-G-6).
//!
//! ## 봉투 (와이어)
//!
//! `[ver 1B][sender_device 32B][seq 8B][kind 1B][utf-8 본문]`
//!
//! - **`sender_device`가 봉투에 있는 이유**(D-21 V1-3): 세션이 이미 상대를 인증하지만, v2에서
//!   **같은 메시지가 다른 기기·다른 경로로 재전달**된다(sender copy — [docs/20] §6). 원 발신 기기를
//!   봉투가 들고 있어야 경로와 무관하게 **같은 메시지임을 판정**할 수 있다. v1 수신자는 이 값이
//!   **세션 인증 상대와 일치하는지 검증**한다(불일치 = 위조 시도 — 거부).
//! - **미지 `ver`는 거부, 미지 `kind`는 [`MessageBody::Unsupported`]** — 버전은 봉투 해석 자체가
//!   불가능하지만 kind는 "새 종류의 내용"일 뿐이다. v2가 종류를 추가해도 v1은 스레드 순서를 유지한
//!   채 *"지원하지 않는 메시지"* 로 표시할 수 있다(조용히 버리면 시퀀스가 구멍난다).
//!
//! ## 중복 제거 (FR-M-9)
//!
//! 열쇠는 **(원 발신 기기, 논리 시퀀스)** 쌍이다. 여러 경로·여러 기기로 같은 메시지가 와도
//! 이 쌍이 같으면 한 번만 표시한다. 이름·IP·세션이 아니라 **봉투의 암호학적 신원 기준**이다.

use crate::identity::{PeerId, Recipients};
use crate::mux::{MuxSession, StreamId};
use crate::session::{Session, SessionError};
use std::collections::{BTreeSet, HashMap};

/// 봉투 버전(미지 버전 거부).
const WIRE_VER: u8 = 1;

/// 본문 종류 와이어 값. 추가는 뒤에 append(값 불변).
// ★ N-1(08-17): kind는 **하위 니블(0~15)** — 상위 니블은 등급(Importance).
// 메시지 종류가 16개면 충분(text·향후 reaction/edit 등). 상위 니블 kind는 못 쓴다.
const KIND_TEXT: u8 = 1;

/// 본문 — v1은 텍스트만. Image·File은 무해화 게이트와 함께 M4.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MessageBody {
    /// UTF-8 텍스트(표시 전 무해화는 M2-6 — [`crate::name`] 계열).
    Text(String),
    /// 미래 버전이 추가한 종류 — 순서는 유지하되 "지원하지 않음"으로 표시(전방 호환).
    Unsupported(u8),
}

/// 메시지 등급(N-1 · ADR-0010 §3-1 — **발신자의 중요도 요청**). 수신자의 알림
/// 강도는 별개 축(수신자가 신뢰 게이트로 판정 · ADR-0010 §3-2). 와이어에는
/// `kind` 바이트의 **상위 니블**로 실린다: `Normal`(0)이면 바이트가 종전과
/// **동일**(완전 하위 호환) · 미지 값(구버전이 못 읽는 신값)은 수신 시 `Normal`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Importance {
    /// 보통(기본) — 알림 강도는 수신자 정책대로.
    #[default]
    Normal,
    /// 알림 — "지금 봐 줬으면".
    Notice,
    /// 긴급 — 확인 마찰 있음(ADR-0010 §3).
    Urgent,
}

impl Importance {
    /// 와이어 니블 값.
    #[must_use]
    pub fn to_nibble(self) -> u8 {
        match self {
            Self::Normal => 0,
            Self::Notice => 1,
            Self::Urgent => 2,
        }
    }

    /// 니블에서 — **미지 값은 Normal**(전방 호환 · ADR-0010).
    #[must_use]
    pub fn from_nibble(n: u8) -> Self {
        match n {
            1 => Self::Notice,
            2 => Self::Urgent,
            _ => Self::Normal,
        }
    }
}

/// 대화 메시지 봉투.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ChatMessage {
    /// 원 발신 **기기**(D-21 V1-3). v1 수신 경로에서 세션 인증 상대와 일치해야 한다.
    pub sender_device: PeerId,
    /// 원 발신 기기 기준 논리 시퀀스(단조 증가) — 중복 제거·정렬의 근거.
    pub seq: u64,
    /// 본문.
    pub body: MessageBody,
    /// 발신자 등급(N-1 · 기본 Normal). 와이어 = kind 상위 니블.
    pub importance: Importance,
}

/// 봉투 해석 오류.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WireError {
    /// 길이가 최소 봉투(42B)보다 짧다.
    Truncated,
    /// 미지 봉투 버전.
    Version(u8),
    /// 본문이 유효한 UTF-8이 아니다.
    Utf8,
    /// 봉투의 발신 기기가 세션 인증 상대와 다르다(위조 시도).
    SenderMismatch,
}

/// 봉투 최소 길이 — ver(1) + sender(32) + seq(8) + kind(1).
const HEADER: usize = 1 + PeerId::LEN + 8 + 1;

impl ChatMessage {
    /// 와이어 직렬화.
    #[must_use]
    pub fn encode(&self) -> Vec<u8> {
        let (kind, text): (u8, &str) = match &self.body {
            MessageBody::Text(t) => (KIND_TEXT, t.as_str()),
            MessageBody::Unsupported(k) => (*k, ""),
        };
        // 등급은 kind 상위 니블(N-1) — Normal(0)이면 종전 바이트와 동일(하위 호환).
        // kind는 하위 니블만(0~15 · 상위는 등급 전용) — 니블 도입의 대가.
        let kind = (kind & 0x0F) | (self.importance.to_nibble() << 4);
        let mut out = Vec::with_capacity(HEADER + text.len());
        out.push(WIRE_VER);
        out.extend_from_slice(self.sender_device.as_bytes());
        out.extend_from_slice(&self.seq.to_be_bytes());
        out.push(kind);
        out.extend_from_slice(text.as_bytes());
        out
    }

    /// 와이어 해석. `authenticated`는 이 봉투를 실어온 **세션의 인증 상대** —
    /// 봉투의 `sender_device`와 일치해야 한다(v1 — 재전달은 v2에서 완화).
    ///
    /// # Errors
    /// 길이·버전·UTF-8·발신자 불일치 시 [`WireError`].
    pub fn decode(bytes: &[u8], authenticated: PeerId) -> Result<Self, WireError> {
        if bytes.len() < HEADER {
            return Err(WireError::Truncated);
        }
        if bytes[0] != WIRE_VER {
            return Err(WireError::Version(bytes[0]));
        }
        let sender_bytes: [u8; PeerId::LEN] = bytes[1..1 + PeerId::LEN]
            .try_into()
            .map_err(|_| WireError::Truncated)?;
        let sender_device = PeerId::from_bytes(sender_bytes);
        if sender_device != authenticated {
            return Err(WireError::SenderMismatch);
        }
        let seq_bytes: [u8; 8] = bytes[1 + PeerId::LEN..1 + PeerId::LEN + 8]
            .try_into()
            .map_err(|_| WireError::Truncated)?;
        let seq = u64::from_be_bytes(seq_bytes);
        let raw = bytes[HEADER - 1];
        let kind = raw & 0x0F; // 하위 니블 = 메시지 종류
        let importance = Importance::from_nibble(raw >> 4); // 상위 니블 = 등급(N-1)
        let body = match kind {
            KIND_TEXT => MessageBody::Text(
                String::from_utf8(bytes[HEADER..].to_vec()).map_err(|_| WireError::Utf8)?,
            ),
            other => MessageBody::Unsupported(other),
        };
        Ok(Self {
            sender_device,
            seq,
            body,
            importance,
        })
    }
}

/// 발신 시퀀서 — 기기당 하나, 단조 증가. 영속(재시작 후 이어가기)은 M2-5 `store` 소관.
#[derive(Debug)]
pub struct Sequencer {
    next: u64,
}

impl Default for Sequencer {
    fn default() -> Self {
        Self::new()
    }
}

impl Sequencer {
    /// **1부터** 시작하는 시퀀서 — 0은 표시·ack 계층의 "seq 없음" 센티널이라
    /// 발급하지 않는다(08-18 실기: 매 세션 첫 메시지(seq 0)만 전달/읽음 마크가
    /// 영영 안 붙었다 — 수신측 읽음 ack 생략 + 발신측 갱신·페인트 제외 동시).
    /// 와이어는 숫자일 뿐이라 구버전 혼용 무해(구버전의 seq 0 첫 메시지만 종전과
    /// 동일하게 마크 없이 지나간다).
    #[must_use]
    pub fn new() -> Self {
        Self { next: 1 }
    }

    /// 재시작 복원용 — 마지막으로 쓴 값 다음부터.
    #[must_use]
    pub fn resume_after(last: u64) -> Self {
        Self { next: last + 1 }
    }

    /// 다음 시퀀스를 발급한다(호출마다 증가).
    pub fn issue(&mut self) -> u64 {
        let s = self.next;
        self.next += 1;
        s
    }
}

/// 발신 기기당 중복 제거 창 크기 — 넘으면 가장 오래된 시퀀스부터 잊는다.
pub const DEDUP_WINDOW: usize = 1024;

/// 수신 중복 제거 — 열쇠는 **(발신 기기, 시퀀스)**(FR-M-9).
#[derive(Debug, Default)]
pub struct DedupIndex {
    seen: HashMap<PeerId, BTreeSet<u64>>,
}

impl DedupIndex {
    /// 빈 인덱스.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// 처음 보는 메시지면 `true`(표시 대상), 이미 본 것이면 `false`(버린다).
    ///
    /// 창을 넘긴 옛 시퀀스가 다시 오면 창 최솟값보다 작은 값은 **중복으로 간주**한다
    /// (fail-closed — 같은 메시지가 두 번 보이는 것보다 낫다).
    pub fn accept(&mut self, sender: PeerId, seq: u64) -> bool {
        let set = self.seen.entry(sender).or_default();
        if let Some(&min) = set.first() {
            if set.len() >= DEDUP_WINDOW && seq < min {
                return false; // 창 밖 과거 — 재전송으로 간주
            }
        }
        if !set.insert(seq) {
            return false;
        }
        while set.len() > DEDUP_WINDOW {
            set.pop_first();
        }
        true
    }

    /// **새 세션 성립 시** 그 기기의 기억을 지운다 — seq 공간은 사실상 프로세스·대화
    /// 수명이라(영속은 M2-5b 전까지 없음), 상대가 재시작·재대화로 처음부터 다시
    /// 발급하면 옛 기억이 새 메시지를 **조용히 중복 폐기**한다(08-13 실기 — 재대화
    /// 메시지 증발). 옛 세션 메시지의 재생은 Noise 세션 키가 원천 차단하므로, 중복
    /// 창의 소임(재전송·다중 경로)은 **세션 안**에서만 유효하다 — 세션 경계에서
    /// 리셋해도 안전성이 줄지 않는다.
    pub fn reset_device(&mut self, sender: PeerId) {
        self.seen.remove(&sender);
    }
}

/// 팬아웃 결과 — 개별 전달 상태(FR-G-4). 스레드 UI가 이 목록으로 전달/실패를 표시한다.
pub type FanoutReport = Vec<(PeerId, Result<(), SessionError>)>;

/// **발신 단일 경로**(FR-G-6) — 1:1(원소 1개)·그룹·다중 기기(v2)가 전부 이 함수 하나를 탄다.
///
/// `sessions`에서 [`Recipients`]에 해당하는 세션을 찾아 Chat 스트림으로 보낸다.
/// 세션이 없는 대상은 [`SessionError::Closed`]로 보고한다(오프라인 큐는 M4-6).
/// **일부 실패가 나머지를 막지 않는다** — 전 대상을 시도하고 개별 결과를 돌려준다.
pub fn fanout<S: Session>(
    sessions: &mut [MuxSession<S>],
    recipients: &Recipients,
    message: &ChatMessage,
) -> FanoutReport {
    let bytes = message.encode();
    recipients
        .peers()
        .iter()
        .map(|&peer| {
            let result = sessions
                .iter_mut()
                .find(|s| s.peer() == peer)
                .map_or(Err(SessionError::Closed), |s| {
                    s.send(StreamId::Chat, &bytes)
                });
            (peer, result)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// N-1 — 등급 니블. Normal은 바이트 완전 동일(하위 호환)·미지 등급 = Normal.
    #[test]
    fn importance_rides_kind_nibble_normal_is_identical() {
        let peer = PeerId::from_bytes([7u8; PeerId::LEN]);
        let base = ChatMessage {
            sender_device: peer,
            seq: 1,
            body: MessageBody::Text("hi".into()),
            importance: Importance::Normal,
        };
        let urgent = ChatMessage {
            importance: Importance::Urgent,
            ..base.clone()
        };
        // Normal 인코딩 = kind 바이트 상위 니블 0 → 종전과 동일.
        let nb = base.encode();
        assert_eq!(nb[HEADER - 1] & 0xF0, 0, "Normal = 상위 니블 0(하위 호환)");
        // Urgent는 상위 니블 2 · 왕복 보존.
        let ub = urgent.encode();
        assert_eq!(ub[HEADER - 1] >> 4, 2);
        assert_eq!(
            ChatMessage::decode(&ub, peer).unwrap().importance,
            Importance::Urgent
        );
        assert_eq!(
            ChatMessage::decode(&nb, peer).unwrap().importance,
            Importance::Normal
        );
        // 미지 등급 니블(예: 9)은 Normal로 강등(전방 호환).
        let mut weird = nb;
        weird[HEADER - 1] = (9 << 4) | (weird[HEADER - 1] & 0x0F);
        assert_eq!(
            ChatMessage::decode(&weird, peer).unwrap().importance,
            Importance::Normal
        );
    }

    fn pid(b: u8) -> PeerId {
        let mut a = [0u8; 32];
        a[0] = b;
        PeerId::from_bytes(a)
    }

    fn msg(sender: PeerId, seq: u64, text: &str) -> ChatMessage {
        ChatMessage {
            sender_device: sender,
            seq,
            body: MessageBody::Text(text.into()),
            importance: Importance::Normal,
        }
    }

    #[test]
    fn roundtrip() {
        let m = msg(pid(1), 42, "안녕, bob");
        let decoded = ChatMessage::decode(&m.encode(), pid(1)).unwrap();
        assert_eq!(decoded, m);
    }

    #[test]
    fn golden_wire_bytes_are_stable() {
        // 와이어 회귀 고정 — 이 바이트가 바뀌면 호환이 깨진 것이다(버전 없이 바꾸지 말 것).
        let m = msg(pid(1), 0x0102, "hi");
        let bytes = m.encode();
        assert_eq!(bytes[0], 1, "ver");
        assert_eq!(&bytes[1..33], pid(1).as_bytes(), "sender 32B");
        assert_eq!(&bytes[33..41], &[0, 0, 0, 0, 0, 0, 1, 2], "seq u64 BE");
        assert_eq!(bytes[41], 1, "kind=Text");
        assert_eq!(&bytes[42..], b"hi", "utf-8 본문");
        assert_eq!(bytes.len(), 44);
    }

    #[test]
    fn sender_mismatch_is_rejected() {
        // 세션 인증 상대와 봉투 발신자가 다르면 위조 시도 — 거부(v1).
        let m = msg(pid(1), 0, "spoof");
        assert_eq!(
            ChatMessage::decode(&m.encode(), pid(2)).err(),
            Some(WireError::SenderMismatch)
        );
    }

    #[test]
    fn unknown_version_rejected_unknown_kind_preserved() {
        let m = msg(pid(1), 7, "x");
        let mut bad_ver = m.encode();
        bad_ver[0] = 99;
        assert_eq!(
            ChatMessage::decode(&bad_ver, pid(1)).err(),
            Some(WireError::Version(99))
        );
        // 미지 kind(하위 니블 · N-1 이후 kind는 4비트)는 거부가 아니라 Unsupported —
        // 스레드 순서 보존(전방 호환). 상위 니블(등급)은 별개로 Normal 강등.
        let mut future_kind = m.encode();
        future_kind[41] = (2 << 4) | 8; // 등급 니블 2(Urgent) + 미지 kind 8
        let decoded = ChatMessage::decode(&future_kind, pid(1)).unwrap();
        assert_eq!(decoded.body, MessageBody::Unsupported(8));
        assert_eq!(
            decoded.importance,
            Importance::Urgent,
            "등급 니블은 독립 해석"
        );
        assert_eq!(decoded.seq, 7, "시퀀스는 유지 — 구멍나지 않는다");
    }

    #[test]
    fn invalid_utf8_rejected() {
        let m = msg(pid(1), 0, "");
        let mut bytes = m.encode();
        bytes.extend_from_slice(&[0xFF, 0xFE]);
        assert_eq!(
            ChatMessage::decode(&bytes, pid(1)).err(),
            Some(WireError::Utf8)
        );
    }

    #[test]
    fn sequencer_is_monotonic_and_resumable() {
        let mut s = Sequencer::new();
        // 1부터 — 0은 "seq 없음" 센티널이라 발급 금지(08-18 첫 메시지 마크 소실).
        assert_eq!((s.issue(), s.issue(), s.issue()), (1, 2, 3));
        let mut r = Sequencer::resume_after(3);
        assert_eq!(r.issue(), 4, "재시작 후 이어간다");
    }

    #[test]
    fn dedup_key_is_sender_plus_seq() {
        let mut d = DedupIndex::new();
        assert!(d.accept(pid(1), 0), "처음");
        assert!(!d.accept(pid(1), 0), "같은 (기기,seq) = 중복");
        assert!(d.accept(pid(2), 0), "다른 기기의 같은 seq는 별개");
        assert!(d.accept(pid(1), 1), "다음 seq");
    }

    #[test]
    fn dedup_window_forgets_oldest_but_stays_fail_closed() {
        let mut d = DedupIndex::new();
        for i in 0..DEDUP_WINDOW as u64 {
            assert!(d.accept(pid(1), i));
        }
        // 창을 넘기면 최솟값이 밀려난다.
        assert!(d.accept(pid(1), DEDUP_WINDOW as u64));
        // 밀려난 옛 시퀀스의 재전송은 "중복으로 간주"(두 번 보이는 것보다 낫다).
        assert!(!d.accept(pid(1), 0), "창 밖 과거 = 중복 간주");
    }

    #[test]
    fn dedup_reset_device_accepts_restarted_seq() {
        // 08-13 실기 — 상대가 재시작·재대화로 seq를 처음부터 다시 발급하면
        // 옛 기억이 새 메시지를 조용히 버렸다. 새 세션 성립 = reset_device.
        let mut d = DedupIndex::new();
        assert!(d.accept(pid(1), 1));
        assert!(!d.accept(pid(1), 1), "같은 세션 재전송 = 중복");
        d.reset_device(pid(1));
        assert!(d.accept(pid(1), 1), "새 세션의 seq 재시작은 새 메시지");
        // 다른 기기는 영향 없다.
        assert!(d.accept(pid(2), 1));
        d.reset_device(pid(1));
        assert!(!d.accept(pid(2), 1), "리셋은 그 기기만");
    }

    mod fanout_tests {
        use super::*;
        use crate::session::Session;
        use crate::TrustLevel;
        use std::sync::mpsc::{channel, Receiver, Sender};

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

        /// (내 쪽 mux, 상대 쪽 mux) 페어.
        fn pair(me: PeerId, other: PeerId) -> (MuxSession<PairSession>, MuxSession<PairSession>) {
            let (atx, brx) = channel();
            let (btx, arx) = channel();
            (
                MuxSession::new(PairSession {
                    peer: other,
                    tx: atx,
                    rx: arx,
                }),
                MuxSession::new(PairSession {
                    peer: me,
                    tx: btx,
                    rx: brx,
                }),
            )
        }

        #[test]
        fn one_to_one_and_group_use_the_same_path() {
            let me = pid(9);
            let (s2, mut r2) = pair(me, pid(2));
            let (s3, mut r3) = pair(me, pid(3));
            let mut sessions = vec![s2, s3];
            let m = msg(me, 0, "to all");

            // 그룹 = 집합, 1:1 = 원소 1개 — 같은 fanout 함수.
            let report = fanout(
                &mut sessions,
                &Recipients::from_peers(vec![pid(2), pid(3)]),
                &m,
            );
            assert!(report.iter().all(|(_, r)| r.is_ok()), "{report:?}");
            let got2 = ChatMessage::decode(&r2.recv(StreamId::Chat).unwrap(), me).unwrap();
            let got3 = ChatMessage::decode(&r3.recv(StreamId::Chat).unwrap(), me).unwrap();
            assert_eq!(got2, m);
            assert_eq!(got3, m);

            let solo = fanout(&mut sessions, &Recipients::one(pid(2)), &m);
            assert_eq!(solo.len(), 1);
            assert!(solo[0].1.is_ok());
        }

        #[test]
        fn partial_failure_does_not_block_the_rest() {
            let me = pid(9);
            let (s2, mut r2) = pair(me, pid(2));
            let mut sessions = vec![s2];
            let m = msg(me, 1, "hello");
            // pid(4)는 세션 없음(오프라인) — pid(2)는 그래도 받아야 한다.
            let report = fanout(
                &mut sessions,
                &Recipients::from_peers(vec![pid(2), pid(4)]),
                &m,
            );
            let by_peer: std::collections::HashMap<_, _> = report.into_iter().collect();
            assert!(by_peer[&pid(2)].is_ok());
            assert_eq!(
                by_peer[&pid(4)],
                Err(SessionError::Closed),
                "세션 없음 = 실패 보고"
            );
            assert_eq!(
                ChatMessage::decode(&r2.recv(StreamId::Chat).unwrap(), me).unwrap(),
                m
            );
        }
    }
}
