//! **GUI 앱 셸** — 창 생성·이벤트 루프·위젯 배선(구 `main.rs`의 `app_window` 모듈).
//!
//! 조립 지점의 창 계층. 도메인은 `nbeep-core`, 렌더는 `nbeep-ui`가 갖고 여기는
//! **winit ↔ 위젯 번역 + 창 생명주기**만 맡는다. 창 코드의 `nbeep-plat` 이관은 M3-2.

use std::collections::{HashMap, VecDeque};
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
    /// `via_addr` = 수동 등록(DR-19)으로 성립했을 때 그 주소 — 재연결용으로 기억한다.
    Outbound {
        session: Box<InboundSession>,
        via_addr: Option<String>,
        /// **클릭한 상대**(연결 시도 래치를 넣을 때 쓴 키 — `ConnectLatch` 참조).
        /// 핸드셰이크로 밝혀진 신원과 다를 수 있어(주소 재사용) 따로 나른다. 수동 등록은 `None`.
        intent: Option<PeerId>,
    },
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
        /// `.beepq` 경로 — 이미지면 썸네일(imgdec 격리 디코드 · M4-5ⓑ) 시도용.
        qpath: String,
    },
    /// 전송 실패·거절 — 사유 문장(표시 전용).
    XferFailed { peer: PeerId, why: String },
    /// 상대가 **수락**했다 — 대기 창을 닫고 스트리밍이 시작된다.
    XferAccepted { peer: PeerId },
    /// 발신 1건 **전송 끝(청크+Done 발신)** — 큐의 다음 파일로 넘어가되, UI는 아직 "완료"가
    /// 아니라 **확인 대기**다(수신 ack `XferAcked`가 와야 완료 · M4-9).
    XferSendDone { peer: PeerId },
    /// **수신 종단 확인**(M4-9) — 상대가 격리까지 마쳤(`ok=true`)거나 실패(`ok=false`)했다.
    XferAcked { peer: PeerId, ok: bool },
    /// 수동 주소 연결 실패(DR-19 · M2-8 잔여 — 워커에서 돌아온다. 성공은 `Outbound`).
    AddFailed { addr: String, why: String },
    /// 상대가 내 프로필을 요청(M3-17) — 응답 구성은 메인 스레드가 한다(공개 정책
    /// 판단 단일 지점 — TOFU와 같은 문법).
    ProfileRequested { peer: PeerId },
    /// 상대 프로필 도착(M3-17) — 켠 필드만 실려 온다. 이미지는 바이트 그대로
    /// (디코드는 M4-5 imgdec 몫 — 여기서는 캐시만).
    PeerProfile {
        peer: PeerId,
        name: Option<String>,
        email: Option<String>,
        phone: Option<String>,
        image: Option<Vec<u8>>,
    },
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
    /// Control 스트림으로 보낼 인코딩된 프레임들(프로필 요청/응답 — M3-17).
    /// 내용 구성은 메인 스레드 몫(공개 정책 판단 단일 지점) — 액터는 나르기만 한다.
    Control(Vec<Vec<u8>>),
}

/// 연결 시도 래치(M2-8 중복 클릭 가드) — **넣은 키로 뺀다.**
///
/// ⚠️ 08-13에 이걸로 한 번 물렸다. 넣을 때는 **클릭한 상대**(목록 행의 `PeerId`)로 넣고
/// 뺄 때는 **핸드셰이크로 밝혀진 상대**로 뺐는데, 이 둘은 **다를 수 있다.** 수동 주소
/// 폴백(`manual_addrs`)은 *주소*로 붙기 때문에, 그 주소의 상대가 신원을 새로 만들면
/// (컨테이너 재기동 — 실측: 실행마다 `me=`가 바뀐다) 성립한 세션은 **다른 `PeerId`** 다.
/// 그러면 클릭한 쪽 키가 래치에 영원히 남아 그 행은 **다시는 열리지 않는다**
/// ("연결 중… (이미 시도 중)"이 계속 뜬다 — 재시작 전까지 회복 불가).
#[derive(Debug, Default)]
struct ConnectLatch(std::collections::HashSet<PeerId>);

impl ConnectLatch {
    /// 시도 시작 — 이미 진행 중이면 `false`(중복 클릭 가드).
    fn begin(&mut self, peer: PeerId) -> bool {
        self.0.insert(peer)
    }

    /// 시도 종료 — **클릭한 상대**(`intent`)와 **성립한 상대**(`actual`) 둘 다 해제한다.
    /// 둘을 함께 지우는 것이 이 타입의 존재 이유다(위 ⚠️).
    fn finish(&mut self, intent: Option<PeerId>, actual: Option<PeerId>) {
        for p in [intent, actual].into_iter().flatten() {
            self.0.remove(&p);
        }
    }

    fn contains(&self, peer: PeerId) -> bool {
        self.0.contains(&peer)
    }

    /// 신원 교체(백업 복원 등) — 진행 중 시도는 전부 무효.
    fn clear(&mut self) {
        self.0.clear();
    }
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

/// 상대 프로필(M3-17 — 세션 경유 수신 · ADR-0008). 켠 필드만 온다.
#[derive(Debug, Default)]
struct PeerProfile {
    /// 프로필 표시 이름(무해화 완료) — 있으면 발견 이름보다 우선 표시.
    name: Option<nbeep_core::DisplayName>,
    /// 이메일(옵트인).
    email: Option<String>,
    /// 전화번호(옵트인).
    phone: Option<String>,
    /// 캐시된 이미지 파일(`data/profiles/…`).
    image_file: Option<std::path::PathBuf>,
    /// imgdec 격리 디코드 결과(원형 마스크 완료 · M4-5) — 목록·카드가 그린다.
    avatar: Option<std::rc::Rc<nbeep_ui::IconImage>>,
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
        // 프로필 이미지 조립 상태(M3-17) — (텍스트 필드, 기대 총량, 누적 바이트).
        type PendingProfile = (
            (Option<String>, Option<String>, Option<String>),
            u32,
            Vec<u8>,
        );
        let mut pending_profile: Option<PendingProfile> = None;
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
                    SessionCmd::Control(frames) => {
                        let mut r = Ok(());
                        for f in frames {
                            r = session.send(StreamId::Control, &f);
                            if r.is_err() {
                                break;
                            }
                        }
                        r
                    }
                };
                if sent.is_err() {
                    let _ = proxy.send_event(AppEvent::Closed { peer });
                    return;
                }
            }
            // ★ 수신은 **스트림별이 아니라 도착 순서로** 뽑는다(08-13 — 스트림별 폴링은
            // 파일 전송 2MiB 지점에서 Backpressure로 끊겼다. 근거 = `MuxSession::recv_any`).
            let (stream, bytes) = match session.recv_any() {
                Ok(v) => v,
                Err(nbeep_core::SessionError::TimedOut) => continue, // 정상 — 송신 교대로
                Err(_) => {
                    let _ = proxy.send_event(AppEvent::Closed { peer });
                    return;
                }
            };
            match stream {
                // 대화 수신.
                StreamId::Chat => {
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
                // 프로필 수신(Control — M3-17). 요청은 메인으로 올리고(정책 단일 지점),
                // 응답은 여기서 조립해 완성본만 올린다(대용량 조립이 메인을 막지 않게).
                StreamId::Control => match nbeep_core::ProfileMsg::decode(&bytes) {
                    Some(nbeep_core::ProfileMsg::Request) => {
                        if proxy
                            .send_event(AppEvent::ProfileRequested { peer })
                            .is_err()
                        {
                            return;
                        }
                    }
                    Some(nbeep_core::ProfileMsg::Info {
                        name,
                        email,
                        phone,
                        image_len,
                    }) => {
                        let len = image_len as usize;
                        if len == 0 || len > nbeep_core::PROFILE_IMAGE_MAX {
                            // 이미지 없음 또는 상한 초과 주장 — 텍스트만 반영(fail-closed).
                            let _ = proxy.send_event(AppEvent::PeerProfile {
                                peer,
                                name,
                                email,
                                phone,
                                image: None,
                            });
                            pending_profile = None;
                        } else {
                            pending_profile =
                                Some(((name, email, phone), image_len, Vec::with_capacity(len)));
                        }
                    }
                    Some(nbeep_core::ProfileMsg::ImageChunk {
                        offset,
                        last,
                        bytes,
                    }) => {
                        if let Some((fields, want, buf)) = &mut pending_profile {
                            let in_order = buf.len() == offset as usize
                                && buf.len() + bytes.len() <= nbeep_core::PROFILE_IMAGE_MAX;
                            if in_order {
                                buf.extend_from_slice(&bytes);
                            }
                            if !in_order || last {
                                // 종결 — 정합(순서·총량 일치)일 때만 이미지 채택.
                                let (name, email, phone) = fields.clone();
                                let ok = in_order && last && buf.len() == *want as usize;
                                let image = ok.then(|| std::mem::take(buf));
                                let _ = proxy.send_event(AppEvent::PeerProfile {
                                    peer,
                                    name,
                                    email,
                                    phone,
                                    image,
                                });
                                pending_profile = None;
                            }
                        }
                    }
                    None => {} // 미지 kind — 전방 호환 무시
                },
                // 파일 수신.
                StreamId::File => {
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
                    // ★ 종단 확인(M4-9) — 격리까지 성공했으니 발신자에게 Received를 돌려준다.
                    //    이걸 받아야 상대의 "완료"가 참이 된다("보냈다"≠"닿았다").
                    let _ = session.send(StreamId::File, &XferMsg::Received { id }.encode());
                    let _ = proxy.send_event(AppEvent::XferDone {
                        peer,
                        name: q.name,
                        risk: q.risk,
                        mismatch: q.mismatch,
                        qpath: q.path.to_string_lossy().into_owned(),
                    });
                }
                Err(e) => {
                    // 수신측 실패 — 발신자가 거짓 완료를 남기지 않게 Failed를 되돌린다.
                    let _ = session.send(StreamId::File, &XferMsg::Failed { id }.encode());
                    fail(format!("{e}"));
                }
            },
            Err(e) => {
                let _ = session.send(StreamId::File, &XferMsg::Failed { id }.encode());
                fail(format!("완료 실패: {e} — 폐기"));
            }
        },
        // ★ 발신측이 받는 종단 확인(M4-9) — 확인 대기 항목을 완료/실패로 닫는다.
        Ok(XferMsg::Received { .. }) => {
            let _ = proxy.send_event(AppEvent::XferAcked { peer, ok: true });
        }
        Ok(XferMsg::Failed { .. }) => {
            let _ = proxy.send_event(AppEvent::XferAcked { peer, ok: false });
        }
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
    /// 프로필 변경 화면(M3-17 — 이미지·이름·연락처 + 공개 토글).
    Profile,
    /// 상대 프로필 보기 카드(M3-17 — 목록 우클릭 ▸ 프로필 보기).
    PeerInfo(PeerId),
    /// 격리함 — 수신 파일 승인·삭제(M4-3 · [docs/11] §7 등급별 마찰).
    Quarantine,
    /// 발신 대기 — 상대 승인을 기다리는 창(타임아웃 후 자동 취소).
    Sending(PeerId),
    /// 수신 승인 — 제안 정보를 보여 주고 결정을 받는 창(타임아웃 = 취소).
    Approve(PeerId),
    /// 주소 직접 입력 모달(DR-19 · M3-16 — `⌘/Ctrl+K`·툴바 +).
    AddEndpoint,
    /// 경고 모달(08-13 — 상태바 한 줄로는 지나치는 실패를 눈앞에 세운다).
    Alert,
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

