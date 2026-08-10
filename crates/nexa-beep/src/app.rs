//! **GUI 앱 셸** — 창 생성·이벤트 루프·위젯 배선(구 `main.rs`의 `app_window` 모듈).
//!
//! 조립 지점의 창 계층. 도메인은 `nbeep-core`, 렌더는 `nbeep-ui`가 갖고 여기는
//! **winit ↔ 위젯 번역 + 창 생명주기**만 맡는다. 창 코드의 `nbeep-plat` 이관은 M3-2.

use std::collections::{HashMap, HashSet, VecDeque};
use std::num::NonZeroU32;
use std::rc::Rc;
use std::time::Instant;
use winit::application::ApplicationHandler;
use winit::event::{ElementState, MouseButton, MouseScrollDelta, WindowEvent};
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop};
use winit::keyboard::{Key as WKey, NamedKey};
use winit::window::{Window, WindowId};

use nbeep_core::PeerId;
use nbeep_ui::{
    AboutInfo, AboutWidget, ChatLine, ChatViewWidget, ComboItem, Control as _, DrawCtx,
    GalleryWidget, InputEvent, Invalidations, Key, LinkState, MenuBar, MenuDef, MenuEntry,
    PeerListWidget, PeerRow, RasterCtx, Rect, SettingsState, SettingsWidget, Theme, ToolIcon,
    ToolItem, Toolbar, Widget,
};

type SbSurface = softbuffer::Surface<Rc<Window>, Rc<Window>>;

/// 메뉴바 구성(i18n 현재 언어 기준) — 초기화·언어 전환 시 재호출.
fn build_menus() -> Vec<MenuDef> {
    use nbeep_core::{t, Msg};
    vec![
        MenuDef::new(
            t(Msg::MenuLabel),
            vec![
                MenuEntry::Item(ComboItem::new("settings", t(Msg::SettingsTitle))),
                MenuEntry::Item(ComboItem::new("quarantine", t(Msg::QuarantineTitle))),
                MenuEntry::Item(ComboItem::new("gallery", t(Msg::MenuGallery))),
            ],
        ),
        MenuDef::new(
            t(Msg::MenuHelp),
            vec![MenuEntry::Item(ComboItem::new("about", "About"))],
        ),
    ]
}

/// 창 모드(DR-26). 설정 연동(`chat.window_mode`)은 M3-11.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum WindowMode {
    /// 한 창에서 목록 ↔ 대화 전환(v1 기본).
    Single,
    /// 대화마다 상대별 OS 창(동시 대화).
    Separate,
}

/// 열린 대화의 실물 세션 스택 — Noise(암호화) 위 TOFU(신뢰) 위 다중화.
type LiveSession = nbeep_core::MuxSession<
    nbeep_core::TrustedSession<nbeep_crypto::NoiseSession<Box<dyn nbeep_core::Link>>>,
>;

/// GUI 이벤트 루프를 깨우는 커스텀 이벤트(M2-7 비동기 수신 펌프) — 세션 액터 스레드가
/// `EventLoopProxy`로 보낸다. winit 단일 스레드 규약을 지키면서 백그라운드 수신을 GUI에 반영.
#[derive(Debug)]
enum AppEvent {
    /// 상대가 보낸 대화 한 줄(복호·검증·무해화 완료).
    Recv {
        peer: PeerId,
        text: nbeep_core::SafeText,
        seq: u64,
        sender: PeerId,
    },
    /// 세션 종료(상대 이탈·오류).
    Closed { peer: PeerId },
    /// **인바운드** — 남이 나에게 연결해 핸드셰이크가 끝난 세션(아직 TOFU 미판정).
    /// GUI(메인 스레드)가 TrustStore로 판정 후 대화·창을 연다.
    Inbound { session: Box<InboundSession> },
    /// **아웃바운드 성립**(M2-8) — 워커 스레드가 connect+Noise를 마친 세션(TOFU 미판정).
    /// 연결 수립이 이벤트 루프를 막지 않게 하는 절반(인바운드와 대칭).
    Outbound { session: Box<InboundSession> },
    /// 아웃바운드 연결 실패(M2-8) — 죽은 상대를 클릭해도 UI는 살아 있고 이것만 온다.
    ConnectFailed { peer: PeerId, why: String },
    /// 파일 수신 제안 도착 — 사용자가 수락/거절할 때까지 데이터는 오지 않는다(FR-X-3).
    XferOffer {
        peer: PeerId,
        id: nbeep_core::XferId,
        name: String,
        size: u64,
    },
    /// 전송 진행률(수신·발신 공용) — 상태바 표시용.
    XferProgress {
        peer: PeerId,
        got: u64,
        total: u64,
        sending: bool,
    },
    /// 수신 완료 → **격리 보관까지 끝남**(실체화는 승인 후 별도 · FR-S-9).
    XferDone {
        peer: PeerId,
        name: String,
        risk: nbeep_core::RiskLevel,
        mismatch: bool,
    },
    /// 전송 실패·거절 — 사유 문장(표시 전용).
    XferFailed { peer: PeerId, why: String },
    /// 상대가 **수락**했다 — 대기 창을 닫고 스트리밍이 시작된다.
    XferAccepted { peer: PeerId },
    /// 발신 1건 완료 — 큐의 다음 파일로 넘어간다.
    XferSendDone { peer: PeerId },
}

/// 세션 액터에 보내는 명령 — 대화 바이트와 파일 제어를 한 채널로 교대한다.
enum SessionCmd {
    /// 인코딩된 `ChatMessage`.
    Chat(Vec<u8>),
    /// 파일 오퍼 발신(원본은 액터가 들고 있다가 상대 수락 시 청크 전송).
    OfferFile {
        id: nbeep_core::XferId,
        name: String,
        sha: [u8; 32],
        bytes: Vec<u8>,
    },
    /// 수신 제안 수락/거절(사용자 결정 — 자동 경로 없음).
    AcceptXfer {
        id: nbeep_core::XferId,
        /// 수신측 상한(B/s · 0 = 무제한) — Accept에 실어 발신측이 협상한다.
        rate_cap: u64,
    },
    /// 발신 취소(타임아웃·사용자) — 상대에게 Cancel 통지.
    CancelXfer(nbeep_core::XferId),
    RejectXfer {
        id: nbeep_core::XferId,
        why: nbeep_core::RejectWhy,
    },
}

/// 인바운드 세션 봉투 — `AppEvent`가 Debug라야 해서 수동 Debug.
struct InboundSession {
    session: nbeep_crypto::NoiseSession<Box<dyn nbeep_core::Link>>,
}
impl std::fmt::Debug for InboundSession {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("InboundSession").finish_non_exhaustive()
    }
}

/// 대화 상태 — **뷰(창)와 분리**(DR-26). 세션은 **액터 스레드**가 소유하고, 여기엔 그
/// 액터로 보내는 송신 채널과 스레드 이력만 둔다(M2-7 — 비동기 수신 펌프).
struct Conversation {
    /// 액터에 보낼 명령(대화 바이트·파일 제어). 드롭 = 액터 종료 신호.
    out_tx: std::sync::mpsc::Sender<SessionCmd>,
    lines: Vec<ChatLine>,
}

/// 세션 액터 — 세션을 전용 스레드로 옮겨 **수신(GUI로 프록시)과 송신(채널)을 교대**한다.
/// snow `TransportState`가 read/write에 `&mut`를 요구해 한 세션은 한 스레드가 소유해야
/// 하므로, 송신은 채널로 요청받는 액터 모델이 정석이다.
fn spawn_session_actor(
    mut session: LiveSession,
    out_rx: std::sync::mpsc::Receiver<SessionCmd>,
    proxy: winit::event_loop::EventLoopProxy<AppEvent>,
    send_rate: nbeep_core::RateLimit,
) {
    use nbeep_core::mux::StreamId;
    use nbeep_core::{XferInbox, XferMsg};
    let peer = session.peer();
    // 수신 폴 타임아웃 — recv가 100ms마다 TimedOut으로 돌아와 송신과 교대한다.
    session.set_recv_timeout(Some(std::time::Duration::from_millis(100)));
    std::thread::spawn(move || {
        // 파일 상태는 액터가 소유한다 — GUI 스레드는 이벤트만 받는다(대용량 조립이
        // 메인 스레드를 막지 않게).
        let mut inbox = XferInbox::new();
        let mut outgoing: HashMap<nbeep_core::XferId, Vec<u8>> = HashMap::new();
        let mut send_meter = nbeep_core::RateMeter::default();
        loop {
            // 송신 먼저(즉시성) — 대기 중 발신 요청을 모두 흘려보낸다.
            loop {
                let cmd = match out_rx.try_recv() {
                    Ok(c) => c,
                    Err(std::sync::mpsc::TryRecvError::Empty) => break,
                    Err(std::sync::mpsc::TryRecvError::Disconnected) => return, // 대화 닫힘
                };
                let sent = match cmd {
                    SessionCmd::Chat(bytes) => session.send(StreamId::Chat, &bytes),
                    SessionCmd::OfferFile {
                        id,
                        name,
                        sha,
                        bytes,
                    } => {
                        let offer = XferMsg::Offer {
                            id,
                            size: bytes.len() as u64,
                            sha256: sha,
                            name: name.into_bytes(),
                        };
                        outgoing.insert(id, bytes);
                        session.send(StreamId::File, &offer.encode())
                    }
                    SessionCmd::AcceptXfer { id, rate_cap } => {
                        if inbox.accept(&id).is_ok() {
                            session.send(StreamId::File, &XferMsg::Accept { id, rate_cap }.encode())
                        } else {
                            Ok(())
                        }
                    }
                    SessionCmd::CancelXfer(id) => {
                        outgoing.remove(&id);
                        session.send(StreamId::File, &XferMsg::Cancel { id }.encode())
                    }
                    SessionCmd::RejectXfer { id, why } => {
                        inbox.drop_xfer(&id);
                        session.send(
                            StreamId::File,
                            &XferMsg::Reject { id, why, limit: 0 }.encode(),
                        )
                    }
                };
                if sent.is_err() {
                    let _ = proxy.send_event(AppEvent::Closed { peer });
                    return;
                }
            }
            // 대화 수신 폴.
            match session.recv(StreamId::Chat) {
                Ok(bytes) => {
                    if let Ok(m) = nbeep_core::ChatMessage::decode(&bytes, peer) {
                        if let nbeep_core::MessageBody::Text(t) = m.body {
                            let ev = AppEvent::Recv {
                                peer,
                                text: nbeep_core::sanitize_message(&t),
                                seq: m.seq,
                                sender: m.sender_device,
                            };
                            if proxy.send_event(ev).is_err() {
                                return; // 이벤트 루프 종료
                            }
                        }
                    }
                }
                Err(nbeep_core::SessionError::TimedOut) => {} // 정상 — 송신 교대로
                Err(_) => {
                    let _ = proxy.send_event(AppEvent::Closed { peer });
                    return;
                }
            }
            // 파일 수신 폴.
            match session.recv(StreamId::File) {
                Ok(bytes) => {
                    if xfer_step(
                        &bytes,
                        peer,
                        &mut session,
                        &mut inbox,
                        &mut outgoing,
                        &proxy,
                        send_rate,
                        &mut send_meter,
                    )
                    .is_err()
                    {
                        let _ = proxy.send_event(AppEvent::Closed { peer });
                        return;
                    }
                }
                Err(nbeep_core::SessionError::TimedOut) => {}
                Err(_) => {
                    let _ = proxy.send_event(AppEvent::Closed { peer });
                    return;
                }
            }
        }
    });
}

/// 현재 Unix 초(표시용 벽시계 — 정책 판단은 단조 시계를 쓴다).
fn unix_now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}

/// 스레드 기록용 타임스탬프 — **밀리초 원본 + 표시용 지역 벽시계**(사용자 확정 08-10:
/// 보관은 전체 정밀도, 표시는 분 단위).
fn now_stamp() -> (u64, nbeep_ui::WallTime) {
    let ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| u64::try_from(d.as_millis()).unwrap_or(u64::MAX));
    let lt = nbeep_plat::clock::local_time(ms / 1000);
    (
        ms,
        nbeep_ui::WallTime {
            y: lt.y,
            mo: lt.mo,
            d: lt.d,
            h: lt.h,
            m: lt.m,
        },
    )
}

/// 속도 표기(B/s → 사람이 읽는 단위).
fn rate_label(bps: u64) -> String {
    if bps == 0 {
        return "무제한".into();
    }
    format!("{}/s", human_size(bps))
}

/// 사람이 읽는 크기 표기(상태바) — 1024 단위.
fn human_size(bytes: u64) -> String {
    const K: u64 = 1024;
    match bytes {
        b if b >= K * K => format!("{:.1}MiB", b as f64 / (K * K) as f64),
        b if b >= K => format!("{:.1}KiB", b as f64 / K as f64),
        b => format!("{b}B"),
    }
}

/// 파일 스트림 한 프레임 처리(액터 스레드) — 오류 = 세션 종료 신호.
#[allow(clippy::too_many_arguments)] // 액터 한 프레임의 협력자 — 묶으면 오히려 흐릿해진다
fn xfer_step(
    bytes: &[u8],
    peer: PeerId,
    session: &mut LiveSession,
    inbox: &mut nbeep_core::XferInbox,
    outgoing: &mut HashMap<nbeep_core::XferId, Vec<u8>>,
    proxy: &winit::event_loop::EventLoopProxy<AppEvent>,
    send_rate: nbeep_core::RateLimit,
    meter: &mut nbeep_core::RateMeter,
) -> Result<(), ()> {
    /// 액터 스레드의 단조 시각(ms).
    fn now_ms() -> u64 {
        use std::sync::OnceLock;
        static T0: OnceLock<std::time::Instant> = OnceLock::new();
        T0.get_or_init(std::time::Instant::now)
            .elapsed()
            .as_millis() as u64
    }
    use nbeep_core::mux::StreamId;
    use nbeep_core::{chunks_of, XferMsg};
    let fail = |why: String| {
        let _ = proxy.send_event(AppEvent::XferFailed { peer, why });
    };
    match XferMsg::decode(bytes) {
        Ok(XferMsg::Offer { id, size, name, .. }) => {
            let m = XferMsg::decode(bytes).map_err(|_| ())?;
            match inbox.offer(&m) {
                Ok(()) => {
                    let _ = proxy.send_event(AppEvent::XferOffer {
                        peer,
                        id,
                        name: String::from_utf8_lossy(&name).into_owned(),
                        size,
                    });
                }
                Err(e) => {
                    // 상한 초과 = 자동 거절 + **수신측 상한 공지**(발신자 재시도 판단 근거).
                    let msg = XferMsg::Reject {
                        id,
                        why: nbeep_core::RejectWhy::TooLarge,
                        limit: inbox.max_file(),
                    };
                    session
                        .send(StreamId::File, &msg.encode())
                        .map_err(|_| ())?;
                    fail(format!("수신 거부: {e}"));
                }
            }
        }
        Ok(XferMsg::Accept { id, rate_cap }) => {
            if let Some(data) = outgoing.remove(&id) {
                let _ = proxy.send_event(AppEvent::XferAccepted { peer });
                let total = data.len() as u64;
                // ★ 쌍방 협상 — 내 상한과 상대가 공지한 상한 중 **낮은 쪽**으로 창을 잡는다.
                let local = send_rate.target_bps(meter);
                let mut pacer = nbeep_core::Pacer::new(nbeep_core::negotiate(local, rate_cap));
                for c in chunks_of(id, &data) {
                    let nbeep_core::XferMsg::Chunk {
                        offset, ref data, ..
                    } = c
                    else {
                        continue;
                    };
                    let n = data.len() as u64;
                    // 창이 모자라면 그만큼 쉰다(대역을 통째로 점유하지 않는다).
                    let wait = pacer.take(n, now_ms());
                    if wait > 0 {
                        std::thread::sleep(std::time::Duration::from_millis(wait));
                    }
                    let done = offset + n;
                    session.send(StreamId::File, &c.encode()).map_err(|_| ())?;
                    meter.observe(n, now_ms());
                    let _ = proxy.send_event(AppEvent::XferProgress {
                        peer,
                        got: done,
                        total,
                        sending: true,
                    });
                }
                session
                    .send(StreamId::File, &XferMsg::Done { id }.encode())
                    .map_err(|_| ())?;
                let _ = proxy.send_event(AppEvent::XferSendDone { peer });
            }
        }
        Ok(XferMsg::Reject { id, why, limit }) => {
            outgoing.remove(&id);
            let extra = if limit > 0 {
                format!(" (상대 수신 상한 {}MiB)", limit / (1024 * 1024))
            } else {
                String::new()
            };
            fail(format!("상대가 거절: {why:?}{extra}"));
        }
        Ok(XferMsg::Chunk { id, offset, data }) => match inbox.chunk(&id, offset, &data) {
            Ok(()) => {
                if let Some((got, total)) = inbox.progress(&id) {
                    let _ = proxy.send_event(AppEvent::XferProgress {
                        peer,
                        got,
                        total,
                        sending: false,
                    });
                }
            }
            Err(e) => fail(format!("수신 오류: {e} — 폐기")),
        },
        Ok(XferMsg::Done { id }) => match inbox.done(&id) {
            Ok(got) => match crate::gate::quarantine_received(&got, peer, crate::gate::CH_GUI) {
                Ok(q) => {
                    let _ = proxy.send_event(AppEvent::XferDone {
                        peer,
                        name: q.name,
                        risk: q.risk,
                        mismatch: q.mismatch,
                    });
                }
                Err(e) => fail(format!("{e}")),
            },
            Err(e) => fail(format!("완료 실패: {e} — 폐기")),
        },
        Ok(XferMsg::Cancel { id }) => {
            inbox.drop_xfer(&id);
            outgoing.remove(&id);
            fail("상대가 취소".into());
        }
        Err(e) => fail(format!("와이어 오류: {e}")),
    }
    Ok(())
}

