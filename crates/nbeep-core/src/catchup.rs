//! 형제 기기 **따라잡기**(ADR-0015 S4 · docs/46 §5-4 · docs/48 §3-6) — Control 스트림 태그 5·6·7·20.
//!
//! 같은 사용자의 기기(형제 · XXpsk3 세션)끼리는 신뢰된 통로라 **대화 기록을 세션 위로 건넨다**.
//! 꺼져 있던 기기가 돌아오면 ① 양쪽이 [`SyncDigest`](스레드별·원 발신 기기별 최대 seq)를 교환하고
//! ② 부족한 쪽이 [`SyncPull`]로 그 뒤를 청하고 ③ 가진 쪽이 [`SyncLines`]로 평문 라인을 돌려준다
//! (세션 암호화 · 받은 기기는 **자기 키로** 봉인 저장 — 기록 파일은 옮기지 않는다 [17 §3]).
//! ④ [`SyncRead`]는 "이 스레드를 여기까지 봤다"(읽음 동기 — 한 기기에서 본 대화의 안읽음이 다른
//! 기기에서도 걷힌다).
//!
//! ## 라인 열쇠 = `(origin, seq)`
//!
//! 대화는 append-only라 충돌이 없고 정렬·dedup은 **원 발신 기기 + 그 기기의 seq**다([32 §3-8-2]).
//! 그래서 seq는 **재시작을 넘어 단조**여야 한다 — 앱은 부팅 때 `max(영속값+1, 현재 Unix ms)`로
//! 시퀀서를 잇는다([`crate::Sequencer::resume_after`] · 종전엔 프로세스마다 1부터라 열쇠가 겹쳤다).
//! seq 0인 줄(로컬 안내 · 구본 기록)은 열쇠가 없어 **동기 대상이 아니다**.
//!
//! ## 상한(D-32-4 · 예산 DR-5)
//!
//! 스레드당 최근 [`SYNC_LINES_MAX`]줄 · 세션당 응답 [`SYNC_BYTES_PER_SESSION`]이 **기본값**이고,
//! 앱 설정(`user.sync_lines` · `user.sync_budget_mb`)이 바꾼다(10-19). 줄 수의 와이어 천장은
//! [`SYNC_LINES_HARD_MAX`] — 청구측은 자기 설정으로 `max`를 청하고 응답측은 `min(청구, 내 설정)`으로
//! 다시 죈다(구판 요청자는 200을 청하니 그 이상을 받지 않는다 = 전방 호환). 나머지는
//! 컨텐츠 모드(S5) 몫. 파일 본문은 따라잡지 않는다(격리 실체화는 기기별 · ADR-0007 §6-2) —
//! 텍스트 줄만 싣는다.
//!
//! ## 프레임 분할
//!
//! 응답은 세션 페이로드 상한(`mux::MAX_PAYLOAD` 65,518B) 안에서 **여러 [`SyncLines`] 프레임**으로
//! 나눠 보낸다([`SyncLines::encode_frames`]) — 받는 쪽은 프레임마다 `(origin, seq)` dedup으로 합치니
//! 분할은 보이지 않는다. 종전(10-10)엔 한 프레임이라 200줄 × 평균 330B를 넘으면 `TooLarge`로
//! **형제 세션이 끊겼다**(10-19 발견 · 전송 실패 = 세션 종료 fail-closed 경로를 타서).
//!
//! 봉투 원리: 형제 세션 안에서만 오가며, 받는 쪽은 형제 세션이 아니면 버린다(앱 몫 · fail-closed).
//! 침해 형제의 라인 주입은 새 면이 아니다(형제 = 전면 신뢰 · [48 §5] T-8) — dedup·상한만 둔다.

use crate::identity::PeerId;
use std::collections::BTreeMap;

/// 디제스트 태그(세션 성립 직후 양방향).
pub const SYNC_DIGEST_TAG: u8 = 5;
/// 청구 태그.
pub const SYNC_PULL_TAG: u8 = 6;
/// 응답(라인) 태그.
pub const SYNC_LINES_TAG: u8 = 7;
/// 읽음 동기 태그(5~7은 예약분 · 8~19 사용 중 다음 빈 번호).
pub const SYNC_READ_TAG: u8 = 20;

