//! **파일 전송 와이어 + 수신 조립기** — 협상→수락→청크→완료(M4-2 슬라이스 1 · FR-X-3/4/6).
//!
//! ## 와이어 (`StreamId::File` 스트림)
//!
//! `[ver 1B][kind 1B][xfer_id 16B][이하 kind별]`
//!
//! | kind | 값 | 본문 |
//! |---|---|---|
//! | Offer | 1 | `[size 8B][sha256 32B][name_len 2B][name bytes(원본)]` |
//! | Accept | 2 | — |
//! | Reject | 3 | `[reason 1B][limit 8B]` — `limit` = 수신측 상한 공지(0 = 미공개) |
//! | Chunk | 4 | `[offset 8B][data …]` |
//! | Done | 5 | — |
//! | Cancel | 6 | — |
//!
//! 원칙: **수락 전에는 데이터가 오지 않는다**(협상 — 게이트 1단계 · [docs/04]). 수신 조립기는
//! 크기 상한·순서(연속 오프셋)·선언 크기 초과를 전부 거부하고(fail-closed), 완료 시 **선언
//! SHA-256과 실측 해시 대조는 호출자(조립 지점)의 해시 포트** 몫이다 — core는 크립토를 모른다
//! (DR-21). 통과한 원본은 `.beepq` 봉인(`nbeep-safe`)으로 넘어간다 — **실행 파일이 평문으로
//! 디스크에 닿는 경로가 없다.**
//!
//! 미지 `ver`·`kind`·잘림은 전부 [`XferError`] — 조용히 넘어가지 않는다.

use std::collections::HashMap;

/// 와이어 버전(미지 버전 거부).
const WIRE_VER: u8 = 1;

/// 전송 식별자(발신자가 생성 — 세션 내 유일).
pub type XferId = [u8; 16];

/// 청크 최대 크기(32 KiB) — Noise 프레임 상한(65535 − AEAD 태그 16 − mux/xfer 헤더 27)
/// 안에 **여유 있게** 들어가야 한다. 64KiB로 잡으면 헤더+태그를 더한 뒤 상한을 넘겨
/// 전송 자체가 실패한다(실측: 128KiB 전송이 첫 청크에서 세션 끊김 — 08-09).
pub const MAX_CHUNK: usize = 32 * 1024;

/// 수신 상한 **기본값**(256 MiB) — 실제 상한은 **수신측이 설정**한다
/// ([`XferInbox::with_max_file`] — 사용자 확정 08-09: 핸드셰이크에서 수신측 제약).
pub const MAX_FILE: u64 = 256 * 1024 * 1024;

/// 거부 사유 와이어 값.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RejectWhy {
    /// 수신자가 거절.
    Declined,
    /// 크기 상한 초과.
    TooLarge,
    /// 동시 전송 상한 등 일시 불가.
    Busy,
}

impl RejectWhy {
    fn to_byte(self) -> u8 {
        match self {
            Self::Declined => 0,
            Self::TooLarge => 1,
            Self::Busy => 2,
        }
    }
    fn from_byte(b: u8) -> Self {
        match b {
            1 => Self::TooLarge,
            2 => Self::Busy,
            _ => Self::Declined, // 미지 사유 = 일반 거절(전방 호환)
        }
    }
}

/// 파일 스트림 메시지.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum XferMsg {
    /// 전송 제안 — 이름은 **원본 바이트**(정규화는 실체화 직전 · [docs/11 §4]).
    Offer {
        /// 전송 id.
        id: XferId,
        /// 원본 전체 크기.
        size: u64,
        /// 원본 전체의 SHA-256(발신 선언 — 수신이 재검증).
        sha256: [u8; 32],
        /// 원본 파일명 바이트.
        name: Vec<u8>,
    },
    /// 수락(이때부터 청크 허용).
    Accept {
        /// 전송 id.
        id: XferId,
    },
    /// 거절 — `TooLarge`면 수신측 상한을 공지해 발신자가 재시도 판단을 할 수 있다.
    Reject {
        /// 전송 id.
        id: XferId,
        /// 사유.
        why: RejectWhy,
        /// 수신측 상한 공지(바이트 · 0 = 미공개/무관).
        limit: u64,
    },
    /// 데이터 청크(연속 오프셋 강제).
    Chunk {
        /// 전송 id.
        id: XferId,
        /// 시작 오프셋.
        offset: u64,
        /// 데이터(≤ [`MAX_CHUNK`]).
        data: Vec<u8>,
    },
    /// 발신 완료 선언.
    Done {
        /// 전송 id.
        id: XferId,
    },
    /// 취소(양쪽 어디서든).
    Cancel {
        /// 전송 id.
        id: XferId,
    },
}

