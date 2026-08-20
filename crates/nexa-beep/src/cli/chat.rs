//! 인터랙티브 채팅 — `--chat-serve`/`--chat-connect`/`--chat-live` + **파일 전송**.
//!
//! 대화(`StreamId::Chat`)와 파일(`StreamId::File`)을 한 세션에서 다중화한다.
//! 수신 파일은 [`receive_into_quarantine`]에서 **무해화 게이트로 합류**한다 —
//! 해시 대조 → 위험 판정 → `.beepq` 봉인 → 격리 저장([docs/11]).

use nbeep_core::PeerId;

/// 대화 중 쓸 수 있는 명령 안내 — **한 곳에서만 적는다**(안내와 구현이 갈리면 거짓말이 된다).
const HELP_COMMANDS: &str =
    "[명령] /quit(/exit) = 종료 · /send <파일…> = 파일 전송(공백 구분 다중 · 요청당 최대 5) · \
/accept = 요청 전체 수락 · /reject = 요청 전체 거절 · /help · Ctrl+D = 종료";

/// 상대를 기다리는 동안 쓸 수 있는 것 — 이때는 전송할 상대가 없으니 종료만 가능하다.
const HELP_WAITING: &str =
    "[대기] /quit + Enter 또는 Ctrl+D(Windows는 Ctrl+Z+Enter) = 종료 (Ctrl+C도 됩니다)";

/// 공유 stdin 줄 채널 — **raw 미지원 경로**(Windows 콘솔 · 파이프) 전용.
///
/// 스레드 하나가 stdin을 전담해 줄 단위로 채널에 넣는다. 대기 루프와 대화 루프가
/// **같은 채널**을 번갈아 소비한다 — 각자 stdin을 직접 read하면 서로 줄을 뺏는다
/// (대기 스레드가 대화의 첫 줄을 삼키는 식). 블로킹 read를 스레드로 밀어낸 이유는
/// 폴링 루프가 연결·종료 신호를 계속 봐야 해서다(08-13 실기 — Windows 대기 중
/// `/quit` 불통). EOF(Ctrl+Z+Enter · 파이프 끝) = 송신단 드롭 = 채널 닫힘.
fn stdin_lines() -> &'static std::sync::Mutex<std::sync::mpsc::Receiver<String>> {
    use std::sync::{mpsc, Mutex, OnceLock};
    static CH: OnceLock<Mutex<mpsc::Receiver<String>>> = OnceLock::new();
    CH.get_or_init(|| {
        let (tx, rx) = mpsc::channel();
        std::thread::spawn(move || {
            use std::io::BufRead as _;
            let stdin = std::io::stdin();
            for line in stdin.lock().lines() {
                let Ok(line) = line else { break };
                if tx.send(line).is_err() {
                    break; // 소비자가 사라졌다(프로세스 종료 경로)
                }
            }
        });
        Mutex::new(rx)
    })
}

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
    // 대화형 = 콘솔 입력을 독점해야 한다(08-20 windows 서브시스템 전환 — cmd/pwsh는
    // GUI 앱을 안 기다려 셸과 입력 경합). 필요 시 자기 콘솔 창으로 옮긴다.
    let _ = nbeep_plat::launch::own_console_for_interactive();
    let identity = nbeep_crypto::Identity::generate();

    // 역할별로 Link 하나를 얻는다(서버=accept, 클라=connect). 이후 경로는 100% 같다.
    let link: Box<dyn nbeep_core::Link> = match &role {
        ChatRole::Serve(port) => {
            let listener = std::net::TcpListener::bind(("0.0.0.0", *port)).expect("포트 바인딩");
            // 논블로킹 — 기다리는 동안 stdin(`/quit`)과 종료 신호도 함께 본다.
            listener.set_nonblocking(true).expect("논블로킹");
            println!(
                "[대기] {} 에서 상대를 기다립니다… (me={})",
                port,
                identity.peer_id().short()
            );
            // `.ok()` = WouldBlock 포함 실패는 "아직" — 계속 기다린다.
            let Some((stream, from)) = wait_with_quit(|| listener.accept().ok()) else {
                return;
            };
            // 대화 루프는 블로킹 소켓을 전제한다(수신 타임아웃은 세션이 건다).
            stream.set_nonblocking(false).expect("블로킹 복귀");
            println!("[연결] {from} 에서 연결됨");
            Box::new(nbeep_net::TcpLink::new(stream).expect("링크"))
        }
        ChatRole::Connect(addr) => {
            let mut instance = [0u8; 16];
            instance
                .copy_from_slice(&nbeep_crypto::Identity::generate().peer_id().as_bytes()[..16]);
            let name = nbeep_core::DisplayName::parse("chat").expect("라벨");
            // 수신 포트 0(임의) — 발신 전용 경로. 같은 PC GUI의 기본 포트를 뺏지 않는다.
            let transport =
                nbeep_net::LocalDirect::spawn_on(identity.peer_id(), instance, name, 5000, 1, 0)
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
            // ⚠️ `--chat-serve`는 **accept 1회**가 문서화된 성질이라(26 §9) 여기서 끝난다.
            // 떠돌이 연결에도 죽지 않아야 하는 쪽은 광고를 계속 띄우는 `--chat-live`고,
            // 그쪽은 대기 루프로 고쳤다(08-13).
            eprintln!("[실패] 핸드셰이크: {e}");
            return;
        }
    };
    run_interactive(session, identity.peer_id());
}

/// 상대를 기다리는 동안 **stdin도 함께 본다** — `/quit`·Ctrl+D로 나갈 수 있게.
///
/// ★ 왜 필요한가: 예전에는 대기 구간이 `accept()`/`recv()`에 **통째로 갇혀** 있어서
/// 키를 읽는 쪽이 아예 없었다. 그래서 Ctrl+D가 듣지 않았고(실측 08-11), Ctrl+C만
/// 남았는데 그건 `Drop`을 건너뛰어 **터미널을 raw 모드로 남긴다**(에코가 죽어 셸이
/// 먹통처럼 보인다). 폴링 raw 모드로 바꿔 두 문제를 함께 없앤다.
///
/// `poll`은 100ms마다 불리며 `Some(T)`를 주면 기다림이 끝난 것이다.
/// 사용자가 종료를 원하면 `None`을 돌려준다(호출 측은 조용히 반환 — 정리는 `Drop`).
fn wait_with_quit<T>(poll: impl FnMut() -> Option<T>) -> Option<T> {
    wait_with_quit_or(poll, |_| {
        println!("\r{HELP_WAITING}");
        None
    })
}