/// 스레드당 따라잡기 줄 상한 **기본값**(D-32-4 "스레드당 200줄" · 설정 `user.sync_lines`).
pub const SYNC_LINES_MAX: usize = 200;
/// 줄 상한의 와이어 천장 — 디코더가 받아 주는 최대(= 설정 선택지의 최대 · 10-19). 구판 디코더는
/// 200에서 멈추지만 응답측이 청구 `max`로 죄므로 구판에 이보다 큰 프레임이 가지 않는다.
pub const SYNC_LINES_HARD_MAX: usize = 500;
/// 세션당 응답 바이트 상한 **기본값**(D-32-4 "세션당 1MiB" · 설정 `user.sync_budget_mb`) — 넘으면
/// 그 세션에서는 더 응답하지 않는다.
pub const SYNC_BYTES_PER_SESSION: usize = 1024 * 1024;
/// 디제스트 항목 상한(가장 최근 스레드부터 — 프레임 1개 ≤ 약 18KB).
pub const SYNC_DIGEST_MAX: usize = 255;
/// 한 번의 디제스트 대조로 내는 청구 상한.
pub const SYNC_PULL_MAX: usize = 64;
/// 라인 본문 상한(바이트 · 메시지 상한과 같은 결 — 넘치면 문자 경계로 자른다).
pub const SYNC_TEXT_MAX: usize = 4096;
/// 발신자 라벨 상한(바이트).
pub const SYNC_FROM_MAX: usize = 64;

/// 디제스트 한 항목 — 이 스레드(상대 **기기** = 저장 키)에서 `origin`이 보낸 줄의 최대 seq.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DigestEntry {
    /// 스레드(저장 키 = 상대 기기).
    pub thread: PeerId,
    /// 원 발신 기기(상대 · 나 · 내 형제).
    pub origin: PeerId,
    /// 그 기기의 줄 가운데 최대 seq(≥1).
    pub max_seq: u64,
}

/// 디제스트 프레임: `tag ‖ n(u8) ‖ (thread 32 ‖ origin 32 ‖ max_seq 8)×n`.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SyncDigest {
    /// 항목(≤255 · 순서 = 보내는 쪽이 정한 우선순위).
    pub entries: Vec<DigestEntry>,
}

impl SyncDigest {
    /// 인코딩(상한 초과분은 버린다).
    #[must_use]
    pub fn encode(&self) -> Vec<u8> {
        let n = self.entries.len().min(SYNC_DIGEST_MAX);
        let mut out = Vec::with_capacity(2 + 72 * n);
        out.push(SYNC_DIGEST_TAG);
        #[allow(clippy::cast_possible_truncation)] // ≤ 255
        out.push(n as u8);
        for e in &self.entries[..n] {
            out.extend_from_slice(e.thread.as_bytes());
            out.extend_from_slice(e.origin.as_bytes());
            out.extend_from_slice(&e.max_seq.to_be_bytes());
        }
        out
    }

    /// 디코딩 — 태그·길이 정확 일치 · seq 0 항목 거부.
    #[must_use]
    pub fn decode(bytes: &[u8]) -> Option<Self> {
        if bytes.len() < 2 || bytes[0] != SYNC_DIGEST_TAG {
            return None;
        }
        let n = usize::from(bytes[1]);
        if bytes.len() != 2 + 72 * n {
            return None;
        }
        let mut entries = Vec::with_capacity(n);
        for i in 0..n {
            let p = 2 + 72 * i;
            let thread = PeerId::from_bytes(bytes[p..p + 32].try_into().ok()?);
            let origin = PeerId::from_bytes(bytes[p + 32..p + 64].try_into().ok()?);
            let max_seq = u64::from_be_bytes(bytes[p + 64..p + 72].try_into().ok()?);
            if max_seq == 0 {
                return None;
            }
            entries.push(DigestEntry {
                thread,
                origin,
                max_seq,
            });
        }
        Some(Self { entries })
    }
}

/// 청구 프레임: `tag ‖ thread 32 ‖ origin 32 ‖ after_seq 8 ‖ max u16`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SyncPull {
    /// 스레드(저장 키).
    pub thread: PeerId,
    /// 원 발신 기기.
    pub origin: PeerId,
    /// 이 seq **초과**분을 청한다(0 = 전부).
    pub after_seq: u64,
    /// 최대 줄 수(청구측 설정 · 응답측이 자기 설정과 [`SYNC_LINES_HARD_MAX`]로 다시 죈다).
    pub max: u16,
}