/// OS 창 하나 — 역할(목록/특정 대화) + 표면 + 창별 상태.
struct WinEntry {
    role: Role,
    window: Rc<Window>,
    surface: SbSurface,
    cursor: (i32, i32),
    scale: f32,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Role {
    /// 주 창 — 목록(단일 모드에서는 대화 전환도 이 창에서).
    Main,
    /// 상대별 대화 창(Separate 모드).
    Chat(PeerId),
    /// 설정 창(`Cmd/Ctrl+,` — DR-24).
    Settings,
    /// 컨트롤 갤러리(임시 검수 — `Cmd/Ctrl+G` 또는 하단 버튼).
    Gallery,
    /// 파일 선택 모달 창(Choose… — ChoosePicker 어댑터 내용을 별도 창으로).
    Picker,
    /// About 창(메뉴 → About — 브랜딩·링크).
    About,
    /// 격리함 — 수신 파일 승인·삭제(M4-3 · [docs/11] §7 등급별 마찰).
    Quarantine,
    /// 발신 대기 — 상대 승인을 기다리는 창(타임아웃 후 자동 취소).
    Sending(PeerId),
    /// 수신 승인 — 제안 정보를 보여 주고 결정을 받는 창(타임아웃 = 취소).
    Approve(PeerId),
}

/// 에코 봇 — 버스에 실물 신원으로 참여해 수신 세션을 받고, Chat 메시지를 에코한다.
/// 발견 힌트의 `PeerId` == Noise 정적 키(실제 아키텍처 그대로 — 인증이 발견을 검증한다).
fn spawn_echo_bot(bus: &std::sync::Arc<nbeep_net::inmem::InMemoryBus>, name: &str) {
    use nbeep_core::mux::{MuxSession, StreamId};
    use nbeep_core::{ChatMessage, DisplayName, MessageBody, Sequencer, Session as _};
    let identity = nbeep_crypto::Identity::generate();
    let display = DisplayName::parse(name).unwrap();
    let transport = bus.join(identity.peer_id(), display, nbeep_net::Caps::default());
    std::thread::spawn(move || {
        use nbeep_net::Transport as _;
        let incoming = transport.incoming();
        while let Ok(link) = incoming.recv() {
            let Ok(session) = nbeep_crypto::NoiseSession::accept(link, &identity) else {
                continue;
            };
            let user = session.peer();
            let mut mux = MuxSession::new(session);
            let mut seq = Sequencer::new();
            while let Ok(bytes) = mux.recv(StreamId::Chat) {
                let Ok(msg) = ChatMessage::decode(&bytes, user) else {
                    break; // 위조·손상 = 세션 종료(fail-closed)
                };
                let MessageBody::Text(text) = msg.body else {
                    continue;
                };
                let reply = ChatMessage {
                    sender_device: identity.peer_id(),
                    seq: seq.issue(),
                    body: MessageBody::Text(format!("에코: {text}")),
                };
                if mux.send(StreamId::Chat, &reply.encode()).is_err() {
                    break;
                }
            }
        }
    });
}

/// 인바운드 세션 수락 펌프 — 남이 나에게 연결하면 accept 후 에코 응답(대칭 대화 데모).
/// GUI 스레드로의 실시간 수신 반영(양쪽 창에 표시)은 M2-7(비동기 수신 펌프·EventLoopProxy).
/// 인바운드 수락 펌프 — 남이 나에게 연결하면 **핸드셰이크만** 하고(블로킹) 완성 세션을
/// GUI로 넘긴다. TOFU 판정·대화 생성은 메인 스레드(user_event) 몫 — TrustStore가 거기 있다.
fn spawn_inbound_accept(
    incoming: std::sync::mpsc::Receiver<Box<dyn nbeep_core::Link>>,
    identity: std::sync::Arc<nbeep_crypto::Identity>,
    proxy: winit::event_loop::EventLoopProxy<AppEvent>,
) {
    std::thread::spawn(move || {
        while let Ok(link) = incoming.recv() {
            let identity = std::sync::Arc::clone(&identity);
            let proxy = proxy.clone();
            std::thread::spawn(move || {
                // 핸드셰이크(블로킹) — 실패(자기 키 복제 U-P2 포함)면 조용히 버린다.
                let Ok(session) = nbeep_crypto::NoiseSession::accept(link, &identity) else {
                    return;
                };
                let _ = proxy.send_event(AppEvent::Inbound {
                    session: Box::new(InboundSession { session }),
                });
            });
        }
    });
}

/// 샘플 찾기 어댑터 — 한 폴더의 **단일 파일 선택기**(Adapter 패턴 실증).
/// `nbeep_ui::ChoosePicker`를 구현한 어떤 화면도 Choose에 꽂을 수 있다(UI 계층은 I/O를 모른다).
#[derive(Debug)]
struct FilePicker {
    dir: std::path::PathBuf,
}

impl nbeep_ui::ChoosePicker for FilePicker {
    fn title(&self) -> String {
        format!("파일 선택 — {}", self.dir.display())
    }
    fn items(&self) -> Vec<nbeep_ui::ComboItem> {
        // 투명 배경 이미지 아이콘(파일 · 공유 Rc).
        let icon = std::rc::Rc::new(nbeep_ui::IconImage::swatch(16, (0x8A, 0x91, 0x9C)));
        let mut v = Vec::new();
        if let Ok(rd) = std::fs::read_dir(&self.dir) {
            for e in rd.flatten() {
                if e.file_type().map(|t| t.is_file()).unwrap_or(false) {
                    let name = e.file_name().to_string_lossy().into_owned();
                    v.push(nbeep_ui::ComboItem::new(name.clone(), name).with_image(icon.clone()));
                }
            }
        }
        v.sort_by(|a, b| a.label.cmp(&b.label));
        v
    }
}

struct App {
    mode: WindowMode,
    windows: HashMap<WindowId, WinEntry>,
    main_id: Option<WindowId>,
    font: nbeep_gfx::Font,
    theme: Theme,
    list: PeerListWidget,
    /// 대화 뷰 — Separate 모드는 창당 1개, Single 모드는 주 창에 최대 1개.
    chats: HashMap<PeerId, ChatViewWidget>,
    /// Single 모드에서 주 창에 표시 중인 대화(None = 목록).
    single_open: Option<PeerId>,
    /// 내 신원 — 발견·세션·발신 봉투가 전부 이 키 하나에서 나온다.
    identity: std::sync::Arc<nbeep_crypto::Identity>,
    seq: nbeep_core::Sequencer,
    /// 전송 — 데모(InMemory) 또는 실물(`LocalDirect`). 같은 `Transport` 트레이트라 App은 구별 안 함.
    /// `Arc` = 연결 수립 워커(M2-8)와 공유 — 죽은 상대 연결이 UI 스레드를 막지 않는다.
    transport: std::sync::Arc<dyn nbeep_net::Transport + Send + Sync>,
    /// 연결 수립 중인 상대(M2-8 — 중복 클릭 가드).
    connecting: HashSet<PeerId>,
    discovery: std::sync::mpsc::Receiver<nbeep_net::DiscoveryEvent>,
    table: nbeep_core::PeerTable,
    trust: nbeep_core::MemoryTrustStore,
    /// 상대별 대화 상태 — 뷰와 무관하게 유지(동시 대화의 실체).
    conversations: HashMap<PeerId, Conversation>,
    dedup: nbeep_core::DedupIndex,
    started: Instant,
    /// 주 창 하단 상태바 문구.
    status: String,
    /// 설정 값(런타임 — 영속은 M2-5). `chat.window_mode`·`ui.theme`·`font.*`.
    settings: SettingsState,
    /// 영역별 글꼴 설정(설정에서 파생 — 크기·굵기·기울임).
    fonts: nbeep_ui::FontPrefs,
    /// 열린 설정 창의 뷰(설정 창은 항상 별도 OS 창 1개).
    settings_view: Option<SettingsWidget>,
    /// 컨트롤 갤러리 뷰(임시 검수) — 열려 있을 때만 Some.
    gallery_view: Option<GalleryWidget>,
    /// 앱 창 아이콘(브랜딩 · 전 창 공통). 지원 안 되면 None.
    icon: Option<winit::window::Icon>,
    /// 파일 선택 모달 뷰(Choose… — 열려 있을 때만 Some).
    picker_view: Option<nbeep_ui::TreeView>,
    /// About 뷰(열려 있을 때만 Some).
    about_view: Option<AboutWidget>,
    quarantine_view: Option<nbeep_ui::QuarantineWidget>,
    /// 상대별 진행 중 전송(목록 막대·대화창 진척 줄 공용).
    xfer_progress: HashMap<PeerId, nbeep_ui::XferProgress>,
    /// 상대별 대화 왕래 장부 — 파일 전송 자격(상호 확인)의 근거.
    ledger: nbeep_core::ExchangeLedger,
    /// 전역 승인 정책(설정 `xfer.approval` · 기간 만료 시 자동 복귀).
    approval: nbeep_core::ApprovalPolicy,
    /// 기간 자동 승인 길이(설정 `xfer.approval_window`).
    approval_window: nbeep_core::AutoWindow,
    /// 기간 자동 승인 시작 시각(Unix 초 · 표시용).
    approval_started_unix: Option<u64>,
    /// 하단 정보를 마지막으로 갱신한 초(1초 주기 판단).
    approval_footer_sec: u64,
    /// 대기 시간(초 · 설정 `xfer.timeout_sec`) — 승인 창·발신 대기 공용.
    wait_timeout_sec: u64,
    /// 상대별 승인 화면(열려 있는 동안 유지).
    approve_view: HashMap<PeerId, nbeep_ui::OfferPromptWidget>,
    /// 다음 루프에서 만들 승인 창.
    pending_approve_window: Option<PeerId>,
    /// 슬롯별 얼굴(설정 글꼴명으로 로드 · 없으면 기본 폰트).
    face_peerlist: Option<nbeep_gfx::Font>,
    face_message: Option<nbeep_gfx::Font>,
    face_status: Option<nbeep_gfx::Font>,
    /// 고정폭 얼굴 — 지정 없으면 OS 기본.
    face_mono: Option<nbeep_gfx::Font>,
    /// 상대별 수락 대기 큐 — **오퍼 1건당 승인 1번**(2번 보내면 2번 물어본다).
    pending_offers: HashMap<PeerId, VecDeque<(nbeep_core::XferId, String, u64)>>,
    /// 상대별 발신 대기 파일 큐(다중 드롭 — 한 번에 하나씩 협상한다).
    send_queue: HashMap<PeerId, VecDeque<std::path::PathBuf>>,
    /// 상대별 배치 집계 (보낸 파일 수, 총 파일 수, 보낸 바이트, 총 바이트).
    send_batch: HashMap<PeerId, (u32, u32, u64, u64)>,
    /// 발신 협상 대기 중인가(수락 전) — 큐 진행 판단.
    awaiting_accept: HashMap<PeerId, nbeep_core::XferId>,
    /// 발신 대기 창의 타임아웃 버튼(상대별).
    send_wait: HashMap<PeerId, nbeep_ui::TimeoutButton>,
    /// 다음 루프에서 만들 발신 대기 창(창 생성은 ActiveEventLoop가 필요하다).
    pending_send_window: Option<PeerId>,
    /// 보내기 속도 상한(설정 `xfer.send_rate`).
    send_rate: nbeep_core::RateLimit,
    /// 받기 속도 상한(설정 `xfer.recv_rate`) — Accept로 상대에게 공지된다.
    recv_rate: nbeep_core::RateLimit,
    /// 처리량 계측 — 자동 모드의 근거(관측 최고치의 50%).
    send_meter: nbeep_core::RateMeter,
    recv_meter: nbeep_core::RateMeter,
    /// 주 창 상단 Pull-down 메뉴(목록 모드 전용).
    menu: MenuBar,
    /// 주 창 툴바(메뉴 아래 · 이미지 버튼 · 목록 모드 전용).
    toolbar: Toolbar,
    /// 세션이 끊긴 상대(AppEvent::Closed) — 목록 상태 점 Lost 근거. 재수립 시 제거.
    closed_peers: std::collections::HashSet<PeerId>,
    /// IME 조합 중(macOS — Preedit 활성). 조합 중엔 KeyboardInput 문자/백스페이스를
    /// 라우팅하지 않는다(같은 키가 Ime 경로로도 와서 자모가 이중 유입되던 버그).
    ime_composing: bool,
    /// 판정 보류 중인 단독 자모(창, 문자, 시각). Character("ㄱ")는 ①곧 Preedit가 따라오는
    /// **중복**이거나 ②IME가 아직 안 붙은 **진짜 입력**(한영 전환 직후 첫 키)이다 —
    /// 즉시 버리면 ②가 유실되므로 보류했다가 Ime 이벤트가 오면 폐기, 안 오면 라우팅한다.
    pending_jamo: Option<(WindowId, char, u64)>,
    /// OS 주 수식키(⌘/Ctrl) 눌림 상태 — `Cmd/Ctrl+,` 판정.
    primary_down: bool,
    /// Shift 눌림 상태 — Shift+Enter 줄바꿈·Shift+이동 선택(08-10).
    shift_down: bool,
    /// 세션 액터가 GUI를 깨우는 통로(M2-7).
    proxy: winit::event_loop::EventLoopProxy<AppEvent>,
    /// 수동 엔드포인트 입력 중 버퍼(DR-19 · `⌘/Ctrl+K`). None = 입력 아님.
    adding: Option<String>,
    /// 종료 신호(R-16 · FR-P-7) — SIGINT/SIGTERM 시 el.exit() → Drop 체인이 GOODBYE·정리.
    shutdown: nbeep_plat::shutdown::Shutdown,
}

impl App {
    fn now_ms(&self) -> u64 {
        u64::try_from(self.started.elapsed().as_millis()).unwrap_or(u64::MAX)
    }

    /// 보류 자모 방출 — Ime 이벤트가 따라오지 않았다 = 진짜 단독 입력이었다.
    fn flush_pending_jamo(&mut self, el: &ActiveEventLoop) {
        if let Some((id, c, _)) = self.pending_jamo.take() {
            let now_ms = self.now_ms();
            self.route(id, InputEvent::Char { c, now_ms }, el);
        }
    }

    /// 목록 모드 상단 크롬(메뉴+툴바) 높이(물리 px). 대화 중엔 0.
    fn chrome_h(&self, scale: f32) -> i32 {
        if self.single_open.is_some() {
            return 0;
        }
        let menu_h = (30.0 * scale).round() as i32;
        let tb_h = (self.toolbar.preferred_height() as f32 * scale).round() as i32;
        menu_h + tb_h
    }

    /// About 창을 연다(메뉴 → About).
    fn open_about(&mut self, el: &ActiveEventLoop) {
        if let Some((aid, _)) = self.windows.iter().find(|(_, e)| e.role == Role::About) {
            if let Some(e) = self.windows.get(aid) {
                e.window.focus_window();
            }
            return;
        }
        let attrs = Window::default_attributes()
            .with_title("Nexa Beep — About")
            .with_inner_size(winit::dpi::LogicalSize::new(420.0, 520.0))
            .with_resizable(false) // 모달 대화상자 — 크기 고정
            .with_window_icon(self.icon.clone());
        let window = Rc::new(el.create_window(attrs).unwrap());
        let scale = window.scale_factor() as f32;
        let context = softbuffer::Context::new(window.clone()).unwrap();
        let surface = SbSurface::new(&context, window.clone()).unwrap();
        let id = window.id();
        self.windows.insert(
            id,
            WinEntry {
                role: Role::About,
                window,
                surface,
                cursor: (0, 0),
                scale,
            },
        );
        self.about_view = Some(AboutWidget::new(AboutInfo {
            app: "Nexa Beep".into(),
            version: env!("CARGO_PKG_VERSION").into(),
            tagline: "제로 컨피그 로컬 네트워크 메신저 · Zero-config LAN messenger".into(),
            links: vec![
                ("SosomLab".into(), "https://sosomlab.com".into()),
                (
                    "Nexa Beep 홈페이지".into(),
                    "https://sosomlab.com/apps/nexa-beep/".into(),
                ),
                (
                    "GitHub".into(),
                    "https://github.com/SosomLab/nexa-beep".into(),
                ),
            ],
        }));
        self.layout_window(id);
        self.request_redraw(id);
    }

    /// 주 창 IME 토글 — 목록(직접 조합) = off · 대화(실제 텍스트) = on.
    fn set_main_ime(&self, on: bool) {
        if let Some(mid) = self.main_id {
            if let Some(e) = self.windows.get(&mid) {
                e.window.set_ime_allowed(on);
            }
        }
    }

    fn bar_h(scale: f32) -> i32 {
        (26.0 * scale).round() as i32
    }

    fn request_redraw(&self, id: WindowId) {
        if let Some(e) = self.windows.get(&id) {
            e.window.request_redraw();
        }
    }

    /// 이 창에 현재 열린 대화 상대(있으면) — IME 프리에딧 라우팅용.
    fn chat_peer_for(&self, id: WindowId) -> Option<PeerId> {
        match self.windows.get(&id).map(|e| e.role) {
            Some(Role::Chat(peer)) => Some(peer),
            Some(Role::Main) => self.single_open,
            _ => None,
        }
    }

    /// 드롭된 파일을 **큐에 넣는다**(다중 드롭 = 여러 번 호출된다 · winit는 파일마다
    /// 이벤트를 준다). 협상은 한 번에 하나씩 — 승인도 파일마다 받아야 하기 때문이다.
    fn offer_file(&mut self, id: WindowId, path: &std::path::Path) {
        let Some(peer) = self.chat_peer_for(id) else {
            self.status = "파일 전송은 대화를 연 뒤에 — 상대를 먼저 선택하세요".into();
            self.request_redraw(id);
            return;
        };
        // 사전 점검 — 막혀 있어도 시도는 가능하지만, 미리 알려 주면 헛되이 기다리지 않는다.
        {
            use nbeep_core::TrustStore as _;
            if let Err(reason) =
                nbeep_core::check_send_eligibility(self.trust.level(peer), self.ledger.get(peer))
            {
                self.status = format!("보낼 수 없습니다 — {}", reason.message());
                self.request_redraw(id);
                return;
            }
        }
        let size = std::fs::metadata(path).map(|m| m.len()).unwrap_or(0);
        self.send_queue
            .entry(peer)
            .or_default()
            .push_back(path.to_path_buf());
        let b = self.send_batch.entry(peer).or_insert((0, 0, 0, 0));
        b.1 += 1;
        b.3 += size;
        self.status = format!(
            "전송 대기 {}개 · 총 {}",
            self.send_queue.get(&peer).map_or(0, VecDeque::len),
            human_size(b.3)
        );
        self.pump_send_queue(peer);
        self.request_redraw(id);
    }

