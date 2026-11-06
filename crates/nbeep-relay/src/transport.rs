//! X-2 릴레이 **Transport 승격**(DR-8 ② · ADR-0003 §3 "구현 3종: LocalDirect · InMemory · Relay" · 11-06) +
//! X-2f **발견 스트림 합류**(ADR-0003 §9 · 11-06).
//!
//! - [`RelayTransport`] = 붙어 있는 [`RelayClient`] 하나 위의 `Transport`(`nbeep_net::Transport`) — `connect`는 랑데부 사다리
//!   (`open_ladder` · 홀펀칭 UDP → 릴레이 · **첫 회신이 온 단에 고정**)로 **핸드셰이크 전 `Link`**
//!   ([`LadderLink`])를 돌려주고, `incoming`은 서버가 넘긴 인바운드를 첫 프레임 링크 추종
//!   (`accept_link_via`)으로 잇는다. 종단 Noise는 호출자(App·CLI)가 LAN과 **같은 코드**로 얹는다 —
//!   "암호화는 Transport 위"(ADR-0003 핵심 규칙)가 릴레이 경로에도 성립한다.
//!   `discovery`는 서버 **공개 명부**(X-2e roster)를 `Appeared(origin Rendezvous · Eventful · profile)`/`Withdrawn`으로 올린다 —
//!   카드 정제(이름 = `DisplayName` · 이메일·소개 = 제어 문자 제거·200자)와 명부 상한(`ROSTER_MAX`)·자기 PeerId 제외는
//!   **여기(경계)** 에서 끝난다. 병합 판정은 호스트의 `PeerTable`(FR-D-6 · 경로 토큰 = 이 전송의 세대 `source`).
//! - [`LadderTransport`] = 로컬 직접([`nbeep_net::LocalDirect`] 등 임의의 `Transport`) + 기억한 수동 주소 + 릴레이를
//!   **한 `Transport`** 로 묶는다: `connect` = 발견 경로 → 수동 주소 → 랑데부(ADR-0003 §4 "로컬 > 수동 > 릴레이")
//!   · `incoming`·`discovery` = 두 통로가 **한 수신단**으로 합류 · 설정류(이름·태그·조용한 모드·필터)는 로컬에 위임.
//!   릴레이는 호스트가 서버에 붙였다 떼는 대로 [`LadderTransport::set_relay`]로 갈아 끼운다(hot-swap · 세대마다 다른
//!   `source` 토큰 — 옛 세대의 늦은 `Withdrawn`이 새 세대 명부를 지우지 못한다). 수명은 **호스트의 `Arc<RelayClient>`**가
//!   정하고 여기는 `Weak`만 든다 — 호스트가 접속을 내려놓으면 펌프가 명부를 일괄 `Withdrawn`하고 꺼진다.
//!
//! 봉투 원리: 이 모듈은 상대 `PeerId`(RID 유도·명부)와 서버 주소만 안다. 호스트는 릴레이 와이어(`RosterEvent`·카드 인코딩)를
//! 더 알지 않는다(X-2f 이전에는 App이 `poll_roster`를 직접 소비했다).

use super::{
    accept_link_via, current_epoch_day, decode_card, open_ladder, rid_for, RelayClient,
    RosterEvent, ViaError,
};
use nbeep_core::link::Link;
use nbeep_core::{default_display_name, DisplayName, HintOrigin, Liveness, PeerId, PublicCard};
use nbeep_net::{AddError, Caps, ConnectError, DiscoveryEvent, InboundFilter, PeerHint, Transport};
use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError, Weak};
use std::time::Duration;

/// 랑데부 Open 대기 상한(서버 왕복) — 종전 GUI/CLI 사다리 값(10s).
const OPEN_TIMEOUT: Duration = Duration::from_secs(10);
/// 인바운드 사다리 성립 상한(펀치 병행 · 첫 프레임 대기) — 종전 `spawn_relay_accept` 값(12s).
const ACCEPT_DEADLINE: Duration = Duration::from_secs(12);
/// 펌프 폴 간격 — 인바운드 대기 겸 명부 델타 소비 주기 · `Weak` 승격 재확인 주기.
const ACCEPT_POLL: Duration = Duration::from_secs(1);
/// 명부 보관 상한(악의·폭주 서버 방어 · 종전 App `ROSTER_MAX`) — 초과분은 올리지 않는다(메모리 유계 · 봉투).
pub const ROSTER_MAX: usize = 4096;

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(PoisonError::into_inner)
}

