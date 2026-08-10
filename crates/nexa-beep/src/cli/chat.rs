//! 인터랙티브 채팅 — `--chat-serve`/`--chat-connect`/`--chat-live` + **파일 전송**.
//!
//! 대화(`StreamId::Chat`)와 파일(`StreamId::File`)을 한 세션에서 다중화한다.
//! 수신 파일은 [`receive_into_quarantine`]에서 **무해화 게이트로 합류**한다 —
//! 해시 대조 → 위험 판정 → `.beepq` 봉인 → 격리 저장([docs/11]).

use nbeep_core::PeerId;

/// 인터랙티브 채팅 역할.
pub(crate) enum ChatRole {
    /// 고정 포트로 한 상대의 연결을 기다린다.
    Serve(u16),
    /// 주소로 직접 연결한다(DR-19 수동).
    Connect(String),
}

/// **인터랙티브 헤드리스 채팅**(사람이 stdin으로 타이핑·수신 실시간 출력). docker 터미널
/// (`-it`)과 맥 GUI/터미널이 사람 대 사람으로 양방향 대화한다. 세션 스택은 GUI와 동일
/// (Noise→TOFU→다중화) — 프레젠테이션만 stdin/stdout.
pub(crate) fn chat_interactive(role: ChatRole) {
    let identity = nbeep_crypto::Identity::generate();

    // 역할별로 Link 하나를 얻는다(서버=accept, 클라=connect). 이후 경로는 100% 같다.
    let link: Box<dyn nbeep_core::Link> = match &role {
        ChatRole::Serve(port) => {
            let listener = std::net::TcpListener::bind(("0.0.0.0", *port)).expect("포트 바인딩");
            println!(
                "[대기] {} 에서 상대를 기다립니다… (me={})",
                port,
                identity.peer_id().short()
            );
            let (stream, from) = listener.accept().expect("accept");
            println!("[연결] {from} 에서 연결됨");
            Box::new(nbeep_net::TcpLink::new(stream).expect("링크"))
        }
        ChatRole::Connect(addr) => {
            let mut instance = [0u8; 16];
            instance
                .copy_from_slice(&nbeep_crypto::Identity::generate().peer_id().as_bytes()[..16]);
            let name = nbeep_core::DisplayName::parse("chat").expect("라벨");
            let transport =
                nbeep_net::LocalDirect::spawn(identity.peer_id(), instance, name, 5000, 1)
                    .expect("전송");
            use nbeep_net::Transport as _;
            match transport.add_endpoint(addr) {
                Ok(l) => {
                    println!(
                        "[연결] {addr} 로 연결됨 (me={})",
                        identity.peer_id().short()
                    );
                    l
                }
                Err(e) => {
                    eprintln!("[실패] 연결 실패({addr}): {e:?}");
                    return;
                }
            }
        }
    };

    // 핸드셰이크(서버=accept·클라=initiate) → 신원 확정.
    let session = match &role {
        ChatRole::Serve(_) => nbeep_crypto::NoiseSession::accept(link, &identity),
        ChatRole::Connect(_) => nbeep_crypto::NoiseSession::initiate(link, &identity),
    };
    let session = match session {
        Ok(s) => s,
        Err(e) => {
            eprintln!("[실패] 핸드셰이크: {e}");
            return;
        }
    };
    run_interactive(session, identity.peer_id());
}

