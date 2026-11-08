//! 라우팅 — `(app, rid)` 바인딩(Exclusive/Shared) · 채널 표 · Open/Accept/Data/CloseCh 중계 · 큐 상한.
//!
//! 서버는 `kind·rid·ch·길이`만 본다. `Data.bytes`는 불투명(종단 Noise 암호문)이고 Accept 전에는 한 바이트도 나르지 않는다.
//!
//! **Shared 바인딩**(docs/44 I-2 "한 RID에 여러 기기" · ADR-0016 §7 결정 11-08): `Incoming`은 바인딩된 **전 연결**에
//! 팬아웃하고 **첫 Accept**가 채널을 잡는다 — 나머지 후보에는 `ChClosed`. 후보 전원이 거절(CloseCh)하면 여는 쪽에 `Busy`.

use crate::{ConnId, Shared};
use nbeep_relay_proto::{BindMode, Channel, ErrorCode, Kind, Rid, STATUS_OK};
use std::net::SocketAddr;
use std::sync::atomic::Ordering;
use std::sync::Arc;

/// `(app, rid)` 하나의 바인딩.
#[derive(Debug)]
pub(crate) struct Binding {
    pub(crate) conns: Vec<ConnId>,
    pub(crate) shared: bool,
}

/// 채널 — `a` = 여는 쪽 · `b` = 수락한 쪽(`None` = 미성립) · `candidates` = Incoming을 받은 후보(Shared 팬아웃).
#[derive(Debug)]
pub(crate) struct Chan {
    pub(crate) a: ConnId,
    pub(crate) b: Option<ConnId>,
    pub(crate) candidates: Vec<ConnId>,
    /// 여는 쪽 대조 토큰(Accept 시 OpenResult에 되돌린다).
    pub(crate) token: u32,
}

impl Chan {
    fn involves(&self, c: ConnId) -> bool {
        self.a == c || self.b == Some(c)
    }
}

fn frame(kind: Kind, f: impl FnOnce(&mut Channel)) -> Vec<u8> {
    let mut c = Channel::empty();
    f(&mut c);
    c.encode(kind).encode()
}

pub(crate) fn open_result(token: u32, status: u16, ch: u32, ep: Option<SocketAddr>) -> Vec<u8> {
    frame(Kind::OpenResult, |c| {
        c.token = Some(token);
        c.status = Some(status);
        if status == STATUS_OK {
            c.ch = Some(ch);
        }
        if let Some(sa) = ep {
            c.ep = crate::udp_obs::sock_to_ep(sa);
        }
    })
}

fn ch_closed(ch: u32) -> Vec<u8> {
    frame(Kind::ChClosed, |c| c.ch = Some(ch))
}

/// 연결의 UDP 관측값(프로브했다면).
fn udp_of(shared: &Arc<Shared>, conn: ConnId) -> Option<SocketAddr> {
    shared
        .conns
        .lock()
        .ok()
        .and_then(|c| c.get(&conn).and_then(|h| h.udp_obs))
}

/// RID 바인딩(Register). RID마다 status — 상한 초과분은 `LimitExceeded`(바인딩하지 않음).
/// Exclusive = 최신 등록 우선(재접속 잔재 교체) · Shared = 같은 모드끼리 합류(모드가 다르면 최신이 전부 교체).
pub(crate) fn bind(
    shared: &Arc<Shared>,
    me: ConnId,
    app: &str,
    rids: &[Rid],
    mode: BindMode,
) -> Vec<(Rid, u16)> {
    let max = usize::from(shared.limits.max_rids);
    let shared_mode = mode == BindMode::Shared;
    let mut out = Vec::with_capacity(rids.len());
    let mut bound = Vec::new();
    // 재등록이면 이전 바인딩부터 회수.
    unbind_all(shared, me);
    if let Ok(mut map) = shared.rids.lock() {
        for (i, rid) in rids.iter().enumerate() {
            if i >= max {
                out.push((*rid, ErrorCode::LimitExceeded as u16));
                continue;
            }
            let key = (app.to_string(), *rid);
            match map.get_mut(&key) {
                Some(b) if shared_mode && b.shared => {
                    if !b.conns.contains(&me) {
                        b.conns.push(me);
                    }
                }
                Some(b) => {
                    *b = Binding {
                        conns: vec![me],
                        shared: shared_mode,
                    };
                }
                None => {
                    map.insert(
                        key,
                        Binding {
                            conns: vec![me],
                            shared: shared_mode,
                        },
                    );
                }
            }
            bound.push(*rid);
            out.push((*rid, STATUS_OK));
        }
    }
    if let Ok(mut conns) = shared.conns.lock() {
        if let Some(h) = conns.get_mut(&me) {
            h.rids = bound;
        }
    }
    out
}