/// 와이어 오류 — 전부 명시 거부(fail-closed).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum XferError {
    /// 잘린 메시지.
    Truncated,
    /// 미지 버전.
    Version(u8),
    /// 미지 kind.
    Kind(u8),
    /// 청크 크기 초과.
    ChunkTooBig,
    /// 파일 크기 상한 초과(오퍼 거부 사유).
    FileTooBig,
    /// 미지 전송 id.
    UnknownXfer,
    /// 수락 전 청크 도착(프로토콜 위반).
    NotAccepted,
    /// 오프셋 불연속(순서 위반).
    OutOfOrder,
    /// 선언 크기 초과 수신.
    Overflow,
    /// 완료 시점 크기 불일치.
    SizeMismatch,
}

impl core::fmt::Display for XferError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "{self:?}")
    }
}
impl std::error::Error for XferError {}

const K_OFFER: u8 = 1;
const K_ACCEPT: u8 = 2;
const K_REJECT: u8 = 3;
const K_CHUNK: u8 = 4;
const K_DONE: u8 = 5;
const K_CANCEL: u8 = 6;

impl XferMsg {
    /// 와이어 인코딩.
    #[must_use]
    pub fn encode(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(64);
        out.push(WIRE_VER);
        match self {
            Self::Offer {
                id,
                size,
                sha256,
                name,
            } => {
                out.push(K_OFFER);
                out.extend_from_slice(id);
                out.extend_from_slice(&size.to_le_bytes());
                out.extend_from_slice(sha256);
                let nlen = u16::try_from(name.len()).unwrap_or(u16::MAX);
                out.extend_from_slice(&nlen.to_le_bytes());
                out.extend_from_slice(&name[..nlen as usize]);
            }
            Self::Accept { id } => {
                out.push(K_ACCEPT);
                out.extend_from_slice(id);
            }
            Self::Reject { id, why, limit } => {
                out.push(K_REJECT);
                out.extend_from_slice(id);
                out.push(why.to_byte());
                out.extend_from_slice(&limit.to_le_bytes());
            }
            Self::Chunk { id, offset, data } => {
                out.push(K_CHUNK);
                out.extend_from_slice(id);
                out.extend_from_slice(&offset.to_le_bytes());
                out.extend_from_slice(data);
            }
            Self::Done { id } => {
                out.push(K_DONE);
                out.extend_from_slice(id);
            }
            Self::Cancel { id } => {
                out.push(K_CANCEL);
                out.extend_from_slice(id);
            }
        }
        out
    }

    /// 와이어 해석 — 미지 버전/kind·잘림은 오류(fail-closed).
    ///
    /// # Errors
    /// [`XferError::Truncated`]·[`XferError::Version`]·[`XferError::Kind`]·[`XferError::ChunkTooBig`].
    pub fn decode(bytes: &[u8]) -> Result<Self, XferError> {
        if bytes.len() < 2 + 16 {
            return Err(XferError::Truncated);
        }
        if bytes[0] != WIRE_VER {
            return Err(XferError::Version(bytes[0]));
        }
        let kind = bytes[1];
        let mut id = [0u8; 16];
        id.copy_from_slice(&bytes[2..18]);
        let rest = &bytes[18..];
        Ok(match kind {
            K_OFFER => {
                if rest.len() < 8 + 32 + 2 {
                    return Err(XferError::Truncated);
                }
                let size = u64::from_le_bytes(rest[0..8].try_into().expect("8B"));
                let mut sha256 = [0u8; 32];
                sha256.copy_from_slice(&rest[8..40]);
                let nlen = u16::from_le_bytes([rest[40], rest[41]]) as usize;
                if rest.len() < 42 + nlen {
                    return Err(XferError::Truncated);
                }
                Self::Offer {
                    id,
                    size,
                    sha256,
                    name: rest[42..42 + nlen].to_vec(),
                }
            }
            K_ACCEPT => Self::Accept { id },
            K_REJECT => {
                if rest.len() < 9 {
                    return Err(XferError::Truncated);
                }
                let why = RejectWhy::from_byte(rest[0]);
                let limit = u64::from_le_bytes(rest[1..9].try_into().expect("8B"));
                Self::Reject { id, why, limit }
            }
            K_CHUNK => {
                if rest.len() < 8 {
                    return Err(XferError::Truncated);
                }
                let offset = u64::from_le_bytes(rest[0..8].try_into().expect("8B"));
                let data = rest[8..].to_vec();
                if data.len() > MAX_CHUNK {
                    return Err(XferError::ChunkTooBig);
                }
                Self::Chunk { id, offset, data }
            }
            K_DONE => Self::Done { id },
            K_CANCEL => Self::Cancel { id },
            k => return Err(XferError::Kind(k)),
        })
    }
}