impl SyncPull {
    const LEN: usize = 1 + 32 + 32 + 8 + 2;

    /// 인코딩.
    #[must_use]
    pub fn encode(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(Self::LEN);
        out.push(SYNC_PULL_TAG);
        out.extend_from_slice(self.thread.as_bytes());
        out.extend_from_slice(self.origin.as_bytes());
        out.extend_from_slice(&self.after_seq.to_be_bytes());
        out.extend_from_slice(&self.max.to_be_bytes());
        out
    }

    /// 디코딩 — 길이 정확 일치.
    #[must_use]
    pub fn decode(bytes: &[u8]) -> Option<Self> {
        if bytes.len() != Self::LEN || bytes[0] != SYNC_PULL_TAG {
            return None;
        }
        Some(Self {
            thread: PeerId::from_bytes(bytes[1..33].try_into().ok()?),
            origin: PeerId::from_bytes(bytes[33..65].try_into().ok()?),
            after_seq: u64::from_be_bytes(bytes[65..73].try_into().ok()?),
            max: u16::from_be_bytes(bytes[73..75].try_into().ok()?),
        })
    }
}

/// 따라잡기 줄 하나(텍스트만).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SyncLine {
    /// 원 발신 기기.
    pub origin: PeerId,
    /// 그 기기의 seq(≥1).
    pub seq: u64,
    /// 보낸 기기 기준 "내 말풍선"인가(내 기기가 보낸 줄 = 받는 형제에게도 내 말풍선).
    pub mine: bool,
    /// 기록 시각(Unix ms · 보낸 기기의 기록값 그대로 — 받는 쪽은 자기 시각으로 바꾸지 않는다).
    pub at_ms: u64,
    /// 등급(0~2).
    pub importance: u8,
    /// 발신자 라벨(수신 줄 · 빈 문자열 = 없음).
    pub from: String,
    /// 본문.
    pub text: String,
}

/// 응답 프레임: `tag ‖ thread 32 ‖ n u16 ‖ line×n` ·
/// line = `origin 32 ‖ seq 8 ‖ flags u8(bit0 mine · bit1~2 등급) ‖ at_ms 8 ‖ from_len u8 ‖ from ‖ text_len u16 ‖ text`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SyncLines {
    /// 스레드(저장 키).
    pub thread: PeerId,
    /// 줄(seq 오름차순이 관례 · 받는 쪽은 의존하지 않는다).
    pub lines: Vec<SyncLine>,
}

/// UTF-8 문자 경계를 지키며 바이트 상한으로 자른다.
fn clip_utf8(s: &str, max: usize) -> &str {
    if s.len() <= max {
        return s;
    }
    let mut end = max;
    while !s.is_char_boundary(end) {
        end -= 1;
    }
    &s[..end]
}

impl SyncLines {
    /// 프레임 머리 길이(태그 1 + 스레드 32 + 줄 수 2).
    const HEADER: usize = 35;

    /// 한 줄의 인코딩 길이(라벨·본문은 상한으로 잘린 뒤 기준).
    fn line_len(l: &SyncLine) -> usize {
        32 + 8
            + 1
            + 8
            + 1
            + clip_utf8(&l.from, SYNC_FROM_MAX).len()
            + 2
            + clip_utf8(&l.text, SYNC_TEXT_MAX).len()
    }

    /// 인코딩(한 프레임) — 줄 수는 [`SYNC_LINES_HARD_MAX`]로, 라벨·본문은 바이트 상한(문자 경계)으로
    /// 죈다. 세션 페이로드 상한을 넘을 수 있는 응답은 [`Self::encode_frames`]로.
    #[must_use]
    pub fn encode(&self) -> Vec<u8> {
        let n = self.lines.len().min(SYNC_LINES_HARD_MAX);
        Self::encode_slice(self.thread, &self.lines[..n])
    }