    /// 큐에서 다음 파일을 꺼내 오퍼를 보낸다(협상 중이면 대기).
    fn pump_send_queue(&mut self, peer: PeerId) {
        if self.awaiting_accept.contains_key(&peer) {
            return; // 앞 파일 협상 중
        }
        let Some(path) = self.send_queue.get_mut(&peer).and_then(VecDeque::pop_front) else {
            // 큐가 비었다 — 배치 종료.
            self.send_batch.remove(&peer);
            return;
        };
        let bytes = match std::fs::read(&path) {
            Ok(b) => b,
            Err(e) => {
                self.status = format!("파일 읽기 실패: {e}");
                self.pump_send_queue(peer);
                return;
            }
        };
        let name = path
            .file_name()
            .map_or_else(|| "file".to_string(), |n| n.to_string_lossy().into_owned());
        // 전송 id — 새 키의 앞 16B(세션 내 유일하면 충분한 간이 난수).
        let mut xid = [0u8; 16];
        xid.copy_from_slice(&nbeep_crypto::Identity::generate().peer_id().as_bytes()[..16]);
        let sha = nbeep_crypto::sha256(&bytes);
        let size = bytes.len() as u64;
        let sent = self.conversations.get(&peer).is_some_and(|c| {
            c.out_tx
                .send(SessionCmd::OfferFile {
                    id: xid,
                    name: name.clone(),
                    sha,
                    bytes,
                })
                .is_ok()
        });
        if !sent {
            self.status = "세션이 끊겨 전송할 수 없습니다".into();
            self.send_queue.remove(&peer);
            self.send_batch.remove(&peer);
            return;
        }
        self.awaiting_accept.insert(peer, xid);
        self.status = format!("파일 제안: {name} ({}) — 상대 승인 대기", human_size(size));
        // 스레드에 송신 항목 추가(승인 대기 → 진행 → 완료가 이 항목 위에서 갱신된다).
        self.push_xfer_line(peer, true, &name, size);
        self.open_send_wait(peer, &name);
    }

    /// 발신 대기 창 — **60초 타임아웃 버튼**이 자동으로 눌려 창을 닫고 전송을 취소한다.
    fn open_send_wait(&mut self, peer: PeerId, name: &str) {
        let ms = self.wait_timeout_sec.saturating_mul(1000);
        let mut tb = nbeep_ui::TimeoutButton::new(format!("전송 취소 — {name}"), ms);
        tb.start(self.now_ms());
        self.send_wait.insert(peer, tb);
        self.pending_send_window = Some(peer); // 다음 about_to_wait에서 창을 만든다
    }

    /// 발신 대기 종료 — 창을 닫고 상태를 정리한다.
    fn close_send_wait(&mut self, peer: PeerId) {
        self.send_wait.remove(&peer);
        if let Some((wid, _)) = self
            .windows
            .iter()
            .find(|(_, e)| e.role == Role::Sending(peer))
        {
            let wid = *wid;
            self.windows.remove(&wid);
        }
    }

    /// 발신 취소(타임아웃·사용자) — 상대에게 Cancel을 보내고 큐를 비운다.
    fn cancel_send(&mut self, peer: PeerId, by_timeout: bool) {
        if let Some(xid) = self.awaiting_accept.remove(&peer) {
            if let Some(c) = self.conversations.get(&peer) {
                let _ = c.out_tx.send(SessionCmd::CancelXfer(xid));
            }
        }
        self.set_xfer_line(
            peer,
            true,
            nbeep_ui::XferLineState::Failed {
                why: if by_timeout {
                    "응답 없음 — 시간 초과로 취소".into()
                } else {
                    "취소함".into()
                },
            },
        );
        self.send_queue.remove(&peer);
        self.send_batch.remove(&peer);
        self.close_send_wait(peer);
        self.clear_xfer(peer);
        self.status = if by_timeout {
            format!(
                "{}초 동안 응답이 없어 전송을 취소했습니다",
                self.wait_timeout_sec
            )
        } else {
            "전송을 취소했습니다".into()
        };
        if let Some(mid) = self.main_id {
            self.request_redraw(mid);
        }
    }

    /// 수신 제안에 답한다(⌘/Ctrl+Y 수락 · ⌘/Ctrl+N 거절) — **사용자 명시 결정만**(FR-S-9).
    /// 큐에서 **한 건씩** 꺼낸다 — 2번 보냈으면 2번 눌러야 한다(사용자 확정 08-09).
    fn answer_offer(&mut self, id: WindowId, accept: bool) {
        let Some(peer) = self.chat_peer_for(id) else {
            return;
        };
        let Some((xid, name, _)) = self
            .pending_offers
            .get_mut(&peer)
            .and_then(VecDeque::pop_front)
        else {
            self.status = "수락 대기 중인 파일 제안이 없습니다".into();
            self.request_redraw(id);
            return;
        };
        let ok = self.send_xfer_decision(peer, xid, accept, nbeep_core::RejectWhy::Declined);
        if !accept {
            self.set_xfer_line(
                peer,
                false,
                nbeep_ui::XferLineState::Failed {
                    why: "거절함".into(),
                },
            );
        }
        let left = self.pending_offers.get(&peer).map_or(0, VecDeque::len);
        self.status = if ok {
            let head = if accept {
                format!("수락 — {name} 수신 시작")
            } else {
                format!("거절 — {name}")
            };
            if left > 0 {
                format!("{head} · 대기 중인 제안 {left}건 더 있음")
            } else {
                head
            }
        } else {
            "세션이 끊겨 응답하지 못했습니다".into()
        };
        self.request_redraw(id);
    }

    /// 대기열 맨 앞 제안으로 승인 화면 내용을 만든다(없으면 `None`).
    fn front_offer_info(&self, peer: PeerId) -> Option<nbeep_ui::OfferInfo> {
        let q = self.pending_offers.get(&peer)?;
        let (_, name, size) = q.front()?;
        let sender = self
            .table
            .list()
            .into_iter()
            .find(|e| e.peer == peer)
            .map_or_else(
                || peer.short(),
                |e| format!("{} ({})", e.name.as_str(), peer.short()),
            );
        Some(nbeep_ui::OfferInfo {
            sender,
            when: nbeep_plat::clock::local_hms(unix_now()).hms(),
            name: name.clone(),
            size: *size,
            queued: q.len(),
        })
    }

    /// 승인 화면 갱신 — 대기열이 비면 창을 닫는다.
    fn refresh_approve_view(&mut self, peer: PeerId) {
        let Some(info) = self.front_offer_info(peer) else {
            self.close_approve(peer);
            return;
        };
        let now = self.now_ms();
        let secs = self.wait_timeout_sec;
        let w = self
            .approve_view
            .entry(peer)
            .or_insert_with(|| nbeep_ui::OfferPromptWidget::new(info.clone(), secs));
        *w = nbeep_ui::OfferPromptWidget::new(info, secs); // 다음 건 = 시간도 새로 센다
        w.start(now);
        if let Some((wid, _)) = self
            .windows
            .iter()
            .find(|(_, e)| e.role == Role::Approve(peer))
        {
            let wid = *wid;
            self.layout_window(wid);
            self.request_redraw(wid);
        }
    }

    /// 승인 창 닫기.
    fn close_approve(&mut self, peer: PeerId) {
        self.approve_view.remove(&peer);
        if let Some((wid, _)) = self
            .windows
            .iter()
            .find(|(_, e)| e.role == Role::Approve(peer))
        {
            let wid = *wid;
            self.windows.remove(&wid);
        }
    }

    /// 승인 화면의 결정을 실행한다.
    fn run_offer_choice(&mut self, peer: PeerId, choice: nbeep_ui::OfferChoice) {
        use nbeep_ui::OfferChoice;
        let front = self
            .pending_offers
            .get_mut(&peer)
            .and_then(VecDeque::pop_front);
        let Some((xid, name, _)) = front else {
            self.close_approve(peer);
            return;
        };
        match choice {
            OfferChoice::Approve => {
                self.send_xfer_decision(peer, xid, true, nbeep_core::RejectWhy::Declined);
                self.status = format!("수락 — {name} 수신 시작");
            }
            OfferChoice::Cancel { by_timeout } => {
                self.send_xfer_decision(peer, xid, false, nbeep_core::RejectWhy::Declined);
                self.set_xfer_line(
                    peer,
                    false,
                    nbeep_ui::XferLineState::Failed {
                        why: if by_timeout {
                            "응답 없음 — 시간 초과로 거절".into()
                        } else {
                            "거절함".into()
                        },
                    },
                );
                self.status = if by_timeout {
                    format!(
                        "{}초 동안 응답이 없어 거절했습니다 — {name}",
                        self.wait_timeout_sec
                    )
                } else {
                    format!("거절 — {name}")
                };
            }
            OfferChoice::AutoFor(code) => {
                // 설정 화면까지 가지 않고 여기서 기간 자동 수락을 켠다(설정 값도 함께 맞춘다).
                if let Some(w) = nbeep_core::AutoWindow::from_code(code) {
                    let now = self.now_ms();
                    self.approval_window = w;
                    self.approval = self.approval.start_timed(w, now);
                    self.approval_started_unix = Some(unix_now());
                    self.settings.set("xfer.approval", "timed".to_string());
                    self.settings.set("xfer.approval_window", code.to_string());
                    if let Some(sv) = &mut self.settings_view {
                        let mut inv = Invalidations::default();
                        sv.set_value("xfer.approval", "timed", &mut inv);
                        sv.set_value("xfer.approval_window", code, &mut inv);
                    }
                    self.refresh_approval_ui();
                }
                self.send_xfer_decision(peer, xid, true, nbeep_core::RejectWhy::Declined);
                self.status = format!("자동 수락 시작 — {name} 포함 이후 제안을 자동 수락합니다");
            }
        }
        // 다음 제안이 있으면 이어서 묻는다(오퍼 1건당 승인 1번).
        self.refresh_approve_view(peer);
        if let Some(mid) = self.main_id {
            self.request_redraw(mid);
        }
    }

    /// 액터에 수락/거절 명령을 보낸다(성공 여부).
    fn send_xfer_decision(
        &mut self,
        peer: PeerId,
        xid: nbeep_core::XferId,
        accept: bool,
        why: nbeep_core::RejectWhy,
    ) -> bool {
        let cmd = if accept {
            // 수신 상한을 함께 알린다 — 발신측이 **둘 중 낮은 쪽**으로 맞춘다.
            let rate_cap = self.recv_rate.target_bps(&self.recv_meter);
            SessionCmd::AcceptXfer { id: xid, rate_cap }
        } else {
            SessionCmd::RejectXfer { id: xid, why }
        };
        self.conversations
            .get(&peer)
            .is_some_and(|c| c.out_tx.send(cmd).is_ok())
    }

    /// 진행률을 위젯에 반영(목록 행 + 열려 있는 대화창).
    fn apply_xfer_view(&mut self, peer: PeerId) {
        let xp = self.xfer_progress.get(&peer).copied();
        let mut inv = Invalidations::default();
        if let Some(chat) = self.chats.get_mut(&peer) {
            chat.set_xfer(xp, &mut inv);
        }
        self.refresh_rows(&mut inv);
    }

    /// 전송 종료 — 진행률 정리 + 목록 갱신.
    fn clear_xfer(&mut self, peer: PeerId) {
        self.xfer_progress.remove(&peer);
        self.apply_xfer_view(peer);
        if let Some(mid) = self.main_id {
            self.request_redraw(mid);
        }
    }

    /// 파일 전송 항목을 대화 스레드에 추가 — **저장소+뷰 동시**(DR-26). 완료 후에도
    /// 기록으로 남고, 뷰를 닫았다 열어도 복원된다(사용자 요청 08-10).
    fn push_xfer_line(&mut self, peer: PeerId, mine: bool, name: &str, size: u64) {
        // 파일명은 원격 제공 값일 수 있다 — 스레드 표시 전 무해화(RLO 등).
        let (at_ms, wall) = now_stamp();
        let line = ChatLine::xfer(mine, nbeep_core::sanitize_message(name), size, at_ms, wall);
        if let Some(conv) = self.conversations.get_mut(&peer) {
            conv.lines.push(line.clone());
        }
        let mut inv = Invalidations::default();
        if let Some(chat) = self.chats.get_mut(&peer) {
            chat.push_line(line, &mut inv);
        }
        self.redraw_conversation(peer);
    }

    /// 진행 중 전송 항목(방향 일치·마지막 미종결)의 상태 갱신 — 저장소+뷰 동시.
    fn set_xfer_line(&mut self, peer: PeerId, mine: bool, state: nbeep_ui::XferLineState) {
        if let Some(conv) = self.conversations.get_mut(&peer) {
            nbeep_ui::update_xfer_in(&mut conv.lines, mine, state.clone());
        }
        let mut inv = Invalidations::default();
        if let Some(chat) = self.chats.get_mut(&peer) {
            chat.update_xfer_line(mine, state, &mut inv);
        }
        self.redraw_conversation(peer);
    }

    /// 승인 정책 만료 확인 — 기간이 끝났으면 **직전 방식으로 되돌리고** 알린다.
    fn tick_approval(&mut self) -> bool {
        use nbeep_core::{ApprovalPolicy, BasicApproval};
        let now = self.now_ms();
        let (next, reverted) = self.approval.tick(now);
        if reverted {
            self.approval = next;
            self.approval_started_unix = None;
            // 설정 화면의 값도 함께 되돌린다 — 화면과 실제가 어긋나면 안 된다.
            let code = match next {
                ApprovalPolicy::Basic(BasicApproval::Auto) => "auto",
                ApprovalPolicy::Basic(BasicApproval::Block) => "block",
                _ => "manual",
            };
            self.settings.set("xfer.approval", code.to_string());
            if let Some(sv) = &mut self.settings_view {
                let mut inv = Invalidations::default();
                sv.set_value("xfer.approval", code, &mut inv);
            }
            self.status = "자동 수락 기간이 끝나 직전 방식으로 되돌렸습니다".into();
            self.refresh_approval_ui();
        }
        reverted
    }

    /// 설정의 글꼴명으로 슬롯 얼굴을 다시 로드한다(빈 값 = 기본 폰트 사용).
    fn reload_faces(&mut self) {
        let load = |name: &str| -> Option<nbeep_gfx::Font> {
            if name.trim().is_empty() {
                return None;
            }
            let (bytes, idx) = nbeep_plat::font::find_font_by_family(name)?;
            nbeep_gfx::Font::from_static(bytes, idx).ok()
        };
        self.face_peerlist = load(self.settings.get("font.peerlist.family"));
        self.face_message = load(self.settings.get("font.message.family"));
        self.face_status = load(self.settings.get("font.status.family"));
        // 고정폭: 지정이 있으면 그것, 없으면 **OS 기본 고정폭**(사용자 확정 08-09).
        self.face_mono = load(self.settings.get("font.mono.family")).or_else(|| {
            let (bytes, idx) = nbeep_plat::font::system_mono_font()?;
            nbeep_gfx::Font::from_static(bytes, idx).ok()
        });
    }

    /// 설정 화면의 **잠금·하단 정보**를 현재 승인 정책에 맞춘다(1초 주기 갱신).
    fn refresh_approval_ui(&mut self) {
        let now = self.now_ms();
        let remain = self.approval.remaining_ms(now);
        let started = self.approval_started_unix;
        let Some(sv) = &mut self.settings_view else {
            return;
        };
        let mut inv = Invalidations::default();
        // 네 값을 **한 줄·고정 자리**로 보여 준다. 쓰이지 않을 때도 자리를 지키고
        // 00:00:00을 표시한다 — 줄이 생겼다 사라지면 아래 항목이 출렁인다.
        let (start_s, elapsed_s, remain_s, end_s) =
            if let (Some(remain_ms), Some(start_unix)) = (remain, started) {
                let elapsed = unix_now().saturating_sub(start_unix);
                let remain_secs = remain_ms / 1000;
                let end_unix = start_unix + elapsed + remain_secs;
                let st = nbeep_plat::clock::local_hms(start_unix);
                let en = nbeep_plat::clock::local_hms(end_unix);
                (
                    st.hms(),
                    nbeep_plat::clock::clock_hms(elapsed),
                    nbeep_plat::clock::clock_hms(remain_secs),
                    en.hms(),
                )
            } else {
                let z = || "00:00:00".to_string();
                (z(), z(), z(), z())
            };
        sv.set_row_note(
            "xfer.approval_window",
            &format!("시작 {start_s}, 경과 {elapsed_s}, 잔여 {remain_s}, 종료 {end_s}"),
            &mut inv,
        );
        if remain.is_some() {
            sv.set_disabled(&[], &mut inv);
        } else {
            // 기간 자동이 아니면 기간 설정은 쓰이지 않는다 — 잠근다.
            sv.set_disabled(&["xfer.approval_window"], &mut inv);
        }
    }

    /// `peer` 대화가 보이는 창을 다시 그린다(Separate = 그 창, Single = 주 창이 이 대화일 때).
    fn redraw_conversation(&self, peer: PeerId) {
        match self.mode {
            WindowMode::Separate => {
                if let Some((id, _)) = self
                    .windows
                    .iter()
                    .find(|(_, e)| e.role == Role::Chat(peer))
                {
                    self.request_redraw(*id);
                }
            }
            WindowMode::Single => {
                if self.single_open == Some(peer) {
                    if let Some(id) = self.main_id {
                        self.request_redraw(id);
                    }
                }
            }
        }
    }

