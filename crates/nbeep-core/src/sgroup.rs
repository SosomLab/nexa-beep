//! 공유 그룹(진짜 그룹 — 한 방 대화) 도메인 · 와이어(M5-1g · [docs/31] ADR-0012).
//!
//! - **roster(명부)가 그룹의 진실** — 소유자(생성자)만 바꾸고, `version` 단조 증가
//!   (낮은 버전 무시 — 구버전 재생 방지 · G-6).
//! - **배포는 소유자의 인증 세션 직접 한정**(v1.5) — Noise 세션이 "소유자가 보냈다"를
//!   보증하므로 별도 서명이 없다. 구성원 간 중계·오프라인 검증이 필요해지는 단계
//!   (G4+/v2)에서 서명(DR-20 인프라)을 얹는다. ⚠️ 그래서 **roster 갱신은 소유자
//!   세션에서 온 것만 수용해야 한다**(수용 규칙은 [`Roster::accepts_update`] + 호출자의
//!   발신자 확인 — 둘 다 필요).
//! - **초대 수락제**(G-4) — roster에 있어도 내가 수락해야 내 화면에 방이 생긴다.
//! - 순서 = 발신자별 seq + 도착순 병합(G-5) — 전역 합의 없음.
//!
//! 여기엔 I/O·암호화가 없다(순수 도메인 — 난수는 호출자가 주입).

use crate::identity::PeerId;
use crate::name::DisplayName;

/// 공유 그룹 전역 식별자 — 생성 시 호출자가 주입한 무작위 32B.
///
/// 세션 인증 배포 모델에서는 값 자체의 위조가 의미 없다 — 수신자는 uid를 "소유자
/// 세션에서 받은 초대"로만 알게 되고, 이후 갱신도 그 소유자 세션에서만 수용한다.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct GroupUid(pub [u8; 32]);

impl GroupUid {
    /// 표시용 축약(핀 지문과 같은 8자리).
    #[must_use]
    pub fn short(&self) -> String {
        self.0[..4].iter().map(|b| format!("{b:02x}")).collect()
    }
}

impl std::fmt::Debug for GroupUid {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "GroupUid({}…)", self.short())
    }
}

/// 명부 — 그룹의 진실(소유자만 갱신 · version 단조 증가).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Roster {
    /// 전역 식별자.
    pub uid: GroupUid,
    /// 그룹 이름(무해화됨 — 소유자 주장 · 수신측은 표시 전 재무해화).
    pub name: DisplayName,
    /// 소유자(생성자) — 멤버십 갱신 권한의 주체.
    pub owner: PeerId,
    /// 구성원(소유자 포함 · 정렬 — 결정적 인코딩).
    pub members: Vec<PeerId>,
    /// 단조 증가 버전 — 낮은 버전은 무시한다.
    pub version: u64,
    /// **구성원 초대 허용**(방 정책 · 기본 true — 사용자 확정 08-13).
    /// 꺼지면 소유자만 초대. 정책은 명부의 일부라 소유자가 바꾸고 배포한다.
    /// 와이어는 P-11 확장 영역에 실린다(빈 확장 = 기본 true — 구버전 호환).
    pub member_invite: bool,
}

impl Roster {
    /// 이 갱신을 받아들여도 되는가 — **같은 uid·같은 소유자·더 높은 버전**만.
    /// ⚠️ 호출자는 추가로 "이 roster가 `owner`와의 세션에서 왔는가"를 확인해야 한다
    /// (세션 인증이 서명을 대체하는 전제 — 모듈 문서 참조).
    #[must_use]
    pub fn accepts_update(&self, new: &Roster) -> bool {
        self.uid == new.uid && self.owner == new.owner && new.version > self.version
    }

    /// 구성원인가.
    #[must_use]
    pub fn has_member(&self, peer: PeerId) -> bool {
        self.members.contains(&peer)
    }

