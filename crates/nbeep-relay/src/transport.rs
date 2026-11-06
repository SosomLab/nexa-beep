//! X-2 릴레이 **Transport 승격**(DR-8 ② · ADR-0003 §3 "구현 3종: LocalDirect · InMemory · Relay" · 11-06).
//!
//! - [`RelayTransport`] = 붙어 있는 [`RelayClient`] 하나 위의 `Transport`(`nbeep_net::Transport`) — `connect`는 랑데부 사다리
//!   (`open_ladder` · 홀펀칭 UDP → 릴레이 · **첫 회신이 온 단에 고정**)로 **핸드셰이크 전 `Link`**
//!   ([`LadderLink`])를 돌려주고, `incoming`은 서버가 넘긴 인바운드를 첫 프레임 링크 추종
//!   (`accept_link_via`)으로 잇는다. 종단 Noise는 호출자(App·CLI)가 LAN과 **같은 코드**로 얹는다 —
//!   "암호화는 Transport 위"(ADR-0003 핵심 규칙)가 릴레이 경로에도 성립한다. 종전(X-2b · 08-22)은 사다리 안에서
//!   핸드셰이크를 해 App이 경로마다 다른 코드(`connect_via`/`accept_via` vs `Transport::connect`)를 들었다.
//! - [`LadderTransport`] = 로컬 직접([`nbeep_net::LocalDirect`] 등 임의의 `Transport`) + 기억한 수동 주소 + 릴레이를
//!   **한 `Transport`** 로 묶는다: `connect` = 발견 경로 → 수동 주소 → 랑데부(ADR-0003 §4 우선순위 "로컬 > 수동 > 릴레이")
//!   · `incoming` = 두 통로가 **한 수신단**으로 합류(호스트의 핸드셰이크 상한·타임아웃·psk 백오프가 릴레이 인바운드에도
//!   그대로 적용) · 발견 스트림과 설정류(이름·태그·조용한 모드·필터)는 로컬에 위임. 릴레이는 호스트가 서버에 붙였다
//!   떼는 대로 [`LadderTransport::set_relay`]로 갈아 끼운다(hot-swap). 수명은 **호스트의 `Arc<RelayClient>`**가 정하고
//!   여기는 `Weak`만 든다 — 호스트가 접속을 내려놓으면(해제·재접속·오프라인) 승격 실패로 조용히 꺼진다.
//!
//! 봉투 원리: 이 모듈은 상대 `PeerId`(RID 유도용)와 서버 주소만 안다. 발견 **명부**(roster · X-2e)는 호스트 몫으로 남긴다 —
//! `Transport::discovery` 스트림에 합치려면 `PeerHint` 출처 표식과 `PeerTable` 수명 모델(광고 타임아웃 없는 상대)이
//! 먼저 필요하다(ADR-0003 개정 후보 · [docs/32 §12-7]). 여기서의 "발견 증강"은 **발견 밖 상대도 `connect` 하나로 닿는다**
//! (roster·64hex 지문 상대 = LAN 주소록이 비어도 랑데부 단이 받는다)는 뜻이다.

use super::{accept_link_via, current_epoch_day, open_ladder, rid_for, RelayClient, ViaError};
use nbeep_core::link::Link;
use nbeep_core::{DisplayName, PeerId};
use nbeep_net::{AddError, Caps, ConnectError, DiscoveryEvent, InboundFilter, Transport};
use std::collections::HashMap;
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError, Weak};
use std::time::Duration;