/// 수립된 세션 위 **인터랙티브 대화 루프** — stdin 라인 = 전송, 수신은 실시간 출력, Ctrl+D 종료.
/// serve/connect/live가 공용(세션 출처만 다르고 대화는 같다).
fn run_interactive<L: nbeep_core::Link + 'static>(
    mut session: nbeep_crypto::NoiseSession<L>,
    me: PeerId,
) {
    use nbeep_core::mux::{MuxSession, StreamId};
    use nbeep_core::{
        chunks_of, ChatMessage, MessageBody, Sequencer, Session as _, XferInbox, XferMsg,
    };
    use std::io::BufRead as _;

    /// stdin 스레드 → 네트 스레드 명령.
    enum Cmd {
        Chat(Vec<u8>),
        /// 파일 오퍼(발신) — id·이름·선언 해시·원본.
        Offer {
            id: nbeep_core::XferId,
            name: String,
            sha: [u8; 32],
            bytes: Vec<u8>,
        },
        Accept,
        Reject,
    }

    let peer = session.peer();
    println!(
        "[대화 시작] 상대={} · 한 줄 = 전송 · /send <파일> = 파일 전송 · Ctrl+D = 종료",
        peer.short()
    );
    // 수신 스레드: 세션 소유·recv 폴·도착 출력(액터 — 한 세션 1스레드). 송신은 채널 교대.
    session.set_recv_timeout(Some(std::time::Duration::from_millis(100)));
    let (out_tx, out_rx) = std::sync::mpsc::channel::<Cmd>();
    let net = std::thread::spawn(move || {
        let mut mux = MuxSession::new(session);
        // 수신 상한 = **수신측 설정**(사용자 확정 08-09) — CLI --xfer-limit-mib(기본 256MiB).
        // GUI 경로는 설정 키로 연동 예정(hot-swap 규약).
        let args: Vec<String> = std::env::args().collect();
        let limit = args
            .iter()
            .position(|a| a == "--xfer-limit-mib")
            .and_then(|i| args.get(i + 1))
            .and_then(|v| v.parse::<u64>().ok())
            .map_or(nbeep_core::MAX_FILE, |mib| mib * 1024 * 1024);
        // 수신 속도 상한(--xfer-rate-kb · 0/미지정 = 무제한). Accept로 상대에게 공지되어
        // **쌍방 중 낮은 쪽**이 적용된다.
        let recv_cap: u64 = args
            .iter()
            .position(|a| a == "--xfer-rate-kb")
            .and_then(|i| args.get(i + 1))
            .and_then(|v| v.parse::<u64>().ok())
            .map_or(0, |kb| kb * 1024);
        if recv_cap > 0 {
            println!(
                "[파일] 수신 속도 상한 {} KB/s (상대에게 공지)",
                recv_cap / 1024
            );
        }
        let mut inbox = XferInbox::with_max_file(limit);
        // 파일 수신 정책 — GUI와 **같은 도메인 함수**를 쓴다(정책이 두 벌이면 뚫린다).
        let mut ledger = nbeep_core::ExchangeLedger::new();
        let approval = nbeep_core::ApprovalPolicy::default(); // CLI 기본 = 수동
                                                              // 진행률 집계: (보낸 파일수, 총 파일수, 누적 바이트, 총 바이트).
        let mut batch = (0u32, 0u32, 0u64, 0u64);
        println!(
            "[파일] 수신 상한 {}MiB (수신측 설정 — --xfer-limit-mib)",
            inbox.max_file() / (1024 * 1024)
        );
        // 발신 대기(수락을 기다리는 원본) · 수신 대기(수락/거절을 기다리는 오퍼 id).
        let mut outgoing: std::collections::HashMap<nbeep_core::XferId, Vec<u8>> =
            std::collections::HashMap::new();
        let mut pending_in: Option<nbeep_core::XferId> = None;
        loop {
            loop {
                match out_rx.try_recv() {
                    Ok(Cmd::Chat(bytes)) => {
                        ledger.note_sent(peer); // 왕래 장부(상호 확인 근거)
                        if mux.send(StreamId::Chat, &bytes).is_err() {
                            println!("[종료] 세션 끊김");
                            return;
                        }
                    }
                    Ok(Cmd::Offer {
                        id,
                        name,
                        sha,
                        bytes,
                    }) => {
                        // 발신 자격 사전 점검 — 상대가 어차피 거절할 것을 미리 알린다.
                        if let Err(r) = nbeep_core::check_send_eligibility(
                            nbeep_core::TrustLevel::Pinned,
                            ledger.get(peer),
                        ) {
                            println!("[파일] 보낼 수 없음 — {}", r.message());
                            continue;
                        }
                        batch.1 += 1;
                        batch.3 += bytes.len() as u64;
                        let offer = XferMsg::Offer {
                            id,
                            size: bytes.len() as u64,
                            sha256: sha,
                            name: name.into_bytes(),
                        };
                        outgoing.insert(id, bytes);
                        if mux.send(StreamId::File, &offer.encode()).is_err() {
                            println!("[종료] 세션 끊김");
                            return;
                        }
                        println!("[파일] 오퍼 전송 — 상대 수락 대기");
                    }
                    Ok(Cmd::Accept) => {
                        if let Some(id) = pending_in {
                            if inbox.accept(&id).is_ok() {
                                let _ = mux.send(
                                    StreamId::File,
                                    &XferMsg::Accept {
                                        id,
                                        rate_cap: recv_cap,
                                    }
                                    .encode(),
                                );
                                println!("[파일] 수락 — 수신 시작");
                            }
                        } else {
                            println!("[파일] 대기 중인 오퍼가 없다");
                        }
                    }
                    Ok(Cmd::Reject) => {
                        if let Some(id) = pending_in.take() {
                            inbox.drop_xfer(&id);
                            let _ = mux.send(
                                StreamId::File,
                                &XferMsg::Reject {
                                    id,
                                    why: nbeep_core::RejectWhy::Declined,
                                    limit: 0,
                                }
                                .encode(),
                            );
                            println!("[파일] 거절");
                        }
                    }
                    Err(std::sync::mpsc::TryRecvError::Empty) => break,
                    Err(std::sync::mpsc::TryRecvError::Disconnected) => return,
                }
            }
            // 대화 스트림.
            match mux.recv(StreamId::Chat) {
                Ok(bytes) => {
                    if let Ok(m) = ChatMessage::decode(&bytes, peer) {
                        ledger.note_recv(peer); // 왕래 장부(상호 확인 근거)
                        if let MessageBody::Text(t) = m.body {
                            let safe = nbeep_core::sanitize_message(&t);
                            println!("{}> {}", peer.short(), safe.as_str());
                        }
                    }
                }
                Err(nbeep_core::SessionError::TimedOut) => {}
                Err(_) => {
                    println!("[종료] 세션 끊김");
                    return;
                }
            }
            // 파일 스트림.
            match mux.recv(StreamId::File) {
                Ok(bytes) => match XferMsg::decode(&bytes) {
                    Ok(XferMsg::Offer { id, size, name, .. }) => {
                        let m = XferMsg::decode(&bytes).expect("방금 성공한 디코드");
                        // ★ 자격 판정 먼저 — 상호 확인 없는 상대는 무조건 거부(사용자 규칙).
                        let now = std::time::SystemTime::now()
                            .duration_since(std::time::UNIX_EPOCH)
                            .map_or(0, |d| d.as_millis() as u64);
                        let verdict = nbeep_core::judge_offer(
                            nbeep_core::TrustLevel::Pinned,
                            ledger.get(peer),
                            approval,
                            now,
                        );
                        if let nbeep_core::OfferVerdict::Deny(reason) = verdict {
                            let why = match reason {
                                nbeep_core::DenyReason::Blocked => nbeep_core::RejectWhy::Blocked,
                                _ => nbeep_core::RejectWhy::Unverified,
                            };
                            let _ = mux.send(
                                StreamId::File,
                                &XferMsg::Reject { id, why, limit: 0 }.encode(),
                            );
                            println!("[파일] 수신 거부 — {}", reason.message());
                            continue;
                        }
                        match inbox.offer(&m) {
                            Ok(()) => {
                                pending_in = Some(id);
                                println!(
                                    "[파일] 오퍼: {} ({size}B) — /accept 또는 /reject",
                                    String::from_utf8_lossy(&name)
                                );
                                if matches!(verdict, nbeep_core::OfferVerdict::Accept)
                                    && inbox.accept(&id).is_ok()
                                {
                                    {
                                        let _ = mux.send(
                                            StreamId::File,
                                            &XferMsg::Accept {
                                                id,
                                                rate_cap: recv_cap,
                                            }
                                            .encode(),
                                        );
                                        println!("[파일] 자동 수락 — 수신 시작");
                                    }
                                }
                            }
                            Err(e) => {
                                let _ = mux.send(
                                    StreamId::File,
                                    &XferMsg::Reject {
                                        id,
                                        why: nbeep_core::RejectWhy::TooLarge,
                                        limit: inbox.max_file(), // 수신측 상한 공지
                                    }
                                    .encode(),
                                );
                                println!("[파일] 오퍼 자동 거부: {e} (상한 공지)");
                            }
                        }
                    }
                    Ok(XferMsg::Accept { id, rate_cap }) => {
                        // 상대가 수락 — 청크 스트리밍 + 완료.
                        if let Some(bytes) = outgoing.remove(&id) {
                            let total = bytes.len() as u64;
                            let mut sent_bytes = 0u64;
                            // 쌍방 협상 — 상대가 공지한 상한을 그대로 쓴다(CLI 자체 상한 없음).
                            let mut pacer = nbeep_core::Pacer::new(rate_cap);
                            if rate_cap > 0 {
                                println!(
                                    "[파일] 속도 협상 = {} KB/s (창 {}B)",
                                    rate_cap / 1024,
                                    pacer.burst_bytes()
                                );
                            }
                            let t0 = std::time::Instant::now();
                            for c in chunks_of(id, &bytes) {
                                if let XferMsg::Chunk { ref data, .. } = c {
                                    sent_bytes += data.len() as u64;
                                }
                                let wait = pacer.take(
                                    match c {
                                        XferMsg::Chunk { ref data, .. } => data.len() as u64,
                                        _ => 0,
                                    },
                                    t0.elapsed().as_millis() as u64,
                                );
                                if wait > 0 {
                                    std::thread::sleep(std::time::Duration::from_millis(wait));
                                }
                                if mux.send(StreamId::File, &c.encode()).is_err() {
                                    println!("[종료] 세션 끊김");
                                    return;
                                }
                                // 요청 형식: 전송완료용량/전체용량 (보낸파일수/총파일수)
                                println!(
                                    "[파일] 전송 {} / {} ({}/{})",
                                    human(batch.2 + sent_bytes),
                                    human(batch.3.max(total)),
                                    batch.0,
                                    batch.1
                                );
                            }
                            batch.0 += 1;
                            batch.2 += total;
                            let _ = mux.send(StreamId::File, &XferMsg::Done { id }.encode());
                            println!(
                                "[파일] 전송 완료 {} / {} ({}/{})",
                                human(batch.2),
                                human(batch.3),
                                batch.0,
                                batch.1
                            );
                        }
                    }
                    Ok(XferMsg::Reject { id, why, limit }) => {
                        outgoing.remove(&id); // 발신 대기물 폐기
                        if limit > 0 && matches!(why, nbeep_core::RejectWhy::TooLarge) {
                            println!(
                                "[파일] 상대가 거절: {why:?} — 상대 수신 상한 {}MiB",
                                limit / (1024 * 1024)
                            );
                        } else {
                            println!("[파일] 상대가 거절: {why:?}");
                        }
                    }
                    Ok(XferMsg::Chunk { id, offset, data }) => {
                        match inbox.chunk(&id, offset, &data) {
                            Ok(()) => {
                                if let Some((got, tot)) = inbox.progress(&id) {
                                    println!("[파일] 수신 {} / {} (1/1)", human(got), human(tot));
                                }
                            }
                            Err(e) => {
                                println!("[파일] 수신 오류: {e} — 폐기");
                                pending_in = None;
                            }
                        }
                    }
                    Ok(XferMsg::Done { id }) => match inbox.done(&id) {
                        Ok(got) => {
                            pending_in = None;
                            receive_into_quarantine(&got, peer);
                        }
                        Err(e) => println!("[파일] 완료 실패: {e} — 폐기"),
                    },
                    Ok(XferMsg::Cancel { id }) => {
                        inbox.drop_xfer(&id);
                        pending_in = None;
                        println!("[파일] 상대가 취소");
                    }
                    Err(e) => println!("[파일] 와이어 오류: {e}"),
                },
                Err(nbeep_core::SessionError::TimedOut) => {}
                Err(_) => {
                    println!("[종료] 세션 끊김");
                    return;
                }
            }
        }
    });
    let mut seq = Sequencer::new();
    let stdin = std::io::stdin();
    for line in stdin.lock().lines() {
        let Ok(line) = line else { break };
        if line.is_empty() {
            continue;
        }
        if let Some(path) = line.strip_prefix("/send ") {
            match std::fs::read(path.trim()) {
                Ok(bytes) => {
                    let sha = nbeep_crypto::sha256(&bytes);
                    // 전송 id — 새 키의 앞 16B(프로세스 내 유일 · 간이 난수).
                    let mut id = [0u8; 16];
                    id.copy_from_slice(
                        &nbeep_crypto::Identity::generate().peer_id().as_bytes()[..16],
                    );
                    let name = std::path::Path::new(path.trim())
                        .file_name()
                        .map(|n| n.to_string_lossy().into_owned())
                        .unwrap_or_else(|| "file".into());
                    if out_tx
                        .send(Cmd::Offer {
                            id,
                            name,
                            sha,
                            bytes,
                        })
                        .is_err()
                    {
                        break;
                    }
                }
                Err(e) => println!("[파일] 읽기 실패: {e}"),
            }
            continue;
        }
        if line.trim() == "/accept" {
            if out_tx.send(Cmd::Accept).is_err() {
                break;
            }
            continue;
        }
        if line.trim() == "/reject" {
            if out_tx.send(Cmd::Reject).is_err() {
                break;
            }
            continue;
        }
        let msg = ChatMessage {
            sender_device: me,
            seq: seq.issue(),
            body: MessageBody::Text(line),
        };
        if out_tx.send(Cmd::Chat(msg.encode())).is_err() {
            break;
        }
    }
    drop(out_tx);
    let _ = net.join();
    println!("[끝]");
}