    /// 발견 이벤트를 PeerTable에 접고 목록을 다시 만든다.
    fn poll_discovery(&mut self) {
        use nbeep_net::DiscoveryEvent;
        let mut changed = false;
        let now = nbeep_core::MonoInstant(
            u64::try_from(self.started.elapsed().as_nanos()).unwrap_or(u64::MAX),
        );
        while let Ok(ev) = self.discovery.try_recv() {
            match ev {
                DiscoveryEvent::Appeared(hint) => {
                    self.table
                        .observe(hint.peer, hint.name, nbeep_core::SourceId(0), now);
                }
                DiscoveryEvent::Vanished(peer) => {
                    self.table.goodbye(peer, nbeep_core::SourceId(0));
                }
            }
            changed = true;
        }
        if changed {
            let mut inv = Invalidations::default();
            self.refresh_rows(&mut inv);
            if let Some(id) = self.main_id {
                self.request_redraw(id);
            }
        }
    }

    /// 목록 행 재구성 — 발견(PeerTable) + 신뢰(TrustStore)의 조립.
    fn refresh_rows(&mut self, inv: &mut Invalidations) {
        use nbeep_core::TrustStore as _;
        let rows = self
            .table
            .list()
            .into_iter()
            .map(|entry| {
                let trust = self.trust.level(entry.peer);
                // 세션 상태 점(사용자 요청): 대화 중=Active · 끊김 기록=Lost · 그 외=Idle.
                let link = if self.conversations.contains_key(&entry.peer) {
                    LinkState::Active
                } else if self.closed_peers.contains(&entry.peer) {
                    LinkState::Lost
                } else {
                    LinkState::Idle
                };
                // 진행 중 전송이 있으면 이름 아래 막대로 보인다(슬라이스 4에서 채운다).
                let xfer = self.xfer_progress.get(&entry.peer).copied();
                PeerRow {
                    entry,
                    trust,
                    link,
                    xfer,
                }
            })
            .collect();
        self.list.set_rows(rows, inv);
    }

    /// 팔레트 + 사용자 색 오버라이드(설정 `theme.{dark|light}.*`)로 테마 재구성(08-10).
    /// 다크/라이트 **각각** 자기 오버라이드를 갖는다 — 전환해도 상대 테마 색은 안 섞인다.
    fn rebuild_theme(&mut self) {
        let light = self.settings.get("ui.theme") == "light";
        let mut t = if light { Theme::light() } else { Theme::dark() };
        let prefix = if light { "theme.light" } else { "theme.dark" };
        let ov = |settings: &SettingsState, field: &mut nbeep_ui::Color, key: &str| {
            if let Some(c) =
                nbeep_ui::theme::color_from_hex(settings.get(&format!("{prefix}.{key}")))
            {
                *field = c;
            }
        };
        ov(&self.settings, &mut t.accent, "accent");
        ov(&self.settings, &mut t.bubble_peer, "bubble_peer");
        ov(&self.settings, &mut t.panel_bg, "panel_bg");
        ov(&self.settings, &mut t.text, "text");
        self.theme = t;
    }

    /// 연결 수립을 **워커 스레드**로 시작한다(M2-8 — 사용자 실기 08-10 "응답 없음").
    ///
    /// connect(후보 순차 시도 — 최악 수십 초)+Noise 핸드셰이크가 이벤트 루프에서 돌면
    /// 죽은 상대(강제 종료 → GOODBYE 없이 잔존)를 클릭하는 순간 GUI 전체가 멈춘다.
    /// 인바운드와 대칭으로 워커가 세션을 만들어 `AppEvent::Outbound`로 돌아오고,
    /// **TOFU 판정은 지금처럼 메인 스레드**(TrustStore가 여기 있다).
    fn start_connect(&mut self, peer: PeerId) {
        if !self.connecting.insert(peer) {
            self.status = format!("연결 중… {} (이미 시도 중)", self.peer_title(peer));
            return; // 중복 클릭 가드
        }
        self.status = format!("연결 중… {}", self.peer_title(peer));
        let transport = std::sync::Arc::clone(&self.transport);
        let identity = std::sync::Arc::clone(&self.identity);
        let proxy = self.proxy.clone();
        std::thread::spawn(move || {
            let r = transport
                .connect(peer)
                .map_err(|e| format!("{e:?}"))
                .and_then(|link| {
                    nbeep_crypto::NoiseSession::initiate(link, &identity).map_err(|e| e.to_string())
                });
            let _ = match r {
                Ok(session) => proxy.send_event(AppEvent::Outbound {
                    session: Box::new(InboundSession { session }),
                }),
                Err(why) => proxy.send_event(AppEvent::ConnectFailed { peer, why }),
            };
        });
    }

    /// 수동 주소로 세션 수립(DR-19) — add_endpoint(발견 우회)→Noise→TOFU→대화 등록.
    /// 반환 = 확정된 `PeerId`(주소는 힌트·신원은 지문). ⚠️ 핸드셰이크는 블로킹(LAN 수십 ms).
    fn open_session_addr(&mut self, addr: &str) -> Result<PeerId, String> {
        use nbeep_core::Session as _;
        let link = self
            .transport
            .add_endpoint(addr)
            .map_err(|e| format!("{e:?}"))?;
        let noise = nbeep_crypto::NoiseSession::initiate(link, &self.identity)
            .map_err(|e| e.to_string())?;
        let est =
            nbeep_core::TrustedSession::wrap(noise, &mut self.trust).map_err(|e| e.to_string())?;
        let peer = est.session.peer();
        self.install_conversation(nbeep_core::MuxSession::new(est.session));
        Ok(peer)
    }

    /// 수동 입력 확정 — 주소로 연결하고 대화를 연다.
    fn commit_manual_add(&mut self, addr: String, el: &ActiveEventLoop) {
        let addr = addr.trim().to_string();
        self.adding = None;
        if addr.is_empty() {
            return;
        }
        match self.open_session_addr(&addr) {
            Ok(peer) => {
                self.status = format!("수동 연결 성공 — {}", peer.short());
                // 대화 뷰 열기(이미 conversation 등록됨 → ensure는 복원 경로).
                self.activate(peer, el);
            }
            Err(e) => self.status = format!("수동 연결 실패({addr}): {e}"),
        }
        if let Some(mid) = self.main_id {
            self.request_redraw(mid);
        }
    }

    fn peer_title(&self, peer: PeerId) -> String {
        self.table
            .get(peer)
            .map_or_else(|| format!("{peer:?}"), |e| e.name.as_str().to_string())
    }

    /// 수립된 세션을 액터로 옮기고 대화 상태를 등록한다(아웃바운드·인바운드 공용).
    fn install_conversation(&mut self, session: LiveSession) {
        let peer = session.peer();
        let (out_tx, out_rx) = std::sync::mpsc::channel();
        spawn_session_actor(session, out_rx, self.proxy.clone(), self.send_rate);
        self.conversations.insert(
            peer,
            Conversation {
                out_tx,
                lines: Vec::new(),
            },
        );
        // 세션 재수립 = 끊김 상태 해제(목록 점 Active로).
        self.closed_peers.remove(&peer);
        let mut inv = Invalidations::default();
        self.refresh_rows(&mut inv);
    }

    /// 대화 뷰 생성(스레드 복원 — 상태-뷰 분리).
    fn build_chat_view(&self, peer: PeerId) -> ChatViewWidget {
        let mut chat = ChatViewWidget::new(self.peer_title(peer));
        let mut inv = Invalidations::default();
        // 시각 표시 형식(설정 — 08-10).
        chat.set_time_format(
            self.settings.get("chat.time_24h") != "off",
            self.settings.get("chat.date_format") == "short",
            &mut inv,
        );
        if let Some(conv) = self.conversations.get(&peer) {
            for line in &conv.lines {
                chat.push_line(line.clone(), &mut inv);
            }
        }
        chat
    }

    /// Separate 모드 — `peer` 대화의 별도 창을 생성한다(이미 있으면 포커스).
    fn open_separate_window(&mut self, peer: PeerId, chat: ChatViewWidget, el: &ActiveEventLoop) {
        if let Some((id, _)) = self
            .windows
            .iter()
            .find(|(_, e)| e.role == Role::Chat(peer))
        {
            if let Some(e) = self.windows.get(id) {
                e.window.focus_window();
            }
            return;
        }
        let title = format!("Nexa Beep — {}", self.peer_title(peer));
        let attrs = Window::default_attributes()
            .with_title(title)
            .with_window_icon(self.icon.clone());
        let window = Rc::new(el.create_window(attrs).unwrap());
        window.set_ime_allowed(true);
        let scale = window.scale_factor() as f32;
        let context = softbuffer::Context::new(window.clone()).unwrap();
        let surface = SbSurface::new(&context, window.clone()).unwrap();
        let id = window.id();
        self.windows.insert(
            id,
            WinEntry {
                role: Role::Chat(peer),
                window,
                surface,
                cursor: (0, 0),
                scale,
            },
        );
        self.chats.insert(peer, chat);
        self.layout_window(id);
        let mut inv = Invalidations::default();
        self.refresh_rows(&mut inv);
        if let Some(mid) = self.main_id {
            self.request_redraw(mid);
        }
        self.request_redraw(id);
    }

    /// 설정 창을 연다(있으면 포커스) — `Cmd/Ctrl+,`.
    fn open_settings(&mut self, el: &ActiveEventLoop) {
        if let Some((id, _)) = self.windows.iter().find(|(_, e)| e.role == Role::Settings) {
            if let Some(e) = self.windows.get(id) {
                e.window.focus_window();
            }
            return;
        }
        let attrs = Window::default_attributes()
            .with_title(format!(
                "Nexa Beep — {}",
                nbeep_core::t(nbeep_core::Msg::SettingsTitle)
            ))
            .with_window_icon(self.icon.clone());
        let window = Rc::new(el.create_window(attrs).unwrap());
        window.set_ime_allowed(true);
        let scale = window.scale_factor() as f32;
        let context = softbuffer::Context::new(window.clone()).unwrap();
        let surface = SbSurface::new(&context, window.clone()).unwrap();
        let id = window.id();
        self.windows.insert(
            id,
            WinEntry {
                role: Role::Settings,
                window,
                surface,
                cursor: (0, 0),
                scale,
            },
        );
        let mut sv = SettingsWidget::new(&self.settings);
        {
            // "(시스템 기본)"이 무엇인지 식별(사용자 지적 08-10) — plat에서 이름 조회.
            let mut inv = Invalidations::default();
            sv.set_default_font_names(
                nbeep_plat::font::system_ui_font_name().unwrap_or(""),
                nbeep_plat::font::system_mono_font_name().unwrap_or(""),
                &mut inv,
            );
        }
        self.settings_view = Some(sv);
        self.refresh_approval_ui(); // 잠금·하단 정보 초기 반영
        self.layout_window(id);
        self.request_redraw(id);
    }

    /// 컨트롤 갤러리 창을 연다(임시 검수 — 이미 열려 있으면 포커스).
    /// 격리함 행 적재 — `.beepq`를 읽어 메타를 표시용으로 옮긴다(호스트가 IO 담당).
    fn quarantine_rows(&self) -> Vec<nbeep_ui::QRow> {
        use nbeep_core::TrustStore as _;
        use nbeep_safe::{Beepq, QuarantineDir};
        let Ok(dir) = QuarantineDir::open(crate::gate::quarantine_root(crate::gate::CH_GUI)) else {
            return Vec::new();
        };
        let Ok(paths) = dir.list() else {
            return Vec::new();
        };
        paths
            .into_iter()
            .filter_map(|p| {
                let bytes = std::fs::read(&p).ok()?;
                let bq = Beepq::open(&bytes).ok()?;
                // 표시용 불일치 경고 — 저장 시점 판정을 다시 계산하지 않고
                // 메타(선언 확장자 vs 매직 형식)만 대조한다.
                let mismatch = bq.meta.detected_kind != "unknown"
                    && !bq.meta.declared_ext.is_empty()
                    && !bq.meta.detected_kind.contains(&bq.meta.declared_ext);
                // 출처 — 보낸 사람(목록에 있으면 이름, 없으면 지문)과 수신 시각.
                let from = self
                    .table
                    .list()
                    .into_iter()
                    .find(|e| e.peer == bq.meta.sender)
                    .map_or_else(
                        || bq.meta.sender.short(),
                        |e| format!("{} ({})", e.name.as_str(), bq.meta.sender.short()),
                    );
                let age = unix_now().saturating_sub(bq.meta.received_at);
                let when = if age >= 86_400 {
                    format!("{}일 전", age / 86_400)
                } else {
                    nbeep_plat::clock::local_hms(bq.meta.received_at).hms()
                };
                Some(nbeep_ui::QRow {
                    name: String::from_utf8_lossy(&bq.meta.orig_name).into_owned(),
                    risk: bq.meta.risk,
                    mismatch,
                    size: bq.original_size,
                    trust: self.trust.level(bq.meta.sender),
                    from,
                    when,
                    path: p.to_string_lossy().into_owned(),
                })
            })
            .collect()
    }

    /// 격리함 창(메뉴 → 격리함) — 승인 = 실체화, 삭제 = `.beepq` 제거.
    fn open_quarantine(&mut self, el: &ActiveEventLoop) {
        if let Some((qid, _)) = self
            .windows
            .iter()
            .find(|(_, e)| e.role == Role::Quarantine)
        {
            if let Some(e) = self.windows.get(qid) {
                e.window.focus_window();
            }
            return;
        }
        let attrs = Window::default_attributes()
            .with_title(format!(
                "Nexa Beep — {}",
                nbeep_core::t(nbeep_core::Msg::QuarantineTitle)
            ))
            .with_inner_size(winit::dpi::LogicalSize::new(620.0, 420.0))
            .with_window_icon(self.icon.clone());
        let window = Rc::new(el.create_window(attrs).unwrap());
        let scale = window.scale_factor() as f32;
        let context = softbuffer::Context::new(window.clone()).unwrap();
        let surface = SbSurface::new(&context, window.clone()).unwrap();
        let id = window.id();
        self.windows.insert(
            id,
            WinEntry {
                role: Role::Quarantine,
                window,
                surface,
                cursor: (0, 0),
                scale,
            },
        );
        self.quarantine_view = Some(nbeep_ui::QuarantineWidget::new(self.quarantine_rows()));
        self.layout_window(id);
        self.request_redraw(id);
    }

    /// 격리함 결정 실행 — 승인 = 실체화(해시 재검증·OS 표식), 삭제 = `.beepq` 제거.
    fn run_quarantine_action(&mut self, act: nbeep_ui::QAction, id: WindowId) {
        use nbeep_safe::{Beepq, HashPort, MarkOutcome, MarkPort, QuarantineDir};
        struct CryptoHash;
        impl HashPort for CryptoHash {
            fn sha256(&self, data: &[u8]) -> [u8; 32] {
                nbeep_crypto::sha256(data)
            }
        }
        struct OsMark;
        impl MarkPort for OsMark {
            fn apply(&self, path: &std::path::Path) -> std::io::Result<MarkOutcome> {
                Ok(if nbeep_plat::quarantine::apply_quarantine_mark(path)? {
                    MarkOutcome::Applied
                } else {
                    MarkOutcome::Unsupported
                })
            }
        }
        // 결과는 주 창 상태바 + **격리함 창 자체**에 모두 남긴다(사용자 지적 08-09:
        // 승인해도 그 창에선 아무 일도 안 난 것처럼 보였다).
        let mut done_path: Option<String> = None;
        let mut msg_err = false;
        match act {
            nbeep_ui::QAction::Approve(path) => {
                // 실체화 기본 대상 = **다운로드 폴더**([docs/11] §4-1).
                // 없으면 임시 폴더로 떨어뜨리되 경로를 그대로 보여 준다(숨기지 않는다).
                let dest = nbeep_plat::paths::downloads_dir()
                    .unwrap_or_else(|| std::env::temp_dir().join("nexa-beep-materialized"));
                let out = std::fs::read(&path)
                    .map_err(|e| e.to_string())
                    .and_then(|b| Beepq::open(&b).map_err(|e| format!("{e:?}")))
                    .and_then(|bq| {
                        QuarantineDir::materialize(&bq, &dest, &CryptoHash, &OsMark)
                            .map_err(|e| e.to_string())
                    });
                self.status = match out {
                    Ok(m) => {
                        // 표식 실패도 **명시**한다(조용히 넘어가지 않는다 — [docs/11] §5).
                        let mark = match m.mark {
                            Ok(MarkOutcome::Applied) => "OS 보호 표식 부착",
                            Ok(MarkOutcome::Unsupported) => "⚠️ OS 보호 표식 미지원",
                            Err(_) => "⚠️ OS 보호 표식 실패",
                        };
                        done_path = Some(path);
                        format!("실체화 완료: {} · {mark}", m.path.display())
                    }
                    Err(e) => {
                        msg_err = true;
                        format!("실체화 실패: {e} — 격리 유지")
                    }
                };
            }
            nbeep_ui::QAction::Reject(path) => {
                self.status = match std::fs::remove_file(&path) {
                    Ok(()) => "격리물을 삭제했습니다".into(),
                    Err(e) => {
                        msg_err = true;
                        format!("삭제 실패: {e}")
                    }
                };
            }
            nbeep_ui::QAction::Clear => {
                // 비우기 — 위젯이 2단계 확인을 통과시킨 상태(사용자 요청 08-10).
                let (mut n, mut failed) = (0u32, 0u32);
                if let Ok(dir) =
                    QuarantineDir::open(crate::gate::quarantine_root(crate::gate::CH_GUI))
                {
                    if let Ok(paths) = dir.list() {
                        for p in paths {
                            match std::fs::remove_file(&p) {
                                Ok(()) => n += 1,
                                Err(_) => failed += 1,
                            }
                        }
                    }
                }
                self.status = if failed == 0 {
                    format!("격리함을 비웠습니다 — {n}건 삭제")
                } else {
                    msg_err = true;
                    format!("격리함 비우기 — {n}건 삭제 · {failed}건 실패")
                };
            }
        }
        let rows = self.quarantine_rows();
        let msg = self.status.clone();
        if let Some(v) = &mut self.quarantine_view {
            let mut inv = Invalidations::default();
            v.set_rows(rows, &mut inv);
            if let Some(p) = done_path {
                v.mark_done(p);
            }
            v.set_message(msg, msg_err, &mut inv);
        }
        self.request_redraw(id);
        if let Some(mid) = self.main_id {
            self.request_redraw(mid);
        }
    }