/// 발신 청커 — 원본을 [`MAX_CHUNK`] 단위 [`XferMsg::Chunk`]로 자른다.
#[must_use]
pub fn chunks_of(id: XferId, original: &[u8]) -> Vec<XferMsg> {
    original
        .chunks(MAX_CHUNK)
        .enumerate()
        .map(|(i, c)| XferMsg::Chunk {
            id,
            offset: (i * MAX_CHUNK) as u64,
            data: c.to_vec(),
        })
        .collect()
}

/// 수신 중 전송 하나의 상태.
#[derive(Debug)]
struct Incoming {
    size: u64,
    sha256: [u8; 32],
    name: Vec<u8>,
    accepted: bool,
    buf: Vec<u8>,
}

/// 완료된 수신물 — 해시 대조·`.beepq` 봉인은 조립 지점 몫(core는 크립토를 모른다).
#[derive(Debug, PartialEq, Eq)]
pub struct Received {
    /// 전송 id.
    pub id: XferId,
    /// 원본 파일명 바이트.
    pub name: Vec<u8>,
    /// 발신이 선언한 SHA-256 — **호출자가 `bytes` 실측 해시와 대조해야 한다**(FR-X-6).
    pub declared_sha256: [u8; 32],
    /// 수신 원본.
    pub bytes: Vec<u8>,
}

/// 수신 조립기 — 세션당 하나. 오퍼 수신→(호스트 결정) 수락→청크 연속 조립→완료.
///
/// **크기 상한은 수신측 정책**이다(사용자 확정 08-09) — 발신측 상수가 아니라 수신자가
/// [`XferInbox::with_max_file`]/[`XferInbox::set_max_file`]로 정하고, 초과 오퍼는
/// [`XferError::FileTooBig`]으로 거부하며 거절 와이어에 상한을 공지한다.
#[derive(Debug)]
pub struct XferInbox {
    inflight: HashMap<XferId, Incoming>,
    /// 수신 허용 상한(바이트).
    max_file: u64,
}

impl Default for XferInbox {
    fn default() -> Self {
        Self::new()
    }
}

impl XferInbox {
    /// 새 조립기(상한 = [`MAX_FILE`] 기본값).
    #[must_use]
    pub fn new() -> Self {
        Self {
            inflight: HashMap::new(),
            max_file: MAX_FILE,
        }
    }

    /// 수신 상한을 정해 만든다(수신측 정책 — 설정/호스트가 주입).
    #[must_use]
    pub fn with_max_file(max_file: u64) -> Self {
        Self {
            inflight: HashMap::new(),
            max_file,
        }
    }

    /// 수신 상한 변경(설정 hot-swap 경로).
    pub fn set_max_file(&mut self, max_file: u64) {
        self.max_file = max_file;
    }

    /// 현재 수신 상한(바이트) — 거절 공지·UI 표시용.
    #[must_use]
    pub fn max_file(&self) -> u64 {
        self.max_file
    }

    /// 오퍼 접수 — 상한 검사만 하고 **대기**(수락은 호스트/사용자 결정 · FR-X-3).
    ///
    /// # Errors
    /// [`XferError::FileTooBig`] — 오퍼 즉시 거부 사유.
    pub fn offer(&mut self, msg: &XferMsg) -> Result<(), XferError> {
        let XferMsg::Offer {
            id,
            size,
            sha256,
            name,
        } = msg
        else {
            return Ok(()); // 오퍼 아님 — 무시(호출 편의)
        };
        if *size > self.max_file {
            return Err(XferError::FileTooBig);
        }
        self.inflight.insert(
            *id,
            Incoming {
                size: *size,
                sha256: *sha256,
                name: name.clone(),
                accepted: false,
                buf: Vec::new(),
            },
        );
        Ok(())
    }