    /// 와이어·저장 공용 인코딩(결정적 — members 정렬은 생성자가 보장).
    ///
    /// 끝에 **정책 확장 영역**(`ext_len u16 + bytes` · 지금은 0)을 둔다 —
    /// [docs/32 §8 P-11]: `history_on_join` 등 정책 필드가 나중에 들어올 자리.
    /// 지금 안 만들면 필드 추가가 포맷 변경(전 roster 재배포)이 된다.
    #[must_use]
    pub fn encode(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(64 + 32 * self.members.len());
        out.extend_from_slice(&self.uid.0);
        let nb = self.name.as_str().as_bytes();
        let nlen = u16::try_from(nb.len()).unwrap_or(u16::MAX);
        out.extend_from_slice(&nlen.to_be_bytes());
        out.extend_from_slice(&nb[..usize::from(nlen)]);
        out.extend_from_slice(self.owner.as_bytes());
        out.extend_from_slice(&self.version.to_be_bytes());
        out.extend_from_slice(
            &u32::try_from(self.members.len())
                .unwrap_or(u32::MAX)
                .to_be_bytes(),
        );
        for m in &self.members {
            out.extend_from_slice(m.as_bytes());
        }
        // 정책 확장 영역(P-11) — v1 정책 = 플래그 1B(bit0 = 구성원 초대 허용).
        out.extend_from_slice(&1u16.to_be_bytes());
        out.push(u8::from(self.member_invite));
        out
    }

    /// 디코딩 — `(roster, 소비 바이트)`. 손상은 `None`(fail-closed).
    #[must_use]
    pub fn decode(bytes: &[u8]) -> Option<(Self, usize)> {
        let mut p = 0usize;
        let take = |p: &mut usize, n: usize| -> Option<&[u8]> {
            let s = bytes.get(*p..*p + n)?;
            *p += n;
            Some(s)
        };
        let uid = GroupUid(take(&mut p, 32)?.try_into().ok()?);
        let nlen = usize::from(u16::from_be_bytes(take(&mut p, 2)?.try_into().ok()?));
        let name = DisplayName::parse(std::str::from_utf8(take(&mut p, nlen)?).ok()?).ok()?;
        let owner = PeerId::from_bytes(take(&mut p, 32)?.try_into().ok()?);
        let version = u64::from_be_bytes(take(&mut p, 8)?.try_into().ok()?);
        let mc = u32::from_be_bytes(take(&mut p, 4)?.try_into().ok()?);
        // 상한 — 미쳐 큰 값의 할당 폭탄 방지(LAN 규모 · NFR-B-6).
        if mc > 4096 {
            return None;
        }
        let mut members = Vec::with_capacity(mc as usize);
        for _ in 0..mc {
            members.push(PeerId::from_bytes(take(&mut p, 32)?.try_into().ok()?));
        }
        // 정책 확장 영역(P-11) — 첫 바이트 bit0 = 구성원 초대 허용(없으면 기본 true).
        // 그 뒤 미지 내용은 **읽고 버린다**(전방 호환 · 경계는 길이 접두라 안전).
        let ext_len = usize::from(u16::from_be_bytes(take(&mut p, 2)?.try_into().ok()?));
        let ext = take(&mut p, ext_len)?;
        let member_invite = ext.first().is_none_or(|b| b & 1 != 0);
        Some((
            Self {
                uid,
                name,
                owner,
                version,
                members,
                member_invite,
            },
            p,
        ))
    }
}

/// 그룹 스트림([`crate::mux::StreamId::Group`]) 와이어 메시지(ADR-0012 §4).
/// 미지 종류는 `decode = None` — 수신측이 조용히 버린다(전방 호환).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SGroupMsg {
    /// 초대장 = 명부 전체(소유자 → 신규 구성원). 수신 UI는 수락/거절 카드.
    Invite {
        /// 서명 대신 세션 인증(모듈 문서) — 발신자가 `roster.owner`인지 확인할 것.
        roster: Roster,
    },
    /// 초대 수락(구성원 → 소유자).
    Accept {
        /// 대상 그룹.
        uid: GroupUid,
    },
    /// 초대 거절.
    Decline {
        /// 대상 그룹.
        uid: GroupUid,
    },
    /// 명부 갱신 배포(소유자 → 전 구성원 · version 증가).
    Roster {
        /// 새 명부.
        roster: Roster,
    },
    /// 방 본문(발신자 → 전 구성원 직접 팬아웃).
    Msg {
        /// 대상 그룹.
        uid: GroupUid,
        /// 발신자별 단조 seq(중복 창 — 세션 재수립 시 리셋은 dedup 규칙과 동일).
        seq: u64,
        /// 본문(수신측 표시 전 무해화).
        text: String,
    },
    /// 탈퇴 통지(구성원 → 소유자).
    Leave {
        /// 대상 그룹.
        uid: GroupUid,
    },
    /// **구성원의 초대 요청**(구성원 → 소유자 · 08-13 사용자 확정 — 기본 허용).
    /// 명부 갱신은 언제나 소유자(G-6 유지 — 단일 진실이 부분 가시성 분기를 막는다).
    /// 소유자는 방 정책(`member_invite`)이 켜져 있을 때만 반영한다.
    Suggest {
        /// 대상 그룹.
        uid: GroupUid,
        /// 초대하려는 상대들.
        members: Vec<PeerId>,
    },
}