/// 연결의 바인딩 전부 회수(종료·재등록).
pub(crate) fn unbind_all(shared: &Arc<Shared>, me: ConnId) {
    if let Ok(mut map) = shared.rids.lock() {
        map.retain(|_, b| {
            b.conns.retain(|c| *c != me);
            !b.conns.is_empty()
        });
    }
}

/// 내가 당사자인 채널 수(상한 판정).
fn chan_count(shared: &Arc<Shared>, me: ConnId) -> usize {
    shared
        .chans
        .lock()
        .map(|c| c.values().filter(|ch| ch.involves(me)).count())
        .unwrap_or(usize::MAX)
}

/// Open — 성공이면 후보들에게 `Incoming`을 넣고 `None`(응답은 Accept 때), 실패면 **여는 쪽에 바로 보낼** OpenResult 프레임.
pub(crate) fn open(
    shared: &Arc<Shared>,
    me: ConnId,
    app: &str,
    token: u32,
    dst: Rid,
) -> Option<Vec<u8>> {
    let candidates: Vec<ConnId> = shared
        .rids
        .lock()
        .ok()
        .and_then(|m| {
            m.get(&(app.to_string(), dst))
                .map(|b| b.conns.iter().copied().filter(|c| *c != me).collect())
        })
        .unwrap_or_default();
    if candidates.is_empty() {
        // 대상 없음 · 자기 자신뿐(랑데부 무의미) · 다른 앱(이름공간 — 보이지 않는다).
        return Some(open_result(token, ErrorCode::NoSuchPeer as u16, 0, None));
    }
    if chan_count(shared, me) >= usize::from(shared.limits.max_chans) {
        return Some(open_result(token, ErrorCode::LimitExceeded as u16, 0, None));
    }
    let ch = shared.next_ch.fetch_add(1, Ordering::SeqCst);
    let (my_udp, src_rid) = shared
        .conns
        .lock()
        .ok()
        .and_then(|c| c.get(&me).map(|h| (h.udp_obs, h.rids.first().copied())))
        .unwrap_or((None, None));
    if let Ok(mut chans) = shared.chans.lock() {
        chans.insert(
            ch,
            Chan {
                a: me,
                b: None,
                candidates: candidates.clone(),
                token,
            },
        );
    }
    let incoming = frame(Kind::Incoming, |c| {
        c.ch = Some(ch);
        c.rid = src_rid;
        if let Some(sa) = my_udp {
            c.ep = crate::udp_obs::sock_to_ep(sa);
        }
    });
    let mut delivered = 0usize;
    for cand in &candidates {
        if enqueue(shared, *cand, incoming.clone()) {
            delivered += 1;
        }
    }
    if delivered == 0 {
        if let Ok(mut chans) = shared.chans.lock() {
            chans.remove(&ch);
        }
        return Some(open_result(token, ErrorCode::Busy as u16, 0, None));
    }
    shared.log(&format!(
        "conn#{me} open ch#{ch} → {}후보 {delivered}전달",
        candidates.len()
    ));
    None
}

/// Accept — 첫 수락자가 채널을 잡는다. 여는 쪽에 OpenResult(성립), 다른 후보에 ChClosed.
pub(crate) fn accept(shared: &Arc<Shared>, me: ConnId, ch: u32) {
    let taken = {
        let Ok(mut chans) = shared.chans.lock() else {
            return;
        };
        match chans.get_mut(&ch) {
            Some(c) if c.b.is_none() && c.candidates.contains(&me) => {
                c.b = Some(me);
                let others: Vec<ConnId> = c.candidates.drain(..).filter(|x| *x != me).collect();
                Some((c.a, c.token, others))
            }
            _ => None,
        }
    };
    let Some((opener, token, others)) = taken else {
        return;
    };
    let my_udp = udp_of(shared, me);
    let _ = enqueue(shared, opener, open_result(token, STATUS_OK, ch, my_udp));
    for o in others {
        let _ = enqueue(shared, o, ch_closed(ch));
    }
    shared.metrics.inc(&shared.metrics.channels_total);
    shared.log(&format!("ch#{ch} 성립(양방향)"));
}