    fn open_gallery(&mut self, el: &ActiveEventLoop) {
        if let Some((gid, _)) = self.windows.iter().find(|(_, e)| e.role == Role::Gallery) {
            if let Some(e) = self.windows.get(gid) {
                e.window.focus_window();
            }
            return;
        }
        let attrs = Window::default_attributes()
            .with_title("Nexa Beep — 컨트롤 갤러리 (임시)")
            .with_window_icon(self.icon.clone());
        let window = Rc::new(el.create_window(attrs).unwrap());
        window.set_ime_allowed(true);
        let scale = window.scale_factor() as f32;
        let context = softbuffer::Context::new(window.clone()).unwrap();
        let surface = SbSurface::new(&context, window.clone()).unwrap();
        let id = window.id();
        self.windows.insert(
            id,
            WinEntry {
                role: Role::Gallery,
                window,
                surface,
                cursor: (0, 0),
                scale,
            },
        );
        // 어댑터 미주입 → Choose…가 take_choose_request로 올라와 **별도 모달 창**을 연다.
        self.gallery_view = Some(GalleryWidget::new());
        self.layout_window(id);
        self.request_redraw(id);
    }

    /// 파일 선택 **모달 창**(Choose… · ChoosePicker 어댑터 내용을 별도 창으로 · 사용자 확정).
    /// 항목 클릭 = 선택 확정(값 반영) 후 닫힘 · Esc/닫기 = 취소.
    fn open_picker(&mut self, el: &ActiveEventLoop) {
        if let Some((pid, _)) = self.windows.iter().find(|(_, e)| e.role == Role::Picker) {
            if let Some(e) = self.windows.get(pid) {
                e.window.focus_window();
            }
            return;
        }
        // 어댑터: HOME 단일 파일 선택기(ChoosePicker 인터페이스 — 어떤 구현도 가능).
        let dir = std::env::var_os("HOME")
            .map(std::path::PathBuf::from)
            .or_else(|| std::env::current_dir().ok())
            .unwrap_or_default();
        let picker = FilePicker { dir };
        use nbeep_ui::ChoosePicker as _;
        let title = picker.title();
        let roots: Vec<nbeep_ui::TreeNode> = picker
            .items()
            .into_iter()
            .map(|it| {
                let mut n = nbeep_ui::TreeNode::leaf(it.label);
                if let Some(img) = it.image {
                    n = n.with_image(img);
                }
                n
            })
            .collect();
        let mut tree = nbeep_ui::TreeView::new(nbeep_ui::TreeModel::new(roots));
        {
            use nbeep_ui::Control as _;
            tree.set_focused(true);
        }

        let attrs = Window::default_attributes()
            .with_title(title)
            .with_window_icon(self.icon.clone());
        let window = Rc::new(el.create_window(attrs).unwrap());
        let scale = window.scale_factor() as f32;
        let context = softbuffer::Context::new(window.clone()).unwrap();
        let surface = SbSurface::new(&context, window.clone()).unwrap();
        let id = window.id();
        self.windows.insert(
            id,
            WinEntry {
                role: Role::Picker,
                window,
                surface,
                cursor: (0, 0),
                scale,
            },
        );
        self.picker_view = Some(tree);
        self.layout_window(id);
        self.request_redraw(id);
    }

    /// 설정 변경 즉시 적용(DR-24 — 저장 버튼 없음).
    /// 설정 값에서 영역별 글꼴 설정을 만든다(크기 키 s/m/l/xl → px).
    fn fonts_from_settings(settings: &SettingsState) -> nbeep_ui::FontPrefs {
        // 글꼴명(font.{region}.family)은 SettingsState에 저장되지만, 실제 패밀리 로드는
        // 시스템 폰트 열거(M3-3 확장) 후 연결한다 — 지금은 크기만 렌더에 반영.
        let slot = |region: &str, base: f32| -> nbeep_ui::SlotFont {
            let size = match settings.get(&format!("font.{region}.size")) {
                "s" => base - 2.0,
                "l" => base + 3.0,
                "xl" => base + 7.0,
                _ => base, // "m"·미설정 = 기본
            };
            nbeep_ui::SlotFont {
                size,
                bold: false,
                italic: false,
            }
        };
        nbeep_ui::FontPrefs {
            base: slot("base", 16.0),
            peerlist: slot("peerlist", 16.0),
            message: slot("message", 18.0),
            status: slot("status", 13.0),
        }
    }