    /// 인코딩 — `max_bytes`(세션 페이로드 상한) 안에 들어가도록 줄 순서를 지키며 **여러 프레임**으로
    /// 나눈다. 한 줄은 최대 약 4.2KB라 프레임마다 최소 1줄은 들어간다. 줄이 없으면 빈 목록.
    #[must_use]
    pub fn encode_frames(&self, max_bytes: usize) -> Vec<Vec<u8>> {
        let lines = &self.lines[..self.lines.len().min(SYNC_LINES_HARD_MAX)];
        let mut out = Vec::new();
        let mut start = 0usize;
        let mut size = Self::HEADER;
        for (i, l) in lines.iter().enumerate() {
            let s = Self::line_len(l);
            if i > start && size + s > max_bytes {
                out.push(Self::encode_slice(self.thread, &lines[start..i]));
                start = i;
                size = Self::HEADER;
            }
            size += s;
        }
        if start < lines.len() {
            out.push(Self::encode_slice(self.thread, &lines[start..]));
        }
        out
    }

    fn encode_slice(thread: PeerId, lines: &[SyncLine]) -> Vec<u8> {
        let n = lines.len();
        let mut out = Vec::with_capacity(Self::HEADER + n * 64);
        out.push(SYNC_LINES_TAG);
        out.extend_from_slice(thread.as_bytes());
        #[allow(clippy::cast_possible_truncation)] // ≤ SYNC_LINES_HARD_MAX
        out.extend_from_slice(&(n as u16).to_be_bytes());
        for l in lines {
            let from = clip_utf8(&l.from, SYNC_FROM_MAX);
            let text = clip_utf8(&l.text, SYNC_TEXT_MAX);
            out.extend_from_slice(l.origin.as_bytes());
            out.extend_from_slice(&l.seq.to_be_bytes());
            out.push(u8::from(l.mine) | ((l.importance.min(2)) << 1));
            out.extend_from_slice(&l.at_ms.to_be_bytes());
            #[allow(clippy::cast_possible_truncation)] // ≤ 64
            out.push(from.len() as u8);
            out.extend_from_slice(from.as_bytes());
            #[allow(clippy::cast_possible_truncation)] // ≤ 4096
            out.extend_from_slice(&(text.len() as u16).to_be_bytes());
            out.extend_from_slice(text.as_bytes());
        }
        out
    }

    /// 디코딩 — 태그·길이 정확 일치 · 줄 수 상한 · seq 0 거부 · UTF-8 검증.
    #[must_use]
    pub fn decode(bytes: &[u8]) -> Option<Self> {
        if bytes.len() < 35 || bytes[0] != SYNC_LINES_TAG {
            return None;
        }
        let thread = PeerId::from_bytes(bytes[1..33].try_into().ok()?);
        let n = usize::from(u16::from_be_bytes([bytes[33], bytes[34]]));
        if n > SYNC_LINES_HARD_MAX {
            return None;
        }
        let mut p = 35usize;
        let take = |p: &mut usize, k: usize| -> Option<&[u8]> {
            let s = bytes.get(*p..*p + k)?;
            *p += k;
            Some(s)
        };
        let mut lines = Vec::with_capacity(n);
        for _ in 0..n {
            let origin = PeerId::from_bytes(take(&mut p, 32)?.try_into().ok()?);
            let seq = u64::from_be_bytes(take(&mut p, 8)?.try_into().ok()?);
            if seq == 0 {
                return None;
            }
            let flags = take(&mut p, 1)?[0];
            let at_ms = u64::from_be_bytes(take(&mut p, 8)?.try_into().ok()?);
            let fl = usize::from(take(&mut p, 1)?[0]);
            if fl > SYNC_FROM_MAX {
                return None;
            }
            let from = std::str::from_utf8(take(&mut p, fl)?).ok()?.to_string();
            let tl = usize::from(u16::from_be_bytes(take(&mut p, 2)?.try_into().ok()?));
            if tl > SYNC_TEXT_MAX {
                return None;
            }
            let text = std::str::from_utf8(take(&mut p, tl)?).ok()?.to_string();
            lines.push(SyncLine {
                origin,
                seq,
                mine: flags & 1 != 0,
                at_ms,
                importance: ((flags >> 1) & 0x3).min(2),
                from,
                text,
            });
        }
        (p == bytes.len()).then_some(Self { thread, lines })
    }
}