/// 사람이 읽는 크기 표기(진행률).
fn human(b: u64) -> String {
    const K: u64 = 1024;
    match b {
        v if v >= K * K => format!("{:.1}MiB", v as f64 / (K * K) as f64),
        v if v >= K => format!("{:.1}KiB", v as f64 / K as f64),
        v => format!("{v}B"),
    }
}

/// 수신 완료물 → **무해화 게이트 합류**(공용 [`crate::gate`]) 후 결과를 stdout에 보고.
fn receive_into_quarantine(got: &nbeep_core::Received, sender: PeerId) {
    match crate::gate::quarantine_received(got, sender, crate::gate::CH_CLI) {
        Ok(q) => {
            println!(
                "[파일] 격리 수신 완료: {} · risk={:?}{} · {}",
                q.name,
                q.risk,
                if q.mismatch {
                    " · ⚠️ 형식 불일치"
                } else {
                    ""
                },
                q.path.display()
            );
            println!("       (실체화는 승인 후 --quarantine-demo 참조 — 자동 실체화 없음)");
        }
        Err(e) => println!("[파일] {e} — 수신물 폐기"),
    }
}

/// **발견 가능한 인터랙티브 클라이언트**(`--chat-live [이름]`) — LocalDirect로 **발견 광고**(GUI
/// 목록에 뜬다) + 첫 인바운드(GUI가 클릭해 연결) 수락 → 인터랙티브 대화. 실행 중인 `--window --live`
/// GUI를 터미널에서 붙어 테스트하는 용도.
pub(crate) fn chat_live(name: &str) {
    use nbeep_net::Transport as _;
    let identity = nbeep_crypto::Identity::generate();
    let mut instance = [0u8; 16];
    instance.copy_from_slice(&nbeep_crypto::Identity::generate().peer_id().as_bytes()[..16]);
    let display = nbeep_core::DisplayName::parse(name)
        .unwrap_or_else(|_| nbeep_core::DisplayName::parse("chat-live").expect("라벨"));
    let transport =
        match nbeep_net::LocalDirect::spawn(identity.peer_id(), instance, display, 800, 1) {
            Ok(t) => t,
            Err(e) => {
                eprintln!("[실패] 전송 시작: {e}");
                return;
            }
        };
    println!(
        "[대기] '{name}'(me={}) 로 발견 광고 중 — 실행 중인 GUI(--window --live) 목록에서 클릭하세요…",
        identity.peer_id().short()
    );
    let incoming = transport.incoming();
    let Ok(link) = incoming.recv() else {
        eprintln!("[실패] 인바운드 없음");
        return;
    };
    match nbeep_crypto::NoiseSession::accept(link, &identity) {
        Ok(session) => run_interactive(session, identity.peer_id()),
        Err(e) => eprintln!("[실패] 핸드셰이크: {e}"),
    }
}