const K_INVITE: u8 = 1;
const K_ACCEPT: u8 = 2;
const K_DECLINE: u8 = 3;
const K_ROSTER: u8 = 4;
const K_MSG: u8 = 5;
const K_LEAVE: u8 = 6;
const K_SUGGEST: u8 = 7;

impl SGroupMsg {
    /// 와이어 인코딩(kind 1B + 본문).
    #[must_use]
    pub fn encode(&self) -> Vec<u8> {
        match self {
            SGroupMsg::Invite { roster } => {
                let mut v = vec![K_INVITE];
                v.extend_from_slice(&roster.encode());
                v
            }
            SGroupMsg::Accept { uid } => {
                let mut v = vec![K_ACCEPT];
                v.extend_from_slice(&uid.0);
                v
            }
            SGroupMsg::Decline { uid } => {
                let mut v = vec![K_DECLINE];
                v.extend_from_slice(&uid.0);
                v
            }
            SGroupMsg::Roster { roster } => {
                let mut v = vec![K_ROSTER];
                v.extend_from_slice(&roster.encode());
                v
            }
            SGroupMsg::Msg { uid, seq, text } => {
                let mut v = vec![K_MSG];
                v.extend_from_slice(&uid.0);
                v.extend_from_slice(&seq.to_be_bytes());
                v.extend_from_slice(text.as_bytes());
                v
            }
            SGroupMsg::Leave { uid } => {
                let mut v = vec![K_LEAVE];
                v.extend_from_slice(&uid.0);
                v
            }
            SGroupMsg::Suggest { uid, members } => {
                let mut v = vec![K_SUGGEST];
                v.extend_from_slice(&uid.0);
                v.extend_from_slice(
                    &u32::try_from(members.len())
                        .unwrap_or(u32::MAX)
                        .to_be_bytes(),
                );
                for m in members {
                    v.extend_from_slice(m.as_bytes());
                }
                v
            }
        }
    }