/// 읽음 동기 프레임: `tag ‖ thread 32 ‖ upto_at_ms 8` — "이 스레드를 `upto_at_ms`까지 봤다".
/// 받는 쪽은 그 시각 이하의 수신 줄을 읽음으로 보고 안읽음을 걷는다(더 새 줄이 있으면 그만큼 남긴다).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SyncRead {
    /// 스레드(저장 키 — 받는 쪽이 뷰 키로 접는다).
    pub thread: PeerId,
    /// 본 줄의 최대 기록 시각(Unix ms).
    pub upto_at_ms: u64,
}

impl SyncRead {
    const LEN: usize = 1 + 32 + 8;

    /// 인코딩.
    #[must_use]
    pub fn encode(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(Self::LEN);
        out.push(SYNC_READ_TAG);
        out.extend_from_slice(self.thread.as_bytes());
        out.extend_from_slice(&self.upto_at_ms.to_be_bytes());
        out
    }

    /// 디코딩 — 길이 정확 일치.
    #[must_use]
    pub fn decode(bytes: &[u8]) -> Option<Self> {
        if bytes.len() != Self::LEN || bytes[0] != SYNC_READ_TAG {
            return None;
        }
        Some(Self {
            thread: PeerId::from_bytes(bytes[1..33].try_into().ok()?),
            upto_at_ms: u64::from_be_bytes(bytes[33..41].try_into().ok()?),
        })
    }
}

/// 내 디제스트 표 — `(thread, origin) → max_seq`.
pub type DigestMap = BTreeMap<(PeerId, PeerId), u64>;

/// 디제스트 표 만들기(순수) — `lines` = (스레드, 원 발신 기기, seq) 전수. seq 0은 제외.
#[must_use]
pub fn digest_of<I: IntoIterator<Item = (PeerId, PeerId, u64)>>(lines: I) -> DigestMap {
    let mut m = DigestMap::new();
    for (t, o, s) in lines {
        if s == 0 {
            continue;
        }
        let e = m.entry((t, o)).or_insert(0);
        *e = (*e).max(s);
    }
    m
}

/// 디제스트 프레임 구성(순수) — 상한을 넘으면 `recent`(스레드 우선순위 · 최근 대화 순)가 앞선
/// 스레드부터 담는다. `recent`에 없는 스레드는 뒤에(키 순).
#[must_use]
pub fn digest_frame(mine: &DigestMap, recent: &[PeerId]) -> SyncDigest {
    let rank = |t: &PeerId| recent.iter().position(|r| r == t).unwrap_or(usize::MAX);
    let mut entries: Vec<DigestEntry> = mine
        .iter()
        .map(|(&(thread, origin), &max_seq)| DigestEntry {
            thread,
            origin,
            max_seq,
        })
        .collect();
    entries.sort_by(|a, b| {
        rank(&a.thread)
            .cmp(&rank(&b.thread))
            .then_with(|| a.thread.as_bytes().cmp(b.thread.as_bytes()))
            .then_with(|| a.origin.as_bytes().cmp(b.origin.as_bytes()))
    });
    entries.truncate(SYNC_DIGEST_MAX);
    SyncDigest { entries }
}

/// 상대 디제스트와 대조해 **내가 부족한 것**의 청구 목록(순수) — 상대 max_seq > 내 max_seq(없으면 0)
/// 인 항목마다 `after_seq = 내 max_seq`. 상대 디제스트 순서(상대의 우선순위) 유지 · [`SYNC_PULL_MAX`].
/// `max_lines` = 청구측 설정(스레드당 줄 상한 · 1~[`SYNC_LINES_HARD_MAX`]로 죈다).
#[must_use]
pub fn pulls_for(mine: &DigestMap, theirs: &SyncDigest, max_lines: u16) -> Vec<SyncPull> {
    #[allow(clippy::cast_possible_truncation)] // 500
    let max = max_lines.clamp(1, SYNC_LINES_HARD_MAX as u16);
    theirs
        .entries
        .iter()
        .filter_map(|e| {
            let have = mine.get(&(e.thread, e.origin)).copied().unwrap_or(0);
            (e.max_seq > have).then_some(SyncPull {
                thread: e.thread,
                origin: e.origin,
                after_seq: have,
                max,
            })
        })
        .take(SYNC_PULL_MAX)
        .collect()
}