/// `discovery()`/`incoming()`은 한 번만 — 수신단을 호출자가 소유한다(LocalDirect와 같은 규약).
fn take_once<T>(slot: &Mutex<Option<Receiver<T>>>, what: &str) -> Receiver<T> {
    lock(slot)
        .take()
        .unwrap_or_else(|| panic!("{what}는 한 번만 호출한다"))
}

/// 수신단 → 송신단 중계 스레드(합류용) — 어느 쪽이 닫히든 끝난다.
fn forward<T: Send + 'static>(rx: Receiver<T>, tx: Sender<T>) {
    std::thread::spawn(move || {
        while let Ok(v) = rx.recv() {
            if tx.send(v).is_err() {
                break;
            }
        }
    });
}

fn via_to_connect(e: ViaError) -> ConnectError {
    match e {
        ViaError::NotFound => ConnectError::Unreachable,
        ViaError::Limit => ConnectError::Busy,
        ViaError::Dead => ConnectError::Lost,
        // 링크 수준 사다리에서는 나지 않는다(핸드셰이크는 호출자 몫) — 방어적 매핑.
        ViaError::Handshake | ViaError::WrongPeer => ConnectError::Unreachable,
    }
}

/// 공개 카드 바이트 → (표시 이름, 정제된 카드). 서버 중계 = 미검증 입력 · fail-closed: 이름은 `DisplayName`을 통과한
/// 것만, 통과하지 못하면 기본 라벨(지문 유도 · `default_display_name`) · 이메일·소개는 제어 문자 제거·200자.
fn card_hint(peer: PeerId, card: &[u8]) -> (DisplayName, Option<PublicCard>) {
    let clean = |s: &str| -> String { s.chars().filter(|c| !c.is_control()).take(200).collect() };
    match decode_card(card) {
        Some((n, e, b)) => {
            let name = DisplayName::parse(&n).ok();
            let profile = PublicCard {
                name: name
                    .as_ref()
                    .map_or_else(String::new, |d| d.as_str().to_string()),
                email: clean(&e),
                bio: clean(&b),
            };
            (
                name.unwrap_or_else(|| default_display_name(None, &peer)),
                Some(profile),
            )
        }
        None => (default_display_name(None, &peer), None),
    }
}

/// 명부 집합을 비우며 전부 `Withdrawn`(세대 토큰) — 접속 소멸·공개 해제·단 교체 공용.
fn withdraw_all(known: &Mutex<HashSet<PeerId>>, disc_tx: &Sender<DiscoveryEvent>, source: u32) {
    let drained: Vec<PeerId> = lock(known).drain().collect();
    for peer in drained {
        let _ = disc_tx.send(DiscoveryEvent::Withdrawn { peer, source });
    }
}

/// 랑데부 서버 하나 위의 [`Transport`](X-2 · X-2f).
pub struct RelayTransport {
    /// 호스트가 든 접속의 약한 참조 — 승격 실패 = 호스트가 내려놓았다.
    client: Weak<RelayClient>,
    /// 경로 토큰(세대) — 이 전송이 올리는 힌트·이탈의 `source`.
    source: u32,
    /// 서버 명부(공개 사용자) 집합 — 상한·Down 판정·일괄 이탈 재료. 펌프와 공유.
    known: Arc<Mutex<HashSet<PeerId>>>,
    disc_tx: Sender<DiscoveryEvent>,
    discovery_rx: Mutex<Option<Receiver<DiscoveryEvent>>>,
    incoming_rx: Mutex<Option<Receiver<Box<dyn Link>>>>,
}

impl core::fmt::Debug for RelayTransport {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("RelayTransport")
            .field("alive", &self.live_client().is_some())
            .field("source", &self.source)
            .field("roster", &lock(&self.known).len())
            .finish_non_exhaustive()
    }
}