/// 파일 선택 창의 용도(M2-5a 백업·복원 확장) — 같은 Role::Picker 창을 용도별로 쓴다.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum PickerPurpose {
    /// 갤러리 Choose… 실증(기존 — HOME 평면 목록).
    GallerySample,
    /// 신원 키 **백업 폴더** 선택 — 폴더 탐색 + "여기에 저장".
    BackupDir,
    /// 신원 키 **백업 파일** 선택(복원) — 폴더 탐색 + 파일 선택.
    RestoreKey,
    /// 프로필 이미지 선택(M3-17) — 폴더 탐색 + 이미지 파일만.
    ProfileImage,
}

/// 탐색형 피커의 행 하나가 뜻하는 것(라벨 → 행위 매핑).
#[derive(Clone, Debug)]
enum PickEntry {
    /// 상위 폴더로.
    Up,
    /// 하위 폴더 진입.
    Dir(std::path::PathBuf),
    /// 파일 선택(복원 대상).
    File(std::path::PathBuf),
    /// 현재 폴더에 저장(백업).
    SaveHere,
}

/// 열린 피커의 상태 — 용도·현재 폴더·라벨→행위 매핑.
#[derive(Debug)]
struct PickerCtx {
    purpose: PickerPurpose,
    dir: std::path::PathBuf,
    entries: Vec<(String, PickEntry)>,
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
    connecting: ConnectLatch,
    discovery: std::sync::mpsc::Receiver<nbeep_net::DiscoveryEvent>,
    table: nbeep_core::PeerTable,
    /// TOFU 신뢰 저장 — 파일 영속(M2-5a · 변경 즉시 암호화 저장 · R-17 해소).
    trust: nbeep_store::FileTrustStore,
    /// 상대별 대화 상태 — 뷰와 무관하게 유지(동시 대화의 실체).
    conversations: HashMap<PeerId, Conversation>,
    dedup: nbeep_core::DedupIndex,
    started: Instant,
    /// 주 창 하단 상태바 문구.
    status: String,
    /// 설정 값(런타임). `chat.window_mode`·`ui.theme`·`font.*`.
    settings: SettingsState,
    /// 설정 영속(M3-15 · ADR-0011) — 변경은 mark만, 저장은 tick/종료 flush가 1회씩.
    conf: nexa_conf::Store,
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
    /// 피커 용도·탐색 상태(M2-5a 백업·복원 — None = 갤러리 실증 모드).
    picker_ctx: Option<PickerCtx>,
    /// 설정 화면에서 요청된 피커 열기(이벤트 루프 참조가 없는 지점 → about_to_wait에서 연다).
    pending_picker: Option<PickerPurpose>,
    /// 데이터 디렉터리(신원 키·핀 세그먼트·설정 — 백업·복원이 원본 위치로 쓴다).
    data_dir: std::path::PathBuf,
    /// 실물 전송 여부 — 신원 복원 핫 로딩 시 전송 재시작 판단(데모는 교체만).
    live: bool,
    /// 실제로 듣고 있는 세션 포트(live 한정 · 선호 포트 점유 시 폴백 값) — 상태바 우측 표시.
    /// 발견이 닿지 않는 상대에게 사람이 알려줄 값이라 **화면에 항상 보여야 한다**(DR-19).
    listen_port: Option<u16>,
    /// 프로필 변경 화면 뷰(M3-17 — 열려 있을 때만 Some).
    profile_view: Option<nbeep_ui::ProfileWidget>,
    /// 상대 프로필(M3-17 — 세션 경유 수신 · 목록·제목 표시 우선).
    peer_profiles: HashMap<PeerId, PeerProfile>,
    /// 상대 프로필 보기 뷰(우클릭 ▸ 프로필 보기 — 열려 있을 때만 Some).
    peer_info_view: Option<nbeep_ui::PeerInfoWidget>,
    /// 주소 입력 모달 뷰(DR-19 · M3-16 — 열려 있을 때만 Some).
    addr_view: Option<nbeep_ui::AddrPromptWidget>,
    /// About 뷰(열려 있을 때만 Some).
    about_view: Option<AboutWidget>,
    /// 경고 모달 뷰(열려 있을 때만 Some).
    alert_view: Option<nbeep_ui::AlertWidget>,
    /// 열어야 할 경고(제목, 본문) — 이벤트 루프 참조가 없는 지점에서 요청되면
    /// `about_to_wait`가 연다(pending_picker와 같은 패턴).
    pending_alert: Option<(String, String)>,
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
    /// 발신했으나 **수신 종단 확인(ack) 대기 중**인 건수(상대별 · M4-9). 종료 가드가 본다.
    awaiting_ack: HashMap<PeerId, u32>,
    /// 종료 가드 — 미확인 전송이 있을 때 첫 닫기는 경고만, 두 번째로 확정(파괴적 확인 문법).
    close_armed: bool,
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
    /// **비발견 상대**(수동 등록·다른 서브넷 인바운드 — 사용자 실기 08-13) — 발견
    /// 테이블에 ANNOUNCE가 안 닿아도 목록에 유지한다(대화창을 닫으면 사라지던 버그).
    /// 이름은 성립 시 스냅샷(프로필 이름은 2줄째로 따로 표시된다).
    extra_peers: HashMap<PeerId, nbeep_core::DisplayName>,
    /// 성공한 수동 등록 주소(DR-19) — 세션이 끊긴 뒤 목록에서 다시 클릭하면
    /// 발견 주소록에 없어도 이 주소로 재연결한다.
    manual_addrs: HashMap<PeerId, String>,
    /// 읽지 않은 수신 메시지 수(③ — 대화 뷰가 닫혀 있는 동안 도착한 것).
    unread: HashMap<PeerId, u32>,
    /// 그 대화를 마지막으로 확인한 시각(뷰가 열려 있던 마지막 순간 — 목록에 표시).
    last_read: HashMap<PeerId, nbeep_ui::WallTime>,
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