    /// 와이어 디코딩 — 미지 kind·손상 = `None`(fail-closed · 전방 호환).
    #[must_use]
    pub fn decode(bytes: &[u8]) -> Option<Self> {
        let (&kind, rest) = bytes.split_first()?;
        let uid32 =
            |b: &[u8]| -> Option<GroupUid> { Some(GroupUid(b.get(..32)?.try_into().ok()?)) };
        match kind {
            K_INVITE => {
                let (roster, used) = Roster::decode(rest)?;
                (used == rest.len()).then_some(SGroupMsg::Invite { roster })
            }
            K_ACCEPT => (rest.len() == 32).then(|| SGroupMsg::Accept {
                uid: uid32(rest).unwrap_or(GroupUid([0; 32])),
            }),
            K_DECLINE => (rest.len() == 32).then(|| SGroupMsg::Decline {
                uid: uid32(rest).unwrap_or(GroupUid([0; 32])),
            }),
            K_ROSTER => {
                let (roster, used) = Roster::decode(rest)?;
                (used == rest.len()).then_some(SGroupMsg::Roster { roster })
            }
            K_MSG => {
                let uid = uid32(rest)?;
                let seq = u64::from_be_bytes(rest.get(32..40)?.try_into().ok()?);
                let text = std::str::from_utf8(rest.get(40..)?).ok()?.to_string();
                Some(SGroupMsg::Msg { uid, seq, text })
            }
            K_LEAVE => (rest.len() == 32).then(|| SGroupMsg::Leave {
                uid: uid32(rest).unwrap_or(GroupUid([0; 32])),
            }),
            K_SUGGEST => {
                let uid = uid32(rest)?;
                let mc = u32::from_be_bytes(rest.get(32..36)?.try_into().ok()?);
                if mc > 4096 {
                    return None; // 할당 폭탄 방지(roster와 같은 상한)
                }
                let mut members = Vec::with_capacity(mc as usize);
                let mut p = 36usize;
                for _ in 0..mc {
                    members.push(PeerId::from_bytes(rest.get(p..p + 32)?.try_into().ok()?));
                    p += 32;
                }
                (p == rest.len()).then_some(SGroupMsg::Suggest { uid, members })
            }
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pid(b: u8) -> PeerId {
        let mut a = [0u8; 32];
        a[0] = b;
        PeerId::from_bytes(a)
    }
    fn uid(b: u8) -> GroupUid {
        let mut a = [0u8; 32];
        a[0] = b;
        GroupUid(a)
    }
    fn roster(v: u64) -> Roster {
        Roster {
            uid: uid(9),
            name: DisplayName::parse("개발팀").unwrap(),
            owner: pid(1),
            members: vec![pid(1), pid(2), pid(3)],
            version: v,
            member_invite: true,
        }
    }

    /// G-6 — 같은 uid·같은 소유자·더 높은 버전만 수용(구버전 재생·소유자 바꿔치기 거부).
    #[test]
    fn roster_update_rules() {
        let cur = roster(3);
        assert!(cur.accepts_update(&roster(4)));
        assert!(!cur.accepts_update(&roster(3)), "같은 버전 거부");
        assert!(!cur.accepts_update(&roster(2)), "구버전 재생 거부");
        let mut hijack = roster(9);
        hijack.owner = pid(7);
        assert!(
            !hijack.accepts_update(&roster(10)) || {
                // 소유자가 다른 명부는 uid가 같아도 거부.
                !cur.accepts_update(&hijack)
            }
        );
        assert!(!cur.accepts_update(&hijack), "소유자 바꿔치기 거부");
    }

    #[test]
    fn roster_codec_roundtrip() {
        let r = roster(7);
        let enc = r.encode();
        let (back, used) = Roster::decode(&enc).unwrap();
        assert_eq!(back, r);
        assert_eq!(used, enc.len());
        // 꼬리 잘림 = 손상.
        assert!(Roster::decode(&enc[..enc.len() - 1]).is_none());
    }

    /// 정책 필드(P-11 확장 영역) — off 왕복 + 빈 확장 = 기본 허용(구버전 호환).
    #[test]
    fn policy_roundtrip_and_empty_ext_defaults_open() {
        let mut r = roster(1);
        r.member_invite = false;
        let (back, _) = Roster::decode(&r.encode()).unwrap();
        assert!(!back.member_invite, "소유자만 초대 왕복");
        // 확장 영역이 빈 구버전 인코딩 — 기본 true.
        let mut old = roster(1).encode();
        let cut = old.len() - 3; // ext(u16 len=1 + 1B) 제거
        old.truncate(cut);
        old.extend_from_slice(&0u16.to_be_bytes()); // 빈 확장
        let (back, used) = Roster::decode(&old).unwrap();
        assert!(back.member_invite, "빈 확장 = 기본 허용");
        assert_eq!(used, old.len());
    }

    #[test]
    fn msg_codec_roundtrip_and_unknown_kind() {
        for m in [
            SGroupMsg::Invite { roster: roster(1) },
            SGroupMsg::Accept { uid: uid(2) },
            SGroupMsg::Decline { uid: uid(3) },
            SGroupMsg::Roster { roster: roster(5) },
            SGroupMsg::Msg {
                uid: uid(4),
                seq: 42,
                text: "안녕 방!".into(),
            },
            SGroupMsg::Leave { uid: uid(6) },
            SGroupMsg::Suggest {
                uid: uid(7),
                members: vec![pid(4), pid(5)],
            },
        ] {
            assert_eq!(SGroupMsg::decode(&m.encode()), Some(m));
        }
        // 미지 kind = None(전방 호환 — 조용히 버림).
        assert_eq!(SGroupMsg::decode(&[99, 1, 2, 3]), None);
        assert_eq!(SGroupMsg::decode(&[]), None);
    }
}