impl RelayTransport {
    /// 붙어 있는 클라이언트 위에 전송을 올린다 — 펌프(인바운드 첫 프레임 추종 + 명부 델타 → 발견 이벤트)를 띄운다.
    /// `me` = 내 PeerId(명부에서 나 자신 제외) · `source` = 이 전송의 경로 토큰(호스트/사다리가 세대마다 다르게 준다 · 0은 LAN 몫).
    /// 소유권은 가져가지 않는다(`Weak`): 호출자의 `Arc`가 마지막으로 떨어지면 펌프가 명부를 일괄 `Withdrawn`하고 끝난다.
    #[must_use]
    pub fn new(client: &Arc<RelayClient>, me: PeerId, source: u32) -> Self {
        let (inc_tx, inc_rx) = channel::<Box<dyn Link>>();
        let (disc_tx, disc_rx) = channel::<DiscoveryEvent>();
        let known = Arc::new(Mutex::new(HashSet::new()));
        spawn_pump(
            Arc::downgrade(client),
            inc_tx,
            disc_tx.clone(),
            Arc::clone(&known),
            me,
            source,
        );
        Self {
            client: Arc::downgrade(client),
            source,
            known,
            disc_tx,
            discovery_rx: Mutex::new(Some(disc_rx)),
            incoming_rx: Mutex::new(Some(inc_rx)),
        }
    }

    /// 살아 있는 접속(승격 + 액터 생존).
    fn live_client(&self) -> Option<Arc<RelayClient>> {
        self.client.upgrade().filter(|c| c.is_alive())
    }

    /// 프레즌스 공개 지정(X-2e · 옵트인) — `true` = 공개 목록에 실리고 명부 델타(입장 스냅숏 포함)를 받기 시작한다 ·
    /// `false` = 공개 해제 + **지금 아는 명부 전부 `Withdrawn`**(서버가 더 보내지 않으므로 여기서 비운다).
    pub fn set_announce(&self, listed: bool, card: Vec<u8>) {
        if let Some(c) = self.client.upgrade() {
            c.set_announce(listed, card);
        }
        if !listed {
            withdraw_all(&self.known, &self.disc_tx, self.source);
        }
    }

    /// 명부 전부 이탈(단 교체·해제) — 펌프의 늦은 이탈은 같은 토큰이라 무해(테이블 goodbye 멱등).
    pub fn withdraw_all(&self) {
        withdraw_all(&self.known, &self.disc_tx, self.source);
    }

    /// 이 전송의 경로 토큰.
    #[must_use]
    pub fn source(&self) -> u32 {
        self.source
    }
}

/// 펌프 — 서버가 넘긴 랑데부마다 스레드 하나로 사다리를 잇고(최대 [`ACCEPT_DEADLINE`]) 핸드셰이크 전 링크를 수신단에 넘기며,
/// 매 폴마다 명부 델타를 발견 이벤트로 바꾼다. 동시 인바운드가 서로를 기다리지 않는다(상한은 호스트 펌프 — RL-13 ⓐ).
fn spawn_pump(
    weak: Weak<RelayClient>,
    inc_tx: Sender<Box<dyn Link>>,
    disc_tx: Sender<DiscoveryEvent>,
    known: Arc<Mutex<HashSet<PeerId>>>,
    me: PeerId,
    source: u32,
) {
    std::thread::spawn(move || loop {
        let Some(client) = weak.upgrade() else {
            // 호스트가 접속을 내려놓았다 — 명부는 전부 이탈(서버 전용 상대가 유령으로 남지 않게) · 소임 끝.
            withdraw_all(&known, &disc_tx, source);
            break;
        };
        // 명부 델타(X-2e) → 발견 이벤트(X-2f). 상한 밖 신규는 올리지 않는다(Down은 아는 것만).
        for ev in client.poll_roster() {
            match ev {
                RosterEvent::Up(peer, card) => {
                    if peer == me {
                        continue;
                    }
                    {
                        let mut k = lock(&known);
                        if !k.contains(&peer) {
                            if k.len() >= ROSTER_MAX {
                                continue;
                            }
                            k.insert(peer);
                        }
                    }
                    let (name, profile) = card_hint(peer, &card);
                    let hint = PeerHint {
                        peer,
                        name,
                        caps: Caps::default(),
                        gen: 0, // 세대 미상(명부는 존재만) — 호환 판정은 성립 시 세션이
                        origin: HintOrigin::Rendezvous,
                        liveness: Liveness::Eventful,
                        source,
                        profile,
                    };
                    if disc_tx.send(DiscoveryEvent::Appeared(hint)).is_err() {
                        break;
                    }
                }
                RosterEvent::Down(peer) => {
                    if lock(&known).remove(&peer)
                        && disc_tx
                            .send(DiscoveryEvent::Withdrawn { peer, source })
                            .is_err()
                    {
                        break;
                    }
                }
            }
        }
        let Some(inc) = client.accept_incoming(ACCEPT_POLL) else {
            continue; // 타임아웃 — Weak·명부를 다시 보고 돈다
        };
        let tx = inc_tx.clone();
        std::thread::spawn(move || {
            // 실패는 조용히 — 상대가 보는 것은 Closed뿐(ADR-0006 §2 규칙 4 · LAN 핸드셰이크 실패와 같다).
            if let Ok((link, _taken)) = accept_link_via(&client, inc, true, ACCEPT_DEADLINE) {
                let _ = tx.send(link);
            }
        });
    });
}