    fn apply_settings(&mut self, changes: Vec<(&'static str, String)>) {
        for (key, value) in changes {
            self.settings.set(key, value.clone());
            match key {
                "chat.window_mode" => {
                    // 새 대화부터 적용(DR-26 — 열린 창은 유지·소급 강제 없음).
                    self.mode = if value == "separate" {
                        WindowMode::Separate
                    } else {
                        WindowMode::Single
                    };
                    self.status = format!("창 모드 = {value} (새 대화부터 적용)");
                }
                // 시각 표시 형식(08-10) — 열린 대화 위젯 전부에 즉시 적용.
                "chat.time_24h" | "chat.date_format" => {
                    let h24 = self.settings.get("chat.time_24h") != "off";
                    let short = self.settings.get("chat.date_format") == "short";
                    let mut inv = Invalidations::default();
                    for chat in self.chats.values_mut() {
                        chat.set_time_format(h24, short, &mut inv);
                    }
                    let ids: Vec<WindowId> = self.windows.keys().copied().collect();
                    for wid in ids {
                        self.request_redraw(wid);
                    }
                }
                "ui.theme" => {
                    self.rebuild_theme(); // 팔레트 + 사용자 색 오버라이드(08-10)
                                          // 전 창 다시 그리기.
                    for e in self.windows.values() {
                        e.window.request_redraw();
                    }
                }
                // 테마 주요 색 사용자 지정(08-10) — 현재 테마에 즉시 적용.
                k if k.starts_with("theme.") => {
                    self.rebuild_theme();
                    self.status = format!("색 적용 — {k} = {value}");
                    for e in self.windows.values() {
                        e.window.request_redraw();
                    }
                }
                "ui.language" => {
                    // 현재 언어 전환 — 전 위젯이 다음 렌더에서 새 언어로 그린다.
                    nbeep_core::set_lang(nbeep_core::Lang::from_code(&value).unwrap_or_default());
                    // 메뉴 라벨은 생성 시 고정이라 재구성.
                    self.menu.set_menus(build_menus());
                    if let Some(mid) = self.main_id {
                        self.layout_window(mid);
                    }
                    for e in self.windows.values() {
                        e.window.request_redraw();
                    }
                }
                "ui.typeahead_timeout" => {
                    if let Ok(ms) = value.parse::<u64>() {
                        self.list.set_typeahead_timeout(ms);
                    }
                }
                // 전역이라 이 한 줄로 모든 스크롤 영역(목록·트리·갤러리·대화·설정)에 즉시 반영된다.
                "ui.scrollbar_hide" => {
                    if let Ok(ms) = value.parse::<u64>() {
                        nbeep_ui::controls::scroll::set_hide_delay_ms(ms);
                    }
                }
                "ui.typeahead_pos" => {
                    let mut inv = Invalidations::default();
                    self.list
                        .set_hud_pos(nbeep_ui::HudPos::from_code(&value), &mut inv);
                }
                "ui.toolbar_size" => {
                    if let Ok(px) = value.parse::<i32>() {
                        self.toolbar.set_icon_size(px);
                        if let Some(mid) = self.main_id {
                            self.layout_window(mid);
                            self.request_redraw(mid);
                        }
                    }
                }
                "xfer.approval" => {
                    use nbeep_core::{ApprovalPolicy, BasicApproval};
                    let now = self.now_ms();
                    self.approval = match value.as_str() {
                        "auto" => ApprovalPolicy::Basic(BasicApproval::Auto),
                        "block" => ApprovalPolicy::Basic(BasicApproval::Block),
                        // 기간 자동은 **지금 방식을 복귀 대상으로 물고** 시작한다.
                        "timed" => self.approval.start_timed(self.approval_window, now),
                        _ => ApprovalPolicy::Basic(BasicApproval::Manual),
                    };
                    self.approval_started_unix = (value == "timed").then(unix_now);
                    self.status = match self.approval.remaining_ms(now) {
                        Some(ms) => format!("파일 수신: {}분간 자동 수락", ms / 60_000),
                        None => format!("파일 수신 승인 = {value}"),
                    };
                    self.refresh_approval_ui();
                }
                "xfer.send_rate" => {
                    self.send_rate = nbeep_core::RateLimit::from_code(&value);
                    self.status = format!(
                        "보내기 제한 = {}",
                        rate_label(self.send_rate.target_bps(&self.send_meter))
                    );
                }
                "xfer.recv_rate" => {
                    self.recv_rate = nbeep_core::RateLimit::from_code(&value);
                    self.status = format!(
                        "받기 제한 = {} (상대에게 공지됨)",
                        rate_label(self.recv_rate.target_bps(&self.recv_meter))
                    );
                }
                "xfer.timeout_sec" => {
                    if let Ok(v) = value.parse::<u64>() {
                        self.wait_timeout_sec = v.clamp(5, 3600);
                    }
                }
                "xfer.approval_window" => {
                    if let Some(w) = nbeep_core::AutoWindow::from_code(&value) {
                        self.approval_window = w;
                        // 기간 자동이 켜져 있으면 **그때부터 다시 시작**(사용자 확정 08-09).
                        if self.approval.remaining_ms(self.now_ms()).is_some() {
                            let now = self.now_ms();
                            self.approval = self.approval.start_timed(w, now);
                            self.approval_started_unix = Some(unix_now());
                            self.status = "자동 수락 기간을 다시 시작했습니다".into();
                        }
                    }
                    self.refresh_approval_ui();
                }
                "ui.typeahead_space" => self.list.set_typeahead_space(value == "on"),
                "ui.typeahead_special" => self.list.set_typeahead_special(value == "on"),
                k if k.starts_with("font.") => {
                    self.fonts = Self::fonts_from_settings(&self.settings);
                    self.reload_faces(); // 글꼴명 → 실제 얼굴 로드(Enter 확정 시 도달)
                    self.status = "글꼴 설정 적용됨".into();
                    for e in self.windows.values() {
                        e.window.request_redraw();
                    }
                }
                _ => {}
            }
        }
        if let Some(id) = self.main_id {
            self.request_redraw(id);
        }
    }

    /// 대화 활성화 — 모드에 따라 주 창 전환 또는 별도 창 생성/포커스(14 §11).
    ///
    /// 세션이 없으면 **워커로 연결을 시작하고 즉시 돌아온다**(M2-8 — UI 무정지).
    /// 성립하면 `AppEvent::Outbound`가 이 함수를 다시 부른다.
    fn activate(&mut self, peer: PeerId, el: &ActiveEventLoop) {
        if !self.conversations.contains_key(&peer) {
            self.start_connect(peer);
            if let Some(mid) = self.main_id {
                self.request_redraw(mid);
            }
            return;
        }
        self.status = "대화 열림 — 세션 유지 중".into();
        let chat = self.build_chat_view(peer);
        match self.mode {
            WindowMode::Single => {
                self.chats.insert(peer, chat);
                self.single_open = Some(peer);
                self.set_main_ime(true); // 대화 = 실제 텍스트 → IME 켬
                if let Some(id) = self.main_id {
                    self.layout_window(id);
                    let mut inv = Invalidations::default();
                    self.refresh_rows(&mut inv);
                    self.request_redraw(id);
                }
            }
            WindowMode::Separate => self.open_separate_window(peer, chat, el),
        }
    }

    /// 창 크기·배율에 맞춰 그 창의 위젯 경계를 다시 계산한다.
    fn layout_window(&mut self, id: WindowId) {
        let Some(entry) = self.windows.get(&id) else {
            return;
        };
        let size = entry.window.inner_size();
        let scale = entry.scale;
        let role = entry.role;
        let w = i32::try_from(size.width).unwrap_or(i32::MAX);
        let h = i32::try_from(size.height).unwrap_or(i32::MAX);
        let mut inv = Invalidations::default();
        match role {
            Role::Main => {
                let body = (h - Self::bar_h(scale)).max(0);
                // 목록 모드 상단 크롬: Pull-down 메뉴 행 + 툴바(사용자 요청 08-09).
                let chrome = self.chrome_h(scale);
                if chrome > 0 {
                    let menu_h = (30.0 * scale).round() as i32;
                    self.menu.set_scale(scale);
                    self.menu.set_bounds(Rect::new(0, 0, w, menu_h), &mut inv);
                    self.toolbar.set_scale(scale);
                    self.toolbar
                        .set_bounds(Rect::new(0, menu_h, w, chrome - menu_h), &mut inv);
                }
                self.list.set_scale(scale, &mut inv);
                self.list
                    .set_bounds(Rect::new(0, chrome, w, (body - chrome).max(0)), &mut inv);
                if let Some(chat) = self.single_open.and_then(|p| self.chats.get_mut(&p)) {
                    chat.set_scale(scale, &mut inv);
                    chat.set_bounds(Rect::new(0, 0, w, body), &mut inv);
                }
            }
            Role::Chat(peer) => {
                if let Some(chat) = self.chats.get_mut(&peer) {
                    chat.set_scale(scale, &mut inv);
                    chat.set_bounds(Rect::new(0, 0, w, h), &mut inv);
                }
            }
            Role::Settings => {
                if let Some(sv) = &mut self.settings_view {
                    sv.set_scale(scale, &mut inv);
                    sv.set_bounds(Rect::new(0, 0, w, h), &mut inv);
                }
            }
            Role::Gallery => {
                if let Some(gv) = &mut self.gallery_view {
                    gv.set_scale(scale, &mut inv);
                    gv.set_bounds(Rect::new(0, 0, w, h), &mut inv);
                }
            }
            Role::Picker => {
                if let Some(pv) = &mut self.picker_view {
                    pv.set_scale(scale);
                    pv.set_bounds(Rect::new(0, 0, w, h), &mut inv);
                }
            }
            Role::About => {
                if let Some(av) = &mut self.about_view {
                    av.set_scale(scale, &mut inv);
                    av.set_bounds(Rect::new(0, 0, w, h), &mut inv);
                }
            }
            Role::Quarantine => {
                if let Some(qv) = &mut self.quarantine_view {
                    qv.set_scale(scale, &mut inv);
                    qv.set_bounds(Rect::new(0, 0, w, h), &mut inv);
                }
            }
            Role::Approve(peer) => {
                if let Some(pv) = self.approve_view.get_mut(&peer) {
                    pv.set_scale(scale, &mut inv);
                    pv.set_bounds(Rect::new(0, 0, w, h), &mut inv);
                }
            }
            Role::Sending(peer) => {
                if let Some(tb) = self.send_wait.get_mut(&peer) {
                    tb.set_scale(scale);
                    let bw = (200.0 * scale) as i32;
                    let bh = (30.0 * scale) as i32;
                    tb.set_bounds(
                        Rect::new((w - bw) / 2, h - bh - (16.0 * scale) as i32, bw, bh),
                        &mut inv,
                    );
                }
            }
        }
    }

    /// 대화 뷰에서 나온 발신·복귀를 처리한다. `peer` = 그 뷰의 상대.
    fn drain_chat_effects(&mut self, peer: PeerId, id: WindowId) {
        let mut inv = Invalidations::default();
        // 풍선 우클릭 복사(08-10) — 위젯은 OS를 모르므로 여기서 클립보드에 쓴다.
        if let Some(t) = self
            .chats
            .get_mut(&peer)
            .and_then(ChatViewWidget::take_copy_text)
        {
            // 실패도 말한다(조용한 무반응은 디버깅 불가 — 08-10 실기에서 배운 것).
            self.status = if nbeep_plat::clipboard::set_text(&t) {
                "메시지 복사됨".into()
            } else {
                "복사 실패 — 클립보드를 열 수 없습니다".into()
            };
            self.request_redraw(id);
            if let Some(mid) = self.main_id {
                self.request_redraw(mid);
            }
        }
        // 컨텍스트 메뉴의 "붙여넣기" — 위젯은 요청만 남기고, OS 읽기는 여기서 한다.
        if self
            .chats
            .get_mut(&peer)
            .is_some_and(ChatViewWidget::take_paste_request)
        {
            if let Some(t) = nbeep_plat::clipboard::get_text() {
                if let Some(c) = self.chats.get_mut(&peer) {
                    c.paste(&t, &mut inv);
                }
            } else {
                self.status = "붙여넣기 실패 — 클립보드를 읽을 수 없습니다".into();
                if let Some(mid) = self.main_id {
                    self.request_redraw(mid);
                }
            }
            self.request_redraw(id);
        }
        let outgoing = self
            .chats
            .get_mut(&peer)
            .and_then(ChatViewWidget::take_outgoing);
        if let Some(text) = outgoing {
            let msg = nbeep_core::ChatMessage {
                sender_device: self.identity.peer_id(),
                seq: self.seq.issue(),
                body: nbeep_core::MessageBody::Text(text.as_str().to_string()),
            };
            let (at_ms, wall) = now_stamp();
            if let Some(chat) = self.chats.get_mut(&peer) {
                chat.push_line(ChatLine::text(true, text.clone(), at_ms, wall), &mut inv);
            }
            // 왕래 장부 — 파일 전송 자격(상호 확인)의 근거(사용자 확정 08-09).
            self.ledger.note_sent(peer);
            if let Some(conv) = self.conversations.get_mut(&peer) {
                conv.lines.push(ChatLine::text(true, text, at_ms, wall));
                // 액터에 발신 요청 — 수신은 비동기로 AppEvent::Recv로 돌아온다(M2-7).
                if conv.out_tx.send(SessionCmd::Chat(msg.encode())).is_err() {
                    self.status = "세션 종료됨".into();
                } else {
                    self.status = format!("전송 seq={} (응답 대기)", msg.seq);
                }
            }
            self.request_redraw(id);
            if let Some(mid) = self.main_id {
                self.request_redraw(mid); // 상태바 갱신
            }
        }
        if self
            .chats
            .get_mut(&peer)
            .is_some_and(ChatViewWidget::take_back)
        {
            // 뷰만 닫는다 — 대화(세션·스레드)는 유지(DR-26).
            self.chats.remove(&peer);
            match self.mode {
                WindowMode::Single => {
                    self.single_open = None;
                    self.set_main_ime(false); // 목록 복귀 = 직접 조합 모드
                    self.status =
                        "↑↓ 이동 · 타이핑 = 이름 점프(한글 가능) · Enter = 대화 열기".into();
                    // 복귀 재레이아웃 — 대화 중 크롬 0으로 잡힌 목록 bounds를
                    // 크롬 아래로 되돌린다(없으면 목록이 메뉴·툴바 뒤에 숨는다).
                    self.layout_window(id);
                    self.request_redraw(id);
                }
                WindowMode::Separate => {
                    self.windows.remove(&id); // 창 닫힘(드롭) — 대화는 유지
                }
            }
        }
    }

    /// 이 창의 입력 이벤트를 담당 위젯으로 라우팅한다.
    fn route(&mut self, id: WindowId, ev: InputEvent, el: &ActiveEventLoop) {
        let Some(entry) = self.windows.get(&id) else {
            return;
        };
        let role = entry.role;
        // About = 모달 — 열려 있는 동안 다른 창 입력은 삼키고 About으로 포커스 복귀.
        if self.about_view.is_some() && role != Role::About {
            if let Some((_, e)) = self.windows.iter().find(|(_, e)| e.role == Role::About) {
                if matches!(ev, InputEvent::MouseDown { .. } | InputEvent::Key { .. }) {
                    e.window.focus_window();
                }
                return;
            }
        }
        let mut inv = Invalidations::default();
        match role {
            Role::Main => {
                if let Some(peer) = self.single_open {
                    if let Some(chat) = self.chats.get_mut(&peer) {
                        chat.on_event(&ev, &mut inv);
                    }
                    self.drain_chat_effects(peer, id);
                } else {
                    // 메뉴가 열려 있으면 모달 캡처(목록으로 전파 금지).
                    if self.menu.is_open() {
                        self.menu.on_event(&ev, &mut inv);
                        inv.push(self.list.bounds()); // 팝업 영역 재도색
                    } else {
                        self.menu.on_event(&ev, &mut inv);
                        self.toolbar.on_event(&ev, &mut inv);
                        if !self.menu.is_open() {
                            self.list.on_event(&ev, &mut inv);
                        }
                    }
                    // 액션 드레인 — 메뉴/툴바.
                    if let Some(a) = self.menu.take_picked() {
                        match a.as_str() {
                            "settings" => self.open_settings(el),
                            "quarantine" => self.open_quarantine(el),
                            "gallery" => self.open_gallery(el),
                            "about" => self.open_about(el),
                            _ => {}
                        }
                    }
                    if let Some(a) = self.toolbar.take_clicked() {
                        match a.as_str() {
                            "refresh" => {
                                // 목록 갱신(사용자 요청) — 발견 테이블·신뢰·세션 상태 재조립.
                                self.refresh_rows(&mut inv);
                                self.status =
                                    nbeep_core::t(nbeep_core::Msg::RefreshList).to_string();
                            }
                            // 직접 등록(DR-19 수동 엔드포인트) — ⌘/Ctrl+K와 같은 입력 흐름.
                            "add" => {
                                self.adding = Some(String::new());
                                self.status =
                                    "주소 입력(host:port) · Enter 연결 · Esc 취소: ".into();
                            }
                            "quarantine" => self.open_quarantine(el),
                            "gallery" => self.open_gallery(el),
                            _ => {}
                        }
                    }
                    if let Some(peer) = self.list.take_activated() {
                        self.activate(peer, el);
                    }
                }
            }
            Role::Chat(peer) => {
                if let Some(chat) = self.chats.get_mut(&peer) {
                    chat.on_event(&ev, &mut inv);
                }
                self.drain_chat_effects(peer, id);
            }
            Role::Settings => {
                if let Some(sv) = &mut self.settings_view {
                    sv.on_event(&ev, &mut inv);
                    let changes = sv.take_changes();
                    let close = sv.take_back();
                    if !changes.is_empty() {
                        self.apply_settings(changes);
                    }
                    if close {
                        self.settings_view = None;
                        self.windows.remove(&id);
                    }
                }
            }
            Role::Gallery => {
                // Esc = 닫기. 그 외는 갤러리 위젯으로 전달.
                if matches!(
                    ev,
                    InputEvent::Key {
                        key: Key::Escape,
                        ..
                    }
                ) {
                    self.gallery_view = None;
                    self.windows.remove(&id);
                } else if let Some(gv) = &mut self.gallery_view {
                    gv.on_event(&ev, &mut inv);
                    // Choose… → 별도 모달 파일 선택 창.
                    if gv.take_choose_request() {
                        self.open_picker(el);
                    }
                }
            }
            Role::Picker => {
                if matches!(
                    ev,
                    InputEvent::Key {
                        key: Key::Escape,
                        ..
                    }
                ) {
                    self.picker_view = None;
                    self.windows.remove(&id); // 취소
                } else if let Some(pv) = &mut self.picker_view {
                    pv.on_event(&ev, &mut inv);
                    // 항목 클릭 = 선택 확정 → 갤러리 Choose 값 반영 + 창 닫기.
                    if matches!(ev, InputEvent::MouseDown { .. }) {
                        if let Some(label) = pv.selected_label() {
                            if let Some(gv) = &mut self.gallery_view {
                                let mut ginv = Invalidations::default();
                                gv.set_choose_value(&label, &mut ginv);
                            }
                            self.picker_view = None;
                            self.windows.remove(&id);
                            if let Some((gid, _)) =
                                self.windows.iter().find(|(_, e)| e.role == Role::Gallery)
                            {
                                let gid = *gid;
                                self.request_redraw(gid);
                            }
                            return;
                        }
                    }
                }
            }
            Role::About => {
                if let Some(av) = &mut self.about_view {
                    av.on_event(&ev, &mut inv);
                    if av.take_back() {
                        self.about_view = None;
                        self.windows.remove(&id);
                    }
                }
            }
            Role::Approve(peer) => {
                let mut choice = None;
                if let Some(pv) = self.approve_view.get_mut(&peer) {
                    pv.on_event(&ev, &mut inv);
                    choice = pv.take_choice();
                }
                if let Some(c) = choice {
                    self.run_offer_choice(peer, c);
                }
            }
            Role::Sending(peer) => {
                let mut fired = None;
                if let Some(tb) = self.send_wait.get_mut(&peer) {
                    tb.on_event(&ev, &mut inv);
                    fired = tb.take_fired();
                }
                if fired.is_some() {
                    // 클릭이든 만료든 결과는 같다 — 전송 취소 + 창 닫기(사용자 확정).
                    self.cancel_send(peer, false);
                }
            }
            Role::Quarantine => {
                let mut act = None;
                if let Some(qv) = &mut self.quarantine_view {
                    qv.on_event(&ev, &mut inv);
                    act = qv.take_action();
                    if qv.take_back() {
                        self.quarantine_view = None;
                        self.windows.remove(&id);
                    }
                }
                if let Some(a) = act {
                    self.run_quarantine_action(a, id);
                }
            }
        }
        if !inv.is_empty() {
            self.request_redraw(id);
        }
    }

    fn redraw(&mut self, id: WindowId) {
        let theme = self.theme;
        let prefs = self.fonts;
        // 슬롯 얼굴을 **필드에서 직접** 빌린다 — 헬퍼 메서드로 감싸면 self 전체를 빌려
        // 아래 windows 가변 차용과 충돌한다(필드 단위 분할 차용을 쓰기 위한 형태).
        let fonts = nbeep_ui::FontSet {
            base: &self.font,
            peerlist: self.face_peerlist.as_ref(),
            message: self.face_message.as_ref(),
            status: self.face_status.as_ref(),
            mono: self.face_mono.as_ref(),
        };
        let Some(entry) = self.windows.get_mut(&id) else {
            return;
        };
        let size = entry.window.inner_size();
        let (Some(w), Some(h)) = (NonZeroU32::new(size.width), NonZeroU32::new(size.height)) else {
            return;
        };
        entry.surface.resize(w, h).unwrap();
        let mut buffer = entry.surface.buffer_mut().unwrap();
        let mut px =
            nbeep_gfx::Surface::new(&mut buffer, size.width as usize, size.height as usize);
        px.fill(theme.window_bg);
        let mut ctx = RasterCtx::with_font_set(&mut px, fonts)
            .with_fonts(prefs)
            .with_scale(entry.scale);
        match entry.role {
            Role::Main => {
                if let Some(chat) = self.single_open.and_then(|p| self.chats.get(&p)) {
                    chat.paint(&mut ctx, &theme);
                } else {
                    self.list.paint(&mut ctx, &theme);
                    // 상단 크롬(툴바 → 메뉴 순 — 메뉴 팝업이 최상위).
                    self.toolbar.paint(&mut ctx, &theme);
                    self.menu.paint(&mut ctx, &theme);
                }
                // 주 창 하단 상태바.
                let hh = i32::try_from(size.height).unwrap_or(i32::MAX);
                let ww = i32::try_from(size.width).unwrap_or(i32::MAX);
                let bar_h = Self::bar_h(entry.scale);
                let bar = Rect::new(0, hh - bar_h, ww, bar_h);
                ctx.select_font(nbeep_ui::FontSlot::Status, false);
                let pad = (8.0 * entry.scale).round() as i32;
                let dy = (bar_h - (14.0 * entry.scale) as i32) / 2;
                let bar_text = match &self.adding {
                    Some(buf) => format!("주소(host:port): {buf}"),
                    None => self.status.clone(),
                };
                ctx.text_opaque(
                    bar.x + pad,
                    bar.y + dy,
                    bar,
                    &bar_text,
                    theme.text_dim,
                    theme.chrome_bg,
                );
                // 주소 입력 중 — 문자 "▏" 대신 **Beam 캐럿**(글자 실측 높이 · 08-10).
                if self.adding.is_some() {
                    let tw = ctx.text_width(&bar_text);
                    let ch = ctx.text_height();
                    ctx.fill_rect(
                        Rect::new(
                            bar.x + pad + tw + (2.0 * entry.scale) as i32,
                            bar.y + (bar_h - ch) / 2,
                            ((2.0 * entry.scale).round() as i32).max(1),
                            ch,
                        ),
                        theme.accent,
                    );
                }
            }
            Role::Chat(peer) => {
                if let Some(chat) = self.chats.get(&peer) {
                    chat.paint(&mut ctx, &theme);
                }
            }
            Role::Settings => {
                if let Some(sv) = &self.settings_view {
                    sv.paint(&mut ctx, &theme);
                }
            }
            Role::Gallery => {
                if let Some(gv) = &self.gallery_view {
                    gv.paint(&mut ctx, &theme);
                }
            }
            Role::Picker => {
                if let Some(pv) = &self.picker_view {
                    pv.paint(&mut ctx, &theme);
                }
            }
            Role::About => {
                if let Some(av) = &self.about_view {
                    av.paint(&mut ctx, &theme);
                }
            }
            Role::Quarantine => {
                if let Some(qv) = &self.quarantine_view {
                    qv.paint(&mut ctx, &theme);
                }
            }
            Role::Approve(peer) => {
                if let Some(pv) = self.approve_view.get(&peer) {
                    pv.paint(&mut ctx, &theme);
                }
            }
            Role::Sending(peer) => {
                ctx.fill_rect(Rect::new(0, 0, 10_000, 10_000), theme.panel_bg);
                ctx.select_font(nbeep_ui::FontSlot::Base, false);
                let msg = "상대의 승인을 기다리는 중…";
                let tw = ctx.text_width(msg);
                let ww = i32::try_from(size.width).unwrap_or(0);
                ctx.text(
                    (ww - tw) / 2,
                    (40.0 * entry.scale) as i32,
                    Rect::new(0, 0, ww, 10_000),
                    msg,
                    theme.text,
                );
                if let Some(tb) = self.send_wait.get(&peer) {
                    tb.paint(&mut ctx, &theme);
                }
            }
        }
        buffer.present().unwrap();
    }
}

impl ApplicationHandler<AppEvent> for App {
    fn resumed(&mut self, el: &ActiveEventLoop) {
        if self.main_id.is_some() {
            return;
        }
        let attrs = Window::default_attributes()
            .with_title("Nexa Beep")
            .with_inner_size(winit::dpi::LogicalSize::new(460.0, 640.0)) // 기본 크기(사용자 확정 08-09)
            .with_window_icon(self.icon.clone());
        let window = Rc::new(el.create_window(attrs).unwrap());
        // 목록(타입어헤드) = IME **끔** — raw 자모를 앱이 직접 조합(hangul::Composer ·
        // OS 조합 세션 경합 제거). 대화 진입 시 켠다(set_main_ime).
        window.set_ime_allowed(false);
        let scale = window.scale_factor() as f32;
        let context = softbuffer::Context::new(window.clone()).unwrap();
        let surface = SbSurface::new(&context, window.clone()).unwrap();
        let id = window.id();
        self.windows.insert(
            id,
            WinEntry {
                role: Role::Main,
                window,
                surface,
                cursor: (0, 0),
                scale,
            },
        );
        self.main_id = Some(id);
        self.layout_window(id);
    }