    /// 경고 모달을 연다(08-13) — 이미 열려 있으면 내용만 바꾸고 앞으로 가져온다.
    /// 항상 위(AlwaysOnTop) — 상태바 한 줄로는 지나치는 실패를 눈앞에 세우는 것이 목적.
    fn open_alert(&mut self, el: &ActiveEventLoop, title: &str, message: &str) {
        if let Some((aid, _)) = self.windows.iter().find(|(_, e)| e.role == Role::Alert) {
            let aid = *aid;
            let mut inv = Invalidations::default();
            if let Some(av) = &mut self.alert_view {
                av.set_content(title, message, &mut inv);
            }
            if let Some(e) = self.windows.get(&aid) {
                e.window.focus_window();
            }
            self.request_redraw(aid);
            return;
        }
        let attrs = Window::default_attributes()
            .with_title("Nexa Beep — 알림")
            .with_inner_size(winit::dpi::LogicalSize::new(400.0, 170.0))
            .with_resizable(false)
            .with_window_level(winit::window::WindowLevel::AlwaysOnTop)
            .with_window_icon(self.icon.clone());
        let window = Rc::new(el.create_window(attrs).unwrap());
        let scale = window.scale_factor() as f32;
        let context = softbuffer::Context::new(window.clone()).unwrap();
        let surface = SbSurface::new(&context, window.clone()).unwrap();
        let id = window.id();
        self.windows.insert(
            id,
            WinEntry {
                role: Role::Alert,
                window,
                surface,
                cursor: (0, 0),
                scale,
            },
        );
        self.alert_view = Some(nbeep_ui::AlertWidget::new(title, message));
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
                // 모달로 세운다(08-13 사용자 실기 — 상태바 한 줄은 지나쳐서 "그냥 전송이
                // 안 된다"로 보였다). 창 생성은 이벤트 루프 참조가 있는 about_to_wait 몫.
                self.status = format!("보낼 수 없습니다 — {}", reason.message());
                // 사유별로 "그래서 뭘 하면 되는지"까지 — 모달의 존재 이유다.
                let how = match reason {
                    nbeep_core::DenyReason::NoMutualConversation => {
                        "\n\n이 대화에서 메시지를 서로 한 번씩 주고받으면 파일 전송이 열립니다(스팸 방어 — 상호 확인 규칙)."
                    }
                    nbeep_core::DenyReason::NotPinned => {
                        "\n\n상대와 연결(세션)이 성립하면 신원이 고정됩니다 — 목록에서 상대를 열어 연결부터 하세요."
                    }
                    nbeep_core::DenyReason::Blocked => "",
                };
                self.pending_alert = Some((
                    "파일을 보낼 수 없습니다".into(),
                    format!("{}{how}", reason.message()),
                ));
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
                    self.conf_mark();
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
        if !mine {
            // 파일 수신도 "새 소식"이다(③) — 뷰가 닫혀 있으면 배지·제목으로.
            self.note_incoming(peer);
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

    /// 확인 대기(발신) 항목을 수신 ack로 종결한다(M4-9 — `Received`→완료 · `Failed`→실패).
    fn ack_xfer_line(&mut self, peer: PeerId, terminal: nbeep_ui::XferLineState) {
        if let Some(conv) = self.conversations.get_mut(&peer) {
            nbeep_ui::update_xfer_ack(&mut conv.lines, true, terminal.clone());
        }
        let mut inv = Invalidations::default();
        if let Some(chat) = self.chats.get_mut(&peer) {
            chat.ack_xfer_line(true, terminal, &mut inv);
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
            self.conf_mark();
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
        // 발견 목록 + 비발견 상대(④ — 수동 등록·다른 서브넷 인바운드) 병합.
        // 발견이 나중에 닿으면 테이블 항목이 이긴다(같은 PeerId는 한 행).
        let mut entries = self.table.list();
        for (&peer, name) in &self.extra_peers {
            if self.table.get(peer).is_none() {
                entries.push(nbeep_core::PeerEntry {
                    peer,
                    name: name.clone(),
                    paths: 0, // 발견 경로 없음 — 세션·수동 주소로만 닿는 상대
                });
            }
        }
        entries.sort_by(|a, b| {
            a.name
                .as_str()
                .cmp(b.name.as_str())
                .then(a.peer.cmp(&b.peer))
        });
        let rows = entries
            .into_iter()
            .map(|entry| {
                // 프로필 이름은 **2번째 줄**(사용자 확정 08-11 — 굵은 1줄은 언제나
                // 기본(발견) 이름 · 신원은 여전히 키).
                let profile_name = self
                    .peer_profiles
                    .get(&entry.peer)
                    .and_then(|p| p.name.as_ref())
                    .map(|n| n.as_str().to_string());
                let avatar = self
                    .peer_profiles
                    .get(&entry.peer)
                    .and_then(|p| p.avatar.clone());
                let trust = self.trust.level(entry.peer);
                // 세션 상태 점(사용자 요청): 대화 중=Active · 끊김 기록=Lost · 그 외=Idle.
                let link = if self.conversations.contains_key(&entry.peer) {
                    LinkState::Active
                } else if self.connecting.contains(entry.peer) {
                    LinkState::Connecting // 워커가 connect+Noise 진행 중(M2-8)
                } else if self.closed_peers.contains(&entry.peer) {
                    LinkState::Lost
                } else {
                    LinkState::Idle
                };
                // 진행 중 전송이 있으면 이름 아래 막대로 보인다(슬라이스 4에서 채운다).
                let xfer = self.xfer_progress.get(&entry.peer).copied();
                // 읽지 않은 메시지 배지(③) — 개수 + 마지막 확인 시각(있을 때만).
                let unread = self.unread.get(&entry.peer).copied().unwrap_or(0);
                let last_read = (unread > 0)
                    .then(|| {
                        self.last_read.get(&entry.peer).map(|w| {
                            nbeep_ui::fmt_hm(*w, self.settings.get("chat.time_24h") != "off")
                        })
                    })
                    .flatten();
                PeerRow {
                    entry,
                    trust,
                    link,
                    xfer,
                    profile_name,
                    avatar,
                    unread,
                    last_read,
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
        if !self.connecting.begin(peer) {
            self.status = format!("연결 중… {} (이미 시도 중)", self.peer_title(peer));
            return; // 중복 클릭 가드
        }
        self.status = format!("연결 중… {}", self.peer_title(peer));
        // 목록 행 점을 즉시 "연결 중"(강조색)으로(M2-8 잔여).
        let mut inv = Invalidations::default();
        self.refresh_rows(&mut inv);
        let transport = std::sync::Arc::clone(&self.transport);
        let identity = std::sync::Arc::clone(&self.identity);
        let proxy = self.proxy.clone();
        // 비발견 상대(수동 등록 이력)는 발견 주소록이 비어 connect가 실패한다 —
        // 성공했던 수동 주소로 폴백(08-13 실기: 대화창 닫은 뒤 재연결이 이 경로).
        let manual = self.manual_addrs.get(&peer).cloned();
        std::thread::spawn(move || {
            let conn = match transport.connect(peer) {
                Ok(link) => Ok(link),
                Err(e) => match &manual {
                    Some(addr) => transport.add_endpoint(addr).map_err(|e2| format!("{e2:?}")),
                    None => Err(format!("{e:?}")),
                },
            };
            let r = conn.and_then(|link| {
                nbeep_crypto::NoiseSession::initiate(link, &identity).map_err(|e| e.to_string())
            });
            let _ = match r {
                Ok(session) => proxy.send_event(AppEvent::Outbound {
                    session: Box::new(InboundSession { session }),
                    via_addr: None, // 수동 주소는 이미 기억돼 있다(성공 시 갱신 불요)
                    intent: Some(peer), // ★ 래치는 **넣은 키로** 뺀다
                }),
                Err(why) => proxy.send_event(AppEvent::ConnectFailed { peer, why }),
            };
        });
    }

    /// 수동 입력 확정(DR-19 · 모달에서 형식 검증 후 호출) — **워커 스레드**에서
    /// add_endpoint(해석·순차 연결·최대 수 초)+Noise를 수행한다(M2-8 잔여 이관 08-11 —
    /// 죽은 주소를 넣어도 UI가 멈추지 않는다). 성공 = `AppEvent::Outbound`(발견 경로와
    /// 같은 합류점 — TOFU 판정·대화 등록·뷰 열기) · 실패 = `AppEvent::AddFailed`.
    fn commit_manual_add(&mut self, addr: String, _el: &ActiveEventLoop) {
        let addr = addr.trim().to_string();
        if addr.is_empty() {
            return;
        }
        self.status = format!("연결 중… {addr}");
        let transport = std::sync::Arc::clone(&self.transport);
        let identity = std::sync::Arc::clone(&self.identity);
        let proxy = self.proxy.clone();
        std::thread::spawn(move || {
            let r = transport
                .add_endpoint(&addr)
                .map_err(|e| format!("{e:?}"))
                .and_then(|link| {
                    nbeep_crypto::NoiseSession::initiate(link, &identity).map_err(|e| e.to_string())
                });
            let _ = match r {
                Ok(session) => proxy.send_event(AppEvent::Outbound {
                    session: Box::new(InboundSession { session }),
                    via_addr: Some(addr), // 성공한 수동 주소 — 재연결용 기억(④)
                    intent: None,         // 수동 등록은 래치를 쓰지 않는다
                }),
                Err(why) => proxy.send_event(AppEvent::AddFailed { addr, why }),
            };
        });
        if let Some(mid) = self.main_id {
            self.request_redraw(mid);
        }
    }

    fn peer_title(&self, peer: PeerId) -> String {
        // 프로필 이름 우선(M3-17 — 본인이 공개한 이름) · 없으면 발견 이름.
        if let Some(p) = self.peer_profiles.get(&peer) {
            if let Some(n) = &p.name {
                return n.as_str().to_string();
            }
        }
        if let Some(e) = self.table.get(peer) {
            return e.name.as_str().to_string();
        }
        // 비발견 상대(④) — 성립 시 스냅샷한 이름.
        self.extra_peers
            .get(&peer)
            .map_or_else(|| format!("{peer:?}"), |n| n.as_str().to_string())
    }

    /// 그 창의 포커스된 텍스트 컨트롤에서 선택을 복사한다(① 08-13 — 창 역할로 라우팅).
    fn clipboard_copy_for(&self, id: WindowId) -> Option<String> {
        // 대화 입력창(주 창 단일 모드 포함)이 먼저 — chat_peer_for가 둘 다 안다.
        if let Some(t) = self
            .chat_peer_for(id)
            .and_then(|p| self.chats.get(&p))
            .and_then(ChatViewWidget::copy_selection)
        {
            return Some(t);
        }
        match self.windows.get(&id).map(|e| e.role)? {
            Role::AddEndpoint => self.addr_view.as_ref()?.clipboard_copy(),
            Role::Profile => self.profile_view.as_ref()?.clipboard_copy(),
            Role::Settings => self.settings_view.as_ref()?.clipboard_copy(),
            _ => None,
        }
    }

    /// 잘라내기(①) — 복사와 같은 라우팅, 선택 삭제까지.
    fn clipboard_cut_for(&mut self, id: WindowId) -> Option<String> {
        let mut inv = Invalidations::default();
        if let Some(t) = self
            .chat_peer_for(id)
            .and_then(|p| self.chats.get_mut(&p))
            .and_then(|c| c.cut_selection(&mut inv))
        {
            return Some(t);
        }
        match self.windows.get(&id).map(|e| e.role)? {
            Role::AddEndpoint => self.addr_view.as_mut()?.clipboard_cut(&mut inv),
            Role::Profile => self.profile_view.as_mut()?.clipboard_cut(&mut inv),
            Role::Settings => self.settings_view.as_mut()?.clipboard_cut(&mut inv),
            _ => None,
        }
    }

    /// 붙여넣기(①) — 그 창의 포커스된 텍스트 컨트롤로.
    fn clipboard_paste_for(&mut self, id: WindowId, text: &str) {
        let mut inv = Invalidations::default();
        if let Some(peer) = self.chat_peer_for(id) {
            if let Some(c) = self.chats.get_mut(&peer) {
                c.paste(text, &mut inv);
                return;
            }
        }
        match self.windows.get(&id).map(|e| e.role) {
            Some(Role::AddEndpoint) => {
                if let Some(v) = self.addr_view.as_mut() {
                    v.clipboard_paste(text, &mut inv);
                }
            }
            Some(Role::Profile) => {
                if let Some(v) = self.profile_view.as_mut() {
                    v.clipboard_paste(text, &mut inv);
                }
            }
            Some(Role::Settings) => {
                if let Some(v) = self.settings_view.as_mut() {
                    v.clipboard_paste(text, &mut inv);
                }
            }
            _ => {}
        }
    }

    /// 이 상대의 대화 뷰가 **지금 화면에 있는가**(③ — 읽음/안읽음 판정의 기준).
    fn chat_visible(&self, peer: PeerId) -> bool {
        match self.mode {
            WindowMode::Single => self.single_open == Some(peer),
            WindowMode::Separate => self
                .windows
                .values()
                .any(|e| matches!(e.role, Role::Chat(p) if p == peer)),
        }
    }

    /// 읽음 처리(③) — 뷰를 열거나 닫는 순간, 그리고 뷰가 보이는 동안 호출된다.
    /// "마지막 확인한 시각"은 뷰가 화면에 있던 마지막 순간이다.
    fn mark_read(&mut self, peer: PeerId) {
        let (_, wall) = now_stamp();
        self.last_read.insert(peer, wall);
        if self.unread.remove(&peer).is_some() {
            let mut inv = Invalidations::default();
            self.refresh_rows(&mut inv);
            if let Some(mid) = self.main_id {
                self.request_redraw(mid);
            }
        }
        self.update_main_title();
    }

    /// 수신 1건 계상(③) — 뷰가 보이면 확인 시각만 갱신, 아니면 미확인 +1과
    /// 상태바·제목으로 알린다(OS 알림·소리는 M3-8/ADR-0010 확정 후).
    fn note_incoming(&mut self, peer: PeerId) {
        if self.chat_visible(peer) {
            let (_, wall) = now_stamp();
            self.last_read.insert(peer, wall);
            return;
        }
        let n = {
            let e = self.unread.entry(peer).or_insert(0);
            *e = e.saturating_add(1);
            *e
        };
        self.status = format!("새 메시지: {} (읽지 않음 {n})", self.peer_title(peer));
        let mut inv = Invalidations::default();
        self.refresh_rows(&mut inv);
        self.update_main_title();
        if let Some(mid) = self.main_id {
            self.request_redraw(mid);
        }
    }

    /// 주 창 제목에 미확인 총계(③) — 창이 뒤에 있어도 작업 표시줄·독에서 보인다.
    fn update_main_title(&self) {
        let Some(mid) = self.main_id else { return };
        let Some(e) = self.windows.get(&mid) else {
            return;
        };
        let total: u32 = self.unread.values().sum();
        if total > 0 {
            e.window
                .set_title(&format!("Nexa Beep — 새 메시지 {total}"));
        } else {
            e.window.set_title("Nexa Beep");
        }
    }

    /// 내 프로필 응답 프레임(M3-17) — **공개 정책 판단은 여기 한 곳**(메인 스레드).
    /// 켠 필드만 싣고, 이미지는 기본정보 공개가 켜져 있고 상한 이내일 때만 청크로 잇는다.
    fn my_profile_frames(&self) -> Vec<Vec<u8>> {
        use nbeep_core::{ProfileMsg, PROFILE_IMAGE_CHUNK, PROFILE_IMAGE_MAX};
        let on = |k: &str| self.settings.get(k) == "on";
        let share_basic = on("profile.share.basic");
        let name = share_basic.then(|| {
            effective_display_name(&self.settings, &self.identity.peer_id())
                .as_str()
                .to_string()
        });
        let email = (on("profile.share.email"))
            .then(|| self.settings.get("profile.email").to_string())
            .filter(|s| !s.is_empty());
        let phone = (on("profile.share.phone"))
            .then(|| self.settings.get("profile.phone").to_string())
            .filter(|s| !s.is_empty());
        // 이미지 — 기본정보 공개 + 파일 존재 + 상한 이내(초과는 조용히 생략 · 텍스트는 나감).
        let image = share_basic
            .then(|| self.settings.get("profile.image_path").to_string())
            .filter(|p| !p.is_empty())
            .and_then(|p| std::fs::read(p).ok())
            .filter(|b| !b.is_empty() && b.len() <= PROFILE_IMAGE_MAX);
        let image_len = image.as_ref().map_or(0u32, |b| {
            u32::try_from(b.len()).unwrap_or(0) // MAX 256KiB라 실패 불가
        });
        let mut frames = vec![ProfileMsg::Info {
            name,
            email,
            phone,
            image_len,
        }
        .encode()];
        if let Some(bytes) = image {
            let mut off = 0usize;
            while off < bytes.len() {
                let end = (off + PROFILE_IMAGE_CHUNK).min(bytes.len());
                frames.push(
                    ProfileMsg::ImageChunk {
                        offset: u32::try_from(off).unwrap_or(u32::MAX),
                        last: end == bytes.len(),
                        bytes: bytes[off..end].to_vec(),
                    }
                    .encode(),
                );
                off = end;
            }
        }
        frames
    }

    /// 수립된 세션을 액터로 옮기고 대화 상태를 등록한다(아웃바운드·인바운드 공용).
    fn install_conversation(&mut self, session: LiveSession) {
        let peer = session.peer();
        let (out_tx, out_rx) = std::sync::mpsc::channel();
        spawn_session_actor(session, out_rx, self.proxy.clone(), self.send_rate);
        // 프로필 자동 프리페치(M3-17 · ADR-0008) — 세션이 섰으니 요청 1회.
        // 상대가 전부 비공개면 빈 응답이 온다(그래도 요청은 무해).
        let _ = out_tx.send(SessionCmd::Control(vec![
            nbeep_core::ProfileMsg::Request.encode()
        ]));
        self.conversations.insert(
            peer,
            Conversation {
                out_tx,
                lines: Vec::new(),
            },
        );
        // 세션 재수립 = 끊김 상태 해제(목록 점 Active로).
        self.closed_peers.remove(&peer);
        // 비발견 상대(수동 등록·다른 서브넷 인바운드)는 발견 테이블에 없다 — 목록에
        // 유지할 이름을 스냅샷(④ 실기 08-13: 대화창을 닫으면 목록에서 사라지던 버그).
        if self.table.get(peer).is_none() {
            self.extra_peers
                .entry(peer)
                .or_insert_with(|| nbeep_core::default_display_name(None, &peer));
        }
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
                    // 이미지 미리보기(M4-5ⓑ) — 픽셀은 imgdec(격리)가 만든다.
                    // 이미지가 아니면 조용히 없음.
                    thumb: crate::imgdec::thumb_from_beepq(&p, 64).map(std::rc::Rc::new),
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

    /// 사용자 홈 폴더(탐색 시작점) — Windows `USERPROFILE` · Unix `HOME`.
    fn home_dir() -> std::path::PathBuf {
        std::env::var_os("USERPROFILE")
            .or_else(|| std::env::var_os("HOME"))
            .map(std::path::PathBuf::from)
            .or_else(|| std::env::current_dir().ok())
            .unwrap_or_default()
    }

    /// 백업 파일 기본 이름(추천) — 지문이 들어가 어느 신원의 백업인지 파일명으로 구분된다.
    fn default_backup_name(&self) -> String {
        format!("nexa-beep-identity-{}.key", self.identity.peer_id().short())
    }

    /// 탐색형 피커 목록 구성 — (창 제목, 트리 행, 라벨→행위). 라벨 접두로 종류를
    /// 구분한다(글리프 폴백만으로 충분한 문자 — 이모지 금지).
    fn picker_listing(
        purpose: PickerPurpose,
        dir: &std::path::Path,
        save_name: &str,
    ) -> (String, Vec<nbeep_ui::TreeNode>, Vec<(String, PickEntry)>) {
        let mut entries: Vec<(String, PickEntry)> = Vec::new();
        if purpose == PickerPurpose::BackupDir {
            entries.push((format!("[여기에 저장] {save_name}"), PickEntry::SaveHere));
        }
        if let Some(parent) = dir.parent().filter(|p| !p.as_os_str().is_empty()) {
            let _ = parent;
            entries.push(("[..] 상위 폴더".to_string(), PickEntry::Up));
        }
        let mut dirs: Vec<(String, std::path::PathBuf)> = Vec::new();
        let mut files: Vec<(String, std::path::PathBuf)> = Vec::new();
        if let Ok(rd) = std::fs::read_dir(dir) {
            for e in rd.flatten() {
                let name = e.file_name().to_string_lossy().into_owned();
                if name.starts_with('.') {
                    continue; // 숨김 항목 — 백업 대상지로 부적합
                }
                match e.file_type() {
                    Ok(t) if t.is_dir() => dirs.push((name, e.path())),
                    Ok(t) if t.is_file() => {
                        let take = match purpose {
                            PickerPurpose::RestoreKey => true,
                            PickerPurpose::ProfileImage => {
                                let lower = name.to_ascii_lowercase();
                                ["png", "jpg", "jpeg", "gif", "bmp", "webp", "ico"]
                                    .iter()
                                    .any(|ext| lower.ends_with(&format!(".{ext}")))
                            }
                            _ => false,
                        };
                        if take {
                            files.push((name, e.path()));
                        }
                    }
                    _ => {}
                }
            }
        }
        dirs.sort_by(|a, b| a.0.cmp(&b.0));
        files.sort_by(|a, b| a.0.cmp(&b.0));
        for (name, p) in dirs {
            entries.push((format!("[폴더] {name}"), PickEntry::Dir(p)));
        }
        for (name, p) in files {
            entries.push((name, PickEntry::File(p)));
        }
        let title = match purpose {
            PickerPurpose::BackupDir => format!("백업 폴더 선택 — {}", dir.display()),
            PickerPurpose::RestoreKey => format!("백업 파일 선택 — {}", dir.display()),
            PickerPurpose::ProfileImage => format!("프로필 이미지 선택 — {}", dir.display()),
            PickerPurpose::GallerySample => String::new(),
        };
        let roots = entries
            .iter()
            .map(|(label, _)| nbeep_ui::TreeNode::leaf(label.clone()))
            .collect();
        (title, roots, entries)
    }

    /// 파일 선택 **모달 창**(Choose… · ChoosePicker 어댑터 내용을 별도 창으로 · 사용자 확정).
    /// 항목 클릭 = 선택 확정(값 반영) 후 닫힘 · Esc/닫기 = 취소.
    /// M2-5a: `purpose`에 따라 갤러리 실증(평면) / 백업·복원(폴더 탐색)으로 갈린다.
    fn open_picker(&mut self, el: &ActiveEventLoop, purpose: PickerPurpose) {
        if let Some((pid, _)) = self.windows.iter().find(|(_, e)| e.role == Role::Picker) {
            if let Some(e) = self.windows.get(pid) {
                e.window.focus_window();
            }
            return;
        }
        let (title, roots) = if purpose == PickerPurpose::GallerySample {
            // 어댑터: HOME 단일 파일 선택기(ChoosePicker 인터페이스 — 어떤 구현도 가능).
            let picker = FilePicker {
                dir: Self::home_dir(),
            };
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
            self.picker_ctx = None;
            (title, roots)
        } else {
            let dir = Self::home_dir();
            let (title, roots, entries) =
                Self::picker_listing(purpose, &dir, &self.default_backup_name());
            self.picker_ctx = Some(PickerCtx {
                purpose,
                dir,
                entries,
            });
            (title, roots)
        };
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

    /// 탐색형 피커의 폴더 이동 후 재구성 — 트리·제목을 현재 폴더로 갱신.
    fn repopulate_picker(&mut self, id: WindowId) {
        let Some(ctx) = &self.picker_ctx else { return };
        let (title, roots, entries) =
            Self::picker_listing(ctx.purpose, &ctx.dir, &self.default_backup_name());
        if let Some(ctx) = &mut self.picker_ctx {
            ctx.entries = entries;
        }
        let mut tree = nbeep_ui::TreeView::new(nbeep_ui::TreeModel::new(roots));
        {
            use nbeep_ui::Control as _;
            tree.set_focused(true);
        }
        self.picker_view = Some(tree);
        if let Some(e) = self.windows.get(&id) {
            e.window.set_title(&title);
        }
        self.layout_window(id);
        self.request_redraw(id);
    }

    /// 신원 키 백업(M2-5a · 사용자 요청 08-11) — 현재 폴더로 복사. 기본 이름에 지문이
    /// 들어가고, 동명 파일이 있으면 덮어쓰지 않고 번호를 붙인다.
    fn do_backup_identity(&mut self, dir: &std::path::Path) -> String {
        let src = self.data_dir.join("identity.key");
        let base = format!("nexa-beep-identity-{}", self.identity.peer_id().short());
        let mut dst = dir.join(format!("{base}.key"));
        let mut n = 2;
        while dst.exists() {
            dst = dir.join(format!("{base}-{n}.key"));
            n += 1;
        }
        match std::fs::copy(&src, &dst) {
            Ok(_) => format!("신원 키 백업됨 — {} (안전하게 보관하세요)", dst.display()),
            Err(e) => format!("백업 실패: {e}"),
        }
    }

    /// 신원 키 복원 + **핫 로딩**(M2-5a · 사용자 요청 08-11) — 검증 → 원자적 교체 →
    /// 신원·신뢰 저장소·전송을 그 자리에서 갈아끼운다(재시작 불필요).
    /// 이전 신원의 세션·대화는 무효라 닫는다(설정 desc에 고지).
    fn do_restore_identity(&mut self, file: &std::path::Path) -> String {
        // ① 검증 — 매직·길이(keyfile 포맷). 손상 파일로 교체해 버리면 되돌릴 수 없다.
        let bytes = match std::fs::read(file) {
            Ok(b) => b,
            Err(e) => return format!("복원 실패(읽기): {e}"),
        };
        if bytes.len() != 68 || &bytes[..4] != b"NBK1" {
            return "복원 실패: 신원 키 백업 파일이 아닙니다(형식 불일치)".into();
        }
        let mut key = [0u8; 64];
        key.copy_from_slice(&bytes[4..]);
        let restored = nbeep_crypto::Identity::from_key_bytes(&key);
        if restored.peer_id() == self.identity.peer_id() {
            return "이미 사용 중인 신원입니다 — 변경 없음".into();
        }
        // ② 원자적 교체 — temp 기록 후 rename(키 파일 절반 기록 창 방지).
        let dst = self.data_dir.join("identity.key");
        let tmp = dst.with_extension(format!("tmp.{}", std::process::id()));
        if let Err(e) = std::fs::write(&tmp, &bytes) {
            return format!("복원 실패(기록): {e}");
        }
        if let Err(e) = std::fs::rename(&tmp, &dst) {
            let _ = std::fs::remove_file(&tmp);
            return format!("복원 실패(교체): {e}");
        }
        // ③ 핫 로딩 — 신원 → 신뢰 저장소(래핑 원료가 바뀐다) → 대화 정리 → 전송 재시작.
        self.identity = std::sync::Arc::new(restored);
        let (trust, load) = nbeep_store::FileTrustStore::open(
            self.data_dir.join("trust.seg"),
            self.identity.wrap_secret(),
        );
        self.trust = trust;
        self.conversations.clear();
        self.chats.clear();
        self.single_open = None;
        self.connecting.clear();
        let chat_wins: Vec<WindowId> = self
            .windows
            .iter()
            .filter(|(_, e)| matches!(e.role, Role::Chat(_) | Role::Sending(_)))
            .map(|(id, _)| *id)
            .collect();
        for wid in chat_wins {
            self.windows.remove(&wid);
        }
        let mut net_note = "";
        if self.live && self.respawn_transport().is_err() {
            net_note = " · ⚠ 전송 재시작 실패(발견 불가 — 재실행 필요)";
        }
        let mut inv = Invalidations::default();
        self.refresh_rows(&mut inv);
        if let Some(mid) = self.main_id {
            self.request_redraw(mid);
        }
        let trust_note = match load {
            nbeep_store::TrustLoad::Locked => " · ⚠ 신뢰 목록 잠김(이 신원의 백업이 아님)",
            _ => "",
        };
        format!(
            "신원 복원됨 — 새 지문 {} (핫 로딩 완료{trust_note}{net_note})",
            self.identity.peer_id().short()
        )
    }

    /// 프로필 변경 화면을 연다(M3-17 — 툴바 사람 아이콘). 이미 있으면 포커스.
    fn open_profile(&mut self, el: &ActiveEventLoop) {
        if let Some((pid, _)) = self.windows.iter().find(|(_, e)| e.role == Role::Profile) {
            if let Some(e) = self.windows.get(pid) {
                e.window.focus_window();
            }
            return;
        }
        let values = nbeep_ui::ProfileValues {
            display_name: self.settings.get("profile.display_name").to_string(),
            email: self.settings.get("profile.email").to_string(),
            phone: self.settings.get("profile.phone").to_string(),
            image_path: self.settings.get("profile.image_path").to_string(),
            share_basic: self.settings.get("profile.share.basic") == "on",
            share_email: self.settings.get("profile.share.email") == "on",
            share_phone: self.settings.get("profile.share.phone") == "on",
            resolved_name: effective_display_name(&self.settings, &self.identity.peer_id())
                .as_str()
                .to_string(),
            seed: self.identity.peer_id().as_bytes().to_vec(),
            avatar: {
                // 내 사진 미리보기(M4-5) — 저장된 경로를 imgdec로 격리 디코드.
                let p = self.settings.get("profile.image_path");
                (!p.is_empty())
                    .then(|| crate::imgdec::avatar_from_file(std::path::Path::new(p), 256))
                    .flatten()
                    .map(std::rc::Rc::new)
            },
        };
        let attrs = Window::default_attributes()
            .with_title("Nexa Beep — 프로필")
            .with_inner_size(winit::dpi::LogicalSize::new(440.0, 570.0))
            .with_resizable(false)
            // 모달(⑤ 사용자 요청 08-13) — 다른 창에 가려지지 않고 항상 위에서 입력.
            .with_window_level(winit::window::WindowLevel::AlwaysOnTop)
            .with_window_icon(self.icon.clone());
        let window = Rc::new(el.create_window(attrs).unwrap());
        window.set_ime_allowed(true); // 이름·연락처에 한글 입력
        let scale = window.scale_factor() as f32;
        let context = softbuffer::Context::new(window.clone()).unwrap();
        let surface = SbSurface::new(&context, window.clone()).unwrap();
        let id = window.id();
        self.windows.insert(
            id,
            WinEntry {
                role: Role::Profile,
                window,
                surface,
                cursor: (0, 0),
                scale,
            },
        );
        self.profile_view = Some(nbeep_ui::ProfileWidget::new(&values));
        self.layout_window(id);
        self.request_redraw(id);
    }

    /// 상대 프로필 보기 카드(M3-17 — 목록 우클릭). 이미 있으면 갱신·포커스.
    fn open_peer_info(&mut self, peer: PeerId, el: &ActiveEventLoop) {
        let p = self.peer_profiles.get(&peer);
        let info = nbeep_ui::PeerInfo {
            name: self
                .table
                .get(peer)
                .map_or_else(|| format!("{peer:?}"), |e| e.name.as_str().to_string()),
            profile_name: p
                .and_then(|p| p.name.as_ref())
                .map(|n| n.as_str().to_string())
                .unwrap_or_default(),
            email: p.and_then(|p| p.email.clone()).unwrap_or_default(),
            phone: p.and_then(|p| p.phone.clone()).unwrap_or_default(),
            has_image: p.is_some_and(|p| p.image_file.is_some()),
            fingerprint: peer.short(),
            seed: peer.as_bytes().to_vec(),
            avatar: p.and_then(|p| p.avatar.clone()),
        };
        if let Some((wid, _)) = self
            .windows
            .iter()
            .find(|(_, e)| matches!(e.role, Role::PeerInfo(_)))
        {
            let wid = *wid;
            if let Some(e) = self.windows.get_mut(&wid) {
                e.role = Role::PeerInfo(peer);
                e.window.focus_window();
            }
            self.peer_info_view = Some(nbeep_ui::PeerInfoWidget::new(info));
            self.layout_window(wid);
            self.request_redraw(wid);
            return;
        }
        let attrs = Window::default_attributes()
            .with_title("Nexa Beep — 상대 프로필")
            .with_inner_size(winit::dpi::LogicalSize::new(360.0, 380.0))
            .with_resizable(false)
            .with_window_icon(self.icon.clone());
        let window = Rc::new(el.create_window(attrs).unwrap());
        let scale = window.scale_factor() as f32;
        let context = softbuffer::Context::new(window.clone()).unwrap();
        let surface = SbSurface::new(&context, window.clone()).unwrap();
        let id = window.id();
        self.windows.insert(
            id,
            WinEntry {
                role: Role::PeerInfo(peer),
                window,
                surface,
                cursor: (0, 0),
                scale,
            },
        );
        self.peer_info_view = Some(nbeep_ui::PeerInfoWidget::new(info));
        self.layout_window(id);
        self.request_redraw(id);
    }

    /// 프로필 창 닫기(뷰·창 동시 정리).
    fn close_profile(&mut self) {
        self.profile_view = None;
        if let Some((pid, _)) = self.windows.iter().find(|(_, e)| e.role == Role::Profile) {
            let pid = *pid;
            self.windows.remove(&pid);
        }
    }

    /// 주소 직접 입력 모달을 연다(DR-19 · M3-16 — `⌘/Ctrl+K`·툴바 +). 이미 있으면 포커스.
    fn open_add_endpoint(&mut self, el: &ActiveEventLoop) {
        if let Some((aid, _)) = self
            .windows
            .iter()
            .find(|(_, e)| e.role == Role::AddEndpoint)
        {
            if let Some(e) = self.windows.get(aid) {
                e.window.focus_window();
            }
            return;
        }
        let attrs = Window::default_attributes()
            .with_title("Nexa Beep — 주소로 연결")
            .with_inner_size(winit::dpi::LogicalSize::new(380.0, 150.0))
            .with_resizable(false)
            // 모달(⑤ 사용자 요청 08-13) — 다른 창에 가려지지 않고 항상 위에서 입력.
            .with_window_level(winit::window::WindowLevel::AlwaysOnTop)
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
                role: Role::AddEndpoint,
                window,
                surface,
                cursor: (0, 0),
                scale,
            },
        );
        // 포트 생략 시 붙일 기본 = 설정 수신 포트(ⓐ — 조직이 같은 값을 쓰면 IP만으로 붙는다).
        // 내 포트가 0(임의)이면 걸 때의 기본으로 못 쓴다 — 표준 기본 포트로 폴백.
        let dial_default = match session_port_from(&self.settings) {
            0 => nbeep_ui::addr_prompt::DEFAULT_PORT,
            p => p,
        };
        self.addr_view = Some(nbeep_ui::AddrPromptWidget::new(dial_default));
        self.layout_window(id);
        self.request_redraw(id);
    }

    /// 주소 입력 모달을 닫는다.
    fn close_add_endpoint(&mut self) {
        self.addr_view = None;
        if let Some((aid, _)) = self
            .windows
            .iter()
            .find(|(_, e)| e.role == Role::AddEndpoint)
        {
            let aid = *aid;
            self.windows.remove(&aid);
        }
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

    /// 설정 영속 mark — 변경 지점은 플래그만 세운다(직렬화·값 복사 없음 · FR-P-9).
    fn conf_mark(&mut self) {
        self.conf.sched.mark(Instant::now());
    }

    /// 설정 스냅샷 저장 — 주기(tick)·종료(flush) 두 경로가 이 하나를 쓴다(S-2).
    /// 실패는 치명적이지 않다(S-4) — 단 종료 경로 실패는 stderr로 알린다.
    ///
    /// ★ 기간 자동 승인(`xfer.approval` = `timed`)은 **저장하지 않는다**(사용자 확정
    /// 08-09 · TODO M3-15) — 런타임 전용 상태라 저장 시점에 **복귀 대상**(직전 기본
    /// 방식)으로 치환해 기록한다. 재시작하면 기간이 아닌 직전 설정으로 뜬다.
    fn conf_save(&mut self, exiting: bool) {
        use nbeep_core::{ApprovalPolicy, BasicApproval};
        let timed_subst =
            (self.settings.get("xfer.approval") == "timed").then_some(match &self.approval {
                ApprovalPolicy::TimedAuto { revert_to, .. } => match revert_to {
                    BasicApproval::Auto => "auto",
                    BasicApproval::Block => "block",
                    BasicApproval::Manual => "manual",
                },
                _ => "manual",
            });
        let mut pairs = self.settings.known_pairs();
        if let Some(code) = timed_subst {
            if let Some(p) = pairs.iter_mut().find(|(k, _)| *k == "xfer.approval") {
                p.1 = code;
            }
        }
        if let Err(e) = self.conf.save(&pairs) {
            let path = self.conf.path().display();
            if exiting {
                eprintln!("설정 저장 실패(종료 경로) — {path}: {e}");
            } else {
                self.status = format!("설정 저장 실패 — {e} (다음 변경 때 재시도)");
            }
        }
    }

    /// 영속 설정의 부팅 반영(M3-15) — 파생 런타임 상태를 가진 키만.
    /// [`Self::apply_settings`]의 부팅판: status 문구·redraw 없이 상태만 맞춘다
    /// (`ui.language`·`ui.scrollbar_hide`는 run()이 창 생성 전에 이미 반영).
    fn apply_boot_settings(&mut self) {
        use nbeep_core::{ApprovalPolicy, BasicApproval};
        self.rebuild_theme(); // ui.theme + theme.* 색 오버라이드
        if let Ok(ms) = self.settings.get("ui.typeahead_timeout").parse::<u64>() {
            self.list.set_typeahead_timeout(ms);
        }
        let mut inv = Invalidations::default();
        let pos = nbeep_ui::HudPos::from_code(self.settings.get("ui.typeahead_pos"));
        self.list.set_hud_pos(pos, &mut inv);
        self.list
            .set_typeahead_space(self.settings.get("ui.typeahead_space") == "on");
        self.list
            .set_typeahead_special(self.settings.get("ui.typeahead_special") == "on");
        if let Ok(px) = self.settings.get("ui.toolbar_size").parse::<i32>() {
            self.toolbar.set_icon_size(px);
        }
        nbeep_ui::controls::set_control_size_mult(nbeep_ui::controls::control_size_mult_from_code(
            self.settings.get("ui.control_size"),
        ));
        // 파일 수신 승인 — 정상 경로에선 "timed"가 파일에 없다(conf_save가 복귀
        // 대상으로 치환 · 사용자 확정 08-09). 그래도 들어오면(구버전·수기 편집)
        // 시작 시각이 없으니 되살리지 않고 manual로 정규화한다(기간 연장 방지).
        match self.settings.get("xfer.approval") {
            "auto" => self.approval = ApprovalPolicy::Basic(BasicApproval::Auto),
            "block" => self.approval = ApprovalPolicy::Basic(BasicApproval::Block),
            "timed" => {
                self.settings.set("xfer.approval", "manual".to_string());
                self.conf_mark();
            }
            _ => {}
        }
        if let Some(w) =
            nbeep_core::AutoWindow::from_code(self.settings.get("xfer.approval_window"))
        {
            self.approval_window = w;
        }
        self.send_rate = nbeep_core::RateLimit::from_code(self.settings.get("xfer.send_rate"));
        self.recv_rate = nbeep_core::RateLimit::from_code(self.settings.get("xfer.recv_rate"));
        if let Ok(v) = self.settings.get("xfer.timeout_sec").parse::<u64>() {
            self.wait_timeout_sec = v.clamp(5, 3600);
        }
    }

    /// 실물 전송 재시작(신원 복원 핫 로딩 · 수신 포트 변경) — `LocalDirect`를 새로 띄워
    /// 교체한다. 성립해 있는 대화 세션은 **개별 소켓이라 살아남고**, 발견 목록은 상대
    /// 재공지(≤수 초)로 다시 찬다. 실패 시 기존 전송을 그대로 둔다(호출자가 고지).
    /// 반환 = 실제 바인딩된 수신 포트(선호 포트 점유 시 폴백 값).
    fn respawn_transport(&mut self) -> std::io::Result<u16> {
        let mut instance = [0u8; 16];
        instance.copy_from_slice(&nbeep_crypto::Identity::generate().peer_id().as_bytes()[..16]);
        let name = effective_display_name(&self.settings, &self.identity.peer_id());
        let port = session_port_from(&self.settings);
        let local = nbeep_net::LocalDirect::spawn_on(
            self.identity.peer_id(),
            instance,
            name,
            800,
            1,
            port,
        )?;
        use nbeep_net::Transport as _;
        let bound = local.tcp_port();
        self.discovery = local.discovery();
        spawn_inbound_accept(
            local.incoming(),
            std::sync::Arc::clone(&self.identity),
            self.proxy.clone(),
        );
        self.transport = std::sync::Arc::new(local);
        self.table = nbeep_core::PeerTable::new(60_000);
        self.listen_port = Some(bound);
        Ok(bound)
    }

    fn apply_settings(&mut self, changes: Vec<(&'static str, String)>) {
        if !changes.is_empty() {
            self.conf_mark();
        }
        for (key, value) in changes {
            // 행위 항목(값 아님 · M2-5a) — 저장에 태우지 않고 피커를 연다(el은
            // about_to_wait에 있다 — pending_approve_window와 같은 패턴).
            match key {
                "profile.identity.backup" => {
                    self.pending_picker = Some(PickerPurpose::BackupDir);
                    continue;
                }
                "profile.identity.restore" => {
                    self.pending_picker = Some(PickerPurpose::RestoreKey);
                    continue;
                }
                _ => {}
            }
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
                // 컨트롤 글리프 크기(체크·스위치·옵션박스 — 08-11) — 전역 배율이라
                // 전 창 재배치·재그리기.
                "ui.control_size" => {
                    nbeep_ui::controls::set_control_size_mult(
                        nbeep_ui::controls::control_size_mult_from_code(&value),
                    );
                    let ids: Vec<WindowId> = self.windows.keys().copied().collect();
                    for wid in ids {
                        self.layout_window(wid);
                        self.request_redraw(wid);
                    }
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
                // 표시 이름(M1-10) — 즉시 재공지(사용자 확정 08-11). 상대 목록은
                // PeerTable Renamed 경로로 갱신된다.
                "profile.display_name" => {
                    let name = effective_display_name(&self.settings, &self.identity.peer_id());
                    self.transport.set_display_name(name.clone());
                    self.status = format!("표시 이름 = {name} — LAN 전체에 방송됩니다");
                }
                // 수신 포트(08-13 ⓐ — 듣는 포트 = 거는 기본 포트). 즉시 적용 = 전송 재시작
                // (성립한 대화 세션은 개별 소켓이라 유지 · 발견 목록은 재공지로 회복).
                "net.session_port" => {
                    let want = session_port_from(&self.settings);
                    if !self.live {
                        self.status =
                            format!("수신 포트 = {want} (데모 모드 — 실물 전송에서 적용)");
                    } else if self.listen_port == Some(want) {
                        self.status = format!("수신 포트 = {want} (이미 이 포트로 듣는 중)");
                    } else {
                        match self.respawn_transport() {
                            Ok(bound) if bound == want => {
                                self.status = format!("수신 포트 = {bound} — 즉시 적용(재공지)");
                            }
                            // 폴백을 조용히 하지 않는다 — 상대에게 알려줄 값은 실제 포트다.
                            Ok(bound) => {
                                self.status = format!(
                                    "포트 {want} 점유 — 임의 포트 {bound}로 듣는 중(설정값은 유지)"
                                );
                            }
                            Err(e) => {
                                self.status = format!("전송 재시작 실패: {e} — 기존 포트 유지");
                            }
                        }
                    }
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
        // 프로필 연락처가 있으면 대화 열 때 함께 보여준다(M3-17 — 상세 UI 전 최소 노출).
        self.status = match self.peer_profiles.get(&peer) {
            Some(p) if p.email.is_some() || p.phone.is_some() || p.image_file.is_some() => {
                let mut parts: Vec<String> = Vec::new();
                if let Some(e) = &p.email {
                    parts.push(e.clone());
                }
                if let Some(ph) = &p.phone {
                    parts.push(ph.clone());
                }
                if p.image_file.is_some() {
                    parts.push("이미지 있음".into());
                }
                format!("대화 열림 — 프로필: {}", parts.join(" · "))
            }
            _ => "대화 열림 — 세션 유지 중".into(),
        };
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
        self.mark_read(peer); // 뷰가 열렸다 = 확인했다(③)
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
            Role::Alert => {
                if let Some(av) = &mut self.alert_view {
                    av.set_scale(scale, &mut inv);
                    av.set_bounds(Rect::new(0, 0, w, h), &mut inv);
                }
            }
            Role::AddEndpoint => {
                if let Some(av) = &mut self.addr_view {
                    av.set_scale(scale, &mut inv);
                    av.set_bounds(Rect::new(0, 0, w, h), &mut inv);
                }
            }
            Role::Profile => {
                if let Some(pv) = &mut self.profile_view {
                    pv.set_scale(scale, &mut inv);
                    pv.set_bounds(Rect::new(0, 0, w, h), &mut inv);
                }
            }
            Role::PeerInfo(_) => {
                if let Some(pv) = &mut self.peer_info_view {
                    pv.set_scale(scale, &mut inv);
                    pv.set_bounds(Rect::new(0, 0, w, h), &mut inv);
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
            self.mark_read(peer); // 닫는 순간까지 보고 있었다(③ — 마지막 확인 시각)
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
                            // 직접 등록(DR-19 수동 엔드포인트) — 별도 모달 창(M3-16).
                            "add" => self.open_add_endpoint(el),
                            "quarantine" => self.open_quarantine(el),
                            "profile" => self.open_profile(el),
                            "gallery" => self.open_gallery(el),
                            _ => {}
                        }
                    }
                    if let Some(peer) = self.list.take_activated() {
                        self.activate(peer, el);
                    }
                    // 우클릭 ▸ 프로필 보기(M3-17).
                    if let Some(peer) = self.list.take_profile_request() {
                        self.open_peer_info(peer, el);
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
                        self.open_picker(el, PickerPurpose::GallerySample);
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
                    self.picker_ctx = None;
                    self.windows.remove(&id); // 취소
                } else if let Some(pv) = &mut self.picker_view {
                    pv.on_event(&ev, &mut inv);
                    if matches!(ev, InputEvent::MouseDown { .. }) {
                        if let Some(label) = pv.selected_label() {
                            // ── 탐색형(백업·복원) — 라벨을 행위로 해석(M2-5a) ──
                            if let Some(ctx) = &self.picker_ctx {
                                let hit = ctx
                                    .entries
                                    .iter()
                                    .find(|(l, _)| *l == label)
                                    .map(|(_, e)| e.clone());
                                match hit {
                                    Some(PickEntry::Up) => {
                                        if let Some(ctx) = &mut self.picker_ctx {
                                            if let Some(p) = ctx.dir.parent() {
                                                ctx.dir = p.to_path_buf();
                                            }
                                        }
                                        self.repopulate_picker(id);
                                    }
                                    Some(PickEntry::Dir(p)) => {
                                        if let Some(ctx) = &mut self.picker_ctx {
                                            ctx.dir = p;
                                        }
                                        self.repopulate_picker(id);
                                    }
                                    Some(PickEntry::SaveHere) => {
                                        let dir = ctx.dir.clone();
                                        self.status = self.do_backup_identity(&dir);
                                        self.picker_view = None;
                                        self.picker_ctx = None;
                                        self.windows.remove(&id);
                                        if let Some(mid) = self.main_id {
                                            self.request_redraw(mid);
                                        }
                                    }
                                    Some(PickEntry::File(p)) => {
                                        match ctx.purpose {
                                            PickerPurpose::ProfileImage => {
                                                // 프로필 이미지 경로 반영(M3-17) — 저장 +
                                                // 열린 프로필 화면 갱신.
                                                let path = p.to_string_lossy().into_owned();
                                                self.settings
                                                    .set("profile.image_path", path.clone());
                                                self.conf_mark();
                                                // 격리 디코드(M4-5) — 미리보기 즉시 갱신.
                                                let avatar =
                                                    crate::imgdec::avatar_from_file(&p, 256)
                                                        .map(std::rc::Rc::new);
                                                let decoded = avatar.is_some();
                                                if let Some(pv) = &mut self.profile_view {
                                                    let mut pinv = Invalidations::default();
                                                    pv.set_image_path(&path, &mut pinv);
                                                    pv.set_avatar(avatar, &mut pinv);
                                                    let _ = pv.take_changes(); // 저장은 위에서 이미
                                                }
                                                self.status = if decoded {
                                                    format!("프로필 이미지 = {path}")
                                                } else {
                                                    format!("프로필 이미지 = {path} (미리보기 불가 — PNG/JPEG 아님/imgdec 부재)")
                                                };
                                                if let Some((pid, _)) = self
                                                    .windows
                                                    .iter()
                                                    .find(|(_, e)| e.role == Role::Profile)
                                                {
                                                    let pid = *pid;
                                                    self.request_redraw(pid);
                                                }
                                            }
                                            _ => {
                                                self.status = self.do_restore_identity(&p);
                                            }
                                        }
                                        self.picker_view = None;
                                        self.picker_ctx = None;
                                        self.windows.remove(&id);
                                        if let Some(mid) = self.main_id {
                                            self.request_redraw(mid);
                                        }
                                    }
                                    None => {}
                                }
                                return;
                            }
                            // ── 갤러리 실증(기존) — Choose 값 반영 + 창 닫기 ──
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
            Role::Alert => {
                if let Some(av) = &mut self.alert_view {
                    av.on_event(&ev, &mut inv);
                    if av.take_closed() {
                        self.alert_view = None;
                        self.windows.remove(&id);
                    }
                }
            }
            Role::AddEndpoint => {
                let (mut submit, mut cancel) = (None, false);
                if let Some(av) = &mut self.addr_view {
                    av.on_event(&ev, &mut inv);
                    submit = av.take_submit();
                    cancel = av.take_cancel();
                }
                if let Some(addr) = submit {
                    self.close_add_endpoint();
                    self.commit_manual_add(addr, el); // 워커 이관은 M2-8 잔여 — 지금은 기존 경로
                } else if cancel {
                    self.close_add_endpoint();
                    if let Some(mid) = self.main_id {
                        self.request_redraw(mid);
                    }
                }
            }
            Role::PeerInfo(_) => {
                let mut closed = false;
                if let Some(pv) = &mut self.peer_info_view {
                    pv.on_event(&ev, &mut inv);
                    closed = pv.take_closed();
                }
                if closed {
                    self.peer_info_view = None;
                    self.windows.remove(&id);
                }
            }
            Role::Profile => {
                let (mut changes, mut pick, mut closed) = (Vec::new(), false, false);
                if let Some(pv) = &mut self.profile_view {
                    pv.on_event(&ev, &mut inv);
                    changes = pv.take_changes();
                    pick = pv.take_pick_image();
                    closed = pv.take_closed();
                }
                if !changes.is_empty() {
                    // 설정 깔때기 재사용 — display_name 변경은 즉시 재공지 arm을 탄다.
                    self.apply_settings(changes);
                }
                if pick {
                    self.pending_picker = Some(PickerPurpose::ProfileImage);
                }
                if closed {
                    self.close_profile();
                    if let Some(mid) = self.main_id {
                        self.request_redraw(mid);
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
                // 우측: 실제 수신 포트(DR-19 — 발견이 안 닿는 상대에게 알려줄 값이라 상시 표시).
                let mut status_clip = bar;
                if let Some(p) = self.listen_port {
                    let label = format!("수신 :{p}");
                    let lw = ctx.text_width(&label);
                    let lx = bar.right() - pad - lw;
                    ctx.text_opaque(lx, bar.y + dy, bar, &label, theme.text_dim, theme.chrome_bg);
                    // 상태 문구가 포트 표시를 덮지 않게 클립을 좁힌다.
                    status_clip.w = (lx - pad - bar.x).max(0);
                }
                ctx.text_opaque(
                    bar.x + pad,
                    bar.y + dy,
                    status_clip,
                    &self.status,
                    theme.text_dim,
                    theme.chrome_bg,
                );
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
            Role::Alert => {
                if let Some(av) = &self.alert_view {
                    av.paint(&mut ctx, &theme);
                }
            }
            Role::AddEndpoint => {
                if let Some(av) = &self.addr_view {
                    av.paint(&mut ctx, &theme);
                }
            }
            Role::Profile => {
                if let Some(pv) = &self.profile_view {
                    pv.paint(&mut ctx, &theme);
                }
            }
            Role::PeerInfo(_) => {
                if let Some(pv) = &self.peer_info_view {
                    pv.paint(&mut ctx, &theme);
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
                // 읽음/안읽음 계상(③) — 뷰가 닫혀 있으면 배지·제목으로 알린다.
                self.note_incoming(peer);
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
                qpath,
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
                // 이미지면 소형 미리보기 부착(M4-5ⓑ) — 픽셀은 imgdec(격리)가 만든다.
                // 이미지가 아니거나 실패면 조용히 없음(스레드는 텍스트 그대로).
                if let Some(thumb) =
                    crate::imgdec::thumb_from_beepq(std::path::Path::new(&qpath), 96)
                {
                    let thumb = std::rc::Rc::new(thumb);
                    if let Some(conv) = self.conversations.get_mut(&peer) {
                        nbeep_ui::chat_view::attach_xfer_thumb(
                            &mut conv.lines,
                            false,
                            thumb.clone(),
                        );
                    }
                    if let Some(chat) = self.chats.get_mut(&peer) {
                        let mut inv = Invalidations::default();
                        chat.attach_xfer_thumb(false, thumb, &mut inv);
                    }
                }
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
                // ★ 전송 끝 ≠ 완료(M4-9) — 청크·Done을 다 보냈을 뿐, 상대 격리 확인(ack)
                //   전까지는 **확인 대기**다. 미확인 카운트를 올려 종료 가드가 본다.
                self.set_xfer_line(peer, true, nbeep_ui::XferLineState::AwaitingAck);
                *self.awaiting_ack.entry(peer).or_insert(0) += 1;
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
                    self.status = format!("전달됨 {done_f}/{total_f} — 상대 확인 대기");
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
            AppEvent::XferAcked { peer, ok } => {
                // 수신 종단 확인 도착 — 확인 대기 항목을 완료/실패로 닫는다(M4-9).
                let terminal = if ok {
                    nbeep_ui::XferLineState::Done {
                        note: String::new(),
                    }
                } else {
                    nbeep_ui::XferLineState::Failed {
                        why: "상대가 받지 못함(무결성·저장 실패)".into(),
                    }
                };
                self.ack_xfer_line(peer, terminal);
                if let Some(n) = self.awaiting_ack.get_mut(&peer) {
                    *n = n.saturating_sub(1);
                    if *n == 0 {
                        self.awaiting_ack.remove(&peer);
                    }
                }
                if self.awaiting_ack.is_empty() {
                    self.close_armed = false; // 확인이 다 끝났으면 종료 가드 해제
                }
                self.status = if ok {
                    "상대 수신 확인 — 완료".into()
                } else {
                    "상대가 받지 못함 — 실패".into()
                };
                self.redraw_conversation(peer);
                if let Some(mid) = self.main_id {
                    self.request_redraw(mid);
                }
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
                                                // 인바운드 전용 비발견 상대(수동 주소도, 발견 경로도 없음)는 세션이
                                                // 끝나면 **다시 닿을 수단이 없다** — 목록에 유령으로 남기지 않는다(08-13).
                                                // 수동 주소가 있으면 남긴다(빨강 · 클릭 = 그 주소로 재연결).
                if !self.manual_addrs.contains_key(&peer) && self.table.get(peer).is_none() {
                    self.extra_peers.remove(&peer);
                    self.unread.remove(&peer);
                    self.update_main_title();
                }
                self.status = "상대와의 세션이 종료됨".into();
                let mut inv = Invalidations::default();
                self.refresh_rows(&mut inv);
                if let Some(id) = self.main_id {
                    self.request_redraw(id);
                }
            }
            AppEvent::Outbound {
                session,
                via_addr,
                intent,
            } => {
                // 워커가 만든 아웃바운드 세션(M2-8) — TOFU 판정은 여기(메인 · TrustStore 소유).
                use nbeep_core::Session as _;
                let peer = session.session.peer();
                self.connecting.finish(intent, Some(peer));
                if let Some(addr) = via_addr {
                    // 수동 등록 성공(DR-19) — 세션이 끊겨도 이 주소로 재연결한다(④).
                    self.manual_addrs.insert(peer, addr);
                }
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
                self.connecting.finish(Some(peer), None);
                // 닿지 않은 상대 = 목록 점 빨강(08-13 실기 — 실패 후 회색으로 남으면
                // "종료된 상대"라는 사실이 표시되지 않았다).
                self.closed_peers.insert(peer);
                self.status = format!("연결 실패({}): {why}", self.peer_title(peer));
                let mut inv = Invalidations::default();
                self.refresh_rows(&mut inv);
                if let Some(mid) = self.main_id {
                    self.request_redraw(mid);
                }
            }
            AppEvent::AddFailed { addr, why } => {
                // 수동 주소 연결 실패(워커에서 복귀 · M2-8 잔여) — 주소로 알린다(피어 미확정).
                self.status = format!("수동 연결 실패({addr}): {why}");
                if let Some(mid) = self.main_id {
                    self.request_redraw(mid);
                }
            }
            AppEvent::ProfileRequested { peer } => {
                // 응답 구성은 여기(메인) — 공개 정책 판단 단일 지점(M3-17).
                let frames = self.my_profile_frames();
                if let Some(conv) = self.conversations.get(&peer) {
                    let _ = conv.out_tx.send(SessionCmd::Control(frames));
                }
            }
            AppEvent::PeerProfile {
                peer,
                name,
                email,
                phone,
                image,
            } => {
                // 이름은 무해화(DisplayName) 통과분만 채택 · 이력은 신뢰 저장소에도 남긴다.
                let display = name.and_then(|n| nbeep_core::DisplayName::parse(&n).ok());
                if let Some(n) = &display {
                    self.trust.record_name(peer, n.clone());
                }
                // 이미지 바이트 캐시 + **격리 디코드**(M4-5 — imgdec 자식 프로세스 ·
                // 본체는 파서를 링크하지 않는다. 실패 = 이니셜 폴백).
                let mut avatar = None;
                let image_file = image.and_then(|bytes| {
                    let dir = self.data_dir.join("profiles");
                    std::fs::create_dir_all(&dir).ok()?;
                    let path = dir.join(format!("{}.img", peer.short()));
                    std::fs::write(&path, &bytes).ok()?;
                    avatar = crate::imgdec::avatar_from_bytes(&bytes, 256).map(std::rc::Rc::new);
                    Some(path)
                });
                let has_any =
                    display.is_some() || email.is_some() || phone.is_some() || image_file.is_some();
                if has_any {
                    // 받은 항목을 상태바에 요약(연락처 상세 표시 UI는 M3-17 잔여).
                    let mut got: Vec<&str> = Vec::new();
                    if display.is_some() {
                        got.push("이름");
                    }
                    if email.is_some() {
                        got.push("이메일");
                    }
                    if phone.is_some() {
                        got.push("전화");
                    }
                    if image_file.is_some() {
                        got.push("이미지");
                    }
                    self.peer_profiles.insert(
                        peer,
                        PeerProfile {
                            name: display,
                            email,
                            phone,
                            image_file,
                            avatar,
                        },
                    );
                    self.status =
                        format!("프로필 수신({}) — {}", self.peer_title(peer), got.join("·"));
                } else {
                    // 전부 비공개(빈 응답) — 이전 프로필이 있었다면 걷어낸다(철회 반영).
                    self.peer_profiles.remove(&peer);
                }
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
                // ② 자동 열림 금지(사용자 확정 08-13) — 인바운드는 창을 뺏지 않는다.
                // 목록에 행이 뜨고(비발견 상대는 extra_peers ④), 메시지가 오면
                // 배지·제목 카운트(③)로 알린다. 여는 것은 언제나 사용자.
                self.status = format!("연결됨: {title} — 목록에서 열기");
                if let Some(mid) = self.main_id {
                    self.request_redraw(mid);
                }
            }
        }
    }

    fn exiting(&mut self, _el: &ActiveEventLoop) {
        // 종료 강제 flush(S-1) — 디바운스가 마지막 변경을 삼키면 안 된다.
        // 주기 경로와 같은 스냅샷·직렬화를 쓴다(S-2 — conf_save 하나뿐).
        if self.conf.sched.flush_now() {
            self.conf_save(true);
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
        // 설정 영속 tick(FR-P-9) — 조용 1s OR 상한 10s 충족 시 스냅샷 1회 저장.
        if self.conf.sched.tick(Instant::now()) {
            self.conf_save(false);
        }
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
        // 설정에서 요청된 백업·복원 피커 열기(M2-5a).
        if let Some(purpose) = self.pending_picker.take() {
            self.open_picker(el, purpose);
        }
        // 경고 모달 열기(08-13 — 이벤트 루프 참조가 없는 지점의 요청을 여기서 처리).
        if let Some((title, message)) = self.pending_alert.take() {
            self.open_alert(el, &title, &message);
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
                    // ★ 종료 가드(M4-9) — 미확인·진행 중 전송이 있으면 조용히 끊지 않는다.
                    //   "보냈다"가 "닿았다"가 아니라, 확인 전 종료는 수신측 폐기로 이어질 수 있다.
                    let pending: u32 = self.awaiting_ack.values().sum::<u32>()
                        + u32::try_from(self.awaiting_accept.len()).unwrap_or(0)
                        + u32::try_from(self.send_batch.len()).unwrap_or(0);
                    if pending > 0 && !self.close_armed {
                        self.close_armed = true;
                        self.status =
                            format!("전송 {pending}건 확인 대기 중 — 다시 닫으면 그대로 종료");
                        if let Some(mid) = self.main_id {
                            self.request_redraw(mid);
                        }
                        return;
                    }
                    el.exit(); // 주 창 닫기 = 종료(트레이 상주는 M3-2)
                } else if let Some(entry) = self.windows.remove(&id) {
                    match entry.role {
                        // 대화 창 닫기 = 뷰만 닫힘(대화 유지 — DR-26).
                        Role::Chat(peer) => {
                            self.chats.remove(&peer);
                            self.mark_read(peer); // 닫는 순간까지 확인(③)
                        }
                        Role::Settings => self.settings_view = None,
                        Role::Gallery => self.gallery_view = None,
                        Role::Picker => self.picker_view = None,
                        Role::About => self.about_view = None,
                        Role::Alert => self.alert_view = None,
                        Role::AddEndpoint => self.addr_view = None,
                        Role::Profile => self.profile_view = None,
                        Role::PeerInfo(_) => self.peer_info_view = None,
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
                            // 복사/잘라내기/붙여넣기 — **모든 텍스트 컨트롤**(① 08-13 —
                            // 그전엔 대화 입력창만). ui는 OS를 모른다 — plat 어댑터가 잇는다.
                            "c" | "C" => {
                                if let Some(t) = self.clipboard_copy_for(id) {
                                    if nbeep_plat::clipboard::set_text(&t) {
                                        self.status = "복사됨".into();
                                    }
                                }
                                return;
                            }
                            "x" | "X" => {
                                if let Some(t) = self.clipboard_cut_for(id) {
                                    nbeep_plat::clipboard::set_text(&t);
                                    self.request_redraw(id);
                                }
                                return;
                            }
                            "v" | "V" => {
                                if let Some(t) = nbeep_plat::clipboard::get_text() {
                                    self.clipboard_paste_for(id, &t);
                                    self.request_redraw(id);
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
                            // 주소 직접 입력 = 별도 모달 창(M3-16 · 인라인 상태바 입력 대체).
                            "k" | "K" => {
                                self.open_add_endpoint(el);
                                return;
                            }
                            _ => {}
                        }
                    }
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
/// 유효 표시 이름(M1-10 · FR-S-50) — 설정 `profile.display_name`이 "auto"면
/// **정제된 호스트명**(실명 추정 부분 제거 · 실패 시 지문 라벨 `beep-xxxx`),
/// 직접 입력이면 그 이름(무해화 실패 시 기본값 폴백). 어느 경로든 무해화를 거친다.
fn effective_display_name(
    settings: &SettingsState,
    peer: &nbeep_core::PeerId,
) -> nbeep_core::DisplayName {
    let v = settings.get("profile.display_name");
    if v != "auto" {
        if let Ok(n) = nbeep_core::DisplayName::parse(v) {
            return n;
        }
    }
    nbeep_core::default_display_name(nbeep_plat::host::hostname().as_deref(), peer)
}

/// 유효 세션 포트 — 설정 `net.session_port` 관용 파싱(ADR-0011 §4-3 원칙 그대로:
/// 무효·범위 밖은 거부가 아니라 **기본값으로 본다**). 듣는 포트이자 주소 입력에서
/// 포트 생략 시 거는 포트(사용자 확정 08-13 ⓐ — 하나의 값).
/// **`0` = 임의 포트**(테스트·다중 인스턴스 — `--port 0`) — 걸 때의 기본값으로는 못 쓰므로
/// 주소 입력 기본은 47200으로 폴백한다(소비처 참조).
fn session_port_from(settings: &SettingsState) -> u16 {
    settings
        .get("net.session_port")
        .parse::<u16>()
        .ok()
        .unwrap_or(nbeep_net::DEFAULT_SESSION_PORT)
}

/// 데이터 디렉터리(FR-P-3 · DR-4) — 실행 파일 옆 `data/` 쓰기 가능(포터블)
/// → 사용자 설정 폴더 → 임시 폴더(최후 폴백 — 저장은 되지만 재부팅에 진다).
/// 설정(`settings.cfg`)·신원 키(`identity.key`)·핀 세그먼트(`trust.seg`)가 전부
/// 여기 산다. 경로는 여기서 정해 **인자로 넘긴다**(소비 크레이트는 경로 비소유).
fn data_dir() -> std::path::PathBuf {
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            let data = dir.join("data");
            if nexa_conf::dir_writable(&data) {
                return data;
            }
        }
    }
    if let Some(dir) = nexa_conf::user_config_dir("nexa-beep") {
        if nexa_conf::dir_writable(&dir) {
            return dir;
        }
    }
    std::env::temp_dir().join("nexa-beep")
}

pub(crate) fn run(mode: WindowMode, live: bool, port_flag: Option<u16>) {
    let (data, index) = nbeep_plat::font::system_ui_font().expect("시스템 UI 폰트 없음");
    let font = nbeep_gfx::Font::from_static(data, index).expect("폰트 파싱");
    let dir = data_dir();
    // 신원 영속(M2-5a) — 재시작해도 같은 PeerId. 키 파일 손상 시 **덮어쓰지 않고**
    // 임시 신원으로 강등(fail-closed — 조용히 새 키를 만들면 상대 핀에서 남이 된다).
    let (identity, id_note) =
        match nbeep_crypto::keyfile::load_or_generate(&dir.join("identity.key")) {
            Ok((id, created)) => (id, created.then_some("새 신원 키 생성")),
            Err(e) => {
                eprintln!("신원 키 파일 사용 불가({e}) — 이번 실행은 임시 신원");
                (
                    nbeep_crypto::Identity::generate(),
                    Some("⚠ 신원 키 파일 손상 — 임시 신원(재시작하면 바뀜)"),
                )
            }
        };
    let identity = std::sync::Arc::new(identity);
    // 핀 세그먼트(M2-5a · R-17 해소) — 래핑 원료 = 기기 신원 키(ADR-0005 §3 기본 A).
    let (trust, trust_load) =
        nbeep_store::FileTrustStore::open(dir.join("trust.seg"), identity.wrap_secret());
    use nbeep_net::Transport as _;

    // 이벤트 루프·프록시 먼저 — 인바운드 수락 펌프가 프록시를 필요로 한다(M2-7).
    let event_loop = EventLoop::<AppEvent>::with_user_event().build().unwrap();
    let proxy = event_loop.create_proxy();
    let shutdown = nbeep_plat::shutdown::install(); // R-16 — SIGINT/SIGTERM 포트

    // 설정 로드는 전송 생성보다 먼저 — 표시 이름(M1-10)이 발견 광고에 실린다.
    let mut settings = SettingsState::with_defaults();
    // 설정 영속(M3-15 · ADR-0011) — 기본값 시드 후 파일의 아는 키만 덮어쓴다(관용 파싱).
    // 모르는 키는 Store가 보존해 다음 저장 때 그대로 재방출한다(F-1 — 구버전이
    // 신버전 파일을 저장해도 신규 키가 살아남는다).
    let (mut conf, doc) = nexa_conf::Store::open(dir.join("settings.cfg"), 1_000, 10_000);
    for (k, v) in doc.pairs {
        if !settings.set_by_name(&k, &v) {
            conf.keep_unknown(k, v);
        }
    }
    // CLI 플래그는 이 세션만 이긴다 — 저장값을 덮어쓰되 dirty를 세우지 않는다
    // (플래그 실행이 영속 설정을 바꿔버리면 다음 무플래그 실행이 놀란다).
    if mode == WindowMode::Separate {
        settings.set("chat.window_mode", "separate".into());
    }
    // `--port N`(⑥ 08-13) — 이 세션의 수신 포트만 덮는다(dirty 없음 = 영속 안 됨).
    // 0도 유효하다(임의 포트 — 같은 PC에 여러 인스턴스를 띄우는 테스트).
    if let Some(p) = port_flag {
        settings.set("net.session_port", p.to_string());
    }
    // 유효 창 모드 = 영속값 반영(플래그 없으면 저장된 선택이 부팅에 살아난다 — DR-26).
    let mode = if settings.get("chat.window_mode") == "separate" {
        WindowMode::Separate
    } else {
        WindowMode::Single
    };
    // 표시 이름(M1-10 · R-19) — 실명이 아니라 정제된 호스트명/지문 라벨이 기본.
    let display_name = effective_display_name(&settings, &identity.peer_id());

    let mut listen_port: Option<u16> = None;
    let (transport, discovery): (std::sync::Arc<dyn nbeep_net::Transport + Send + Sync>, _) =
        if live {
            // 실물 — LocalDirect(UDP 발견 + TCP 세션). 실기·컨테이너 상대가 목록에 뜬다.
            // 수신 포트 = 설정(기본 47200 · 점유 시 임의 폴백 — 실제 값은 상태바에 표시).
            let mut instance = [0u8; 16];
            instance
                .copy_from_slice(&nbeep_crypto::Identity::generate().peer_id().as_bytes()[..16]);
            let name = display_name;
            let local = nbeep_net::LocalDirect::spawn_on(
                identity.peer_id(),
                instance,
                name,
                800,
                1,
                session_port_from(&settings),
            )
            .expect("LocalDirect 시작(방화벽·인터페이스)");
            listen_port = Some(local.tcp_port());
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
            let transport = bus.join(identity.peer_id(), display_name, nbeep_net::Caps::default());
            let discovery = transport.discovery();
            (std::sync::Arc::new(transport), discovery)
        };

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
    // 신뢰 저장 상태 고지(M2-5a) — 잠김은 fail-closed라 반드시 사용자에게 보인다.
    let trust_hint = match trust_load {
        nbeep_store::TrustLoad::Locked => " · ⚠ 신뢰 목록 잠김(파일 손상 — 전부 미검증 취급)",
        nbeep_store::TrustLoad::Loaded(_) | nbeep_store::TrustLoad::Fresh => "",
    };
    let id_hint = id_note.map(|n| format!(" · {n}")).unwrap_or_default();
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
        connecting: ConnectLatch::default(),
        discovery,
        table: nbeep_core::PeerTable::new(60_000),
        trust,
        conversations: HashMap::new(),
        dedup: nbeep_core::DedupIndex::new(),
        started: Instant::now(),
        status: format!(
            "[{net_hint}] {mode_hint} · ⌘/Ctrl+K = 주소 추가 · ⌘/Ctrl+, = 설정 · ⌘/Ctrl+G = 컨트롤 갤러리{trust_hint}{id_hint}"
        ),
        fonts: App::fonts_from_settings(&settings),
        settings,
        conf,
        settings_view: None,
        gallery_view: None,
        about_view: None,
        alert_view: None,
        pending_alert: None,
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
        awaiting_ack: HashMap::new(),
        close_armed: false,
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
            // 프로필 변경 화면(M3-17 — 사람 실루엣 · 사용자 요청 08-11).
            ToolItem::new(
                "profile",
                ToolIcon::Mask {
                    w: nbeep_ui::icons::PERSON_SIZE,
                    h: nbeep_ui::icons::PERSON_SIZE,
                    alpha: nbeep_ui::icons::PERSON_ALPHA,
                },
            ),
            // 컨트롤 갤러리는 툴바에서 뺐다(사용자 요청 08-10) — 메뉴(보기 ▸ 컨트롤 갤러리)와
            // ⌘/Ctrl+G로 열 수 있으니, 상시 노출할 임시 검수용 항목은 툴바를 차지할 이유가 없다.
        ]),
        closed_peers: std::collections::HashSet::new(),
        extra_peers: HashMap::new(),
        manual_addrs: HashMap::new(),
        unread: HashMap::new(),
        last_read: HashMap::new(),
        icon: winit::window::Icon::from_rgba(
            nbeep_ui::brand::ICON_RGBA.to_vec(),
            nbeep_ui::brand::ICON_SIZE,
            nbeep_ui::brand::ICON_SIZE,
        )
        .ok(),
        picker_view: None,
        profile_view: None,
        peer_profiles: HashMap::new(),
        peer_info_view: None,
        picker_ctx: None,
        pending_picker: None,
        data_dir: dir,
        live,
        listen_port,
        addr_view: None,
        ime_composing: false,
        pending_jamo: None,
        primary_down: false,
        shift_down: false,
        proxy,
        shutdown,
    };
    app.reload_faces(); // 고정폭 등 슬롯 얼굴 초기 로드
    app.apply_boot_settings(); // 영속 설정 → 파생 런타임 상태(테마·정책 등 · M3-15)
    event_loop.run_app(&mut app).unwrap();
}

#[cfg(test)]
mod tests {
    use super::ConnectLatch;
    use nbeep_core::PeerId;

    fn pid(b: u8) -> PeerId {
        let mut a = [0u8; 32];
        a[0] = b;
        PeerId::from_bytes(a)
    }

    /// ★ 실패 재현(08-13) — **성립한 상대로만 빼면 클릭한 상대가 영원히 남는다.**
    ///
    /// 수동 주소 폴백은 *주소*로 붙으므로, 그 주소의 상대가 신원을 새로 만들면
    /// (컨테이너 재기동 — 실측: 실행마다 `me=`가 바뀐다) 성립 세션은 다른 `PeerId`다.
    #[test]
    fn clearing_by_actual_only_strands_the_clicked_peer() {
        let (clicked, actual) = (pid(1), pid(2)); // 주소는 같고 신원만 바뀐 상황
        let mut latch = ConnectLatch::default();
        assert!(latch.begin(clicked));
        latch.finish(None, Some(actual)); // ← 옛 동작
        assert!(
            latch.contains(clicked),
            "이게 남으면 그 행은 다시는 안 열린다"
        );
        assert!(!latch.begin(clicked), "다음 클릭 = '이미 시도 중'");
    }

    /// 회귀 방지 — `intent`를 함께 넘기면 래치가 풀린다.
    #[test]
    fn clearing_by_intent_releases_the_clicked_peer() {
        let (clicked, actual) = (pid(1), pid(2));
        let mut latch = ConnectLatch::default();
        assert!(latch.begin(clicked));
        latch.finish(Some(clicked), Some(actual)); // ← 새 동작
        assert!(!latch.contains(clicked));
        assert!(latch.begin(clicked), "다시 클릭하면 또 시도된다");
    }

    /// 연결 실패도 같은 통로로 풀린다(intent만 있고 성립 세션은 없다).
    #[test]
    fn connect_failure_releases_the_latch() {
        let p = pid(7);
        let mut latch = ConnectLatch::default();
        assert!(latch.begin(p));
        assert!(!latch.begin(p), "진행 중 중복 클릭은 막는다");
        latch.finish(Some(p), None);
        assert!(latch.begin(p));
    }
}