/// 청구에 응할 줄 고르기(순수) — `origin`이 보낸 `after_seq` 초과분을 seq 오름차순으로, **가장 최근**
/// `max`줄(≤ [`SYNC_LINES_HARD_MAX`] · 호출측이 자기 설정으로 먼저 죈다)만. 입력은 (origin, seq, 줄)
/// 아무 순서.
pub fn select_lines<T: Clone>(
    lines: &[(PeerId, u64, T)],
    origin: PeerId,
    after_seq: u64,
    max: u16,
) -> Vec<(u64, T)> {
    let mut v: Vec<(u64, T)> = lines
        .iter()
        .filter(|(o, s, _)| *o == origin && *s > after_seq)
        .map(|(_, s, l)| (*s, l.clone()))
        .collect();
    v.sort_by_key(|(s, _)| *s);
    let cap = usize::from(max).clamp(1, SYNC_LINES_HARD_MAX);
    if v.len() > cap {
        v.drain(..v.len() - cap);
    }
    v
}

/// 세션 응답 예산(순수) — 보낸 바이트 누계가 상한을 넘으면 더 응답하지 않는다(fail-soft).
/// 상한은 세션 성립 시점의 설정([`Self::new`]) · 기본 [`SYNC_BYTES_PER_SESSION`].
#[derive(Debug, Clone, Copy)]
pub struct SyncBudget {
    sent: usize,
    limit: usize,
}

impl Default for SyncBudget {
    fn default() -> Self {
        Self::new(SYNC_BYTES_PER_SESSION)
    }
}

impl SyncBudget {
    /// 상한 `limit` 바이트의 예산.
    #[must_use]
    pub fn new(limit: usize) -> Self {
        Self { sent: 0, limit }
    }

    /// `n` 바이트를 더 보낼 수 있으면 차감하고 참.
    pub fn take(&mut self, n: usize) -> bool {
        if self.sent.saturating_add(n) > self.limit {
            return false;
        }
        self.sent += n;
        true
    }

    /// 상한(바이트).
    #[must_use]
    pub fn limit(&self) -> usize {
        self.limit
    }

    /// 지금까지 보낸 바이트.
    #[must_use]
    pub fn sent(&self) -> usize {
        self.sent
    }
}