    /// 호스트가 수락 확정 — 이후 청크가 허용된다.
    ///
    /// # Errors
    /// [`XferError::UnknownXfer`].
    pub fn accept(&mut self, id: &XferId) -> Result<(), XferError> {
        self.inflight
            .get_mut(id)
            .map(|x| x.accepted = true)
            .ok_or(XferError::UnknownXfer)
    }

    /// 거절/취소 — 부분 수신물 즉시 폐기(잔존 금지).
    pub fn drop_xfer(&mut self, id: &XferId) {
        self.inflight.remove(id);
    }

    /// 청크 조립 — 수락 전·불연속·선언 크기 초과 전부 거부(fail-closed · 해당 전송 폐기).
    ///
    /// # Errors
    /// [`XferError`] — 오류 시 해당 전송은 폐기된다(부분물 잔존 금지).
    pub fn chunk(&mut self, id: &XferId, offset: u64, data: &[u8]) -> Result<(), XferError> {
        let r = self.chunk_inner(id, offset, data);
        if r.is_err() {
            self.inflight.remove(id);
        }
        r
    }

    fn chunk_inner(&mut self, id: &XferId, offset: u64, data: &[u8]) -> Result<(), XferError> {
        if data.len() > MAX_CHUNK {
            return Err(XferError::ChunkTooBig);
        }
        let x = self.inflight.get_mut(id).ok_or(XferError::UnknownXfer)?;
        if !x.accepted {
            return Err(XferError::NotAccepted);
        }
        if offset != x.buf.len() as u64 {
            return Err(XferError::OutOfOrder);
        }
        if x.buf.len() as u64 + data.len() as u64 > x.size {
            return Err(XferError::Overflow);
        }
        x.buf.extend_from_slice(data);
        Ok(())
    }

    /// 완료 처리 — 크기 대조 후 수신물 반환(해시 대조는 호출자 몫 — 문서 참조).
    ///
    /// # Errors
    /// [`XferError::UnknownXfer`]·[`XferError::SizeMismatch`](폐기).
    pub fn done(&mut self, id: &XferId) -> Result<Received, XferError> {
        let x = self.inflight.remove(id).ok_or(XferError::UnknownXfer)?;
        if x.buf.len() as u64 != x.size {
            return Err(XferError::SizeMismatch);
        }
        Ok(Received {
            id: *id,
            name: x.name,
            declared_sha256: x.sha256,
            bytes: x.buf,
        })
    }

