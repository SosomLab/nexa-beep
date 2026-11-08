//! UDP 관측 프로브(STUN-lite · 능력 `UDP_OBSERVE`) — 형식 = [`nbeep_relay_proto::udp_obs`](v1과 동일 바이트).
//!
//! 등록된 토큰에만 답한다(스캔·증폭에 침묵). 관측값은 그 연결의 `udp_obs`에 남아 `Incoming`/`OpenResult`의 `ep`로 상대에게 간다.

use crate::{ConnId, Shared};
use nbeep_relay_proto::{udp_obs, Ep};
use std::net::{SocketAddr, UdpSocket};
use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::Duration;

/// 연결에 UDP 토큰을 발급·등록한다(Register 때).
pub(crate) fn issue_token(shared: &Arc<Shared>, conn_id: ConnId) -> u64 {
    let token = (conn_id << 32)
        ^ u64::from(
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_or(0, |d| d.subsec_nanos()),
        );
    if let Ok(mut tokens) = shared.tokens.lock() {
        tokens.retain(|_, v| *v != conn_id); // 재등록 = 이전 토큰 폐기
        tokens.insert(token, conn_id);
    }
    token
}

/// 연결 종료 시 토큰 회수.
pub(crate) fn forget(shared: &Arc<Shared>, conn_id: ConnId) {
    if let Ok(mut tokens) = shared.tokens.lock() {
        tokens.retain(|_, v| *v != conn_id);
    }
}

pub(crate) fn sock_to_ep(sa: SocketAddr) -> Ep {
    match sa {
        SocketAddr::V4(a) => Ep::V4(a.ip().octets(), a.port()),
        SocketAddr::V6(a) => Ep::V6(a.ip().octets(), a.port()),
    }
}

pub(crate) fn udp_loop(udp: &UdpSocket, shared: &Arc<Shared>) {
    let mut buf = [0u8; 64];
    while !shared.stop.load(Ordering::SeqCst) {
        match udp.recv_from(&mut buf) {
            Ok((n, from)) => {
                let Some(token) = udp_obs::parse_probe(&buf[..n]) else {
                    continue; // 미지 데이터그램은 조용히 버린다(응답 없음 — 증폭 방지)
                };
                let conn_id = shared
                    .tokens
                    .lock()
                    .ok()
                    .and_then(|t| t.get(&token).copied());
                let Some(conn_id) = conn_id else {
                    continue; // 등록된 토큰만 응답(스캔에 침묵)
                };
                if let Ok(mut conns) = shared.conns.lock() {
                    if let Some(c) = conns.get_mut(&conn_id) {
                        c.udp_obs = Some(from);
                    }
                }
                // 에코: [magic][token][관측 엔드포인트] — 클라가 자기 매핑을 안다.
                let _ = udp.send_to(&udp_obs::encode_echo(token, sock_to_ep(from)), from);
            }
            Err(e)
                if matches!(
                    e.kind(),
                    std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
                ) => {}
            // Windows: ICMP Port Unreachable가 ConnReset으로 올라온다 — 소켓은 살아 있다.
            Err(e) if e.kind() == std::io::ErrorKind::ConnectionReset => {}
            Err(_) => std::thread::sleep(Duration::from_millis(100)),
        }
    }
}