/// 읽음 동기 적용 판정(순수) — 스레드의 수신 줄 시각들 가운데 `upto` **초과**인 줄 수 = 남길 안읽음.
#[must_use]
pub fn unread_after<I: IntoIterator<Item = u64>>(received_at_ms: I, upto: u64) -> u32 {
    #[allow(clippy::cast_possible_truncation)]
    {
        received_at_ms.into_iter().filter(|&t| t > upto).count() as u32
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    use super::*;

    fn pid(b: u8) -> PeerId {
        PeerId::from_bytes([b; 32])
    }

    #[test]
    fn digest_roundtrip_and_reject() {
        let d = SyncDigest {
            entries: vec![
                DigestEntry {
                    thread: pid(1),
                    origin: pid(1),
                    max_seq: 7,
                },
                DigestEntry {
                    thread: pid(1),
                    origin: pid(9),
                    max_seq: 1_760_000_000_000,
                },
            ],
        };
        let e = d.encode();
        assert_eq!(SyncDigest::decode(&e), Some(d));
        assert!(SyncDigest::decode(&e[..e.len() - 1]).is_none(), "길이 부족");
        let mut long = e.clone();
        long.push(0);
        assert!(SyncDigest::decode(&long).is_none(), "길이 초과");
        let mut zero = e;
        zero[2 + 64..2 + 72].copy_from_slice(&0u64.to_be_bytes());
        assert!(SyncDigest::decode(&zero).is_none(), "seq 0 거부");
        assert_eq!(
            SyncDigest::decode(&SyncDigest::default().encode()),
            Some(SyncDigest::default())
        );
    }

    #[test]
    fn pull_read_roundtrip() {
        let p = SyncPull {
            thread: pid(3),
            origin: pid(4),
            after_seq: 12,
            max: 200,
        };
        assert_eq!(SyncPull::decode(&p.encode()), Some(p));
        assert!(SyncPull::decode(&p.encode()[1..]).is_none());
        let r = SyncRead {
            thread: pid(3),
            upto_at_ms: 99,
        };
        assert_eq!(SyncRead::decode(&r.encode()), Some(r));
        let mut bad = r.encode();
        bad[0] = SYNC_PULL_TAG;
        assert!(SyncRead::decode(&bad).is_none(), "태그 혼동 거부");
    }

    #[test]
    fn lines_roundtrip_clip_and_reject() {
        let l = SyncLines {
            thread: pid(2),
            lines: vec![
                SyncLine {
                    origin: pid(2),
                    seq: 5,
                    mine: false,
                    at_ms: 1000,
                    importance: 2,
                    from: "상대".into(),
                    text: "안녕".into(),
                },
                SyncLine {
                    origin: pid(7),
                    seq: 1_760_000_000_001,
                    mine: true,
                    at_ms: 2000,
                    importance: 0,
                    from: String::new(),
                    text: String::new(),
                },
            ],
        };
        let e = l.encode();
        assert_eq!(SyncLines::decode(&e), Some(l.clone()));
        assert!(SyncLines::decode(&e[..e.len() - 1]).is_none(), "길이 부족");
        let mut long = e;
        long.push(0);
        assert!(SyncLines::decode(&long).is_none(), "꼬리 초과");
        // 본문·라벨 상한 — 문자 경계로 자른다(한글 3바이트).
        let big = SyncLines {
            thread: pid(2),
            lines: vec![SyncLine {
                origin: pid(2),
                seq: 1,
                mine: false,
                at_ms: 0,
                importance: 9,
                from: "가".repeat(30),
                text: "나".repeat(2000),
            }],
        };
        let back = SyncLines::decode(&big.encode()).unwrap();
        assert_eq!(back.lines[0].from.chars().count(), 21);
        assert_eq!(back.lines[0].text.len(), 4095);
        assert_eq!(back.lines[0].importance, 2, "등급은 2로 죈다");
        // 줄 수 상한 초과 프레임은 거부(인코더는 자르고 디코더는 받지 않는다).
        let mut over = SyncLines {
            thread: pid(2),
            lines: Vec::new(),
        }
        .encode();
        over[33..35].copy_from_slice(&501u16.to_be_bytes());
        assert!(
            SyncLines::decode(&over).is_none(),
            "와이어 천장 초과 = 거부"
        );
        // seq 0 줄 거부.
        let mut zero = l.encode();
        zero[35 + 32..35 + 40].copy_from_slice(&0u64.to_be_bytes());
        assert!(SyncLines::decode(&zero).is_none());
    }

    #[test]
    fn digest_diff_pulls_only_missing() {
        let mine = digest_of(vec![
            (pid(1), pid(1), 5),
            (pid(1), pid(9), 3),
            (pid(2), pid(2), 10),
            (pid(3), pid(3), 0), // seq 0 = 열쇠 없음 = 제외
        ]);
        assert_eq!(mine.len(), 3);
        let theirs = SyncDigest {
            entries: vec![
                DigestEntry {
                    thread: pid(1),
                    origin: pid(1),
                    max_seq: 8,
                }, // 부족 → after 5
                DigestEntry {
                    thread: pid(1),
                    origin: pid(9),
                    max_seq: 3,
                }, // 같음 → 없음
                DigestEntry {
                    thread: pid(2),
                    origin: pid(2),
                    max_seq: 4,
                }, // 내가 더 많음 → 없음
                DigestEntry {
                    thread: pid(4),
                    origin: pid(4),
                    max_seq: 2,
                }, // 모르는 스레드 → after 0
            ],
        };
        let pulls = pulls_for(&mine, &theirs, 200);
        assert_eq!(pulls.len(), 2);
        assert_eq!((pulls[0].thread, pulls[0].after_seq), (pid(1), 5));
        assert_eq!((pulls[1].thread, pulls[1].after_seq), (pid(4), 0));
        assert_eq!(pulls[0].max, 200);
        // 청구 상한은 설정이 정하고 와이어 천장으로 죈다(0 → 1 · 9000 → 500).
        assert_eq!(pulls_for(&mine, &theirs, 50)[0].max, 50);
        assert_eq!(pulls_for(&mine, &theirs, 0)[0].max, 1);
        assert_eq!(pulls_for(&mine, &theirs, 9000)[0].max, 500);
    }

    #[test]
    fn lines_split_into_frames_under_payload_cap() {
        // 10-19 결함 — 한 프레임이 세션 페이로드 상한을 넘으면 액터가 세션을 끊었다. 분할 뒤엔
        // 프레임마다 상한 이하 · 줄 순서·전수 보존 · 각 프레임이 홀로 디코딩된다.
        let line = |seq: u64| SyncLine {
            origin: pid(1),
            seq,
            mine: false,
            at_ms: seq,
            importance: 0,
            from: "a".into(),
            text: "x".repeat(300),
        };
        let l = SyncLines {
            thread: pid(2),
            lines: (1..=200).map(line).collect(),
        };
        let one = l.encode();
        assert!(
            one.len() > 65_518,
            "단일 프레임 = {}B(상한 초과 재현)",
            one.len()
        );
        let frames = l.encode_frames(65_518);
        assert!(frames.len() >= 2);
        let mut seqs = Vec::new();
        for f in &frames {
            assert!(f.len() <= 65_518, "{}B", f.len());
            let d = SyncLines::decode(f).expect("프레임 단독 디코딩");
            assert_eq!(d.thread, pid(2));
            seqs.extend(d.lines.iter().map(|x| x.seq));
        }
        assert_eq!(seqs, (1..=200).collect::<Vec<u64>>(), "순서·전수 보존");
        // 상한이 아주 작아도 프레임마다 최소 1줄(무한 루프·빈 프레임 없음).
        let tiny = l.encode_frames(1);
        assert_eq!(tiny.len(), 200);
        assert!(tiny.iter().all(|f| SyncLines::decode(f).is_some()));
        // 줄이 없으면 프레임도 없다.
        assert!(SyncLines {
            thread: pid(2),
            lines: vec![]
        }
        .encode_frames(65_518)
        .is_empty());
        // 상한 안에 다 들어가면 한 프레임 = encode()와 같다.
        let small = SyncLines {
            thread: pid(2),
            lines: (1..=3).map(line).collect(),
        };
        assert_eq!(small.encode_frames(65_518), vec![small.encode()]);
    }

    #[test]
    fn digest_frame_orders_recent_first_and_caps() {
        let mut mine = DigestMap::new();
        for i in 1..=100u8 {
            mine.insert((pid(i), pid(i)), 1);
            mine.insert((pid(i), pid(200)), 1);
            mine.insert((pid(i), pid(201)), 1);
        }
        let f = digest_frame(&mine, &[pid(50), pid(7)]);
        assert_eq!(f.entries.len(), SYNC_DIGEST_MAX);
        assert_eq!(f.entries[0].thread, pid(50));
        assert_eq!(f.entries[3].thread, pid(7));
        assert_eq!(f.entries[6].thread, pid(1), "나머지는 키 순");
        assert_eq!(SyncDigest::decode(&f.encode()), Some(f));
    }

    #[test]
    fn select_lines_recent_window() {
        let lines: Vec<(PeerId, u64, &str)> = (1..=300u64)
            .map(|s| (pid(1), s, "x"))
            .chain([(pid(2), 999, "other")])
            .collect();
        let v = select_lines(&lines, pid(1), 0, 200);
        assert_eq!(v.len(), 200);
        assert_eq!(v[0].0, 101, "가장 최근 200줄");
        assert_eq!(v[199].0, 300);
        let v = select_lines(&lines, pid(1), 295, 1000);
        assert_eq!(
            v.iter().map(|(s, _)| *s).collect::<Vec<_>>(),
            [296, 297, 298, 299, 300]
        );
        assert!(select_lines(&lines, pid(3), 0, 10).is_empty());
        assert_eq!(
            select_lines(&lines, pid(1), 0, 0).len(),
            1,
            "max 0 = 최소 1"
        );
    }

    #[test]
    fn budget_and_unread() {
        let mut b = SyncBudget::default();
        assert_eq!(b.limit(), SYNC_BYTES_PER_SESSION);
        assert!(b.take(SYNC_BYTES_PER_SESSION - 1));
        assert!(b.take(1));
        assert!(!b.take(1), "상한 초과 = 거절");
        assert_eq!(b.sent(), SYNC_BYTES_PER_SESSION);
        // 설정 상한(10-19) — 4MiB면 4MiB까지.
        let mut c = SyncBudget::new(4 << 20);
        assert!(c.take(4 << 20));
        assert!(!c.take(1));
        assert_eq!(unread_after([10, 20, 30], 20), 1);
        assert_eq!(unread_after([10, 20, 30], 30), 0);
        assert_eq!(unread_after(std::iter::empty(), 0), 0);
    }
}