impl Transport for RelayTransport {
    fn discovery(&self) -> Receiver<DiscoveryEvent> {
        take_once(&self.discovery_rx, "discovery()")
    }

    fn incoming(&self) -> Receiver<Box<dyn Link>> {
        take_once(&self.incoming_rx, "incoming()")
    }

    fn connect(&self, peer: PeerId) -> Result<Box<dyn Link>, ConnectError> {
        let client = self.live_client().ok_or(ConnectError::Lost)?;
        // 에폭 RID 오늘→어제→내일(시계 오차 흡수 · [`super::rids_around`]와 같은 셋).
        let day = current_epoch_day();
        let rids = [
            rid_for(&peer, day),
            rid_for(&peer, day.saturating_sub(1)),
            rid_for(&peer, day + 1),
        ];
        let ladder = open_ladder(&client, &rids, true, OPEN_TIMEOUT).map_err(via_to_connect)?;
        Ok(Box::new(ladder))
    }

    fn caps(&self) -> Caps {
        // 오프라인 큐(서버 보관 · Content 모드)는 별도 결정 — 광고하지 않는다.
        Caps::default()
    }
}

/// 로컬 직접 + 수동 주소 + 릴레이 **사다리 전송**(X-2 · ADR-0003 §4) — 호스트는 이것 하나만 `Transport`로 든다.
pub struct LadderTransport {
    local: Arc<dyn Transport + Send + Sync>,
    /// 내 PeerId — 랑데부 명부에서 나 자신 제외.
    me: PeerId,
    /// 현재 장착된 랑데부 단(None = Unmanaged/미접속) — [`Self::set_relay`]로 교체.
    relay: Mutex<Option<Arc<RelayTransport>>>,
    /// 랑데부 단 세대 — 장착마다 +1 · 경로 토큰 = 세대(LAN 0과 겹치지 않는다).
    relay_gen: AtomicU32,
    /// 기억한 수동 주소(DR-19 ④ · [`Transport::note_endpoint`]) — 발견 경로 다음 단.
    manual: Mutex<HashMap<PeerId, String>>,
    incoming_tx: Sender<Box<dyn Link>>,
    incoming_rx: Mutex<Option<Receiver<Box<dyn Link>>>>,
    discovery_tx: Sender<DiscoveryEvent>,
    discovery_rx: Mutex<Option<Receiver<DiscoveryEvent>>>,
}

impl core::fmt::Debug for LadderTransport {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("LadderTransport")
            .field("relay_attached", &self.relay_attached())
            .field("relay_gen", &self.relay_gen.load(Ordering::Relaxed))
            .field("manual", &lock(&self.manual).len())
            .finish_non_exhaustive()
    }
}

impl LadderTransport {
    /// 로컬 전송을 감싼다 — 로컬의 `incoming()`·`discovery()`를 **여기서 소유**해 합류 수신단으로 중계한다(호출자는 로컬의
    /// 두 수신단을 더 부르지 않는다). `me` = 내 PeerId(명부 자기 제외).
    #[must_use]
    pub fn new(local: Arc<dyn Transport + Send + Sync>, me: PeerId) -> Arc<Self> {
        let (inc_tx, inc_rx) = channel::<Box<dyn Link>>();
        let (disc_tx, disc_rx) = channel::<DiscoveryEvent>();
        forward(local.incoming(), inc_tx.clone());
        forward(local.discovery(), disc_tx.clone());
        Arc::new(Self {
            local,
            me,
            relay: Mutex::new(None),
            relay_gen: AtomicU32::new(0),
            manual: Mutex::new(HashMap::new()),
            incoming_tx: inc_tx,
            incoming_rx: Mutex::new(Some(inc_rx)),
            discovery_tx: disc_tx,
            discovery_rx: Mutex::new(Some(disc_rx)),
        })
    }

