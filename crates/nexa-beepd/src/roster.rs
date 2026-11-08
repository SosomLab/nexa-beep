//! 명부(X-2e roster · 옵트인 — [docs/32 §12-7] · 능력 `ROSTER`) — **앱별**. Announce를 켠 연결만 담고
//! 공개끼리만 서로 보인다(상호성). 카드는 **내용 해석 없이** 크기만 재고 되뿌린다(봉투 원리 — 상한
//! `limits.card_max` 초과 = 빈 카드 취급). PeerUp/PeerDown은 Noise가 증명한 공개키(`peer`)를 나른다.

use crate::{ConnId, Shared};
use nbeep_core::PeerId;
use nbeep_relay_proto::{Kind, Presence};
use std::sync::Arc;

/// 공개 목록 항목.
#[derive(Debug)]
pub(crate) struct Listed {
    pub(crate) app: String,
    pub(crate) peer: PeerId,
    pub(crate) card: Vec<u8>,
}

fn peer_up(peer: PeerId, card: Vec<u8>) -> Vec<u8> {
    Presence {
        listed: true,
        peer: Some(*peer.as_bytes()),
        rid: None,
        card,
    }
    .encode(Kind::PeerUp)
    .encode()
}

fn peer_down(peer: PeerId) -> Vec<u8> {
    Presence {
        listed: false,
        peer: Some(*peer.as_bytes()),
        rid: None,
        card: Vec::new(),
    }
    .encode(Kind::PeerDown)
    .encode()
}

/// Announce 처리. 돌려주는 값 = **나에게 직접 보낼** 프레임들(입장 스냅숏). 남에게 갈 것은 안에서 enqueue.
pub(crate) fn announce(
    shared: &Arc<Shared>,
    me: ConnId,
    app: &str,
    peer: PeerId,
    listed: bool,
    card: Vec<u8>,
) -> Vec<Vec<u8>> {
    if !listed {
        let removed = shared.listed.lock().ok().and_then(|mut m| m.remove(&me));
        if removed.is_some() {
            broadcast_down(shared, me, app, peer);
            shared.log(&format!("conn#{me} 공개 off"));
        }
        return Vec::new();
    }
    let card = if card.len() > usize::from(shared.limits.card_max) {
        Vec::new()
    } else {
        card
    };
    enum Change {
        New,
        CardChanged,
        None,
    }
    let (change, others) = {
        let Ok(mut map) = shared.listed.lock() else {
            return Vec::new();
        };
        let change = match map.entry(me) {
            std::collections::hash_map::Entry::Vacant(v) => {
                v.insert(Listed {
                    app: app.to_string(),
                    peer,
                    card: card.clone(),
                });
                Change::New
            }
            std::collections::hash_map::Entry::Occupied(mut o) => {
                let e = o.get_mut();
                if e.card == card {
                    Change::None // 멱등 재공지 — 방송 없음
                } else {
                    e.card = card.clone();
                    Change::CardChanged
                }
            }
        };
        let others: Vec<(ConnId, PeerId, Vec<u8>)> = map
            .iter()
            .filter(|(id, l)| **id != me && l.app == app)
            .map(|(id, l)| (*id, l.peer, l.card.clone()))
            .collect();
        (change, others)
    };
    match change {
        Change::New => {
            // 입장 스냅숏(기존 공개 → 나) + 등장 방송(나 → 그들).
            let mut snapshot = Vec::with_capacity(others.len());
            for (cid, p, c) in others {
                snapshot.push(peer_up(p, c));
                let _ = crate::router::enqueue(shared, cid, peer_up(peer, card.clone()));
            }
            shared.log(&format!("conn#{me} 공개 on(app={app})"));
            snapshot
        }
        Change::CardChanged => {
            for (cid, _, _) in others {
                let _ = crate::router::enqueue(shared, cid, peer_up(peer, card.clone()));
            }
            shared.log(&format!("conn#{me} 카드 갱신({}B)", card.len()));
            Vec::new()
        }
        Change::None => Vec::new(),
    }
}

/// 연결 종료 — 공개 목록에서 회수 + 같은 앱의 남은 공개 사용자에게 이탈 방송.
pub(crate) fn leave(shared: &Arc<Shared>, me: ConnId, peer: PeerId) {
    let removed = shared.listed.lock().ok().and_then(|mut m| m.remove(&me));
    if let Some(l) = removed {
        broadcast_down(shared, me, &l.app, peer);
    }
}

fn broadcast_down(shared: &Arc<Shared>, except: ConnId, app: &str, peer: PeerId) {
    let targets: Vec<ConnId> = shared
        .listed
        .lock()
        .map(|m| {
            m.iter()
                .filter(|(id, l)| **id != except && l.app == app)
                .map(|(id, _)| *id)
                .collect()
        })
        .unwrap_or_default();
    for cid in targets {
        let _ = crate::router::enqueue(shared, cid, peer_down(peer));
    }
}