/// 랑데부 Open 대기 상한(서버 왕복) — 종전 GUI/CLI 사다리 값(10s).
const OPEN_TIMEOUT: Duration = Duration::from_secs(10);
/// 인바운드 사다리 성립 상한(펀치 병행 · 첫 프레임 대기) — 종전 `spawn_relay_accept` 값(12s).
const ACCEPT_DEADLINE: Duration = Duration::from_secs(12);
/// 인바운드 폴 간격 — 이 주기로 `Weak` 승격을 다시 본다(호스트가 접속을 내려놓으면 펌프가 끝난다).
const ACCEPT_POLL: Duration = Duration::from_secs(1);

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
fn forward(rx: Receiver<Box<dyn Link>>, tx: Sender<Box<dyn Link>>) {
    std::thread::spawn(move || {
        while let Ok(link) = rx.recv() {
            if tx.send(link).is_err() {
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

/// 랑데부 서버 하나 위의 [`Transport`](X-2) — 발견 스트림은 **비어 있다**(명부는 호스트 roster · 모듈 문서).
pub struct RelayTransport {
    /// 호스트가 든 접속의 약한 참조 — 승격 실패 = 호스트가 내려놓았다.
    client: Weak<RelayClient>,
    incoming_rx: Mutex<Option<Receiver<Box<dyn Link>>>>,
    discovery_rx: Mutex<Option<Receiver<DiscoveryEvent>>>,
}

impl core::fmt::Debug for RelayTransport {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("RelayTransport")
            .field("alive", &self.live_client().is_some())
            .finish_non_exhaustive()
    }
}

impl RelayTransport {
    /// 붙어 있는 클라이언트 위에 전송을 올린다 — 인바운드 펌프(서버 → 첫 프레임 링크 추종 → `incoming`)를 띄운다.
    /// 소유권은 가져가지 않는다(`Weak`): 호출자의 `Arc`가 마지막으로 떨어지면 펌프도 끝난다.
    #[must_use]
    pub fn new(client: &Arc<RelayClient>) -> Self {
        let (tx, rx) = channel::<Box<dyn Link>>();
        // 발견 스트림 = 송신단을 바로 버린 빈 채널(recv = Disconnected · 호출자가 폴링하지 않는다).
        let (_disc_tx, disc_rx) = channel::<DiscoveryEvent>();
        spawn_accept_pump(Arc::downgrade(client), tx);
        Self {
            client: Arc::downgrade(client),
            incoming_rx: Mutex::new(Some(rx)),
            discovery_rx: Mutex::new(Some(disc_rx)),
        }
    }

    /// 살아 있는 접속(승격 + 액터 생존).
    fn live_client(&self) -> Option<Arc<RelayClient>> {
        self.client.upgrade().filter(|c| c.is_alive())
    }
}

/// 인바운드 펌프 — 서버가 넘긴 랑데부마다 스레드 하나로 사다리를 잇고(최대 [`ACCEPT_DEADLINE`]) 핸드셰이크 전 링크를
/// 수신단에 넘긴다. 동시 인바운드가 서로를 기다리지 않는다(LAN 펌프와 같은 결 · 상한은 호스트 펌프가 건다 — RL-13 ⓐ).
fn spawn_accept_pump(weak: Weak<RelayClient>, tx: Sender<Box<dyn Link>>) {
    std::thread::spawn(move || loop {
        let Some(client) = weak.upgrade() else {
            break; // 호스트가 접속을 내려놓았다 — 소임 끝
        };
        let Some(inc) = client.accept_incoming(ACCEPT_POLL) else {
            continue; // 타임아웃 — Weak를 다시 보고 돈다
        };
        let tx = tx.clone();
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
    /// 현재 장착된 랑데부 단(None = Unmanaged/미접속) — [`Self::set_relay`]로 교체.
    relay: Mutex<Option<Arc<RelayTransport>>>,
    /// 기억한 수동 주소(DR-19 ④ · [`Transport::note_endpoint`]) — 발견 경로 다음 단.
    manual: Mutex<HashMap<PeerId, String>>,
    incoming_tx: Sender<Box<dyn Link>>,
    incoming_rx: Mutex<Option<Receiver<Box<dyn Link>>>>,
}

impl core::fmt::Debug for LadderTransport {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("LadderTransport")
            .field("relay_attached", &self.relay_attached())
            .field("manual", &lock(&self.manual).len())
            .finish_non_exhaustive()
    }
}

impl LadderTransport {
    /// 로컬 전송을 감싼다 — 로컬의 `incoming()`을 **여기서 소유**해 합류 수신단으로 중계한다(호출자는 로컬의
    /// `incoming()`을 더 부르지 않는다). 발견 스트림은 [`Transport::discovery`]가 로컬 것을 그대로 돌려준다.
    #[must_use]
    pub fn new(local: Arc<dyn Transport + Send + Sync>) -> Arc<Self> {
        let (tx, rx) = channel::<Box<dyn Link>>();
        forward(local.incoming(), tx.clone());
        Arc::new(Self {
            local,
            relay: Mutex::new(None),
            manual: Mutex::new(HashMap::new()),
            incoming_tx: tx,
            incoming_rx: Mutex::new(Some(rx)),
        })
    }

    /// 랑데부 단 장착/해제(hot-swap) — `Some` = 이 접속 위에 [`RelayTransport`]를 새로 올리고 그 인바운드를 합류시킨다 ·
    /// `None` = 단을 내린다(진행 중 인바운드 스레드는 제 상한까지 돈다 · 호스트가 `Arc`를 놓으면 펌프도 끝난다).
    pub fn set_relay(&self, client: Option<&Arc<RelayClient>>) {
        let next = client.map(|c| {
            let rt = Arc::new(RelayTransport::new(c));
            forward(rt.incoming(), self.incoming_tx.clone());
            rt
        });
        *lock(&self.relay) = next;
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
        self.local.discovery()
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
        let ladder = LadderTransport::new(Arc::new(a));
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
        let ladder_b = LadderTransport::new(Arc::new(b));
        let b_in = ladder_b.incoming();
        let _l = a.connect(pid(2)).expect("a → b");
        let got = b_in
            .recv_timeout(Duration::from_secs(2))
            .expect("합류 수신단으로 도착");
        assert_eq!(got.peer(), pid(1));
    }

    /// 수동 주소 기억·망각 — 2단은 로컬 `add_endpoint`(InMemory = Unsupported)라 Unreachable로 떨어지되 패닉 없이.
    #[test]
    fn manual_note_and_forget() {
        let bus = InMemoryBus::new();
        let a = bus.join(pid(1), name("a"), Caps::default());
        let ladder = LadderTransport::new(Arc::new(a));
        ladder.note_endpoint(pid(7), "203.0.113.5:47200");
        assert_eq!(lock(&ladder.manual).len(), 1);
        assert_eq!(
            ladder.connect(pid(7)).err(),
            Some(ConnectError::Unreachable)
        );
        ladder.forget_endpoint(pid(7));
        assert!(lock(&ladder.manual).is_empty());
    }
}