    /// 랑데부 단 장착/해제(hot-swap) — `Some` = 이 접속 위에 [`RelayTransport`]를 **새 세대 토큰**으로 올리고 인바운드·발견을
    /// 합류시킨다 · `None` = 단을 내린다. 어느 쪽이든 옛 단의 명부는 즉시 전부 `Withdrawn`(옛 토큰) — 새 단의 `Appeared`
    /// (새 토큰)와 순서가 어긋나도 테이블은 경로별로 접으므로 행이 잘못 사라지지 않는다.
    pub fn set_relay(&self, client: Option<&Arc<RelayClient>>) {
        let mut slot = lock(&self.relay);
        if let Some(old) = slot.take() {
            old.withdraw_all();
        }
        *slot = client.map(|c| {
            let gen = self.relay_gen.fetch_add(1, Ordering::Relaxed) + 1;
            let rt = Arc::new(RelayTransport::new(c, self.me, gen));
            forward(rt.incoming(), self.incoming_tx.clone());
            forward(rt.discovery(), self.discovery_tx.clone());
            rt
        });
    }

    /// 프레즌스 공개 지정을 현재 랑데부 단에 전달(X-2e · X-2f) — 단이 없으면 무시.
    pub fn set_announce(&self, listed: bool, card: Vec<u8>) {
        let relay = lock(&self.relay).clone();
        if let Some(rt) = relay {
            rt.set_announce(listed, card);
        }
    }

    /// 랑데부 단이 장착돼 있고 그 접속이 살아 있는가.
    #[must_use]
    pub fn relay_attached(&self) -> bool {
        lock(&self.relay)
            .as_ref()
            .is_some_and(|r| r.live_client().is_some())
    }
}

impl Transport for LadderTransport {
    fn discovery(&self) -> Receiver<DiscoveryEvent> {
        take_once(&self.discovery_rx, "discovery()")
    }

    fn incoming(&self) -> Receiver<Box<dyn Link>> {
        take_once(&self.incoming_rx, "incoming()")
    }

    fn connect(&self, peer: PeerId) -> Result<Box<dyn Link>, ConnectError> {
        // 1단 발견 경로(로컬 직접) — 되면 인터넷·서버로 우회하지 않는다(지연·프라이버시 · ADR-0003 §4).
        if let Ok(link) = self.local.connect(peer) {
            return Ok(link);
        }
        // 2단 기억한 수동 주소(DR-19 ④) — 주소로 붙는다. 신원은 호출자 핸드셰이크 + 대조(R-20 · 다른 키면
        // 호스트가 `forget_endpoint`)가 정한다. 잠금은 복제 순간만(연결 대기 중 보유 금지).
        let manual = lock(&self.manual).get(&peer).cloned();
        if let Some(addr) = manual {
            if let Ok(link) = self.local.add_endpoint(&addr) {
                return Ok(link);
            }
        }
        // 3단 랑데부 — Managed 서버가 장착돼 있을 때만. 실패 사유는 그 단의 것(Busy·Lost·Unreachable)을 그대로.
        let relay = lock(&self.relay).clone();
        match relay {
            Some(rt) => rt.connect(peer),
            None => Err(ConnectError::Unreachable),
        }
    }

    fn add_endpoint(&self, addr: &str) -> Result<Box<dyn Link>, AddError> {
        self.local.add_endpoint(addr)
    }

    fn note_endpoint(&self, peer: PeerId, addr: &str) {
        lock(&self.manual).insert(peer, addr.to_string());
    }

    fn forget_endpoint(&self, peer: PeerId) {
        lock(&self.manual).remove(&peer);
    }

    fn link_changed(&self) {
        self.local.link_changed();
    }

    fn caps(&self) -> Caps {
        self.local.caps()
    }

    fn set_display_name(&self, name: DisplayName) {
        self.local.set_display_name(name);
    }

    fn set_user_tag(&self, tag: Option<[u8; 16]>) {
        self.local.set_user_tag(tag);
    }

    fn set_quiet(&self, quiet: bool) {
        self.local.set_quiet(quiet);
    }

    fn set_advertise(&self, on: bool) {
        self.local.set_advertise(on);
    }