    fn user_event(&mut self, el: &ActiveEventLoop, event: AppEvent) {
        // 세션 액터 → GUI(M2-7). 수신 메시지를 해당 대화 스레드에 실시간 반영한다.
        match event {
            AppEvent::Recv {
                peer,
                text,
                seq,
                sender,
            } => {
                if !self.dedup.accept(sender, seq) {
                    return; // 중복(다중 경로 — FR-M-9)
                }
                self.ledger.note_recv(peer); // 왕래 장부(상호 확인)
                let (at_ms, wall) = now_stamp();
                let line = ChatLine::text(false, text, at_ms, wall);
                if let Some(conv) = self.conversations.get_mut(&peer) {
                    conv.lines.push(line.clone());
                }
                let mut inv = Invalidations::default();
                if let Some(chat) = self.chats.get_mut(&peer) {
                    chat.push_line(line, &mut inv);
                }
                // 이 대화가 보이는 창을 다시 그린다.
                self.redraw_conversation(peer);
            }
            AppEvent::XferOffer {
                peer,
                id,
                name,
                size,
            } => {
                use nbeep_core::{
                    judge_offer, DenyReason, OfferVerdict, RejectWhy, TrustStore as _,
                };
                // ★ 판정은 **여기 한 곳**에서만 — 신뢰·왕래 장부·설정이 전부 여기 있다.
                // 액터는 중계만 하므로 정책이 두 벌로 갈라지지 않는다.
                self.tick_approval();
                let verdict = judge_offer(
                    self.trust.level(peer),
                    self.ledger.get(peer),
                    self.approval,
                    self.now_ms(),
                );
                match verdict {
                    OfferVerdict::Accept => {
                        self.send_xfer_decision(peer, id, true, RejectWhy::Declined);
                        self.status = format!("자동 수락: {name} ({}) 수신 시작", human_size(size));
                        self.push_xfer_line(peer, false, &name, size);
                    }
                    OfferVerdict::Ask => {
                        // 스레드에 수신 항목(승인 대기) — 거절하면 이 항목이 실패로 남는다.
                        self.push_xfer_line(peer, false, &name, size);
                        let q = self.pending_offers.entry(peer).or_default();
                        q.push_back((id, name.clone(), size));
                        let n = q.len();
                        // 승인 화면을 띄운다(이미 떠 있으면 그대로 — 큐에서 차례로 처리).
                        if !self.approve_view.contains_key(&peer) {
                            self.pending_approve_window = Some(peer);
                        }
                        let more = if n > 1 {
                            format!(" (대기 {n}건)")
                        } else {
                            String::new()
                        };
                        self.status = format!(
                            "파일 수신 요청: {name} ({}) — ⌘/Ctrl+Y 수락 · ⌘/Ctrl+N 거절{more}",
                            human_size(size)
                        );
                    }
                    OfferVerdict::Deny(reason) => {
                        let why = match reason {
                            DenyReason::Blocked => RejectWhy::Blocked,
                            DenyReason::NotPinned | DenyReason::NoMutualConversation => {
                                RejectWhy::Unverified
                            }
                        };
                        self.send_xfer_decision(peer, id, false, why);
                        self.status = format!("파일 거부({name}): {}", reason.message());
                    }
                }
                self.redraw_conversation(peer);
            }
            AppEvent::XferProgress {
                peer,
                got,
                total,
                sending,
            } => {
                let prev = self.xfer_progress.get(&peer).copied();
                let xp = nbeep_ui::XferProgress {
                    done_bytes: got,
                    total_bytes: total,
                    done_files: prev.map_or(0, |p| p.done_files),
                    total_files: prev.map_or(1, |p| p.total_files.max(1)),
                    sending,
                };
                self.xfer_progress.insert(peer, xp);
                // 스레드 항목 진행률 — 방향 일치(발신=mine)·현재 파일 누적 바이트.
                self.set_xfer_line(peer, sending, nbeep_ui::XferLineState::Active { done: got });
                self.apply_xfer_view(peer);
                self.redraw_conversation(peer);
                if let Some(mid) = self.main_id {
                    self.request_redraw(mid);
                }
            }
            AppEvent::XferDone {
                peer,
                name,
                risk,
                mismatch,
            } => {
                // 격리 보관까지 끝난 상태 — **실체화는 승인 후 별도**(FR-S-9).
                self.status = format!(
                    "파일 격리 완료: {name} · 위험 {risk:?}{} — 승인 전까지 실행 불가",
                    if mismatch {
                        " · ⚠️ 형식 불일치"
                    } else {
                        ""
                    }
                );
                self.set_xfer_line(
                    peer,
                    false,
                    nbeep_ui::XferLineState::Done {
                        note: format!(
                            "격리됨 · 위험 {risk:?}{}",
                            if mismatch { " · 형식 불일치" } else { "" }
                        ),
                    },
                );
                self.clear_xfer(peer);
                self.redraw_conversation(peer);
            }
            AppEvent::XferAccepted { peer } => {
                // 협상 성립 — 대기 창을 닫고 스트리밍 진행률로 넘어간다.
                self.awaiting_accept.remove(&peer);
                self.close_send_wait(peer);
                self.set_xfer_line(peer, true, nbeep_ui::XferLineState::Active { done: 0 });
                self.status = "상대가 수락 — 전송 시작".into();
                self.redraw_conversation(peer);
            }
            AppEvent::XferSendDone { peer } => {
                self.set_xfer_line(
                    peer,
                    true,
                    nbeep_ui::XferLineState::Done {
                        note: String::new(),
                    },
                );
                // 배치 집계 갱신 후 다음 파일로.
                if let Some(b) = self.send_batch.get_mut(&peer) {
                    b.0 += 1;
                    if let Some(xp) = self.xfer_progress.get(&peer) {
                        b.2 = b.2.saturating_add(xp.total_bytes);
                    }
                    let (done_f, total_f, done_b, total_b) = *b;
                    self.xfer_progress.insert(
                        peer,
                        nbeep_ui::XferProgress {
                            done_bytes: done_b,
                            total_bytes: total_b.max(done_b),
                            done_files: done_f,
                            total_files: total_f,
                            sending: true,
                        },
                    );
                    self.apply_xfer_view(peer);
                    self.status = format!("전송 완료 {done_f}/{total_f}");
                }
                let more = self.send_queue.get(&peer).is_some_and(|q| !q.is_empty());
                if more {
                    self.pump_send_queue(peer);
                } else {
                    self.send_batch.remove(&peer);
                    self.clear_xfer(peer);
                }
                self.redraw_conversation(peer);
            }
            AppEvent::XferFailed { peer, why } => {
                // 방향 판정 — 발신이 걸려 있으면 발신 실패, 아니면 수신 실패.
                let mine =
                    self.awaiting_accept.contains_key(&peer) || self.send_batch.contains_key(&peer);
                self.set_xfer_line(
                    peer,
                    mine,
                    nbeep_ui::XferLineState::Failed { why: why.clone() },
                );
                self.status = format!("파일: {why}");
                self.awaiting_accept.remove(&peer);
                self.close_send_wait(peer);
                self.send_queue.remove(&peer);
                self.send_batch.remove(&peer);
                self.clear_xfer(peer);
                self.redraw_conversation(peer);
            }
            AppEvent::Closed { peer } => {
                self.conversations.remove(&peer);
                self.closed_peers.insert(peer); // 목록 상태 점 = 끊김(빨강)
                self.status = "상대와의 세션이 종료됨".into();
                let mut inv = Invalidations::default();
                self.refresh_rows(&mut inv);
                if let Some(id) = self.main_id {
                    self.request_redraw(id);
                }
            }
            AppEvent::Outbound { session } => {
                // 워커가 만든 아웃바운드 세션(M2-8) — TOFU 판정은 여기(메인 · TrustStore 소유).
                use nbeep_core::Session as _;
                let peer = session.session.peer();
                self.connecting.remove(&peer);
                if !self.conversations.contains_key(&peer) {
                    let est =
                        match nbeep_core::TrustedSession::wrap(session.session, &mut self.trust) {
                            Ok(est) => est,
                            Err(e) => {
                                self.status = format!("신뢰 판정 거부: {e}");
                                if let Some(mid) = self.main_id {
                                    self.request_redraw(mid);
                                }
                                return;
                            }
                        };
                    let decision = est.decision;
                    self.install_conversation(nbeep_core::MuxSession::new(est.session));
                    self.activate(peer, el); // 사용자가 클릭한 연결 — 뷰를 연다
                    self.status = match decision {
                        nbeep_core::TrustDecision::FirstContact => {
                            "Noise 세션 수립 — 첫 접촉(TOFU 핀 고정)".into()
                        }
                        d => format!("Noise 세션 수립 — {d:?}"),
                    };
                } else {
                    // 그 사이 인바운드가 먼저 성립 — 이 세션은 버리고 그 대화를 연다.
                    self.activate(peer, el);
                }
                if let Some(mid) = self.main_id {
                    self.request_redraw(mid);
                }
            }
            AppEvent::ConnectFailed { peer, why } => {
                self.connecting.remove(&peer);
                self.status = format!("연결 실패({}): {why}", self.peer_title(peer));
                let mut inv = Invalidations::default();
                self.refresh_rows(&mut inv);
                if let Some(mid) = self.main_id {
                    self.request_redraw(mid);
                }
            }
            AppEvent::Inbound { session } => {
                use nbeep_core::Session as _;
                let peer = session.session.peer();
                if self.conversations.contains_key(&peer) {
                    return; // 이미 이 상대와 대화 중(아웃바운드 세션 존재) — 중복 인바운드 무시
                }
                // TOFU 판정(메인 스레드 — TrustStore가 여기 있다) → 다중화 → 대화 등록.
                let est = match nbeep_core::TrustedSession::wrap(session.session, &mut self.trust) {
                    Ok(est) => est,
                    Err(_) => return, // 차단 상대 등 — fail-closed
                };
                self.install_conversation(nbeep_core::MuxSession::new(est.session));
                let title = self.peer_title(peer);
                let mut inv = Invalidations::default();
                self.refresh_rows(&mut inv);
                match self.mode {
                    WindowMode::Separate => {
                        // 인바운드도 상대별 창을 연다(대칭 실시간 대화).
                        let chat = self.build_chat_view(peer);
                        self.open_separate_window(peer, chat, el);
                    }
                    WindowMode::Single => {
                        if self.single_open.is_none() {
                            // 목록 화면이면 새 대화를 바로 연다.
                            let chat = self.build_chat_view(peer);
                            self.chats.insert(peer, chat);
                            self.single_open = Some(peer);
                            self.set_main_ime(true);
                            if let Some(mid) = self.main_id {
                                self.layout_window(mid);
                            }
                        } else {
                            // 다른 대화 중 — 뺏지 않는다. 백그라운드로 쌓이고 목록에서 열면 복원.
                            self.status = format!("새 대화 도착: {title} (목록에서 열기)");
                        }
                    }
                }
                if let Some(mid) = self.main_id {
                    self.request_redraw(mid);
                }
            }
        }
    }

    fn about_to_wait(&mut self, el: &ActiveEventLoop) {
        // 종료 신호(SIGINT/SIGTERM) → 이벤트 루프 종료. run_app 반환 → App Drop →
        // transport(LocalDirect) Drop → GOODBYE 발신·소켓/스레드 정리(R-16 · FR-P-7).
        if self.shutdown.requested() {
            el.exit();
            return;
        }
        self.poll_discovery();
        // 타입어헤드 유효시간 경과 → 버퍼 초기화·HUD 자동 숨김(마지막 입력 후 N초).
        {
            let now_ms = self.now_ms();
            let mut inv = Invalidations::default();
            // 보류 자모: IME 이벤트가 따라오지 않았으면 진짜 입력 — 다음 틱(~200ms)에 방출.
            if let Some((_, _, t)) = self.pending_jamo {
                if !self.ime_composing && now_ms.saturating_sub(t) >= 150 {
                    self.flush_pending_jamo(el);
                }
            }
            if self.list.typeahead_tick(now_ms, &mut inv) {
                // 직접 조합 모드: TypeAhead.tick이 버퍼+조합기를 리셋 = 그게 전부(결정적).
                // 목록은 IME 자체가 꺼져 있어 세션 경합이 존재하지 않는다.
                self.pending_jamo = None;
                if let Some(id) = self.main_id {
                    self.request_redraw(id);
                }
            }
        }
        // 수신 승인 창 생성.
        if let Some(peer) = self.pending_approve_window.take() {
            if let Some(info) = self.front_offer_info(peer) {
                let mut pv = nbeep_ui::OfferPromptWidget::new(info, self.wait_timeout_sec);
                pv.start(self.now_ms());
                self.approve_view.insert(peer, pv);
                let attrs = Window::default_attributes()
                    .with_title("Nexa Beep — 파일 수신 요청")
                    .with_inner_size(winit::dpi::LogicalSize::new(440.0, 300.0))
                    .with_resizable(false)
                    .with_window_icon(self.icon.clone());
                if let Ok(window) = el.create_window(attrs) {
                    let window = Rc::new(window);
                    let scale = window.scale_factor() as f32;
                    if let Ok(context) = softbuffer::Context::new(window.clone()) {
                        if let Ok(surface) = SbSurface::new(&context, window.clone()) {
                            let id = window.id();
                            self.windows.insert(
                                id,
                                WinEntry {
                                    role: Role::Approve(peer),
                                    window,
                                    surface,
                                    cursor: (0, 0),
                                    scale,
                                },
                            );
                            self.layout_window(id);
                            self.request_redraw(id);
                        }
                    }
                }
            }
        }
        // 승인 창 카운트다운 — 시간이 다 되면 스스로 거절하고 닫힌다.
        {
            let now = self.now_ms();
            let mut choices: Vec<(PeerId, nbeep_ui::OfferChoice)> = Vec::new();
            let mut redraw: Vec<PeerId> = Vec::new();
            for (peer, pv) in &mut self.approve_view {
                if pv.tick(now) {
                    redraw.push(*peer);
                }
                if let Some(c) = pv.take_choice() {
                    choices.push((*peer, c));
                }
            }
            for peer in redraw {
                if let Some((wid, _)) = self
                    .windows
                    .iter()
                    .find(|(_, e)| e.role == Role::Approve(peer))
                {
                    let wid = *wid;
                    self.request_redraw(wid);
                }
            }
            for (peer, c) in choices {
                self.run_offer_choice(peer, c);
            }
        }
        // 발신 대기 창 생성(창 생성은 여기서만 — ActiveEventLoop가 필요하다).
        if let Some(peer) = self.pending_send_window.take() {
            if !self.windows.values().any(|e| e.role == Role::Sending(peer)) {
                let attrs = Window::default_attributes()
                    .with_title("Nexa Beep — 전송 대기")
                    .with_inner_size(winit::dpi::LogicalSize::new(360.0, 130.0))
                    .with_resizable(false)
                    .with_window_icon(self.icon.clone());
                if let Ok(window) = el.create_window(attrs) {
                    let window = Rc::new(window);
                    let scale = window.scale_factor() as f32;
                    if let Ok(context) = softbuffer::Context::new(window.clone()) {
                        if let Ok(surface) = SbSurface::new(&context, window.clone()) {
                            let id = window.id();
                            self.windows.insert(
                                id,
                                WinEntry {
                                    role: Role::Sending(peer),
                                    window,
                                    surface,
                                    cursor: (0, 0),
                                    scale,
                                },
                            );
                            self.layout_window(id);
                            self.request_redraw(id);
                        }
                    }
                }
            }
        }
        // 발신 대기 타임아웃 — 60초가 지나면 **버튼이 스스로 눌려** 전송을 취소한다.
        {
            let now = self.now_ms();
            let mut fired: Vec<PeerId> = Vec::new();
            let mut redraw: Vec<PeerId> = Vec::new();
            for (peer, tb) in &mut self.send_wait {
                if tb.tick(now) {
                    redraw.push(*peer);
                }
                if tb.take_fired().is_some() {
                    fired.push(*peer);
                }
            }
            for peer in redraw {
                if let Some((wid, _)) = self
                    .windows
                    .iter()
                    .find(|(_, e)| e.role == Role::Sending(peer))
                {
                    let wid = *wid;
                    self.request_redraw(wid);
                }
            }
            for peer in fired {
                self.cancel_send(peer, true);
            }
        }
        // 기간 자동 승인 — 만료 확인 + **1초마다 남은 시간 갱신**(사용자 확정 08-09).
        {
            let reverted = self.tick_approval();
            let sec = unix_now();
            // 설정 화면이 떠 있을 때만 1초 갱신한다 — 닫혀 있으면 그릴 곳이 없다.
            if self.settings_view.is_some() && sec != self.approval_footer_sec {
                self.approval_footer_sec = sec;
                self.refresh_approval_ui();
                if let Some((sid, _)) = self.windows.iter().find(|(_, e)| e.role == Role::Settings)
                {
                    let sid = *sid;
                    self.request_redraw(sid);
                }
            }
            if reverted {
                if let Some(mid) = self.main_id {
                    self.request_redraw(mid);
                }
                if let Some((sid, _)) = self.windows.iter().find(|(_, e)| e.role == Role::Settings)
                {
                    let sid = *sid;
                    self.request_redraw(sid);
                }
            }
        }
        // 스크롤바 자동 숨김 틱 — **시각을 넘긴다**(호출 횟수가 아니라 벽시계로 재야
        // 이벤트가 몰리는 드래그 중에도 설정한 시간만큼 보인다 · 08-10).
        let bar_now = self.now_ms();
        if let Some(sv) = &mut self.settings_view {
            if sv.tick(bar_now) {
                if let Some((sid, _)) = self.windows.iter().find(|(_, e)| e.role == Role::Settings)
                {
                    let sid = *sid;
                    self.request_redraw(sid);
                }
            }
        }
        // 대화 스레드·입력창 스크롤바 페이드 틱(~5Hz · 08-10).
        {
            let peers: Vec<PeerId> = self
                .chats
                .iter_mut()
                .filter_map(|(p, c)| c.tick(bar_now).then_some(*p))
                .collect();
            for p in peers {
                self.redraw_conversation(p);
            }
        }
        // 갤러리(트리·그리드 포함) 스크롤바 틱 — 상태 변화 시 재그리기.
        if let Some(gv) = &mut self.gallery_view {
            if gv.tick(bar_now) {
                if let Some((gid, _)) = self.windows.iter().find(|(_, e)| e.role == Role::Gallery) {
                    let gid = *gid;
                    self.request_redraw(gid);
                }
            }
        }
        // 유휴에도 ~5Hz로 깨어나 발견 갱신·종료 신호를 폴한다(입력 없을 때도 목록이 산다).
        el.set_control_flow(ControlFlow::wait_duration(
            std::time::Duration::from_millis(200),
        ));
    }