/// [`wait_with_quit`] + **줄 명령 훅**(08-20 — chat-live 대기 중 `/peers`·`/connect`):
/// `/quit` 계열은 여기서 끝내고(None), 그 외 비어 있지 않은 줄은 `on_line`에 넘긴다.
/// `on_line`이 `Some(v)`를 주면 기다림이 그 값으로 끝난다(아웃바운드 연결 성립 등) ·
/// `None` = 안내/실패를 스스로 찍고 계속 대기.
fn wait_with_quit_or<T>(
    mut poll: impl FnMut() -> Option<T>,
    mut on_line: impl FnMut(&str) -> Option<T>,
) -> Option<T> {
    use nbeep_plat::term::{parse_key, TermKey};
    use std::io::Read as _;

    let shutdown = nbeep_plat::shutdown::install();
    // 폴링 모드 — 키가 없으면 0.1초 뒤 돌아온다(그 틈에 연결·종료 신호를 본다).
    let raw = nbeep_plat::term::RawTerm::enter_polling();
    println!("{HELP_WAITING}");
    // 대화형인가 — 채널 닫힘(EOF)의 해석이 갈린다: 사람 터미널이면 "종료 의사",
    // 파이프·컨테이너(stdin 없음)면 "명령 경로가 없을 뿐"이라 연결 대기는 계속한다.
    let interactive = std::io::IsTerminal::is_terminal(&std::io::stdin());
    let mut chan_dead = false;
    let mut stdin = std::io::stdin();
    let mut pending = Vec::<u8>::new();
    let mut line = String::new();
    let mut chunk = [0u8; 256];
    loop {
        if let Some(v) = poll() {
            return Some(v);
        }
        // Ctrl+C는 이제 핸들러가 잡는다 — 기본 종료가 아니라 여기로 와서
        // `raw`의 Drop이 돌고 터미널이 원래대로 복원된다.
        if shutdown.requested() {
            println!("\r[종료] 중단합니다.");
            return None;
        }
        if !raw.is_raw() {
            // raw 미지원(Windows 콘솔 · 파이프) — 전담 스레드의 줄 채널로 명령을 받는다.
            // 그전엔 여기서 stdin을 아예 안 읽어 **대기 중 /quit이 불통**이었다
            // (08-13 Windows 실기 · 직접 블로킹 read는 연결 폴링을 세운다).
            if chan_dead {
                std::thread::sleep(std::time::Duration::from_millis(100));
                continue;
            }
            match stdin_lines()
                .lock()
                .expect("stdin 채널 잠금")
                .recv_timeout(std::time::Duration::from_millis(100))
            {
                Ok(typed) => {
                    if matches!(typed.trim(), "/quit" | "/exit" | "/q") {
                        println!("[종료] 대기를 중단합니다.");
                        return None;
                    }
                    if !typed.trim().is_empty() {
                        if let Some(v) = on_line(typed.trim()) {
                            return Some(v);
                        }
                    }
                }
                Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {}
                Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => {
                    if interactive {
                        // 사람 터미널의 EOF(Ctrl+Z+Enter) — Ctrl+D와 같은 종료 의사.
                        println!("[종료] 대기를 중단합니다.");
                        return None;
                    }
                    chan_dead = true; // 파이프 끝 — 명령만 없어진 것. 연결은 계속 기다린다.
                }
            }
            continue;
        }
        let n = match stdin.read(&mut chunk) {
            Ok(n) => n,
            Err(_) => continue, // 시그널로 끊긴 read — 위에서 플래그를 다시 본다
        };
        pending.extend_from_slice(&chunk[..n]);
        while let Some((key, used)) = parse_key(&pending) {
            pending.drain(..used);
            match key {
                TermKey::Eof => {
                    println!("\r[종료] 대기를 중단합니다.");
                    return None;
                }
                TermKey::Enter => {
                    let typed = core::mem::take(&mut line);
                    print!("\r\n");
                    let _ = std::io::Write::flush(&mut std::io::stdout());
                    if matches!(typed.trim(), "/quit" | "/exit" | "/q") {
                        println!("\r[종료] 대기를 중단합니다.");
                        return None;
                    }
                    if !typed.trim().is_empty() {
                        if let Some(v) = on_line(typed.trim()) {
                            return Some(v);
                        }
                    }
                }
                TermKey::Backspace => {
                    if line.pop().is_some() {
                        print!("\u{8} \u{8}");
                        let _ = std::io::Write::flush(&mut std::io::stdout());
                    }
                }
                TermKey::Char(c) => {
                    line.push(c);
                    print!("{c}");
                    let _ = std::io::Write::flush(&mut std::io::stdout());
                }
                TermKey::ShiftEnter | TermKey::Other => {}
            }
        }
    }
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

    /// stdin 스레드 → 네트 스레드 명령.
    enum Cmd {
        Chat(Vec<u8>),
        /// 파일 오퍼(발신) — **요청(배치) 단위**(08-20 · GUI M4-2e 미러): 첫 오퍼
        /// 전에 배치 목록(manifest 태그 13)을 공지하고 파일별 Offer를 잇따라
        /// 보낸다. 단일 파일도 요청 1개다(GUI 발신과 같은 모양).
        /// 항목 = (id, 이름, 선언 해시, 원본).
        OfferBatch {
            files: Vec<(nbeep_core::XferId, String, [u8; 32], Vec<u8>)>,
        },
        Accept,
        Reject,
    }

    let peer = session.peer();
    println!("[대화 시작] 상대={} · 한 줄 = 전송", peer.short());
    println!("{HELP_COMMANDS}");
    // 수신 스레드: 세션 소유·recv 폴·도착 출력(액터 — 한 세션 1스레드). 송신은 채널 교대.
    session.set_recv_timeout(Some(std::time::Duration::from_millis(100)));
    let (out_tx, out_rx) = std::sync::mpsc::channel::<Cmd>();
    /// 세션이 끝났음을 입력 루프에 알리는 플래그 가드(08-13).
    ///
    /// 예전에는 세션이 끊겨도 **수신 스레드만** 죽고 입력 루프는 stdin에 묶여 남았다 —
    /// `--chat-live`가 대기로 돌아가지 못해 **살아는 있는데 아무도 다시 붙지 못하는**
    /// 상태가 됐다(실측: 상대 `/quit` 후 재연결 실패).
    /// 수신 스레드에는 `return`이 여러 곳이라 `Drop`으로 거는 게 안전하다.
    struct AliveGuard(std::sync::Arc<std::sync::atomic::AtomicBool>);
    impl Drop for AliveGuard {
        fn drop(&mut self) {
            self.0.store(false, std::sync::atomic::Ordering::Relaxed);
        }
    }
    let alive = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(true));
    let alive_net = std::sync::Arc::clone(&alive);
    let net = std::thread::spawn(move || {
        // 어떤 경로로 끝나든(끊김·오류·정상) 입력 루프에 알린다.
        let _guard = AliveGuard(alive_net);
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
        // 발신 대기(수락을 기다리는 원본) · 수신 대기(수락/거절을 기다리는 오퍼들 —
        // 요청 단위 발신은 오퍼가 한꺼번에 오므로 단일 슬롯이 아니라 목록이다).
        let mut outgoing: std::collections::HashMap<nbeep_core::XferId, Vec<u8>> =
            std::collections::HashMap::new();
        let mut pending_ins: Vec<(nbeep_core::XferId, String, u64)> = Vec::new();
        // ── 요청 단위 승인(08-20 · GUI M4-2e 미러 — 와이어 = manifest 태그 13) ──
        // req_files = 남은 전송 예정 목록(이름·크기 — 제외분은 애초에 안 담는다 ·
        // 오퍼는 **이름+크기 대조** 후에만 자동 처리 = fail-closed). 승인/거절은
        // 요청 전체에 1회. 구버전 발신(무 manifest)은 파일 단위로 자연 폴백.
        let mut req_files: Vec<(String, u64)> = Vec::new();
        let mut req_open = false;
        let mut req_approved = false;
        let mut req_declined = false;
        // 수락 후 Done 전인 수신 개수 — 요청 종결 판정에 필요(목록 소진만 보면
        // 마지막 파일이 아직 오는 중인데 "수신 완료"가 먼저 찍힌다 · 실측 08-20).
        let mut active_recv = 0usize;
        // ── 프로필(M3-17 · 검증 도구) — 테스트 프로필로 응답하고, 상대 프로필도 1회 요청 ──
        // 이름 = "★{표시이름}"(GUI 목록에서 발견 이름과 한눈에 구분되게) · 이미지 = 70KiB
        // 의사 바이트(청크 3개 — 조립 경로 실측용 · 픽셀 아님).
        let my_name = args
            .iter()
            .position(|a| a == "--chat-live")
            .and_then(|i| args.get(i + 1))
            .filter(|s| !s.starts_with("--"))
            .cloned()
            .unwrap_or_else(|| "테스트단말".into());
        let _ = mux.send(StreamId::Control, &nbeep_core::ProfileMsg::Request.encode());
        println!("[프로필] 상대 프로필 요청 발신(자동 프리페치와 동일 경로)");
        let mut img_got = 0usize; // 상대 이미지 수신 누계(도구 — 바이트는 버린다)
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
                    Ok(Cmd::OfferBatch { files }) => {
                        // 발신 자격 사전 점검 — 상대가 어차피 거절할 것을 미리 알린다.
                        if let Err(r) = nbeep_core::check_send_eligibility(
                            nbeep_core::TrustLevel::Pinned,
                            ledger.get(peer),
                        ) {
                            println!("[파일] 보낼 수 없음 — {}", r.message());
                            continue;
                        }
                        // 요청 단위(08-20) — **첫 오퍼 전에** 배치 목록(manifest)을
                        // 공지한다. GUI 수신자는 이걸로 승인 창 하나에 목록·총합을
                        // 보이고 승인 1회로 전체를 받는다. 구 수신자는 미지 Control
                        // 태그를 버리고 파일 단위로 강등(전방 호환).
                        let entries: Vec<(String, u64, bool)> = files
                            .iter()
                            .map(|(_, n, _, b)| (n.clone(), b.len() as u64, false))
                            .collect();
                        if mux
                            .send(
                                StreamId::Control,
                                &nbeep_core::xfer::encode_batch_manifest(&entries),
                            )
                            .is_err()
                        {
                            println!("[종료] 세션 끊김");
                            return;
                        }
                        let n = files.len();
                        let tot: u64 = entries.iter().map(|e| e.1).sum();
                        for (id, name, sha, bytes) in files {
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
                        }
                        println!(
                            "[파일] 요청 전송 — {n}개 · 총 {} · 상대 승인 대기",
                            human(tot)
                        );
                    }
                    Ok(Cmd::Accept) => {
                        if pending_ins.is_empty() {
                            println!("[파일] 대기 중인 오퍼가 없다");
                        } else {
                            // 요청 단위(08-20) — 승인 1회 = 대기분 전부 수락 + 이후
                            // 도착분 자동 수락 무장(목록 대조 통과분만 — fail-closed).
                            for (id, nm, sz) in pending_ins.drain(..) {
                                if inbox.accept(&id).is_ok() {
                                    active_recv += 1;
                                    let _ = mux.send(
                                        StreamId::File,
                                        &XferMsg::Accept {
                                            id,
                                            rate_cap: recv_cap,
                                            resume_offset: 0, // CLI는 재개 미지원(M4-10)
                                            prefix_sha: [0u8; 32],
                                        }
                                        .encode(),
                                    );
                                    println!("[파일] 수락 — 수신 시작: {nm}");
                                }
                                // 목록 소진 — 같은 이름·크기 재오퍼가 무한 자동 수락되지 않게.
                                if let Some(i) =
                                    req_files.iter().position(|(rn, rs)| *rn == nm && *rs == sz)
                                {
                                    req_files.remove(i);
                                }
                            }
                            if req_open && !req_approved {
                                req_approved = true;
                                if !req_files.is_empty() {
                                    println!(
                                        "[파일] 요청 전체 승인 — 남은 {}개는 자동 수락",
                                        req_files.len()
                                    );
                                }
                            }
                        }
                    }
                    Ok(Cmd::Reject) => {
                        if pending_ins.is_empty() {
                            println!("[파일] 대기 중인 오퍼가 없다");
                        } else {
                            // 거절 1회 = 요청 전체(08-20 확정 미러) — 대기분 전부 거절.
                            // 발신자는 첫 Reject(Declined)로 배치를 접으므로 잔여는
                            // 실제로 안 오고, 거절 무장은 경합 안전망이다.
                            for (id, nm, _) in pending_ins.drain(..) {
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
                                println!("[파일] 거절: {nm}");
                            }
                            if req_open {
                                req_declined = true;
                                req_files.clear();
                                println!("[파일] 요청 전체 거절");
                            }
                        }
                    }
                    Err(std::sync::mpsc::TryRecvError::Empty) => break,
                    Err(std::sync::mpsc::TryRecvError::Disconnected) => return,
                }
            }
            // ★ 수신은 **스트림별이 아니라 도착 순서로** 뽑는다(08-13 — 스트림별 폴링은
            // 파일 전송 2MiB 지점에서 Backpressure로 끊겼다. 근거 = `MuxSession::recv_any`).
            let (stream, bytes) = match mux.recv_any() {
                Ok(v) => v,
                Err(nbeep_core::SessionError::TimedOut) => continue,
                Err(_) => {
                    println!("[종료] 세션 끊김");
                    return;
                }
            };
            // 대화 스트림.
            match stream {
                StreamId::Chat => {
                    if let Ok(m) = ChatMessage::decode(&bytes, peer) {
                        ledger.note_recv(peer); // 왕래 장부(상호 확인 근거)
                                                // 수신확인(N-2 · 08-20 사용자 실기 "CLI 상대는 마크가 안 뜬다") —
                                                // **전달**은 파싱 직후 자동(GUI와 동일), **읽음**은 터미널 특성상
                                                // 출력 즉시가 곧 "봄"이라 함께 되쏜다(창 가시성에 해당하는 구분
                                                // 시점이 없다 — 08-20 검토 확정). 검증 도구라 프라이버시 게이트
                                                // 없이 항상 발신(제품 규칙의 게이트는 수신자 설정 축 — GUI 몫).
                        for kind in [nbeep_core::AckKind::Delivered, nbeep_core::AckKind::Read] {
                            let _ = mux.send(
                                StreamId::Control,
                                &nbeep_core::ChatAck {
                                    target_seq: m.seq,
                                    kind,
                                }
                                .encode(),
                            );
                        }
                        if let MessageBody::Text(t) = m.body {
                            let safe = nbeep_core::sanitize_message(&t);
                            println!("{}> {}", peer.short(), safe.as_str());
                        }
                    }
                }
                // 프로필 스트림(Control — M3-17 검증).
                // 공유 그룹(M5-1g) — 테스트 단말은 관찰만(초대·본문을 로그로 · 수락 UI 없음).
                StreamId::Group => match nbeep_core::SGroupMsg::decode(&bytes) {
                    Some(nbeep_core::SGroupMsg::Invite { roster }) => {
                        println!(
                            "[그룹] 초대 수신 — '{}' (구성원 {}명 · 소유자 {}) · 이 단말은 수락 UI가 없다(관찰용)",
                            roster.name.as_str(),
                            roster.members.len(),
                            roster.owner.short()
                        );
                    }
                    Some(nbeep_core::SGroupMsg::Msg { uid, text, .. }) => {
                        let safe = nbeep_core::sanitize_message(&text);
                        println!("[그룹 {}] {}> {}", uid.short(), peer.short(), safe.as_str());
                    }
                    Some(other) => println!("[그룹] 제어 수신 — {other:?}"),
                    None => {}
                },
                // 수신확인 도착(N-2 · 태그 10) — 상대(GUI)가 내 메시지를 받았다/봤다.
                // 그전엔 ProfileMsg 디코드 실패로 **조용히 버려져** CLI에선 안 보였다.
                StreamId::Control if nbeep_core::ChatAck::decode(&bytes).is_some() => {
                    if let Some(a) = nbeep_core::ChatAck::decode(&bytes) {
                        println!(
                            "[확인] {} — 내 메시지 seq={}",
                            match a.kind {
                                nbeep_core::AckKind::Delivered => "전달됨",
                                nbeep_core::AckKind::Read => "읽음",
                            },
                            a.target_seq
                        );
                    }
                }
                // 배치 목록(M4-2e · 태그 13) — 요청 단위 승인의 원료(GUI 미러 · 08-20).
                StreamId::Control if nbeep_core::xfer::decode_batch_manifest(&bytes).is_some() => {
                    if let Some(entries) = nbeep_core::xfer::decode_batch_manifest(&bytes) {
                        let send: Vec<(String, u64)> = entries
                            .iter()
                            .filter(|e| !e.2)
                            .map(|e| (e.0.clone(), e.1))
                            .collect();
                        let tot: u64 = send.iter().map(|e| e.1).sum();
                        let n_ex = entries.len() - send.len();
                        println!(
                            "[파일] 요청 {}개 · 총 {}{} — /accept = 요청 전체 수락 · /reject = 전체 거절",
                            send.len(),
                            human(tot),
                            if n_ex > 0 {
                                format!(" · 제외 {n_ex}개")
                            } else {
                                String::new()
                            }
                        );
                        for (i, (nm, sz, ex)) in entries.iter().enumerate() {
                            println!(
                                "  {}. {} ({}){}",
                                i + 1,
                                nm,
                                human(*sz),
                                if *ex {
                                    " — 제외(전송 안 함)"
                                } else {
                                    ""
                                }
                            );
                        }
                        req_files = send;
                        req_open = true;
                        req_approved = false;
                        req_declined = false;
                    }
                }
                StreamId::Control => match nbeep_core::ProfileMsg::decode(&bytes) {
                    Some(nbeep_core::ProfileMsg::Request) => {
                        println!("[프로필] 상대가 요청 — 테스트 프로필 응답(★{my_name})");
                        let img: Vec<u8> = (0..70_000u32).map(|i| (i % 251) as u8).collect();
                        let mut frames = vec![nbeep_core::ProfileMsg::Info {
                            name: Some(format!("★{my_name}")),
                            email: Some(format!("{my_name}@test.local")),
                            phone: Some("010-0000-0000".into()),
                            image_len: u32::try_from(img.len()).unwrap_or(0),
                            avatar: None, // 테스트 도구 — 내장 아바타·보더는 GUI 몫
                            border: None,
                            image_keep: false, // 언제나 전체 응답(테스트 도구)
                            bio: Some("테스트 소개글".into()),
                        }
                        .encode()];
                        let mut off = 0usize;
                        while off < img.len() {
                            let end = (off + nbeep_core::PROFILE_IMAGE_CHUNK).min(img.len());
                            frames.push(
                                nbeep_core::ProfileMsg::ImageChunk {
                                    offset: u32::try_from(off).unwrap_or(u32::MAX),
                                    last: end == img.len(),
                                    bytes: img[off..end].to_vec(),
                                }
                                .encode(),
                            );
                            off = end;
                        }
                        for f in frames {
                            if mux.send(StreamId::Control, &f).is_err() {
                                println!("[종료] 세션 끊김");
                                return;
                            }
                        }
                    }
                    Some(nbeep_core::ProfileMsg::Info {
                        name,
                        email,
                        phone,
                        image_len,
                        ..
                    }) => {
                        println!(
                            "[프로필] 수신 — 이름={} 이메일={} 전화={} 이미지={image_len}B (미공개 필드는 애초에 안 실려 옴)",
                            name.as_deref().unwrap_or("(비공개)"),
                            email.as_deref().unwrap_or("(비공개)"),
                            phone.as_deref().unwrap_or("(비공개)"),
                        );
                        img_got = 0;
                    }
                    Some(nbeep_core::ProfileMsg::ImageChunk { bytes, last, .. }) => {
                        img_got += bytes.len();
                        if last {
                            println!("[프로필] 이미지 수신 완료 — {img_got}B(도구라 버림)");
                        }
                    }
                    None => {}
                },
                // 파일 스트림.
                StreamId::File => match XferMsg::decode(&bytes) {
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
                                let nm = String::from_utf8_lossy(&name).into_owned();
                                // 요청 목록 대조(이름+크기 — 불일치는 수동 폴백 · fail-closed).
                                let in_req = req_files
                                    .iter()
                                    .position(|(rn, rs)| *rn == nm && *rs == size);
                                if req_declined && in_req.is_some() {
                                    // 거절 무장(경합 안전망 — 정상 경로는 발신자가 접는다).
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
                                    println!("[파일] 자동 거절(요청 거절분) — {nm}");
                                } else if (req_approved && in_req.is_some())
                                    || matches!(verdict, nbeep_core::OfferVerdict::Accept)
                                {
                                    // 요청 승인분(목록 대조 통과) 또는 정책 자동 수락.
                                    if let Some(i) = in_req {
                                        req_files.remove(i);
                                    }
                                    if inbox.accept(&id).is_ok() {
                                        active_recv += 1;
                                        let _ = mux.send(
                                            StreamId::File,
                                            &XferMsg::Accept {
                                                id,
                                                rate_cap: recv_cap,
                                                resume_offset: 0,
                                                prefix_sha: [0u8; 32],
                                            }
                                            .encode(),
                                        );
                                        println!("[파일] 자동 수락 — 수신 시작: {nm}");
                                    }
                                } else {
                                    pending_ins.push((id, nm.clone(), size));
                                    if req_open && in_req.is_some() {
                                        // 목록은 이미 보였다 — 첫 오퍼에서 한 줄만.
                                        if pending_ins.len() == 1 {
                                            println!(
                                                "[파일] 도착: {nm} — /accept = 요청 전체 수락 · /reject = 전체 거절"
                                            );
                                        }
                                    } else {
                                        // 무 manifest(구버전·단발) 또는 목록 불일치 = 파일 단위.
                                        println!(
                                            "[파일] 오퍼: {nm} ({size}B) — /accept 또는 /reject"
                                        );
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
                    Ok(XferMsg::Accept { id, rate_cap, .. }) => {
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
                            let _ = mux.send(
                                StreamId::File,
                                &XferMsg::Done {
                                    id,
                                    sha256: [0u8; 32], // CLI는 Offer에 실선언(구경로)
                                }
                                .encode(),
                            );
                            // ★ 전송 끝 ≠ 완료(M4-9) — 상대의 Received 확인 전까지는 확인 대기.
                            println!(
                                "[파일] 전달됨 {} / {} ({}/{}) — 상대 확인 대기",
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
                                pending_ins.retain(|p| p.0 != id);
                            }
                        }
                    }
                    Ok(XferMsg::Done { id, sha256 }) => match inbox.done(&id) {
                        Ok(mut got) => {
                            pending_ins.retain(|p| p.0 != id);
                            active_recv = active_recv.saturating_sub(1);
                            // 지연 선언(08-18) — Offer가 0(스트리밍 발신 = GUI)이면 Done
                            // 동봉 해시가 선언이다. ★ 이 보정이 GUI 수신(app.rs)에만 있고
                            // 여기 CLI에 빠져, **GUI→CLI 파일이 전부 SHA-256 불일치로
                            // 폐기**됐다(08-20 실기 — *배선 = 곧 빠뜨린 호출부* 재발).
                            // 둘 다 0 = 선언 부재(fail-closed — 검증 없는 수신물은 없다).
                            if got.declared_sha256 == [0u8; 32] {
                                if sha256 == [0u8; 32] {
                                    println!("[파일] 무결성 선언 부재 — 폐기");
                                    let _ =
                                        mux.send(StreamId::File, &XferMsg::Failed { id }.encode());
                                    continue;
                                }
                                got.declared_sha256 = sha256;
                            }
                            let ok = receive_into_quarantine(&got, peer);
                            // ★ 종단 확인(M4-9) — 격리 성패를 발신자에게 되돌린다.
                            let ack = if ok {
                                XferMsg::Received { id }
                            } else {
                                XferMsg::Failed { id }
                            };
                            let _ = mux.send(StreamId::File, &ack.encode());
                            // 요청 종결(08-20) — 목록·대기가 다 비면 배치를 닫는다
                            // (다음 요청의 manifest가 다시 연다).
                            if req_open
                                && req_files.is_empty()
                                && pending_ins.is_empty()
                                && active_recv == 0
                            {
                                req_open = false;
                                req_approved = false;
                                req_declined = false;
                                println!("[파일] 요청 수신 완료");
                            }
                        }
                        Err(e) => {
                            active_recv = active_recv.saturating_sub(1);
                            println!("[파일] 완료 실패: {e} — 폐기");
                            let _ = mux.send(StreamId::File, &XferMsg::Failed { id }.encode());
                        }
                    },
                    // ★ 발신측이 받는 종단 확인(M4-9) — 확인 대기가 여기서 닫힌다.
                    Ok(XferMsg::Received { .. }) => {
                        println!("[파일] 상대 수신 확인 — 완료");
                    }
                    Ok(XferMsg::Failed { .. }) => {
                        println!("[파일] 상대가 받지 못함 — 실패(무결성·저장)");
                    }
                    Ok(XferMsg::Cancel { id }) => {
                        inbox.drop_xfer(&id);
                        pending_ins.retain(|p| p.0 != id);
                        // 상대 취소 = 배치 협상도 접는다(잔여 자동 수락 무장 해제 — 안전측).
                        req_open = false;
                        req_files.clear();
                        req_approved = false;
                        req_declined = false;
                        active_recv = active_recv.saturating_sub(1);
                        println!("[파일] 상대가 취소");
                    }
                    Err(e) => println!("[파일] 와이어 오류: {e}"),
                },
            }
        }
    });
    let mut seq = Sequencer::new();
    // 입력 한 줄을 처리한다(명령 or 전송) — 계속하려면 true.
    let mut handle_line = |line: String| -> bool {
        if line.trim().is_empty() {
            return true;
        }
        if let Some(rest) = line.strip_prefix("/send ") {
            let rest = rest.trim();
            // 다중 파일(08-20 · 요청 단위) — **한 줄 전체가 실재 파일이면 1개**(공백
            // 경로 기존 규약 보존), 아니면 공백 구분 다중으로 해석한다.
            let paths: Vec<String> = if std::path::Path::new(rest).is_file() {
                vec![rest.to_string()]
            } else {
                rest.split_whitespace().map(String::from).collect()
            };
            // 요청당 상한 5(GUI `xfer.batch_max` 기본값 미러) — 초과 = **시도 0건 + 안내**.
            if paths.len() > 5 {
                println!(
                    "[파일] 요청당 최대 5개 — {}개는 나눠 보내세요 (시도 안 함)",
                    paths.len()
                );
                return true;
            }
            let mut files = Vec::new();
            for p in &paths {
                match std::fs::read(p) {
                    Ok(bytes) => {
                        let sha = nbeep_crypto::sha256(&bytes);
                        // 전송 id — 새 키의 앞 16B(프로세스 내 유일 · 간이 난수).
                        let mut id = [0u8; 16];
                        id.copy_from_slice(
                            &nbeep_crypto::Identity::generate().peer_id().as_bytes()[..16],
                        );
                        let name = std::path::Path::new(p).file_name().map_or_else(
                            || "file".to_string(),
                            |n| n.to_string_lossy().into_owned(),
                        );
                        files.push((id, name, sha, bytes));
                    }
                    // 하나라도 못 읽으면 요청 전체 취소(시도 0건 — 반쪽 요청 방지).
                    Err(e) => {
                        println!("[파일] 읽기 실패: {p} — {e} (요청 취소 · 시도 안 함)");
                        return true;
                    }
                }
            }
            if files.is_empty() {
                println!("[파일] 보낼 파일이 없다 — /send <파일…>");
                return true;
            }
            return out_tx.send(Cmd::OfferBatch { files }).is_ok();
        }
        // ★ 명시적 종료 — Ctrl+D를 모르는 사람도, 그 키가 막힌 터미널에서도 나갈 수 있어야 한다.
        //   `false`를 돌려주면 두 입력 경로(raw·파이프)가 **같은 정리 절차**로 빠져나간다.
        if matches!(line.trim(), "/quit" | "/exit" | "/q") {
            println!("\r[종료] 대화를 끝냅니다.");
            return false;
        }
        if line.trim() == "/help" || line.trim() == "/?" {
            println!("\r{HELP_COMMANDS}");
            return true;
        }
        if line.trim() == "/accept" {
            return out_tx.send(Cmd::Accept).is_ok();
        }
        if line.trim() == "/reject" {
            return out_tx.send(Cmd::Reject).is_ok();
        }
        let msg = ChatMessage {
            sender_device: me,
            seq: seq.issue(),
            body: MessageBody::Text(line),
            importance: nbeep_core::Importance::Normal,
        };
        out_tx.send(Cmd::Chat(msg.encode())).is_ok()
    };

    // ── 입력 루프 ──
    // TTY면 **raw 모드**로 키를 직접 읽어 Shift+Enter(줄바꿈)를 Enter(전송)와 구분한다.
    // 파이프 입력(자동화)이면 기존 줄 단위 경로를 그대로 쓴다.
    //
    // ★ **폴링 모드 + 종료 플래그**(실기 08-13 — kitty 프로토콜 누수): 그전엔 블로킹
    // raw였고 connect/live 경로는 시그널 핸들러도 없어서, 대화 중 Ctrl+C = 즉사 =
    // `Drop` 생략 = **터미널이 kitty 모드로 남았다**(그 pane의 Ctrl+C가 `9;5u`로 찍힘).
    // wait_with_quit과 같은 패턴으로 — 신호는 플래그로 받고, 나가는 길은 언제나
    // 정리(Drop → 복원)를 지난다.
    let shutdown = nbeep_plat::shutdown::install();
    let raw = nbeep_plat::term::RawTerm::enter_polling();
    if raw.is_raw() {
        use nbeep_plat::term::{parse_key, TermKey};
        use std::io::{Read as _, Write as _};
        println!("[입력] Enter = 전송 · Shift+Enter = 줄바꿈(터미널이 지원할 때) · \\ 끝 = 줄바꿈");
        let mut stdin = std::io::stdin();
        let mut pending = Vec::<u8>::new();
        let mut line = String::new();
        let mut chunk = [0u8; 256];
        'outer: loop {
            if shutdown.requested() {
                println!("\r[종료] 중단합니다.");
                break;
            }
            if !alive.load(std::sync::atomic::Ordering::Relaxed) {
                break; // 세션이 끝났다 — 호출자(대기 루프)로 돌아간다
            }
            let n = match stdin.read(&mut chunk) {
                Ok(0) => continue,  // 폴링 타임아웃(0.1초) — 종료 신호를 다시 본다
                Err(_) => continue, // EINTR 등 — 위에서 플래그를 다시 본다
                Ok(n) => n,
            };
            pending.extend_from_slice(&chunk[..n]);
            // 버퍼에서 뗄 수 있는 키를 모두 처리한다.
            while let Some((key, used)) = parse_key(&pending) {
                pending.drain(..used);
                match key {
                    TermKey::Eof => break 'outer,
                    TermKey::Enter => {
                        // ★ 대체 수단: 줄 끝 `\`도 줄바꿈으로 친다 — Shift+Enter를
                        //   보고하지 않는 터미널(macOS Terminal.app 등)에서도 여러 줄을 쓴다.
                        if line.ends_with('\\') {
                            line.pop();
                            line.push('\n');
                            print!("\r\n… ");
                            let _ = std::io::stdout().flush();
                            continue;
                        }
                        let sent = core::mem::take(&mut line);
                        print!("\r\n");
                        let _ = std::io::stdout().flush();
                        if !handle_line(sent) {
                            break 'outer;
                        }
                    }
                    TermKey::ShiftEnter => {
                        line.push('\n');
                        print!("\r\n… ");
                        let _ = std::io::stdout().flush();
                    }
                    TermKey::Backspace => {
                        if line.pop().is_some() {
                            // 지운 자리를 시각적으로 되돌린다.
                            print!("\u{8} \u{8}");
                            let _ = std::io::stdout().flush();
                        }
                    }
                    TermKey::Char(c) => {
                        line.push(c);
                        print!("{c}");
                        let _ = std::io::stdout().flush();
                    }
                    TermKey::Other => {}
                }
            }
        }
    } else {
        // raw 미지원(Windows 콘솔 · 파이프) — 대기 루프와 **같은** 공유 채널을 소비한다
        // (직접 read하면 대기 스레드와 줄을 뺏고 뺏긴다). 타임아웃 폴이라 종료 신호와
        // **세션 사망도 즉시** 본다 — 그전엔 블로킹 read라 세션이 끊겨도 Enter를 칠
        // 때까지 갇혀 있었다(08-13 실기: "[종료] 세션 끊김" 후 /exit를 쳐야 했던 것).
        let rx = stdin_lines().lock().expect("stdin 채널 잠금");
        loop {
            if shutdown.requested() {
                println!("[종료] 중단합니다.");
                break;
            }
            if !alive.load(std::sync::atomic::Ordering::Relaxed) {
                break; // 세션이 끝났다 — 호출자(대기 루프)로 돌아간다
            }
            match rx.recv_timeout(std::time::Duration::from_millis(100)) {
                Ok(line) => {
                    if !handle_line(line) {
                        break;
                    }
                }
                Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {}
                Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => break, // EOF
            }
        }
    }
    drop(raw);
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
/// 수신물을 격리하고 **성공 여부**를 돌려준다(M4-9 — 발신자에게 종단 확인 ack를 보내기 위해).
fn receive_into_quarantine(got: &nbeep_core::Received, sender: PeerId) -> bool {
    // 봉인 키 = **영속 신원**(identity.key — CLI 대화의 임시 신원으로 봉인하면
    // 그 실행이 끝나는 순간 아무도 못 연다 · GUI·CLI 격리함이 같은 봉투를 연다).
    let Ok((id, _)) =
        nbeep_crypto::keyfile::load_or_generate(&crate::app::data_dir().join("identity.key"))
    else {
        println!("[파일] 격리 실패: 신원 키를 열 수 없음(봉인 불가 — 평문 저장 안 함)");
        return false;
    };
    match crate::gate::quarantine_received(got, sender, crate::gate::CH_CLI, &id.wrap_secret()) {
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
            true
        }
        Err(e) => {
            println!("[파일] {e} — 수신물 폐기");
            false
        }
    }
}