/// Data — 성립된 채널의 당사자면 상대 연결.
pub(crate) fn other_party(shared: &Arc<Shared>, me: ConnId, ch: u32) -> Option<ConnId> {
    let chans = shared.chans.lock().ok()?;
    let c = chans.get(&ch)?;
    match c.b {
        Some(b) if c.a == me => Some(b),
        Some(b) if b == me => Some(c.a),
        _ => None,
    }
}

/// CloseCh — 수락 전 후보의 거절이면 후보에서 빼고(전원 거절 = 여는 쪽에 Busy), 아니면 채널을 닫는다.
pub(crate) fn close_by(shared: &Arc<Shared>, me: ConnId, ch: u32) {
    let refused_all = {
        let Ok(mut chans) = shared.chans.lock() else {
            return;
        };
        match chans.get_mut(&ch) {
            Some(c) if c.b.is_none() && c.candidates.contains(&me) => {
                c.candidates.retain(|x| *x != me);
                if c.candidates.is_empty() {
                    let (a, token) = (c.a, c.token);
                    chans.remove(&ch);
                    Some((a, token))
                } else {
                    return; // 다른 후보가 아직 결정 전
                }
            }
            Some(c) if c.involves(me) => None,
            _ => return, // 당사자도 후보도 아니다 — 남의 채널은 못 닫는다
        }
    };
    if let Some((opener, token)) = refused_all {
        // 여는 쪽이 open 타임아웃까지 기다리지 않게 **거절**로 즉시 알린다.
        let _ = enqueue(
            shared,
            opener,
            open_result(token, ErrorCode::Busy as u16, 0, None),
        );
        return;
    }
    close_channel(shared, ch, Some(me));
}

/// 채널 제거 + 남은 쪽(들)에 닫힘 통지. `except` = 통지 생략 대상(닫은 본인).
pub(crate) fn close_channel(shared: &Arc<Shared>, ch: u32, except: Option<ConnId>) {
    let removed = shared.chans.lock().ok().and_then(|mut c| c.remove(&ch));
    if let Some(chan) = removed {
        let mut sides = vec![chan.a];
        sides.extend(chan.b);
        sides.extend(chan.candidates);
        for side in sides {
            if Some(side) != except {
                let _ = enqueue(shared, side, ch_closed(ch));
            }
        }
    }
}

/// 연결 종료 — 내가 얽힌 채널 전부 정리(후보였던 것은 후보에서만 빠진다).
pub(crate) fn close_all_of(shared: &Arc<Shared>, me: ConnId) {
    let (mine, candidate_of): (Vec<u32>, Vec<u32>) = shared
        .chans
        .lock()
        .map(|c| {
            let mine = c
                .iter()
                .filter(|(_, ch)| ch.involves(me))
                .map(|(k, _)| *k)
                .collect();
            let cand = c
                .iter()
                .filter(|(_, ch)| !ch.involves(me) && ch.candidates.contains(&me))
                .map(|(k, _)| *k)
                .collect();
            (mine, cand)
        })
        .unwrap_or_default();
    for ch in mine {
        close_channel(shared, ch, Some(me));
    }
    for ch in candidate_of {
        close_by(shared, me, ch);
    }
}

/// 다른 연결의 큐에 넣는다. 상한 초과(느린 소비자)면 그 연결을 절단 지시하고 `false`.
pub(crate) fn enqueue(shared: &Arc<Shared>, conn_id: ConnId, frame: Vec<u8>) -> bool {
    let conns = match shared.conns.lock() {
        Ok(c) => c,
        Err(_) => return false,
    };
    let Some(handle) = conns.get(&conn_id) else {
        return false;
    };
    let cost = frame.len() + 16;
    let cap = usize::try_from(shared.limits.queue_bytes).unwrap_or(usize::MAX);
    if handle.queued.load(Ordering::SeqCst) + cost > cap {
        // 무한 버퍼 대신 정직한 절단(fail-closed) — 쓰기 교착·메모리 폭주 둘 다 회피.
        handle.kill.store(true, Ordering::SeqCst);
        shared.metrics.inc(&shared.metrics.killed_slow);
        return false;
    }
    handle.queued.fetch_add(cost, Ordering::SeqCst);
    handle.tx.send(frame).is_ok()
}