    fn set_inbound_filter(&self, filter: InboundFilter) {
        self.local.set_inbound_filter(filter);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use nbeep_net::inmem::InMemoryBus;

    fn pid(n: u8) -> PeerId {
        PeerId::from_bytes([n; 32])
    }
    fn name(s: &str) -> DisplayName {
        DisplayName::parse(s).unwrap()
    }

    /// 사다리 1단 — 로컬이 아는 상대는 로컬 링크(랑데부 표식 없음) · 랑데부 단 미장착이면 모르는 상대 = Unreachable.
    #[test]
    fn ladder_uses_local_first_and_is_unreachable_without_relay() {
        let bus = InMemoryBus::new();
        let a = bus.join(pid(1), name("a"), Caps::default());
        let b = bus.join(pid(2), name("b"), Caps::default());
        let b_in = b.incoming();
        let ladder = LadderTransport::new(Arc::new(a), pid(1));
        let link = ladder.connect(pid(2)).expect("로컬 경로");
        assert!(!link.rendezvous(), "로컬 직결 = 랑데부 아님");
        assert!(b_in.recv().is_ok(), "상대 쪽 인바운드 도착");
        assert_eq!(
            ladder.connect(pid(9)).err(),
            Some(ConnectError::Unreachable)
        );
        assert!(!ladder.relay_attached());
    }

    /// 합류 — 로컬 인바운드가 사다리의 `incoming()`으로 나온다(호스트 펌프 하나로 LAN·릴레이를 받는 근거).
    #[test]
    fn ladder_incoming_merges_local() {
        let bus = InMemoryBus::new();
        let a = bus.join(pid(1), name("a"), Caps::default());
        let b = bus.join(pid(2), name("b"), Caps::default());
        let ladder_b = LadderTransport::new(Arc::new(b), pid(2));
        let b_in = ladder_b.incoming();
        let _l = a.connect(pid(2)).expect("a → b");
        let got = b_in
            .recv_timeout(Duration::from_secs(2))
            .expect("합류 수신단으로 도착");
        assert_eq!(got.peer(), pid(1));
    }

    /// 발견 합류(X-2f) — 로컬 발견 이벤트가 사다리의 `discovery()`로 그대로(출처 Local · 토큰 0) 나온다.
    #[test]
    fn ladder_discovery_forwards_local_hints() {
        let bus = InMemoryBus::new();
        let a = bus.join(pid(1), name("a"), Caps::default());
        let ladder = LadderTransport::new(Arc::new(a), pid(1));
        let disc = ladder.discovery();
        let _b = bus.join(pid(2), name("b"), Caps::default());
        let ev = disc
            .recv_timeout(Duration::from_secs(2))
            .expect("로컬 등장이 합류 스트림으로");
        match ev {
            DiscoveryEvent::Appeared(h) => {
                assert_eq!(h.peer, pid(2));
                assert_eq!(h.origin, HintOrigin::Local);
                assert_eq!(h.liveness, Liveness::Advertised);
                assert_eq!(h.source, 0);
                assert!(h.profile.is_none());
            }
            other => panic!("예상 밖 이벤트 {other:?}"),
        }
    }

    /// 수동 주소 기억·망각 — 2단은 로컬 `add_endpoint`(InMemory = Unsupported)라 Unreachable로 떨어지되 패닉 없이.
    #[test]
    fn manual_note_and_forget() {
        let bus = InMemoryBus::new();
        let a = bus.join(pid(1), name("a"), Caps::default());
        let ladder = LadderTransport::new(Arc::new(a), pid(1));
        ladder.note_endpoint(pid(7), "203.0.113.5:47200");
        assert_eq!(lock(&ladder.manual).len(), 1);
        assert_eq!(
            ladder.connect(pid(7)).err(),
            Some(ConnectError::Unreachable)
        );
        ladder.forget_endpoint(pid(7));
        assert!(lock(&ladder.manual).is_empty());
    }

    /// 카드 정제(경계 · fail-closed) — 제어 문자 제거·200자 · 이름은 DisplayName 통과분만(실패 = 기본 라벨 · 카드 이름 빈 값).
    #[test]
    fn card_hint_sanitizes_at_the_edge() {
        let peer = pid(3);
        let (n, p) = card_hint(
            peer,
            &super::super::encode_card("Alice", "a@x\u{7}y", &"b".repeat(300)),
        );
        assert_eq!(n.as_str(), "Alice");
        let p = p.unwrap();
        assert_eq!(p.email, "a@xy");
        assert_eq!(p.bio.len(), 200);
        let (n2, p2) = card_hint(peer, &super::super::encode_card("", "", ""));
        assert_eq!(n2, default_display_name(None, &peer), "빈 이름 = 기본 라벨");
        assert_eq!(p2.unwrap().name, "");
        let (n3, p3) = card_hint(peer, b"");
        assert_eq!(n3, default_display_name(None, &peer));
        assert!(p3.is_none(), "카드 없음 = None");
    }
}