/// **발견 가능한 인터랙티브 클라이언트**(`--chat-live [이름]`) — LocalDirect로 **발견 광고**(GUI
/// 목록에 뜬다) + 첫 인바운드(GUI가 클릭해 연결) 수락 → 인터랙티브 대화. 실행 중인 `--window --live`
/// GUI를 터미널에서 붙어 테스트하는 용도.
pub(crate) fn chat_live(name: &str, port: u16) {
    // 대화형 — chat_interactive와 같은 콘솔 독점 규약(08-20).
    let _ = nbeep_plat::launch::own_console_for_interactive();
    use nbeep_net::Transport as _;
    let identity = nbeep_crypto::Identity::generate();
    let mut instance = [0u8; 16];
    instance.copy_from_slice(&nbeep_crypto::Identity::generate().peer_id().as_bytes()[..16]);
    let display = nbeep_core::DisplayName::parse(name)
        .unwrap_or_else(|_| nbeep_core::DisplayName::parse("chat-live").expect("라벨"));
    // 수신 포트 — 기본은 0(임의)이고, `--port`로 고정할 수 있다(08-13).
    // 같은 서브넷이면 **발견 광고가 실제 포트를 알리므로** 값이 무엇이든 상관없다.
    // 고정이 필요한 경우는 **발견이 닿지 않는 곳**(다른 서브넷·컨테이너 경계)뿐이다.
    let transport =
        match nbeep_net::LocalDirect::spawn_on(identity.peer_id(), instance, display, 800, 1, port)
        {
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
    // ★ 실제 리슨 포트를 찍는다 — 발견이 닿지 않는 상대(다른 서브넷·컨테이너 경계)에게는
    //   사람이 이 값을 알려줘야 수동 연결이 된다(ADR-0006 §3-1 · 08-13 실기에서 막혔던 지점).
    println!(
        "[포트] 세션 수신 {} — 발견이 닿지 않는 상대에겐 `--chat-connect <내IP>:{}` 로 알려준다",
        transport.tcp_port(),
        transport.tcp_port()
    );
    let incoming = transport.incoming();
    // ── 발견 조회·능동 연결(08-20 사용자 요청 — "CLI에서 다른 PC 조회·IP 추가") ──
    // 대기 중 발견 이벤트를 흘려 받아 번호 목록을 만들고, `/peers` = 조회 ·
    // `/connect <번호|host[:port]>` = 아웃바운드(수동 IP는 DR-19 — GUI ⌘K와 같은
    // 정규화 · M1-14 단일 코어). 인바운드(GUI가 목록에서 클릭)는 종전 그대로.
    let discovery = transport.discovery();
    let peers = std::cell::RefCell::new(Vec::<(nbeep_core::PeerId, String)>::new());
    /// 대기의 결과 — 방향만 다르고(In = accept · Out = initiate) 이후 대화는 같다.
    enum LiveLink {
        In(Box<dyn nbeep_core::Link>),
        Out(Box<dyn nbeep_core::Link>),
    }
    const HELP_LIVE: &str =
        "[대기] /peers = 발견 목록 · /connect <번호|host[:port]> = 연결 · /quit = 종료";
    println!("{HELP_LIVE}");
    // ★ 대기는 **되풀이한다**(08-13). 예전에는 핸드셰이크가 한 번 실패하면 그대로 프로세스가
    //   끝났다 — 즉 **떠돌이 TCP 연결 하나로 단말이 죽는다.** 포트 스캐너·헬스체크·`nc -z`
    //   한 번이면 충분했다(실측: `nc -z 127.0.0.1 43211` → `[실패] 핸드셰이크: 세션 링크가
    //   닫힘` → 컨테이너 종료). 광고를 계속 띄우는 단말이 아무나 건드리면 죽는 건 말이 안 된다.
    //   대화가 끝난 뒤에도 마찬가지로 대기로 돌아간다 — 상대가 나갔다고 내가 종료할 이유는 없다.
    loop {
        // 타임아웃·끊김은 `.ok()`로 흘린다 — 종료 조건은 wait_with_quit_or가 본다.
        let got = wait_with_quit_or(
            || {
                // 발견 이벤트 드레인 — 등장 즉시 알린다(번호 = /connect 대상).
                while let Ok(ev) = discovery.try_recv() {
                    match ev {
                        nbeep_net::DiscoveryEvent::Appeared(h) => {
                            let mut p = peers.borrow_mut();
                            if !p.iter().any(|(id, _)| *id == h.peer) {
                                p.push((h.peer, h.name.as_str().to_string()));
                                println!(
                                    "\r[발견] {}. {} ({}) — `/connect {}` 로 연결",
                                    p.len(),
                                    h.name.as_str(),
                                    h.peer.short(),
                                    p.len()
                                );
                            }
                        }
                        nbeep_net::DiscoveryEvent::Vanished(id) => {
                            let mut p = peers.borrow_mut();
                            if let Some(i) = p.iter().position(|(pid, _)| *pid == id) {
                                let (_, name) = p.remove(i);
                                println!("\r[이탈] {name} — 번호가 당겨졌다(/peers로 재확인)");
                            }
                        }
                    }
                }
                incoming
                    .recv_timeout(std::time::Duration::from_millis(100))
                    .ok()
                    .map(LiveLink::In)
            },
            |line| {
                if matches!(line, "/peers" | "/list") {
                    let p = peers.borrow();
                    if p.is_empty() {
                        println!(
                            "\r[목록] 발견된 상대 없음 — 같은 LAN의 실행 중 단말(GUI·chat-live)이 뜬다"
                        );
                    } else {
                        for (i, (id, name)) in p.iter().enumerate() {
                            println!("\r[목록] {}. {} ({})", i + 1, name, id.short());
                        }
                    }
                    return None;
                }
                if let Some(t) = line.strip_prefix("/connect") {
                    let t = t.trim();
                    if t.is_empty() {
                        println!(
                            "\r[연결] 사용법: /connect <번호|host[:port]> (번호 = /peers 목록)"
                        );
                        return None;
                    }
                    // 번호 = 발견 상대(경로는 전송이 안다 — live_echo와 같은 connect) ·
                    // 그 외 = 수동 엔드포인트(DR-19 · 발견이 닿지 않는 다른 서브넷).
                    if let Ok(n) = t.parse::<usize>() {
                        let target = peers.borrow().get(n.wrapping_sub(1)).cloned();
                        let Some((id, name)) = target else {
                            println!("\r[연결] {n}번은 목록에 없다 — /peers로 확인");
                            return None;
                        };
                        return match transport.connect(id) {
                            Ok(l) => {
                                println!("\r[연결] {name} ({}) 로 연결", id.short());
                                Some(LiveLink::Out(l))
                            }
                            Err(e) => {
                                println!("\r[연결] 실패({name}): {e:?}");
                                None
                            }
                        };
                    }
                    let Some(addr) = nbeep_core::endpoint::normalize_endpoint(
                        t,
                        nbeep_net::DEFAULT_SESSION_PORT,
                    ) else {
                        println!("\r[연결] 주소 형식 오류: {t} (예: 10.0.0.5 · [fe80::1]:47200)");
                        return None;
                    };
                    return match transport.add_endpoint(&addr) {
                        Ok(l) => {
                            println!("\r[연결] {addr} 로 연결");
                            Some(LiveLink::Out(l))
                        }
                        Err(e) => {
                            println!("\r[연결] 실패({addr}): {e:?}");
                            None
                        }
                    };
                }
                println!("\r{HELP_LIVE}");
                None
            },
        );
        let Some(got) = got else {
            return; // 사용자가 /quit·Ctrl+D — 이때만 끝난다
        };
        match got {
            // 인바운드 실패는 **그 연결만** 버린다(신원 미상 — 누구였는지도 적지 않는다).
            LiveLink::In(link) => match nbeep_crypto::NoiseSession::accept(link, &identity) {
                Ok(session) => run_interactive(session, identity.peer_id()),
                Err(e) => eprintln!("[무시] 핸드셰이크 실패 — 계속 기다립니다: {e}"),
            },
            // 아웃바운드(내가 걸었음) = initiate — 이후 대화 루프는 동일.
            LiveLink::Out(link) => match nbeep_crypto::NoiseSession::initiate(link, &identity) {
                Ok(session) => run_interactive(session, identity.peer_id()),
                Err(e) => eprintln!("[실패] 핸드셰이크: {e} — 대기로 돌아갑니다"),
            },
        }
        println!("[대기] 다시 상대를 기다립니다 — {HELP_LIVE}");
    }
}