    /// 진행률(수신 바이트, 선언 크기) — UI 표시용.
    #[must_use]
    pub fn progress(&self, id: &XferId) -> Option<(u64, u64)> {
        self.inflight.get(id).map(|x| (x.buf.len() as u64, x.size))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn xid(b: u8) -> XferId {
        [b; 16]
    }

    #[test]
    fn wire_roundtrip_all_kinds() {
        let msgs = [
            XferMsg::Offer {
                id: xid(1),
                size: 3,
                sha256: [9u8; 32],
                name: "보고서.pdf".as_bytes().to_vec(),
            },
            XferMsg::Accept { id: xid(1) },
            XferMsg::Reject {
                id: xid(1),
                why: RejectWhy::TooLarge,
                limit: 8 * 1024 * 1024,
            },
            XferMsg::Chunk {
                id: xid(1),
                offset: 0,
                data: vec![1, 2, 3],
            },
            XferMsg::Done { id: xid(1) },
            XferMsg::Cancel { id: xid(1) },
        ];
        for m in msgs {
            assert_eq!(XferMsg::decode(&m.encode()).unwrap(), m);
        }
    }

    #[test]
    fn wire_rejects_unknown_and_truncated() {
        assert_eq!(XferMsg::decode(&[]), Err(XferError::Truncated));
        let mut v = XferMsg::Accept { id: xid(1) }.encode();
        v[0] = 9;
        assert_eq!(XferMsg::decode(&v), Err(XferError::Version(9)));
        let mut v = XferMsg::Accept { id: xid(1) }.encode();
        v[1] = 99;
        assert_eq!(XferMsg::decode(&v), Err(XferError::Kind(99)));
    }

    #[test]
    fn full_receive_flow_and_progress() {
        let mut inbox = XferInbox::new();
        let original: Vec<u8> = (0..200_000u32).map(|i| (i % 251) as u8).collect();
        let offer = XferMsg::Offer {
            id: xid(2),
            size: original.len() as u64,
            sha256: [7u8; 32],
            name: b"big.bin".to_vec(),
        };
        inbox.offer(&offer).unwrap();
        inbox.accept(&xid(2)).unwrap();
        for m in chunks_of(xid(2), &original) {
            let XferMsg::Chunk { id, offset, data } = m else {
                unreachable!()
            };
            inbox.chunk(&id, offset, &data).unwrap();
        }
        assert_eq!(
            inbox.progress(&xid(2)),
            Some((original.len() as u64, original.len() as u64))
        );
        let got = inbox.done(&xid(2)).unwrap();
        assert_eq!(got.bytes, original);
        assert_eq!(got.declared_sha256, [7u8; 32], "해시 대조는 호출자 몫");
    }

    #[test]
    fn chunk_before_accept_is_violation_and_drops() {
        let mut inbox = XferInbox::new();
        inbox
            .offer(&XferMsg::Offer {
                id: xid(3),
                size: 10,
                sha256: [0u8; 32],
                name: b"x".to_vec(),
            })
            .unwrap();
        assert_eq!(
            inbox.chunk(&xid(3), 0, b"data"),
            Err(XferError::NotAccepted),
            "수락 전 데이터 = 프로토콜 위반"
        );
        assert!(inbox.progress(&xid(3)).is_none(), "위반 = 즉시 폐기");
    }

    #[test]
    fn out_of_order_and_overflow_drop_partials() {
        let mut inbox = XferInbox::new();
        let offer = |id| XferMsg::Offer {
            id,
            size: 8,
            sha256: [0u8; 32],
            name: b"x".to_vec(),
        };
        inbox.offer(&offer(xid(4))).unwrap();
        inbox.accept(&xid(4)).unwrap();
        assert_eq!(inbox.chunk(&xid(4), 4, b"late"), Err(XferError::OutOfOrder));
        assert!(inbox.progress(&xid(4)).is_none());

        inbox.offer(&offer(xid(5))).unwrap();
        inbox.accept(&xid(5)).unwrap();
        assert_eq!(
            inbox.chunk(&xid(5), 0, b"way too much data"),
            Err(XferError::Overflow)
        );
    }

    #[test]
    fn done_size_mismatch_discards() {
        let mut inbox = XferInbox::new();
        inbox
            .offer(&XferMsg::Offer {
                id: xid(6),
                size: 10,
                sha256: [0u8; 32],
                name: b"x".to_vec(),
            })
            .unwrap();
        inbox.accept(&xid(6)).unwrap();
        inbox.chunk(&xid(6), 0, b"short").unwrap();
        assert_eq!(inbox.done(&xid(6)), Err(XferError::SizeMismatch));
        assert!(inbox.progress(&xid(6)).is_none(), "부분 수신물 잔존 금지");
    }

    #[test]
    fn oversized_offer_rejected() {
        let mut inbox = XferInbox::new();
        let r = inbox.offer(&XferMsg::Offer {
            id: xid(7),
            size: MAX_FILE + 1,
            sha256: [0u8; 32],
            name: b"huge".to_vec(),
        });
        assert_eq!(r, Err(XferError::FileTooBig));
    }

    #[test]
    fn receiver_sets_its_own_limit() {
        // 수신측 정책 — 발신 상수가 아니라 수신자가 정한다(사용자 확정 08-09).
        let mut inbox = XferInbox::with_max_file(1024);
        assert_eq!(inbox.max_file(), 1024);
        let offer = |size| XferMsg::Offer {
            id: xid(8),
            size,
            sha256: [0u8; 32],
            name: b"f".to_vec(),
        };
        assert_eq!(inbox.offer(&offer(1025)), Err(XferError::FileTooBig));
        assert!(inbox.offer(&offer(1024)).is_ok(), "상한 이하 = 접수");
        // hot-swap 축소 후 새 오퍼부터 적용.
        inbox.set_max_file(10);
        assert_eq!(inbox.offer(&offer(11)), Err(XferError::FileTooBig));
    }
}