    fn window_event(&mut self, el: &ActiveEventLoop, id: WindowId, event: WindowEvent) {
        match event {
            WindowEvent::CloseRequested => {
                if Some(id) == self.main_id {
                    el.exit(); // 주 창 닫기 = 종료(트레이 상주는 M3-2)
                } else if let Some(entry) = self.windows.remove(&id) {
                    match entry.role {
                        // 대화 창 닫기 = 뷰만 닫힘(대화 유지 — DR-26).
                        Role::Chat(peer) => {
                            self.chats.remove(&peer);
                        }
                        Role::Settings => self.settings_view = None,
                        Role::Gallery => self.gallery_view = None,
                        Role::Picker => self.picker_view = None,
                        Role::About => self.about_view = None,
                        Role::Quarantine => self.quarantine_view = None,
                        Role::Sending(peer) => self.cancel_send(peer, false),
                        Role::Approve(peer) => {
                            // 창을 닫으면 거절로 본다(응답하지 않은 제안을 남기지 않는다).
                            self.run_offer_choice(
                                peer,
                                nbeep_ui::OfferChoice::Cancel { by_timeout: false },
                            );
                        }
                        Role::Main => {}
                    }
                }
            }
            WindowEvent::Resized(_) => {
                self.layout_window(id);
                self.request_redraw(id);
            }
            WindowEvent::ScaleFactorChanged { scale_factor, .. } => {
                if let Some(e) = self.windows.get_mut(&id) {
                    e.scale = scale_factor as f32;
                }
                self.layout_window(id);
                self.request_redraw(id);
            }
            WindowEvent::Ime(winit::event::Ime::Preedit(text, _)) => {
                // 조합 세션 추적 — 비어 있지 않으면 조합 중(KeyboardInput 문자 차단 근거).
                self.ime_composing = !text.is_empty();
                self.pending_jamo = None; // IME가 살아 있다 = 보류 자모는 중복이었다
                                          // 조합 중 — 대화 뷰면 프리에딧 밑줄(M3-3), 목록 모드면 실시간 타입어헤드.
                if let Some(peer) = self.chat_peer_for(id) {
                    let mut inv = Invalidations::default();
                    if let Some(chat) = self.chats.get_mut(&peer) {
                        chat.set_preedit(text, &mut inv);
                    }
                    self.request_redraw(id);
                } else if Some(id) == self.main_id && self.single_open.is_none() {
                    // 목록에서 한글 조합 즉시 매칭(확정/Space 불필요).
                    let now_ms = self.now_ms();
                    let mut inv = Invalidations::default();
                    self.list.set_preedit(&text, now_ms, &mut inv);
                    self.request_redraw(id);
                }
            }
            WindowEvent::Ime(winit::event::Ime::Commit(text)) => {
                self.ime_composing = false; // 확정 = 조합 종료
                self.pending_jamo = None; // IME 경로 확인 — 보류 자모는 중복이었다
                let now_ms = self.now_ms();
                for c in text.chars().filter(|c| !c.is_control()) {
                    self.route(id, InputEvent::Char { c, now_ms }, el);
                }
            }
            WindowEvent::DroppedFile(path) => {
                // 드래그앤드롭 = 파일 전송 시작(FR-X-1). 대화가 열린 창에서만 의미가 있다.
                self.offer_file(id, &path);
            }
            WindowEvent::CursorMoved { position, .. } => {
                if let Some(e) = self.windows.get_mut(&id) {
                    e.cursor = (position.x as i32, position.y as i32);
                    let (x, y) = e.cursor;
                    // 설정 창 스플리터 hover/드래그 = 좌우 리사이즈 커서(그 외 기본).
                    if e.role == Role::Settings {
                        let resize = self
                            .settings_view
                            .as_ref()
                            .is_some_and(|sv| sv.wants_col_resize_cursor(x, y));
                        e.window.set_cursor(if resize {
                            winit::window::CursorIcon::ColResize
                        } else {
                            winit::window::CursorIcon::Default
                        });
                    }
                    self.route(id, InputEvent::MouseMove { x, y }, el);
                }
            }
            WindowEvent::MouseInput {
                state: ElementState::Pressed,
                button: MouseButton::Left,
                ..
            } => {
                if let Some(e) = self.windows.get(&id) {
                    let (x, y) = e.cursor;
                    // 갤러리 진입은 메뉴·⌘/Ctrl+G — 하단 버튼은 제거(사용자 요청 08-10).
                    self.route(
                        id,
                        InputEvent::MouseDown {
                            x,
                            y,
                            shift: false,
                            primary: false,
                        },
                        el,
                    );
                }
            }
            WindowEvent::MouseInput {
                state: ElementState::Pressed,
                button: MouseButton::Right,
                ..
            } => {
                // 우클릭 — 컨텍스트 메뉴(08-10). 메뉴가 "붙여넣기"를 활성으로 보여도 되는지는
                // 클립보드에 실제로 뭐가 있느냐에 달렸고, 그건 호스트만 안다. 여는 순간
                // 한 번만 확인해 알려준다(매 프레임 클립보드를 긁으면 낭비다).
                if let Some(e) = self.windows.get(&id) {
                    let (x, y) = e.cursor;
                    let has_clip = nbeep_plat::clipboard::get_text()
                        .is_some_and(|t| !t.trim_matches(char::from(0)).is_empty());
                    if let Some(peer) = self.chat_peer_for(id) {
                        if let Some(c) = self.chats.get_mut(&peer) {
                            c.set_clipboard_has_text(has_clip);
                        }
                    }
                    self.route(id, InputEvent::RightDown { x, y }, el);
                }
            }
            WindowEvent::MouseInput {
                state: ElementState::Released,
                button: MouseButton::Left,
                ..
            } => {
                if let Some(e) = self.windows.get(&id) {
                    let (x, y) = e.cursor;
                    self.route(id, InputEvent::MouseUp { x, y }, el);
                }
            }
            WindowEvent::MouseWheel { delta, .. } => {
                let (dx, dy) = match delta {
                    MouseScrollDelta::LineDelta(x, y) => ((x * 120.0) as i32, (y * 120.0) as i32),
                    MouseScrollDelta::PixelDelta(p) => {
                        ((p.x * 120.0 / 38.0) as i32, (p.y * 120.0 / 38.0) as i32)
                    }
                };
                if dy != 0 {
                    self.route(id, InputEvent::Wheel { delta: dy }, el);
                }
                if dx != 0 {
                    self.route(id, InputEvent::HWheel { delta: dx }, el);
                }
            }
            WindowEvent::ModifiersChanged(mods) => {
                let st = mods.state();
                self.primary_down = if cfg!(target_os = "macos") {
                    st.super_key()
                } else {
                    st.control_key()
                };
                // Shift 상태 — Shift+Enter 줄바꿈·Shift+이동 선택에 쓴다(08-10).
                self.shift_down = st.shift_key();
            }
            WindowEvent::KeyboardInput { event, .. } => {
                if event.state != ElementState::Pressed {
                    return;
                }
                // IME 조합 중엔 키를 IME가 소유한다 — 같은 키가 KeyboardInput으로도 와서
                // 자모('ㄱ','ㅣ','ㅁ')가 확정 버퍼에 이중 유입되던 간헐 버그의 차단점.
                // (조합 결과는 Ime::Preedit/Commit으로만 반영한다.)
                if self.ime_composing {
                    return;
                }
                // 새 키가 왔는데 보류 자모가 남아 있으면 = 앞 키에 IME가 안 붙었다 → 먼저 방출.
                self.flush_pending_jamo(el);
                // Cmd/Ctrl+, = 설정 · Cmd/Ctrl+K = 수동 엔드포인트 추가(DR-19).
                if self.primary_down {
                    if let WKey::Character(t) = &event.logical_key {
                        match t.as_str() {
                            "," => {
                                self.open_settings(el);
                                return;
                            }
                            "g" | "G" => {
                                self.open_gallery(el);
                                return;
                            }
                            // 텍스트 기본 단축키 — 전체 선택(사용자 지적 08-09).
                            "a" | "A" => {
                                self.route(id, InputEvent::SelectAll, el);
                                return;
                            }
                            // 복사/잘라내기/붙여넣기 — 대화 입력창 ↔ OS 클립보드(08-10 ·
                            // ui는 OS를 모른다 — plat 어댑터가 잇는다).
                            "c" | "C" => {
                                if let Some(t) = self
                                    .chat_peer_for(id)
                                    .and_then(|p| self.chats.get(&p))
                                    .and_then(ChatViewWidget::copy_selection)
                                {
                                    if nbeep_plat::clipboard::set_text(&t) {
                                        self.status = "복사됨".into();
                                    }
                                }
                                return;
                            }
                            "x" | "X" => {
                                let mut inv = Invalidations::default();
                                if let Some(t) = self
                                    .chat_peer_for(id)
                                    .and_then(|p| self.chats.get_mut(&p))
                                    .and_then(|c| c.cut_selection(&mut inv))
                                {
                                    nbeep_plat::clipboard::set_text(&t);
                                    self.request_redraw(id);
                                }
                                return;
                            }
                            "v" | "V" => {
                                if let Some(peer) = self.chat_peer_for(id) {
                                    if let Some(t) = nbeep_plat::clipboard::get_text() {
                                        let mut inv = Invalidations::default();
                                        if let Some(c) = self.chats.get_mut(&peer) {
                                            c.paste(&t, &mut inv);
                                        }
                                        self.request_redraw(id);
                                    }
                                }
                                return;
                            }
                            "y" | "Y" => {
                                self.answer_offer(id, true);
                                return;
                            }
                            "n" | "N" => {
                                self.answer_offer(id, false);
                                return;
                            }
                            "k" | "K" => {
                                self.adding = Some(String::new());
                                self.status =
                                    "주소 입력(host:port) · Enter 연결 · Esc 취소: ".into();
                                if let Some(mid) = self.main_id {
                                    self.request_redraw(mid);
                                }
                                return;
                            }
                            _ => {}
                        }
                    }
                }
                // 수동 입력 모드 — 문자/Enter/Esc를 목록이 아니라 주소 버퍼로.
                if self.adding.is_some() {
                    match &event.logical_key {
                        WKey::Named(NamedKey::Enter) => {
                            let addr = self.adding.take().unwrap_or_default();
                            self.commit_manual_add(addr, el);
                        }
                        WKey::Named(NamedKey::Escape) => {
                            self.adding = None;
                            self.status = "수동 추가 취소".into();
                            if let Some(mid) = self.main_id {
                                self.request_redraw(mid);
                            }
                        }
                        WKey::Named(NamedKey::Backspace) => {
                            if let Some(buf) = self.adding.as_mut() {
                                buf.pop();
                            }
                            if let Some(mid) = self.main_id {
                                self.request_redraw(mid);
                            }
                        }
                        WKey::Character(t) => {
                            if let Some(buf) = self.adding.as_mut() {
                                buf.push_str(t);
                            }
                            if let Some(mid) = self.main_id {
                                self.request_redraw(mid);
                            }
                        }
                        _ => {}
                    }
                    return;
                }
                let key = match &event.logical_key {
                    WKey::Named(NamedKey::ArrowUp) => Some(Key::Up),
                    WKey::Named(NamedKey::ArrowDown) => Some(Key::Down),
                    WKey::Named(NamedKey::ArrowLeft) => Some(Key::Left),
                    WKey::Named(NamedKey::ArrowRight) => Some(Key::Right),
                    WKey::Named(NamedKey::PageUp) => Some(Key::PageUp),
                    WKey::Named(NamedKey::PageDown) => Some(Key::PageDown),
                    WKey::Named(NamedKey::Home) => Some(Key::Home),
                    WKey::Named(NamedKey::Delete) => Some(Key::Delete),
                    WKey::Named(NamedKey::End) => Some(Key::End),
                    WKey::Named(NamedKey::Enter) => Some(Key::Enter),
                    WKey::Named(NamedKey::Escape) => Some(Key::Escape),
                    _ => None,
                };
                if let Some(key) = key {
                    self.route(
                        id,
                        InputEvent::Key {
                            key,
                            shift: self.shift_down,
                            primary: self.primary_down,
                        },
                        el,
                    );
                    return;
                }
                if let WKey::Named(NamedKey::Backspace) = &event.logical_key {
                    let now_ms = self.now_ms();
                    self.route(id, InputEvent::Char { c: '\u{8}', now_ms }, el);
                    return;
                }
                // 스페이스는 Named(Space)로 와서 Character 경로에 안 잡힌다 — 문자로 라우팅
                // (목록 타입어헤드·대화 입력 공통. 이전엔 직타 스페이스가 유실됐다).
                if let WKey::Named(NamedKey::Space) = &event.logical_key {
                    let now_ms = self.now_ms();
                    self.route(id, InputEvent::Char { c: ' ', now_ms }, el);
                    return;
                }
                if let WKey::Character(text) = &event.logical_key {
                    let now_ms = self.now_ms();
                    if let Some(c) = text.chars().next() {
                        // 단독 한글 자모: ①중복(곧 Preedit가 온다) ②진짜 입력(IME 미접속 —
                        // 한영 전환 직후 첫 키). 즉시 버리면 ②가 유실되므로 **보류** 후
                        // Ime 이벤트가 오면 폐기, 안 오면(~틱) 라우팅한다.
                        let is_jamo = matches!(c,
                            '\u{1100}'..='\u{11FF}' // Hangul Jamo
                            | '\u{3130}'..='\u{318F}' // Compat Jamo(ㄱ·ㅣ 등)
                            | '\u{A960}'..='\u{A97F}'
                            | '\u{D7B0}'..='\u{D7FF}');
                        let list_mode = Some(id) == self.main_id && self.single_open.is_none();
                        if is_jamo && !list_mode {
                            // IME 켜진 창(대화 등): 중복 가능성 → 보류-판정.
                            self.pending_jamo = Some((id, c, now_ms));
                        } else if !c.is_control() {
                            // 목록: IME 꺼짐 = 자모가 유일한 경로 → 즉시 직접 조합기로.
                            self.route(id, InputEvent::Char { c, now_ms }, el);
                        }
                    }
                }
            }
            WindowEvent::RedrawRequested => self.redraw(id),
            _ => {}
        }
    }
}

/// 창을 띄우고 이벤트 루프를 돈다(주 창을 닫으면 종료).
/// 창을 띄운다. `live=true`면 실물 발견(`LocalDirect`), 아니면 InMemory 데모(에코 봇).
pub(crate) fn run(mode: WindowMode, live: bool) {
    let (data, index) = nbeep_plat::font::system_ui_font().expect("시스템 UI 폰트 없음");
    let font = nbeep_gfx::Font::from_static(data, index).expect("폰트 파싱");
    let identity = std::sync::Arc::new(nbeep_crypto::Identity::generate());
    use nbeep_net::Transport as _;

    // 이벤트 루프·프록시 먼저 — 인바운드 수락 펌프가 프록시를 필요로 한다(M2-7).
    let event_loop = EventLoop::<AppEvent>::with_user_event().build().unwrap();
    let proxy = event_loop.create_proxy();
    let shutdown = nbeep_plat::shutdown::install(); // R-16 — SIGINT/SIGTERM 포트

    let (transport, discovery): (std::sync::Arc<dyn nbeep_net::Transport + Send + Sync>, _) =
        if live {
            // 실물 — LocalDirect(UDP 발견 + TCP 세션). 실기·컨테이너 상대가 목록에 뜬다.
            let mut instance = [0u8; 16];
            instance
                .copy_from_slice(&nbeep_crypto::Identity::generate().peer_id().as_bytes()[..16]);
            let name =
                nbeep_core::DisplayName::parse(&format!("나-{}", identity.peer_id().short()))
                    .expect("라벨");
            let local = nbeep_net::LocalDirect::spawn(identity.peer_id(), instance, name, 800, 1)
                .expect("LocalDirect 시작(방화벽·인터페이스)");
            let discovery = local.discovery();
            // 인바운드 수락 펌프 — 남이 나에게 연결하면 accept+에코(대칭 대화·비동기 GUI 펌프는 M2-7).
            spawn_inbound_accept(
                local.incoming(),
                std::sync::Arc::clone(&identity),
                proxy.clone(),
            );
            (std::sync::Arc::new(local), discovery)
        } else {
            // 데모 — InMemory 버스 + 에코 봇. 순환 탐색 테스트용으로 같은 접두사(김*/bob* 등)를
            // 여러 개 둔다(타입어헤드 ↑↓ 순환 확인).
            let bus = nbeep_net::inmem::InMemoryBus::new();
            for name in [
                "김철수의 MacBook",
                "김영희 데스크탑",
                "김민수 노트북",
                "이영희 (개발2팀)",
                "bob-linux",
                "bora-win",
                "bill-mac",
            ] {
                spawn_echo_bot(&bus, name);
            }
            let transport = bus.join(
                identity.peer_id(),
                nbeep_core::DisplayName::parse("나").unwrap(),
                nbeep_net::Caps::default(),
            );
            let discovery = transport.discovery();
            (std::sync::Arc::new(transport), discovery)
        };

    let mut settings = SettingsState::with_defaults();
    if mode == WindowMode::Separate {
        settings.set("chat.window_mode", "separate".into());
    }
    // 현재 언어를 설정값으로 초기화(기본 en — i18n).
    nbeep_core::set_lang(
        nbeep_core::Lang::from_code(settings.get("ui.language")).unwrap_or_default(),
    );
    // 스크롤바 자동 숨김도 부팅 때 한 번 반영한다 — 설정을 바꿔야만 적용되면
    // 첫 실행에서 기본값이 코드 상수와 어긋나도 아무도 모른다.
    if let Ok(ms) = settings.get("ui.scrollbar_hide").parse::<u64>() {
        nbeep_ui::controls::scroll::set_hide_delay_ms(ms);
    }
    let net_hint = if live {
        "실물 발견(LAN)"
    } else {
        "데모(에코 봇)"
    };
    let mode_hint = match mode {
        WindowMode::Single => "Enter = 대화 열기",
        WindowMode::Separate => "Enter = 상대별 새 창(동시 대화)",
    };
    let mut app = App {
        mode,
        windows: HashMap::new(),
        main_id: None,
        font,
        theme: Theme::dark(),
        list: PeerListWidget::new(),
        chats: HashMap::new(),
        single_open: None,
        identity,
        seq: nbeep_core::Sequencer::new(),
        transport,
        connecting: HashSet::new(),
        discovery,
        table: nbeep_core::PeerTable::new(60_000),
        trust: nbeep_core::MemoryTrustStore::new(),
        conversations: HashMap::new(),
        dedup: nbeep_core::DedupIndex::new(),
        started: Instant::now(),
        status: format!(
            "[{net_hint}] {mode_hint} · ⌘/Ctrl+K = 주소 추가 · ⌘/Ctrl+, = 설정 · ⌘/Ctrl+G = 컨트롤 갤러리"
        ),
        fonts: App::fonts_from_settings(&settings),
        settings,
        settings_view: None,
        gallery_view: None,
        about_view: None,
        quarantine_view: None,
        xfer_progress: HashMap::new(),
        ledger: nbeep_core::ExchangeLedger::new(),
        approval: nbeep_core::ApprovalPolicy::default(),
        approval_window: nbeep_core::AutoWindow::Hour1,
        approval_started_unix: None,
        approval_footer_sec: 0,
        wait_timeout_sec: 60,
        approve_view: HashMap::new(),
        pending_approve_window: None,
        face_peerlist: None,
        face_message: None,
        face_status: None,
        face_mono: None,
        pending_offers: HashMap::new(),
        send_queue: HashMap::new(),
        send_batch: HashMap::new(),
        awaiting_accept: HashMap::new(),
        send_wait: HashMap::new(),
        pending_send_window: None,
        send_rate: nbeep_core::RateLimit::Auto,
        recv_rate: nbeep_core::RateLimit::Auto,
        send_meter: nbeep_core::RateMeter::default(),
        recv_meter: nbeep_core::RateMeter::default(),
        menu: MenuBar::new(build_menus()),
        toolbar: Toolbar::new(vec![
            ToolItem::new(
                "refresh",
                ToolIcon::Mask {
                    w: nbeep_ui::icons::REFRESH_SIZE,
                    h: nbeep_ui::icons::REFRESH_SIZE,
                    alpha: nbeep_ui::icons::REFRESH_ALPHA,
                },
            ),
            ToolItem::new(
                "add",
                ToolIcon::Mask {
                    w: nbeep_ui::icons::ADD_SIZE,
                    h: nbeep_ui::icons::ADD_SIZE,
                    alpha: nbeep_ui::icons::ADD_ALPHA,
                },
            ),
            ToolItem::new(
                "quarantine",
                ToolIcon::Mask {
                    w: nbeep_ui::icons::SHIELD_SIZE,
                    h: nbeep_ui::icons::SHIELD_SIZE,
                    alpha: nbeep_ui::icons::SHIELD_ALPHA,
                },
            ),
            // 컨트롤 갤러리는 툴바에서 뺐다(사용자 요청 08-10) — 메뉴(보기 ▸ 컨트롤 갤러리)와
            // ⌘/Ctrl+G로 열 수 있으니, 상시 노출할 임시 검수용 항목은 툴바를 차지할 이유가 없다.
        ]),
        closed_peers: std::collections::HashSet::new(),
        icon: winit::window::Icon::from_rgba(
            nbeep_ui::brand::ICON_RGBA.to_vec(),
            nbeep_ui::brand::ICON_SIZE,
            nbeep_ui::brand::ICON_SIZE,
        )
        .ok(),
        picker_view: None,
        ime_composing: false,
        pending_jamo: None,
        primary_down: false,
        shift_down: false,
        proxy,
        adding: None,
        shutdown,
    };
    app.reload_faces(); // 고정폭 등 슬롯 얼굴 초기 로드
    event_loop.run_app(&mut app).unwrap();
}
