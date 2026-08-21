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
                // 대화함(M3-23 · 08-20 사용자 요청) — take_picked 팔은 이미 있었고
                // 항목만 없었다(진입점 = 툴바뿐이던 것을 메뉴에도).
                MenuEntry::Item(ComboItem::new("convbox", t(Msg::ConvboxTitle))),
                // 공지 보내기(④ 08-20 · FR-M-6) — 발견된 전체에게 Notice 팬아웃.
                MenuEntry::Item(ComboItem::new("broadcast", t(Msg::MenuBroadcast))),
                MenuEntry::Item(ComboItem::new("gallery", t(Msg::MenuGallery))),
                // 종료(08-15 사용자 요청) — close_to_tray가 켜지면 X로는 못 끝낸다.
                MenuEntry::Separator,
                MenuEntry::Item(ComboItem::new("quit", t(Msg::MenuQuit))),
            ],
        ),
        MenuDef::new(
            t(Msg::MenuHelp),
            vec![MenuEntry::Item(ComboItem::new("about", "About"))],
        ),
    ]
}

/// 알림 클릭의 대상(M3-8 · 08-15) — 클릭 = 메인 표시 + 이 대화 열기.
#[derive(Clone, Copy)]
enum NotifyTarget {
    /// 1:1 대화(파일 오퍼 포함 — 승인 흐름도 그 대화 문맥이다).
    Peer(PeerId),
    /// 그룹 방.
    Group(nbeep_core::group::GroupId),
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
        /// 발신자 요청 등급(④ · docs/24 — 0 일반 · 1 알림 · 2 긴급). 강도 판정은
        /// 수신측(여기) 몫 — 미검증은 종전 무음 게이트가 그대로 이긴다.
        importance: u8,
        /// 공지(브로드캐스트) 팬아웃 표식(08-21) — "받지 않기" 설정의 판정 근거.
        broadcast: bool,
    },
    /// 클립보드 이미지 준비 완료(③ 08-20) — 워커가 읽기+PNG 인코딩을 마쳤다.
    /// `png = None` = 클립보드에 이미지 없음/변환 실패(상태바 고지).
    ClipImage {
        win: winit::window::WindowId,
        png: Option<Vec<u8>>,
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
        /// 자동 재연결(ⓑ)로 성립했는가 — **자동은 창을 열지 않는다**(② 규칙과 동일:
        /// 사용자 행위 없는 성립이 포커스를 뺏으면 안 된다).
        auto: bool,
    },
    /// 아웃바운드 연결 실패(M2-8) — 죽은 상대를 클릭해도 UI는 살아 있고 이것만 온다.
    ConnectFailed { peer: PeerId, why: String },
    /// Managed 서버 접속 워커 결과(X-2b) — `gen`이 현재와 다르면 낡은 결과(폐기 —
    /// 그 사이 설정이 바뀌었다 · 틱이 새 목표로 다시 시도한다).
    ServerAttach {
        gen: u64,
        outcome: Result<Box<nbeep_relay::Attached>, ServerAttachFail>,
    },
    /// L1 링크 변화(M1-2 · **디바운스 후**) — Wi-Fi 전환·케이블·절전 복귀.
    /// 전송에 재발견을 시키고 상태바에 알린다(변화 자체는 OS 구독 스레드가 관측).
    LinkChanged,
    /// 격리 아카이브 내용 목록(M4-4 ⓐ · 08-21) — 워커가 개봉·파싱을 마친 본문.
    /// (대형 격리물 개봉이 UI를 얼리지 않게 — 격리함 워커 스캔과 같은 이유.)
    ArchiveList {
        title: String,
        body: String,
        anchor: WindowId,
    },
    /// 배치 목록 도착(M4-2e · Control 13) — 요청 단위 승인·수신 목록의 원료.
    XferManifest {
        peer: PeerId,
        entries: Vec<(String, u64, bool)>,
    },
    /// 파일 수신 제안 도착 — 사용자가 수락/거절할 때까지 데이터는 오지 않는다(FR-X-3).
    XferOffer {
        peer: PeerId,
        id: nbeep_core::XferId,
        name: String,
        size: u64,
        /// 선언 전체 SHA-256(M4-10) — `.part` 재개 후보 판정의 앵커.
        sha256: [u8; 32],
    },
    /// 전송 진행률(수신·발신 공용) — 상태바 표시용.
    /// 상대의 **전체 취소** 통지(M4-2e · Control 16) — 로컬 전체취소 루틴 실행.
    XferCancelAllNotice { peer: PeerId },
    /// 발신측 정지/재개 **통지**(M4-2e — 수신 화면 동기 · Control 14/15 재사용).
    XferPeerPauseNotice {
        peer: PeerId,
        name: String,
        paused: bool,
    },
    /// 정지 확정(M4-2e — 액터가 실제로 보관 슬롯에 넣은 순간 · 단일 진실).
    /// 앱은 이때만 라인 Paused·큐 이동·다음 파일 펌프를 수행한다(수락 경합 제거).
    XferPaused {
        peer: PeerId,
        id: nbeep_core::XferId,
        name: String,
        size: u64,
        done: u64,
    },
    /// 정지 보관분 재개(M4-2e ⓐ) — 액터가 보관 슬롯에서 활성으로 되돌렸다.
    XferResumed {
        peer: PeerId,
        id: nbeep_core::XferId,
        name: String,
        size: u64,
        done: u64,
    },
    XferProgress {
        peer: PeerId,
        got: u64,
        total: u64,
        sending: bool,
        /// 어느 파일인가(M4-2e — 신원 없는 FIFO 갱신이 지연 이벤트에서 다음
        /// 라인을 오염시켰다 · 실기 "Sending 22409%"). 빈 문자열 = 구 경로 FIFO.
        name: String,
    },
    /// 수신 완료 → **격리 보관까지 끝남**(실체화는 승인 후 별도 · FR-S-9).
    XferDone {
        peer: PeerId,
        /// 평균 수신 속도(B/s · 08-16 — 첫 청크~Done 기준).
        avg_bps: u64,
        name: String,
        risk: nbeep_core::RiskLevel,
        mismatch: bool,
        /// `.beepq` 경로 — 이미지면 썸네일(imgdec 격리 디코드 · M4-5ⓑ) 시도용.
        qpath: String,
        /// 검사 결과(FR-S-15 · 08-21) — `Detected`면 수신 즉시 상태바 경고.
        scan: nbeep_core::ScanOutcome,
    },
    /// 전송 실패·거절 — 사유 문장(표시 전용).
    XferFailed { peer: PeerId, why: String },
    /// 상대가 **수락**했다 — 대기 창을 닫고 스트리밍이 시작된다.
    XferAccepted { peer: PeerId },
    /// 격리함 스캔 완료(08-18 — 워커 · gen 불일치 = 낡은 결과 폐기).
    QuarantineScanned { gen: u64, rows: Vec<QRowRaw> },
    /// 격리물 무결성 검증 완료(08-18 — 전체 개봉·해시 확인 성공 · gen 대조).
    /// 도착 = 그 행 `verified=true` → Approve 활성. `ok=false`는 손상(승인 불가 유지).
    QVerified { gen: u64, path: String, ok: bool },
    /// 상대의 수신 파일 상한 공지 도착(08-18 — 해시 전 차단의 근거).
    PeerRecvCap { peer: PeerId, cap: u64 },
    /// 해시 진행(08-18 — 대용량 준비가 "무반응"으로 보이지 않게 · pct 0~100).
    HashProgress { peer: PeerId, name: String, pct: u8 },
    /// 발신 준비 완료(08-18 스트리밍 — 해시 워커 복귀 · None = 읽기 실패).
    SendHashed {
        peer: PeerId,
        path: std::path::PathBuf,
        size: u64,
        sha: Option<[u8; 32]>,
    },
    /// 발신 1건 **전송 끝(청크+Done 발신)** — 큐의 다음 파일로 넘어가되, UI는 아직 "완료"가
    /// 아니라 **확인 대기**다(수신 ack `XferAcked`가 와야 완료 · M4-9).
    XferSendDone {
        peer: PeerId,
        /// 평균 송신 속도(B/s · 08-16 — 청크 첫 발신~Done 송신 기준).
        avg_bps: u64,
        /// 세션 계측기의 **관측 최고**(B/s · 08-16) — Auto 설정 행 하단 표시용.
        /// 평균은 대기가 섞여 링크 능력을 과소 평가한다([`nbeep_core::RateMeter`] 문서).
        peak_bps: u64,
        /// 어느 파일인가(M4-2e — 파일 단위 즉시 종료 처리).
        name: String,
        size: u64,
    },
    /// **수신 종단 확인**(M4-9) — 상대가 격리까지 마쳤(`ok=true`)거나 실패(`ok=false`)했다.
    XferAcked {
        peer: PeerId,
        ok: bool,
        /// 어느 파일의 ack인가(M4-2e — 파일 단위 종결 · 빈 문자열 = FIFO 폴백).
        name: String,
        size: u64,
    },
    /// 대화 수신 확인 도착(N-2 · ADR-0010 §5) — 내가 보낸 메시지(seq)가 전달/확인됨.
    ChatAck {
        peer: PeerId,
        target_seq: u64,
        kind: nbeep_core::AckKind,
    },
    /// 수동 주소 연결 실패(DR-19 · M2-8 잔여 — 워커에서 돌아온다. 성공은 `Outbound`).
    AddFailed { addr: String, why: String },
    /// 공유 그룹 프레임 도착(M5-1g · ADR-0012) — 검증·적용은 메인(명부 단일 지점).
    SGroup {
        peer: PeerId,
        msg: nbeep_core::SGroupMsg,
    },
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
        /// 내장 아바타 키(08-14) — 바이트가 아니라 키만 온다(양쪽이 같은 자산 내장).
        avatar: Option<String>,
        /// 아바타 보더 색 "#RRGGBB"(08-14) — 수신측 parse_border 검증 후 반영.
        border: Option<String>,
        /// 경량 갱신 마커(M3-21) — 참이면 "공유 사진 그대로": 캐시를 지우지 않고
        /// 이전 사진·아바타를 승계한다(와이어 `Info.image_keep`).
        image_keep: bool,
        /// 소개글(08-17 · 와이어 `Info.bio`) — 목록 2번째 줄·카드에 표시.
        bio: Option<String>,
    },
    /// 격리 디코드 완료(워커 → 메인 · M4-5). ★ 그전엔 자식 프로세스 왕복(스폰 +
    /// Defender 검사 — Windows 실측 1~2초)이 **메인 스레드에서 동기**로 돌아, 대화창이
    /// 입력란도 못 그린 채 얼었다(08-13 실기). 픽셀은 원시 RGBA로 건너오고(그리기
    /// 타입 `Rc<IconImage>`는 메인 소유 — Send 아님) 감싸는 건 메인이 한다.
    Decoded {
        target: DecodeTarget,
        image: Option<(u32, u32, Vec<u8>)>,
    },
    /// 트레이 사용자 행동(M3-2a — 트레이 스레드 → 메인. 좌클릭/메뉴 열기·종료).
    Tray(nbeep_plat::tray::TrayEvent),
    /// 프로필 사진 **와이어 축소본** 생성 완료(워커 → 메인 · 08-16). 바이트가
    /// 오면 메인이 세대 일치를 확인하고 `me.wire.png`를 쓴 뒤 Full push로
    /// 실사진을 전파한다(보류했던 push가 여기서 한 번에 나간다 — 2단계 깜빡임
    /// 방지). 실패는 **소리 내어** 알린다(조용한 생략 금지 — 사용자 확정).
    WireAvatar { gen: u64, png: Option<Vec<u8>> },
    /// keytap 관측(G1 · H-26 — mac): 무수식 ASCII keydown이 모니터에 잡혔다.
    /// winit 도달 여부와 대조해 "삼켜진 1byte"만 보충 주입한다(판정은 틱에서).
    ///
    /// ⚠️ **mac 전용으로 막아 둔다** — 생산자(`install_keydown_tap`)가 mac 한정이라
    /// 다른 OS에서는 **한 번도 만들어지지 않는 변형**이 되고, `-D warnings`인 CI가
    /// `dead_code`로 떨군다(08-14 실측: budget·cross-build·test 3-OS 동시 red).
    /// 이 저장소가 반복해 밟는 함정 — **조건부 컴파일은 항상 반대편을 의심한다.**
    #[cfg(target_os = "macos")]
    RawKey(char),
}

/// 격리 디코드 요청의 목적지 — 요청(`spawn_decode`)과 완료([`AppEvent::Decoded`])를 잇는다.
#[derive(Debug)]
enum DecodeTarget {
    /// 상대 프로필 아바타(256 · 원형) — `peer_profiles`에 꽂고 목록 갱신.
    PeerAvatar(PeerId),
    /// 내 프로필 화면 아바타 미리보기(256 · 원형).
    MyAvatar,
    /// 수신 이미지의 대화 스레드 미리보기(96 · 사각 · M4-5ⓑ). 경로는 확대
    /// 미리보기 클릭 키로 스레드 항목에 함께 붙는다(08-16).
    XferThumb(PeerId, String),
    /// 확대 미리보기 원본(1024 · 사각 · 08-16) — 열린 뷰어 창에 꽂는다.
    FullImage(String),
    /// 격리함 행 썸네일(64 · 사각) — `.beepq` 경로 키의 캐시(`qthumbs`)로.
    QThumb(String),
    /// 프로필 "최근 이미지" 캐러셀 썸네일(64 · 원형 · 08-14) — 경로 키로 프로필
    /// 화면에 꽂는다(창이 닫혔으면 버린다 — 다음 열림에 재디코드).
    RecentThumb(String),
}

/// 격리 디코드를 **워커 스레드**로 보낸다(M2-8 연결 워커와 같은 문법 — 블로킹은
/// 워커에서, 복귀는 이벤트로). 실패(`None`)도 이벤트로 돌아온다(대상별로 무시/폴백).
fn spawn_decode(
    proxy: winit::event_loop::EventLoopProxy<AppEvent>,
    target: DecodeTarget,
    job: impl FnOnce() -> Option<(u32, u32, Vec<u8>)> + Send + 'static,
) {
    std::thread::spawn(move || {
        let image = job();
        let _ = proxy.send_event(AppEvent::Decoded { target, image });
    });
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
        /// 원본 경로(08-18 발신 스트리밍 — 바이트를 들고 다니지 않는다).
        path: std::path::PathBuf,
        size: u64,
    },
    /// 수신 제안 수락/거절(사용자 결정 — 자동 경로 없음).
    AcceptXfer {
        id: nbeep_core::XferId,
        /// 수신측 상한(B/s · 0 = 무제한) — Accept에 실어 발신측이 협상한다.
        rate_cap: u64,
        /// **이어받기 프리픽스**(M4-10 — `.part` 보존분). Some = 조립기에 선적재하고
        /// Accept에 재개 오프셋+프리픽스 해시를 실어 발신측 검증을 요청한다.
        resume: Option<Vec<u8>>,
    },
    /// 수신 파일 상한 변경(08-18 — `xfer.recv_max_mb` hot-swap · u64::MAX = 무제한).
    SetRecvMax(u64),
    /// 발신 취소(타임아웃·사용자) — 상대에게 Cancel 통지.
    CancelXfer(nbeep_core::XferId),
    /// 활성 발신 일시정지(M4-2d P2) — 세션 유지한 채 청크 펌프만 멈춘다(와이어
    /// 무변경 · 수신측은 다음 청크를 기다린다). 재개까지 이 전송은 진척 없다.
    PauseXfer(nbeep_core::XferId),
    /// 활성 발신 재개(M4-2d P2) — 멈춘 펌프를 다시 돌린다.
    ResumeXfer(nbeep_core::XferId),
    RejectXfer {
        id: nbeep_core::XferId,
        why: nbeep_core::RejectWhy,
    },
    /// Control 스트림으로 보낼 인코딩된 프레임들(프로필 요청/응답 — M3-17).
    /// 내용 구성은 메인 스레드 몫(공개 정책 판단 단일 지점) — 액터는 나르기만 한다.
    Control(Vec<Vec<u8>>),
    /// Group 스트림(M5-1g · ADR-0012)으로 보낼 인코딩된 `SGroupMsg` 프레임들.
    /// 구성·검증은 메인 몫(roster 소유자 확인 단일 지점) — 액터는 나르기만 한다.
    Group(Vec<Vec<u8>>),
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

/// 자동 재연결 백오프 간격(ms · 사용자 확정 08-13 ⓑ) — 복구 시도이지 감시가 아니다.
/// 상한(마지막 항) 시도까지 실패하면 **중단**한다(포트 스캔처럼 보이지 않게 · 수동
/// 클릭 = 처음부터 재개). 상시 주기 관찰(원격 프레즌스)은 ADR-0006 확장 결정 몫.
const RECONNECT_BACKOFF_MS: [u64; 4] = [5_000, 15_000, 60_000, 300_000];

/// 캐럿 깜빡임 반주기(ms) — Windows 기본 GetCaretBlinkTime≈530(DR-16 "동작 = OS
/// 네이티브"). 위상 감지는 ~5Hz 틱이라 ±200ms 지터가 있지만 점멸 인지엔 충분하다.
const CARET_BLINK_MS: u64 = 530;

/// 포커스 복귀 직후 목록 Enter를 무시하는 창(ms · 08-21) — 모달을 Enter로 제출하면
/// 창 파괴 직후 잔향·키 반복 Enter가 메인에 도달해 캐럿 행 대화가 열렸다(공지 실기).
/// 사람이 포커스 전환 후 이 안에 의도적으로 Enter를 치는 것은 사실상 불가능한 길이.
const ENTER_GUARD_MS: u64 = 300;

/// 이 단계의 대기 시간 — 단계를 다 썼으면 `None`(중단).
/// 서버 접속 실패 분류(X-2b) — 표시·백오프 판단용.
#[derive(Debug)]
enum ServerAttachFail {
    /// 주소 해석 실패(오타·DNS 일시 장애) — 백오프 재시도(DNS는 돌아올 수 있다).
    Resolve,
    /// ★ 핀 불일치 — **자동 재시도 없음**(재핀은 사람의 결정 · DR-28).
    PinMismatch,
    /// 접속·핸드셰이크 등 일시 실패 — 백오프 재시도.
    Other(String),
}

/// Managed 서버 목표(설정 SSOT · X-2b) — None = Unmanaged/주소 없음(S-0 그대로).
/// 주소에 `:`가 있으면 그대로 쓰고(포트 명시·IPv6는 `[..]:port` 표기), 없으면 설정
/// 포트를 붙인다. 해석·기본 포트 보정은 `nbeep_relay::resolve_server` 몫.
fn server_target(mode: &str, address: &str, port: &str) -> Option<String> {
    if mode != "managed" {
        return None;
    }
    let addr = address.trim();
    if addr.is_empty() {
        return None;
    }
    let port = port.trim();
    if addr.contains(':') || port.is_empty() {
        Some(addr.to_string())
    } else {
        Some(format!("{addr}:{port}"))
    }
}

/// 서버 재접속 백오프(X-2b) — 복구 시도이지 감시가 아니다(13 §12-1). **300s 상한 반복**
/// — 상대 재연결 ⓑ와 달리 중단하지 않는다: 서버는 사용자가 명시 등록한 인프라라
/// 돌아오면 다시 붙어 있어야 하고, 5분 1회는 무의미 트래픽이 아니다.
fn server_retry_delay(stage: u8) -> u64 {
    match stage {
        0 => 5_000,
        1 => 15_000,
        2 => 60_000,
        _ => 300_000,
    }
}

fn reconnect_delay(stage: u8) -> Option<u64> {
    RECONNECT_BACKOFF_MS.get(stage as usize).copied()
}

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

/// 요청 대기 슬롯의 원격 세션(M5-3b · §6) — (상대, 핸드셰이크 완료 세션, 경로 등급).
type PendingRemote = (
    PeerId,
    nbeep_crypto::NoiseSession<Box<dyn nbeep_core::Link>>,
    nbeep_core::PathClass,
);

/// 인바운드 세션 봉투 — `AppEvent`가 Debug라야 해서 수동 Debug.
struct InboundSession {
    session: nbeep_crypto::NoiseSession<Box<dyn nbeep_core::Link>>,
    /// 이 세션이 성립한 **경로 등급**(M5-3c · DR-28) — 핸드셰이크 직전 실소켓
    /// 주소로 판정해 세션과 동행한다(광고가 아니라 성립한 세션이 정한다 — §5-1-5).
    path: nbeep_core::PathClass,
}
impl std::fmt::Debug for InboundSession {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("InboundSession").finish_non_exhaustive()
    }
}

/// 확대 미리보기 상태(08-16) — 격리물 경로가 키. 디코드는 워커(imgdec 격리 ·
/// 16MiB 미리보기 상한(08-16 상향) — 초과·손상은 Failed 안내).
/// 격리함 스캔 원시 행(08-18 — 워커 산출물 · 표시 가공은 메인이).
#[derive(Clone, Debug)]
struct QRowRaw {
    path: String,
    name: String,
    size: u64,
    risk: nbeep_core::RiskLevel,
    mismatch: bool,
    sender: PeerId,
    received_at: u64,
    /// 검사 결과(FR-S-15 · 08-21) — 목록 사실 표기(검사됨/탐지/안 됨).
    scan: nbeep_core::ScanOutcome,
    /// 아카이브 정책 위반(M4-4 · 08-21 — zip Slip·폭탄·판정 불가 · 위험색 라벨).
    archive_viol: bool,
    /// 무결성 검증 완료(08-18) — 사이드카로 즉시 목록에 뜨지만 **전체 개봉·해시
    /// 확인**은 백그라운드다. false면 승인(Approve) 비활성(`QVerified`가 켠다).
    verified: bool,
}

struct ImageViewState {
    qpath: String,
    img: ImgLoad,
    /// 이 미리보기를 **연 창**(08-18) — 창 소유자와 같아야 한다. Windows는
    /// 생성 후 소유자를 못 바꾸므로, 진원이 다르면 창을 새로 만든다.
    owner: WindowId,
}

enum ImgLoad {
    Loading,
    Ready(std::rc::Rc<nbeep_ui::IconImage>),
    Failed,
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
    /// 소개글(08-17 · 옵트인 — 목록 2번째 줄·카드). 여러 줄 가능.
    bio: Option<String>,
    /// 캐시된 이미지 파일(`data/profiles/…`).
    image_file: Option<std::path::PathBuf>,
    /// imgdec 격리 디코드 결과(원형 마스크 완료 · M4-5) — 목록·카드가 그린다.
    avatar: Option<std::rc::Rc<nbeep_ui::IconImage>>,
    /// 아바타 보더 색(08-14 — parse_border 검증 통과분).
    border: Option<(u8, u8, u8)>,
    /// 마지막 프로필 수신 시각(unix ms · M3-21 ③) — 0 = 미상(부팅 캐시 복원분은
    /// 파일 mtime · 없으면 0). 카드가 신선도로 표기한다("N분 전 수신").
    received_ms: u64,
}

/// 프로필 캐시 메타 직렬화(08-14 사용자 요청 — 재시작 시 캐시된 프로필 표시).
/// 여기 넣는 것은 **와이어 검증 통과분의 저민감 표시값**(내장 12간지 키·보더 색)뿐이다 —
/// 이름은 trust.seg(암호화 · `record_name`)가, 이미지 바이트는 `profiles/{peer}.img`가
/// 이미 들고 있고, **이메일·전화는 캐시하지 않는다**(연락처 평문 at-rest 회피 ·
/// ADR-0005 결 — 재연결 프리페치가 다시 채운다).
fn encode_profile_meta(avatar_key: Option<&str>, border: Option<(u8, u8, u8)>) -> String {
    let mut out = String::new();
    if let Some(k) = avatar_key {
        out.push_str("avatar=");
        out.push_str(k);
        out.push('\n');
    }
    if let Some(rgb) = border {
        out.push_str("border=");
        out.push_str(&nbeep_core::avatar::border_to_setting(rgb));
        out.push('\n');
    }
    out
}

/// [`encode_profile_meta`]의 역 — 미지 줄은 무시(전방 관용)하되 값은 **다시 검증**한다.
/// 파일은 사람이 고칠 수 있다 — 12간지 밖 키·무효 색은 조용히 버림(fail-closed:
/// 수신 경로와 같은 검증을 복원 경로에도 그대로 태운다).
fn parse_profile_meta(s: &str) -> (Option<String>, Option<(u8, u8, u8)>) {
    let mut key = None;
    let mut border = None;
    for line in s.lines() {
        if let Some(v) = line.strip_prefix("avatar=") {
            if nbeep_core::avatar::ZODIAC.contains(&v) {
                key = Some(v.to_string());
            }
        } else if let Some(v) = line.strip_prefix("border=") {
            border = nbeep_core::avatar::parse_border(v);
        }
    }
    (key, border)
}

/// 프로필 전파 유형(M3-21 — "되도록 묶어 한 번에, 필요하면 분화" · 사용자 확정).
/// 정보량이 작아 기본은 전체 묶음이고, 무거운 것(사진 256KiB)만 유형을 가른다.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ProfileScope {
    /// Info + 사진 청크 전부 — 성립 프리페치·Request 응답·사진에 영향 주는 변경.
    Full,
    /// Info 한 프레임만 — 텍스트·토글·아바타 키·보더 변경. 공유 사진이 있으면
    /// `image_keep` 마커로 "캐시 유지"를 알린다(사진 재전송 생략).
    Info,
}

/// 이 설정 키의 변경이 프로필 전파를 요구하는가 — 요구하면 어떤 유형인가(M3-21).
/// **프로필 응답 구성이 읽는 키 전부**가 여기 있어야 한다(빠지면 그 변경은 이미
/// 연결된 상대에게 영영 안 닿는다 — 08-14 실기: 이메일 공유를 켰는데 상대는 "(비공개)").
fn profile_push_scope(key: &str) -> Option<ProfileScope> {
    match key {
        // 사진 유무·내용이 바뀔 수 있는 키 = 전체(사진 청크 동반).
        "profile.image_path" | "profile.share.basic" => Some(ProfileScope::Full),
        // 텍스트·토글·아바타 키·보더 = 경량(사진은 image_keep으로 유지 통지).
        "profile.display_name"
        | "profile.email"
        | "profile.phone"
        | "profile.bio"
        | "profile.share.email"
        | "profile.share.phone"
        | "profile.avatar"
        | "profile.avatar_border" => Some(ProfileScope::Info),
        _ => None,
    }
}

/// 대화 상태 — **뷰(창)와 분리**(DR-26). 세션은 **액터 스레드**가 소유하고, 여기엔 그
/// 액터로 보내는 송신 채널과 스레드 이력만 둔다(M2-7 — 비동기 수신 펌프).
struct Conversation {
    /// 액터에 보낼 명령(대화 바이트·파일 제어). 드롭 = 액터 종료 신호.
    out_tx: std::sync::mpsc::Sender<SessionCmd>,
    lines: Vec<ChatLine>,
    /// 이 세션이 성립한 경로 등급(M5-3c · DR-28) — 파일 정책은 신뢰 × 경로의 곱.
    path: nbeep_core::PathClass,
}

/// 진행 중 발신 상태(08-16 · 재개형 펌프) — Accept가 등록하고 액터 루프 틱이
/// 조금씩 보낸다. 취소(내 명령·상대 Cancel)는 이 상태를 지우는 것으로 끝난다.
/// 대기 중 수신 제안 한 건 — (전송 id, 파일명, 크기, 전체 sha · M4-10 재개 앵커).
type PendingOffer = (nbeep_core::XferId, String, u64, [u8; 32]);

struct ActiveSend {
    id: nbeep_core::XferId,
    /// 열린 원본 파일(08-18 스트리밍) — 청크는 오프셋에서 직접 읽는다(메모리 O(청크)).
    file: std::fs::File,
    /// 전송하며 쌓는 전체 해시(08-18 지연 선언) — Done에 동봉된다.
    hasher: nbeep_crypto::Sha256Stream,
    /// 원본 경로·시작 시점 mtime(08-18 — **전송 중 원본 변경 가드**): 지연
    /// 선언은 "읽힌 그대로"의 해시라, 도중에 파일이 바뀌면 찢어진 내용이
    /// '무결'로 통과할 수 있다. Done 직전 mtime이 다르면 중단한다.
    src: (std::path::PathBuf, Option<std::time::SystemTime>),
    /// 다음 보낼 오프셋(bytes).
    next: u64,
    total: u64,
    pacer: nbeep_core::Pacer,
    /// 상대가 공지한 수신 상한(0 = 무제한) — 프로브 후 재협상에 다시 쓴다.
    rate_cap: u64,
    /// 프로브(첫 2MiB 무페이싱) 종료 후 실측 기반 재산출을 마쳤는가(M4-11 ⓑ).
    probed: bool,
    /// 발신 시작 시각(ms — 평균 속도 표기용 · 08-16).
    started_ms: u64,
    /// 이번 세션에서 **실제로 보내기 시작한 오프셋**(M4-10b 재개 — 0이 아니면
    /// 이어보내기). 평균 속도는 (total − from)/시간이어야 정직하다.
    from: u64,
    /// 다음 상향 프로브 시작 오프셋(M4-11 ⓐ · u64::MAX = 미정 — 첫 프로브 후 설정).
    next_probe: u64,
    /// 진행 중 상향 버스트의 (시작 오프셋, 시작 시각) — Some = 무페이싱 구간.
    bursting: Option<(u64, u64)>,
}

/// 무페이싱 프로브 구간(M4-11 ⓑ) — 첫 2MiB는 전속으로 보내 **진짜 링크
/// 능력**을 관측한다. 종전엔 하한으로 조인 채 보내 관측이 목표에 갇혔다
/// (페이싱 중 관측은 링크 능력이 아니라 자기 목표를 재는 것 — 되먹임 고착).
const RATE_PROBE_BYTES: u64 = 2 * 1024 * 1024;
/// 주기 상향 프로브(M4-11 ⓐ · 08-16) — 페이싱 중 8MiB마다 512KiB를 전속으로
/// 보내 재관측한다: 첫 프로브가 TCP 슬로우스타트에 걸려 **과소 관측**했으면
/// 여기서 따라 오른다(상향 전용 — note_burst는 max라 내려가지 않는다 · 링크
/// 악화 대응은 범위 밖). 버스트 비중 512KiB/8MiB ≈ 6% — 양보 원칙 실질 유지.
const REPROBE_INTERVAL: u64 = 8 * 1024 * 1024;
const REPROBE_BURST: u64 = 512 * 1024;

/// 액터 쪽 단조 시각(ms) — Pacer·RateMeter가 **같은 기준**을 봐야 해서 한 곳
/// (xfer_step과 펌프 틱이 공유 · 기준이 갈리면 대역 계산이 깨진다).
fn xfer_now_ms() -> u64 {
    use std::sync::OnceLock;
    static T0: OnceLock<std::time::Instant> = OnceLock::new();
    #[allow(clippy::cast_possible_truncation)]
    {
        T0.get_or_init(std::time::Instant::now)
            .elapsed()
            .as_millis() as u64
    }
}

/// 세션 액터 — 세션을 전용 스레드로 옮겨 **수신(GUI로 프록시)과 송신(채널)을 교대**한다.
/// snow `TransportState`가 read/write에 `&mut`를 요구해 한 세션은 한 스레드가 소유해야
/// 하므로, 송신은 채널로 요청받는 액터 모델이 정석이다.
fn spawn_session_actor(
    mut session: LiveSession,
    out_rx: std::sync::mpsc::Receiver<SessionCmd>,
    proxy: winit::event_loop::EventLoopProxy<AppEvent>,
    send_rate: nbeep_core::RateLimit,
    seal_secret: [u8; 32],
    recv_max: u64,
) -> std::thread::JoinHandle<()> {
    use nbeep_core::mux::StreamId;
    use nbeep_core::{XferInbox, XferMsg};
    let peer = session.peer();
    // 수신 폴 타임아웃 — recv가 100ms마다 TimedOut으로 돌아와 송신과 교대한다.
    session.set_recv_timeout(Some(std::time::Duration::from_millis(100)));
    std::thread::spawn(move || {
        // 파일 상태는 액터가 소유한다 — GUI 스레드는 이벤트만 받는다(대용량 조립이
        // 메인 스레드를 막지 않게).
        let mut inbox = XferInbox::with_max_file(recv_max); // 수신 상한 = 설정(08-18)
                                                            // 세션 성립 즉시 내 수신 상한 공지(08-18) — 상대 발신자가 해시 전에
                                                            // 초과를 차단할 근거. 구버전 상대는 미지 태그를 무시한다(전방 호환).
        let _ = session.send(
            StreamId::Control,
            &nbeep_core::xfer::encode_cap_advert(inbox.max_file()),
        );
        // (경로, 크기) — 발신 스트리밍(08-18): 수락 시 파일을 열어 청크 단위로
        // 직접 읽는다(10.7GB ISO DnD가 fs::read 전량 조립으로 앱을 얼렸다).
        let mut outgoing: HashMap<nbeep_core::XferId, (std::path::PathBuf, u64)> = HashMap::new();
        // 수신 시작 시각(xid별 · 08-16) — 평균 수신 속도 표기용(첫 청크 기준).
        let mut recv_started: HashMap<nbeep_core::XferId, u64> = HashMap::new();
        // 수신 파일명(M4-2e — 진행률 이벤트에 신원을 실어 라인 오염 방지).
        let mut recv_names: HashMap<nbeep_core::XferId, String> = HashMap::new();
        // 발신 파일명(M4-2e — 종단 ack를 파일 단위로 그 라인에 닫는다).
        let mut sent_names: HashMap<nbeep_core::XferId, (String, u64)> = HashMap::new();
        // 취소된 전송(08-18 실기) — 취소 후에도 와이어에 남은 청크·Done이 몇 개
        // 도착한다(발신 펌프가 앞서 보낸 것). 아는 취소분은 **조용히 버린다** —
        // 종전엔 UnknownXfer로 "수신 오류" 실패 이벤트가 건마다 발화했다.
        // 세션 수명 동안 전송 수만큼만 자란다(상한 실질 무해).
        let mut canceled: std::collections::HashSet<nbeep_core::XferId> =
            std::collections::HashSet::new();
        let mut send_meter = nbeep_core::RateMeter::default();
        // ★ 재개형 발신 펌프(08-16 · 취소 UX 선행) — 종전엔 Accept 처리 안의
        //   블로킹 루프가 전량을 쏟아, 펌프가 도는 동안 명령(취소)도 수신(상대
        //   취소)도 못 봤다. 상태로 들고 틱마다 조금씩 보내며 교대한다.
        let mut sending: Option<ActiveSend> = None;
        // ★ 일시정지 보관 슬롯(M4-2e ⓐ · 08-19) — 정지된 ActiveSend(열린 파일·
        //   해셔·오프셋)를 **옆에 보관**하고 다음 파일이 sending을 쓴다. 종전
        //   paused_xid 게이트는 다음 Accept가 sending을 덮어써 정지 전송을
        //   파괴했다(실기: 재개 불가·진행률 오염). bool = 재개 예약(활성 종료 후).
        let mut parked: Vec<(ActiveSend, bool)> = Vec::new();
        // 수락 전 정지 의사(오퍼는 나갔는데 아직 Accept 전) — Accept로 활성화되는
        // 즉시 보관 슬롯으로 옮긴다.
        let mut pause_wanted: std::collections::HashSet<nbeep_core::XferId> =
            std::collections::HashSet::new();
        // 수신 폴 타임아웃(ms) — 유휴 100ms / **전송 중 1ms**. 틱당 4청크 뒤
        // 100ms를 꽉 채워 기다리면 처리량이 128KiB/100ms ≈ 1.3MB/s로 조인다
        // (08-16 실기 — localhost 11.6MiB가 9초. 재개형 펌프 도입의 회귀).
        let mut recv_to_ms: u64 = 100;
        // 프로필 이미지 조립 상태(M3-17) — (텍스트 필드+아바타 키+보더, 기대 총량, 누적 바이트).
        type PendingProfile = (
            (
                Option<String>,
                Option<String>,
                Option<String>,
                Option<String>,
                Option<String>,
                Option<String>, // bio(08-17)
            ),
            u32,
            Vec<u8>,
        );
        let mut pending_profile: Option<PendingProfile> = None;
        // ★ 루프를 IIFE로 감싼다(M4-10a) — 내부의 `return`(세션 종료 경로 전부)이
        //   클로저만 빠져나오고, 아래의 **부분 수신물 보존**이 반드시 돈다.
        (|| {
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
                            path,
                            size,
                        } => {
                            let offer = XferMsg::Offer {
                                id,
                                size,
                                sha256: sha,
                                name: name.into_bytes(),
                            };
                            sent_names.insert(
                                id,
                                (
                                    path.file_name()
                                        .map(|n| n.to_string_lossy().into_owned())
                                        .unwrap_or_default(),
                                    size,
                                ),
                            );
                            outgoing.insert(id, (path, size));
                            session.send(StreamId::File, &offer.encode())
                        }
                        SessionCmd::AcceptXfer {
                            id,
                            rate_cap,
                            resume,
                        } => {
                            // 이어받기(M4-10) — 프리픽스 선적재 + 재개 꼬리. 선적재
                            // 실패(범위 초과 등)면 처음부터(재개는 최적화 — fail-open으로
                            // 전송 자체는 살린다 · 손상 방지는 발신측 해시 대조가 맡는다).
                            let (resume_offset, prefix_sha) = match resume {
                                Some(prefix) if !prefix.is_empty() => {
                                    let sha = nbeep_crypto::sha256(&prefix);
                                    let off = prefix.len() as u64;
                                    if inbox.accept_with_prefix(&id, prefix).is_ok() {
                                        (off, sha)
                                    } else if inbox.accept(&id).is_ok() {
                                        (0, [0u8; 32])
                                    } else {
                                        (u64::MAX, [0u8; 32]) // 미지 xfer — 아래서 무시
                                    }
                                }
                                _ => {
                                    if inbox.accept(&id).is_ok() {
                                        (0, [0u8; 32])
                                    } else {
                                        (u64::MAX, [0u8; 32])
                                    }
                                }
                            };
                            if resume_offset == u64::MAX {
                                Ok(())
                            } else {
                                session.send(
                                    StreamId::File,
                                    &XferMsg::Accept {
                                        id,
                                        rate_cap,
                                        resume_offset,
                                        prefix_sha,
                                    }
                                    .encode(),
                                )
                            }
                        }
                        SessionCmd::SetRecvMax(v) => {
                            inbox.set_max_file(v);
                            // 변경 즉시 재공지 — 상대 캐시가 낡지 않게(hot-swap 전파).
                            session.send(StreamId::Control, &nbeep_core::xfer::encode_cap_advert(v))
                        }
                        SessionCmd::CancelXfer(id) => {
                            outgoing.remove(&id);
                            inbox.drop_xfer(&id); // 수신측 취소 — 부분 조립 폐기(08-16)
                            canceled.insert(id); // 잔류 청크 조용히 폐기(08-18)
                            if sending.as_ref().is_some_and(|st| st.id == id) {
                                sending = None; // 진행 중 발신 취소 — 펌프 즉시 중단
                            }
                            parked.retain(|(st, _)| st.id != id); // 보관 정지분도 취소
                            pause_wanted.remove(&id);
                            session.send(StreamId::File, &XferMsg::Cancel { id }.encode())
                        }
                        SessionCmd::PauseXfer(id) => {
                            // 활성이면 보관 슬롯으로(다음 파일이 이어서 나간다 · ⓐ) ·
                            // 수락 전이면 의사만 기억(Accept 즉시 보관).
                            if sending.as_ref().is_some_and(|st| st.id == id) {
                                if let Some(st) = sending.take() {
                                    let _ = proxy.send_event(AppEvent::XferPaused {
                                        peer,
                                        id: st.id,
                                        name: st
                                            .src
                                            .0
                                            .file_name()
                                            .map(|n| n.to_string_lossy().into_owned())
                                            .unwrap_or_default(),
                                        size: st.total,
                                        done: st.next,
                                    });
                                    let _ = session.send(
                                        StreamId::Control,
                                        &nbeep_core::xfer::encode_pause_req(st.id, true),
                                    );
                                    parked.push((st, false));
                                }
                            } else {
                                pause_wanted.insert(id);
                            }
                            Ok(())
                        }
                        SessionCmd::ResumeXfer(id) => {
                            pause_wanted.remove(&id);
                            if let Some(pos) = parked.iter().position(|(st, _)| st.id == id) {
                                if sending.is_none() {
                                    let (st, _) = parked.remove(pos);
                                    let _ = proxy.send_event(AppEvent::XferResumed {
                                        peer,
                                        id: st.id,
                                        name: st
                                            .src
                                            .0
                                            .file_name()
                                            .map(|n| n.to_string_lossy().into_owned())
                                            .unwrap_or_default(),
                                        size: st.total,
                                        done: st.next,
                                    });
                                    let _ = session.send(
                                        StreamId::Control,
                                        &nbeep_core::xfer::encode_pause_req(st.id, false),
                                    );
                                    sending = Some(st);
                                } else {
                                    parked[pos].1 = true; // 활성 종료 후 이어감
                                }
                            }
                            Ok(())
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
                        SessionCmd::Group(frames) => {
                            let mut r = Ok(());
                            for f in frames {
                                r = session.send(StreamId::Group, &f);
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
                // ★ Accept 직후 정지 의사 승계(M4-2e) — 방금 활성화된 전송이
                //   pause_wanted면 즉시 보관 슬롯으로(수락 전 눌린 일시정지).
                if sending
                    .as_ref()
                    .is_some_and(|st| pause_wanted.contains(&st.id))
                {
                    if let Some(st) = sending.take() {
                        pause_wanted.remove(&st.id);
                        let _ = proxy.send_event(AppEvent::XferPaused {
                            peer,
                            id: st.id,
                            name: st
                                .src
                                .0
                                .file_name()
                                .map(|n| n.to_string_lossy().into_owned())
                                .unwrap_or_default(),
                            size: st.total,
                            done: st.next,
                        });
                        let _ = session.send(
                            StreamId::Control,
                            &nbeep_core::xfer::encode_pause_req(st.id, true),
                        );
                        parked.push((st, false));
                    }
                }
                // ★ 재개 예약 스왑인(ⓐ) — 활성 슬롯이 비면 예약 정지분을 이어간다.
                if sending.is_none() {
                    if let Some(pos) = parked.iter().position(|(_, w)| *w) {
                        let (st, _) = parked.remove(pos);
                        let _ = proxy.send_event(AppEvent::XferResumed {
                            peer,
                            id: st.id,
                            name: st
                                .src
                                .0
                                .file_name()
                                .map(|n| n.to_string_lossy().into_owned())
                                .unwrap_or_default(),
                            size: st.total,
                            done: st.next,
                        });
                        let _ = session.send(
                            StreamId::Control,
                            &nbeep_core::xfer::encode_pause_req(st.id, false),
                        );
                        sending = Some(st);
                    }
                }
                // 발신 펌프 틱 — 한 틱 최대 4청크(128KiB) 보내고 명령·수신과 교대한다
                // (취소 지연 상한 = 청크 4개 + 페이싱 대기 · 대역 협상은 Pacer 그대로 ·
                // 정지분은 parked에 있어 sending은 늘 가동 대상이다).
                let want_to: u64 = if sending.is_some() { 1 } else { 100 };
                if want_to != recv_to_ms {
                    recv_to_ms = want_to;
                    session.set_recv_timeout(Some(std::time::Duration::from_millis(want_to)));
                }
                if let Some(st) = sending.as_mut() {
                    let mut done = false;
                    let mut read_fail = false;
                    for _ in 0..4 {
                        if st.next >= st.total {
                            done = true;
                            break;
                        }
                        let off = st.next;
                        let end = (off + nbeep_core::MAX_CHUNK as u64).min(st.total);
                        let n = end - off;
                        // 프로브 경계(M4-11 ⓑ) — 첫 2MiB를 전속으로 보냈으니 meter가
                        // **진짜 peak**를 안다. 목표를 실측 기반으로 재산출(한 번만).
                        if !st.probed && off >= RATE_PROBE_BYTES.min(st.total.saturating_sub(1)) {
                            st.probed = true;
                            // 프로브 실측을 peak에 즉시 반영(M4-11 후속) — 관측 창
                            // (500ms)이 닫히길 기다리면 localhost에선 peak=0인 채
                            // 재산출돼 하한 조임이 재발한다(실기: 1차만 316KiB/s).
                            let probe_dur = xfer_now_ms().saturating_sub(st.started_ms);
                            send_meter.note_burst(off, probe_dur);
                            let local = send_rate.target_bps(&send_meter);
                            st.pacer =
                                nbeep_core::Pacer::new(nbeep_core::negotiate(local, st.rate_cap));
                            st.next_probe = off + REPROBE_INTERVAL;
                        }
                        // 상향 프로브(ⓐ) — 페이싱 중 주기 버스트로 재관측(과소 보정).
                        let mut unpaced = !st.probed;
                        if st.probed {
                            if let Some((b_off, b_t0)) = st.bursting {
                                if off.saturating_sub(b_off) >= REPROBE_BURST {
                                    // 버스트 종료 — 실측 반영·목표 재산출(상향 전용).
                                    send_meter.note_burst(
                                        off - b_off,
                                        xfer_now_ms().saturating_sub(b_t0),
                                    );
                                    let local = send_rate.target_bps(&send_meter);
                                    st.pacer = nbeep_core::Pacer::new(nbeep_core::negotiate(
                                        local,
                                        st.rate_cap,
                                    ));
                                    st.bursting = None;
                                    st.next_probe = off + REPROBE_INTERVAL;
                                } else {
                                    unpaced = true; // 버스트 진행 중 — 전속
                                }
                            } else if off >= st.next_probe {
                                st.bursting = Some((off, xfer_now_ms()));
                                unpaced = true;
                            }
                        }
                        if !unpaced {
                            // 창이 모자라면 그만큼 쉰다(종전 문법 그대로 — 예약 후 대기).
                            let wait = st.pacer.take(n, xfer_now_ms());
                            if wait > 0 {
                                std::thread::sleep(std::time::Duration::from_millis(wait));
                            }
                        }
                        // 파일에서 직접 읽기(08-18 스트리밍) — 실패 = 전송 중단
                        // (원본이 이동·삭제된 경우 — 조용한 부분 전송 금지).
                        let mut data = vec![0u8; usize::try_from(n).unwrap_or(0)];
                        {
                            use std::io::{Read as _, Seek as _};
                            if st.file.seek(std::io::SeekFrom::Start(off)).is_err()
                                || st.file.read_exact(&mut data).is_err()
                            {
                                read_fail = true;
                                break;
                            }
                        }
                        st.hasher.update(&data); // 지연 선언 해시(순차 보장)
                        let chunk = XferMsg::Chunk {
                            id: st.id,
                            offset: off,
                            data,
                        };
                        if session.send(StreamId::File, &chunk.encode()).is_err() {
                            let _ = proxy.send_event(AppEvent::Closed { peer });
                            return;
                        }
                        st.next = end;
                        send_meter.observe(n, xfer_now_ms());
                        let _ = proxy.send_event(AppEvent::XferProgress {
                            peer,
                            got: end,
                            total: st.total,
                            sending: true,
                            name: st
                                .src
                                .0
                                .file_name()
                                .map(|n| n.to_string_lossy().into_owned())
                                .unwrap_or_default(),
                        });
                    }
                    if read_fail {
                        let id = st.id;
                        sending = None;
                        let _ = session.send(StreamId::File, &XferMsg::Cancel { id }.encode());
                        let _ = proxy.send_event(AppEvent::XferFailed {
                            peer,
                            why: nbeep_core::t(nbeep_core::Msg::XferSrcReadFail).into(),
                        });
                        continue;
                    }
                    if done {
                        let id = st.id;
                        let dur = xfer_now_ms().saturating_sub(st.started_ms).max(1);
                        // 재개면 이번 세션에 보낸 분량 기준(M4-10b — total로 나누면 과대).
                        let avg_bps = st.total.saturating_sub(st.from).saturating_mul(1000) / dur;
                        let st_own = sending.take().expect("done 판정 직후");
                        // 전송 중 원본 변경 가드 — 바뀌었으면 찢어진 파일이다(중단).
                        let mtime_now = std::fs::metadata(&st_own.src.0)
                            .ok()
                            .and_then(|m| m.modified().ok());
                        if mtime_now != st_own.src.1 {
                            let _ = session.send(StreamId::File, &XferMsg::Cancel { id }.encode());
                            let _ = proxy.send_event(AppEvent::XferFailed {
                                peer,
                                why: nbeep_core::t(nbeep_core::Msg::XferSrcChanged).into(),
                            });
                            continue;
                        }
                        let sha256 = st_own.hasher.finalize(); // 지연 선언(08-18)
                        if session
                            .send(StreamId::File, &XferMsg::Done { id, sha256 }.encode())
                            .is_err()
                        {
                            let _ = proxy.send_event(AppEvent::Closed { peer });
                            return;
                        }
                        let _ = proxy.send_event(AppEvent::XferSendDone {
                            peer,
                            avg_bps,
                            peak_bps: send_meter.peak_bps(),
                            name: st_own
                                .src
                                .0
                                .file_name()
                                .map(|n| n.to_string_lossy().into_owned())
                                .unwrap_or_default(),
                            size: st_own.total,
                        });
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
                            let importance = match m.importance {
                                nbeep_core::Importance::Urgent => 2,
                                nbeep_core::Importance::Notice => 1,
                                nbeep_core::Importance::Normal => 0,
                            };
                            if let nbeep_core::MessageBody::Text(t) = m.body {
                                let ev = AppEvent::Recv {
                                    peer,
                                    text: nbeep_core::sanitize_message(&t),
                                    seq: m.seq,
                                    sender: m.sender_device,
                                    importance,
                                    broadcast: m.broadcast,
                                };
                                if proxy.send_event(ev).is_err() {
                                    return; // 이벤트 루프 종료
                                }
                            }
                        }
                    }
                    // 프로필 수신(Control — M3-17). 요청은 메인으로 올리고(정책 단일 지점),
                    // 응답은 여기서 조립해 완성본만 올린다(대용량 조립이 메인을 막지 않게).
                    // 수신 상한 공지·질의(08-18) — 태그 11/12 · 다른 Control보다 먼저.
                    StreamId::Control if nbeep_core::xfer::decode_cap_advert(&bytes).is_some() => {
                        if let Some(cap) = nbeep_core::xfer::decode_cap_advert(&bytes) {
                            let _ = proxy.send_event(AppEvent::PeerRecvCap { peer, cap });
                        }
                    }
                    StreamId::Control if nbeep_core::xfer::is_cap_request(&bytes) => {
                        let _ = session.send(
                            StreamId::Control,
                            &nbeep_core::xfer::encode_cap_advert(inbox.max_file()),
                        );
                    }
                    // 배치 목록(M4-2e · 태그 13) — 요청 단위 승인·수신 목록의 원료.
                    StreamId::Control
                        if nbeep_core::xfer::decode_batch_manifest(&bytes).is_some() =>
                    {
                        if let Some(entries) = nbeep_core::xfer::decode_batch_manifest(&bytes) {
                            let _ = proxy.send_event(AppEvent::XferManifest { peer, entries });
                        }
                    }
                    // 큐 단계 정지 통지(M4-2e ①ⓐ · 태그 17) — 발신측이 오퍼 전
                    // 파일을 멈췄다/이었다: 이름 기반 통지 → 수신 라인 동기화.
                    StreamId::Control if nbeep_core::xfer::decode_qpause(&bytes).is_some() => {
                        if let Some((name, paused)) = nbeep_core::xfer::decode_qpause(&bytes) {
                            let _ = proxy.send_event(AppEvent::XferPeerPauseNotice {
                                peer,
                                name,
                                paused,
                            });
                        }
                    }
                    // 전체 취소 전파(M4-2e · 태그 16) — 상대가 Cancel all: 내 발신
                    // 펌프·보관 정지분을 즉시 중단하고 앱이 로컬 전체취소를 돌린다.
                    StreamId::Control if nbeep_core::xfer::is_cancel_all(&bytes) => {
                        sending = None;
                        parked.clear();
                        pause_wanted.clear();
                        let _ = proxy.send_event(AppEvent::XferCancelAllNotice { peer });
                    }
                    // 수신측 일시정지/재개 요청(M4-2e · 태그 14/15) — 발신 펌프 게이트.
                    StreamId::Control if nbeep_core::xfer::decode_pause_req(&bytes).is_some() => {
                        if let Some((id, pause)) = nbeep_core::xfer::decode_pause_req(&bytes) {
                            // ★ 내가 그 파일의 **수신자**면 = 발신측 상태 통지
                            //   (M4-2e — 수신 화면도 정지/재개가 보여야 한다).
                            if let Some(name) = recv_names.get(&id).cloned() {
                                let _ = proxy.send_event(AppEvent::XferPeerPauseNotice {
                                    peer,
                                    name,
                                    paused: pause,
                                });
                            } else if pause {
                                if sending.as_ref().is_some_and(|st| st.id == id) {
                                    if let Some(st) = sending.take() {
                                        let _ = proxy.send_event(AppEvent::XferPaused {
                                            peer,
                                            id: st.id,
                                            name: st
                                                .src
                                                .0
                                                .file_name()
                                                .map(|n| n.to_string_lossy().into_owned())
                                                .unwrap_or_default(),
                                            size: st.total,
                                            done: st.next,
                                        });
                                        let _ = session.send(
                                            StreamId::Control,
                                            &nbeep_core::xfer::encode_pause_req(st.id, true),
                                        );
                                        parked.push((st, false));
                                    }
                                } else {
                                    pause_wanted.insert(id);
                                }
                            } else {
                                pause_wanted.remove(&id);
                                if let Some(pos) = parked.iter().position(|(st, _)| st.id == id) {
                                    if sending.is_none() {
                                        let (st, _) = parked.remove(pos);
                                        let _ = proxy.send_event(AppEvent::XferResumed {
                                            peer,
                                            id: st.id,
                                            name: st
                                                .src
                                                .0
                                                .file_name()
                                                .map(|n| n.to_string_lossy().into_owned())
                                                .unwrap_or_default(),
                                            size: st.total,
                                            done: st.next,
                                        });
                                        let _ = session.send(
                                            StreamId::Control,
                                            &nbeep_core::xfer::encode_pause_req(st.id, false),
                                        );
                                        sending = Some(st);
                                    } else {
                                        parked[pos].1 = true;
                                    }
                                }
                            }
                        }
                    }
                    StreamId::Control if nbeep_core::ChatAck::decode(&bytes).is_some() => {
                        // 수신 확인(N-2) — 태그로 프로필과 구분. 도착만 메인에 올린다.
                        if let Some(ack) = nbeep_core::ChatAck::decode(&bytes) {
                            let ev = AppEvent::ChatAck {
                                peer,
                                target_seq: ack.target_seq,
                                kind: ack.kind,
                            };
                            if proxy.send_event(ev).is_err() {
                                return;
                            }
                        }
                    }
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
                            avatar,
                            border,
                            image_keep,
                            bio,
                        }) => {
                            let len = image_len as usize;
                            if len == 0 || len > nbeep_core::PROFILE_IMAGE_MAX {
                                // 이미지 없음 또는 상한 초과 주장 — 텍스트만 반영(fail-closed).
                                // image_keep(M3-21 경량 갱신)은 그대로 올린다 — 캐시 유지
                                // 판단은 메인 몫(peer_profiles가 거기 있다).
                                let _ = proxy.send_event(AppEvent::PeerProfile {
                                    peer,
                                    name,
                                    email,
                                    phone,
                                    image: None,
                                    avatar,
                                    border,
                                    image_keep,
                                    bio,
                                });
                                pending_profile = None;
                            } else {
                                pending_profile = Some((
                                    (name, email, phone, avatar, border, bio),
                                    image_len,
                                    Vec::with_capacity(len),
                                ));
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
                                    let (name, email, phone, avatar, border, bio) = fields.clone();
                                    let ok = in_order && last && buf.len() == *want as usize;
                                    let image = ok.then(|| std::mem::take(buf));
                                    let _ = proxy.send_event(AppEvent::PeerProfile {
                                        peer,
                                        name,
                                        email,
                                        phone,
                                        image,
                                        avatar,
                                        border,
                                        // 청크가 따라온 응답은 언제나 전체(Full) — 유지
                                        // 마커가 아니라 실물이 왔다.
                                        image_keep: false,
                                        bio,
                                    });
                                    pending_profile = None;
                                }
                            }
                        }
                        None => {} // 미지 kind — 전방 호환 무시
                    },
                    // 공유 그룹(M5-1g) — 검증(소유자·명부)은 메인 몫, 액터는 해독·전달만.
                    StreamId::Group => {
                        if let Some(msg) = nbeep_core::SGroupMsg::decode(&bytes) {
                            if proxy.send_event(AppEvent::SGroup { peer, msg }).is_err() {
                                return;
                            }
                        } // 미지 kind — 전방 호환 무시
                    }
                    // 파일 수신.
                    StreamId::File => {
                        if xfer_step(
                            &bytes,
                            peer,
                            &mut session,
                            &mut inbox,
                            &mut outgoing,
                            &mut sending,
                            &mut recv_started,
                            &mut recv_names,
                            &mut sent_names,
                            &mut canceled,
                            &proxy,
                            send_rate,
                            &mut send_meter,
                            &seal_secret,
                        )
                        .is_err()
                        {
                            let _ = proxy.send_event(AppEvent::Closed { peer });
                            return;
                        }
                    }
                }
            }
        })();
        // 세션 종료 — 수락된 부분 수신물을 봉인 보존한다(M4-10a · D-31).
        // 재개 협상의 원료: 같은 sha+size Offer가 다시 오면 "이어받기"가 된다.
        for p in inbox.take_partials() {
            crate::part::save_partial(crate::gate::CH_GUI, &seal_secret, peer, &p);
        }
    })
}

/// 평균 속도 라벨(08-16 — 완료 항목 "평균 N/s"). 0은 표기 생략용 빈 문자열.
fn speed_label(bps: u64) -> String {
    const K: u64 = 1024;
    if bps == 0 {
        return String::new();
    }
    #[allow(clippy::cast_precision_loss)]
    let t = match bps {
        v if v >= K * K => format!("{:.1}MiB/s", v as f64 / (K * K) as f64),
        v if v >= K => format!("{:.1}KiB/s", v as f64 / K as f64),
        v => format!("{v}B/s"),
    };
    t
}

/// 현재 Unix 밀리초(목록 속성 — 최근 접속·대화 기록 · 08-15).
fn unix_now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| u64::try_from(d.as_millis()).unwrap_or(u64::MAX))
}

/// 상대 시각 표기(08-15 — 프로필 카드 "최근 접속/대화"). 0 = 기록 없음 = 빈 문자열.
fn ago_label(unix_ms: u64) -> String {
    if unix_ms == 0 {
        return String::new();
    }
    let now = unix_now_ms();
    let s = now.saturating_sub(unix_ms) / 1000;
    // 상대 시각(08-17 i18n) — 달 = 30일·년 = 365일 근사(사람이 읽는 어림).
    use nbeep_core::{tf, Msg};
    let n = |v: u64| v.to_string();
    match s {
        0..=59 => nbeep_core::t(Msg::AgoJustNow).to_string(),
        60..=3_599 => tf(Msg::AgoMinutes, &[&n(s / 60)]),
        3_600..=86_399 => tf(Msg::AgoHours, &[&n(s / 3_600)]),
        86_400..=2_591_999 => tf(Msg::AgoDays, &[&n(s / 86_400)]),
        2_592_000..=31_535_999 => tf(Msg::AgoMonths, &[&n(s / 2_592_000)]),
        _ => tf(Msg::AgoYears, &[&n(s / 31_536_000)]),
    }
}

/// 목록 정렬 키(08-15 사용자 확정 · 3차 개정) — **고정 구획 → 접속 계층 → 모드별
/// 시각 사슬**. 반환은 오름차순 정렬용 튜플(작을수록 앞) · 이름 동률은 호출자 몫.
///
/// - 접속 계층 `tier`: 0 = 세션 중(녹색) · 1 = 발견됨 · 2 = 오프라인 — 발견만 된
///   상대와 대화 세션 중인 상대를 같은 층에 두지 않는다(사용자 "온라인이 더 위").
/// - ★ **최근 접속은 분 단위 버킷**으로 비교한다(08-15 실기 — 발견 비컨(800ms)이
///   ms 단위 last_seen을 계속 밀어 올려, 마지막 비컨의 주인이 매 갱신 바뀌며
///   **목록 순서가 갱신마다 뒤집혔다**. 버킷 동률은 이름순이라 안정된다).
/// - 모드: "seen"(최근 접속·기본) · "chat"(최근 대화) · "name"(이름).
fn peer_order_key(mode: &str, fav: bool, tier: u8, seen: u64, chat: u64) -> (u8, u8, u64, u64) {
    let inv = |t: u64| u64::MAX - t;
    let seen_b = seen / 60_000; // 분 버킷 — 비컨 잡음이 순서를 못 흔든다
    let head = u8::from(!fav);
    match mode {
        // 이름순 — 상태·시각 무시(동률 = 이름 비교는 호출자).
        "name" => (head, 0, 0, 0),
        // 최근 접속순 — **온라인 여부 무관**(사용자 확정 08-15 4모드).
        "seen" => (head, 0, inv(seen_b), inv(chat)),
        // 온라인 우선 — 접속 계층 먼저, 그 안은 최근 접속순.
        "online" => (head, tier, inv(seen_b), inv(chat)),
        // ★ 기본 = "chat"(같은 접속 구획 안에서 ① 최근 대화 ② 최근 접속 —
        //   접속·비접속 구획 동일 기준 · 미지 저장값도 여기로).
        _ => (head, tier, inv(chat), inv(seen_b)),
    }
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

/// 저장된 시각(unix ms)에서 표시용 벽시계 복원(M2-5b — 기록 로드).
fn wall_from_ms(ms: u64) -> nbeep_ui::WallTime {
    let lt = nbeep_plat::clock::local_time(ms / 1000);
    nbeep_ui::WallTime {
        y: lt.y,
        mo: lt.mo,
        d: lt.d,
        h: lt.h,
        m: lt.m,
    }
}

/// 대화 기록 보관 상한(줄) — 스레드당. 초과 = 오래된 것부터(시간 만료는 Q-32-11).
const HISTORY_MAX: usize = 2000;

/// 대화 기록 직렬화(M2-5b · 봉인 전 평문) — 텍스트 줄만(파일 기록은 후속 · 런타임
/// 필드가 붙어 직렬화 대상 아님). 레코드 = tag(1) ‖ mine(1) ‖ at_ms(8 LE) ‖
/// len(4 LE) ‖ utf8. tag로 전방 확장.
fn encode_history(lines: &[ChatLine]) -> Vec<u8> {
    use nbeep_ui::{ChatBody, XferLineState as St};
    // 보관 대상 = 텍스트 전부 + **종결 전송(Done/Failed)**. 진행 중 전송은 재시작
    // 시점에 이미 중단됐으므로 이력이 아니다(Waiting/Active/AwaitingAck 제외).
    let keep: Vec<&ChatLine> = lines
        .iter()
        .filter(|l| match &l.body {
            ChatBody::Text(_) => true,
            ChatBody::Xfer(x) => matches!(x.state, St::Done { .. } | St::Failed { .. }),
        })
        .collect();
    let start = keep.len().saturating_sub(HISTORY_MAX);
    let mut out = Vec::new();
    let put_str = |out: &mut Vec<u8>, sx: &str| {
        let b = sx.as_bytes();
        out.extend_from_slice(&(u32::try_from(b.len()).unwrap_or(0)).to_le_bytes());
        out.extend_from_slice(b);
    };
    for l in &keep[start..] {
        match &l.body {
            ChatBody::Text(t) => {
                // 발신자 라벨이 있으면 tag 3(그룹 수신 풍선 · 08-19 — 복원 후에도
                // "누가 보냈나"가 남아야 한다). 없으면 종전 tag 1 그대로 —
                // 1:1 세그먼트는 바이트 불변(전방 호환 · 구판은 미지 tag에서 멈춘다).
                if let Some(from) = &l.from {
                    out.push(3u8);
                    out.push(u8::from(l.mine));
                    out.extend_from_slice(&l.at_ms.to_le_bytes());
                    put_str(&mut out, from.as_str());
                    put_str(&mut out, t.as_str());
                } else {
                    out.push(1u8);
                    out.push(u8::from(l.mine));
                    out.extend_from_slice(&l.at_ms.to_le_bytes());
                    put_str(&mut out, t.as_str());
                }
            }
            ChatBody::Xfer(x) => {
                let (term, msg) = match &x.state {
                    St::Done { note } => (0u8, note.as_str()),
                    St::Failed { why } => (1u8, why.as_str()),
                    _ => continue,
                };
                out.push(2u8);
                out.push(u8::from(l.mine));
                out.extend_from_slice(&l.at_ms.to_le_bytes());
                out.push(term);
                out.extend_from_slice(&x.size.to_le_bytes());
                put_str(&mut out, x.name.as_str());
                put_str(&mut out, msg);
            }
        }
    }
    out
}

/// 대화 기록 역직렬화(M2-5b) — 손상·미지 tag는 거기서 멈춘다(fail-soft).
fn decode_history(bytes: &[u8]) -> Vec<ChatLine> {
    use nbeep_ui::{ChatBody, XferLine, XferLineState as St};
    // 길이-접두 문자열 읽기 — (문자열, 다음 오프셋). 범위 밖이면 None(손상 멈춤).
    let read_str = |b: &[u8], p: usize| -> Option<(String, usize)> {
        let e = p.checked_add(4)?;
        if e > b.len() {
            return None;
        }
        let n = u32::from_le_bytes(b[p..e].try_into().ok()?) as usize;
        let end = e.checked_add(n).filter(|x| *x <= b.len())?;
        Some((String::from_utf8_lossy(&b[e..end]).into_owned(), end))
    };
    let mut out = Vec::new();
    let mut i = 0;
    while i + 10 <= bytes.len() {
        let tag = bytes[i];
        let mine = bytes[i + 1] != 0;
        let at_ms = u64::from_le_bytes(bytes[i + 2..i + 10].try_into().unwrap_or([0; 8]));
        match tag {
            1 => {
                let Some((text, next)) = read_str(bytes, i + 10) else {
                    break;
                };
                out.push(ChatLine::text(
                    mine,
                    nbeep_core::sanitize_message(&text),
                    at_ms,
                    wall_from_ms(at_ms),
                ));
                i = next;
            }
            3 => {
                // 발신자 라벨 동반 텍스트(그룹 수신 풍선 · 08-19).
                let Some((from, p1)) = read_str(bytes, i + 10) else {
                    break;
                };
                let Some((text, next)) = read_str(bytes, p1) else {
                    break;
                };
                out.push(
                    ChatLine::text(
                        mine,
                        nbeep_core::sanitize_message(&text),
                        at_ms,
                        wall_from_ms(at_ms),
                    )
                    .with_from(nbeep_core::sanitize_message(&from).as_str()),
                );
                i = next;
            }
            2 => {
                if i + 19 > bytes.len() {
                    break;
                }
                let term = bytes[i + 10];
                let size = u64::from_le_bytes(bytes[i + 11..i + 19].try_into().unwrap_or([0; 8]));
                let Some((name, p1)) = read_str(bytes, i + 19) else {
                    break;
                };
                let Some((msg, next)) = read_str(bytes, p1) else {
                    break;
                };
                let state = if term == 0 {
                    St::Done { note: msg }
                } else {
                    St::Failed { why: msg }
                };
                out.push(ChatLine {
                    mine,
                    body: ChatBody::Xfer(XferLine {
                        thumb: None,
                        qpath: None,
                        name: nbeep_core::sanitize_message(&name),
                        size,
                        state,
                    }),
                    at_ms,
                    wall: wall_from_ms(at_ms),
                    from: None,
                    seq: 0,
                    delivered: false,
                    read: false,
                    queued: false,
                    importance: 0,
                });
                i = next;
            }
            _ => break, // 미지 tag — 여기까지(전방 확장은 append라 안전)
        }
    }
    out
}

/// 1:1 오프라인 대기 항목(M4-6 · 08-20) — flush 시 fresh seq로 실제 발신된다.
#[derive(Clone, Debug, PartialEq, Eq)]
struct PendingDirect {
    /// 본문(무해화 전 원문 — 발신 경로가 원문을 보내고, 표시는 SafeText로).
    text: String,
    /// 입력 시각(밀리초) — 대기 풍선과의 대응 키(resolve_queued).
    at_ms: u64,
    /// 등급(하위 2비트: 0 Normal · 1 Notice · 2 Urgent) + **bit2(`0x4`) = 공지
    /// 표식**(08-21 — 대기 편입된 공지가 flush 때도 공지로 발신되도록 · 저장
    /// 포맷 불변: 구 데이터의 0/1/2는 그대로 읽힌다).
    importance: u8,
}

/// 상대별 대기 상한(NFR-B-6 — 상한 없는 큐 금지) · 초과 = 오래된 것부터 정리.
const PENDING_DIRECT_MAX: usize = 200;

/// 대기 큐 직렬화 — `imp(1) ‖ at_ms(8 LE) ‖ len(4 LE) ‖ utf8` 반복.
/// (wall은 저장하지 않는다 — 복원 시 `wall_from_ms`가 재계산 · history와 동일 원리.)
fn encode_pending(q: &[PendingDirect]) -> Vec<u8> {
    let mut out = Vec::new();
    for m in q {
        out.push(m.importance);
        out.extend_from_slice(&m.at_ms.to_le_bytes());
        let b = m.text.as_bytes();
        out.extend_from_slice(&(u32::try_from(b.len()).unwrap_or(0)).to_le_bytes());
        out.extend_from_slice(&b[..b.len().min(u32::MAX as usize)]);
    }
    out
}

/// 대기 큐 해석 — 손상은 그 지점까지(fail-soft · 앞선 항목은 살린다).
fn decode_pending(bytes: &[u8]) -> Vec<PendingDirect> {
    let mut out = Vec::new();
    let mut i = 0usize;
    while i + 13 <= bytes.len() {
        let importance = bytes[i].min(2);
        let at_ms = u64::from_le_bytes(bytes[i + 1..i + 9].try_into().unwrap_or([0; 8]));
        let n = u32::from_le_bytes(bytes[i + 9..i + 13].try_into().unwrap_or([0; 4])) as usize;
        let Some(end) = (i + 13).checked_add(n).filter(|e| *e <= bytes.len()) else {
            break;
        };
        out.push(PendingDirect {
            text: String::from_utf8_lossy(&bytes[i + 13..end]).into_owned(),
            at_ms,
            importance,
        });
        i = end;
    }
    out
}

/// 속도 표기(B/s → 사람이 읽는 단위).
/// Auto 속도 행의 하단 정보(08-16) — 실측 최고와 그 절반(발신 목표) 또는 무주장
/// 원칙(수신 — [`nbeep_core::RateLimit::advertised_cap`])을 보여 준다.
/// Auto가 아니면 빈 문자열 = 줄 제거. 발신 peak가 세션 재시작으로 비면 "실측 전".
fn auto_rate_note(
    limit: nbeep_core::RateLimit,
    meter: &nbeep_core::RateMeter,
    sending: bool,
) -> String {
    if limit != nbeep_core::RateLimit::Auto {
        return String::new();
    }
    let peak = meter.peak_bps();
    use nbeep_core::{t, tf, Msg};
    if sending {
        if peak == 0 {
            tf(
                Msg::RateSendFloor,
                &[&rate_label(nbeep_core::rate::AUTO_FLOOR_BPS)],
            )
        } else {
            tf(
                Msg::RateSendMeasured,
                &[&rate_label(peak), &rate_label(meter.auto_target())],
            )
        }
    } else if peak == 0 {
        t(Msg::RateRecvUnclaimed).to_string()
    } else {
        tf(Msg::RateRecvMeasured, &[&rate_label(peak)])
    }
}

/// 파일 크기 상한 설정 해석(08-18) — MiB 숫자 · "unlimited" = None(무제한) ·
/// 미지/파싱 실패 = 기본 256MiB. 커스텀은 1MiB~1TiB로 클램프.
fn cap_from_setting(v: &str) -> Option<u64> {
    if v == "unlimited" {
        return None;
    }
    let mb = v.parse::<u64>().unwrap_or(256).clamp(1, 1024 * 1024);
    Some(mb * 1024 * 1024)
}

/// 요청 단위 결정의 **잔여 목록**(M4-2e · 08-20) — manifest 비제외분에서 방금
/// 결정(승인/거절)한 파일 하나(이름+크기 · 첫 일치만)를 뺀 나머지. 승인·거절이
/// 같은 함수를 쓴다 — 두 축이 어긋나면 "승인은 배치, 거절은 단건"이 재발한다.
fn batch_remainder(man: Vec<(String, u64, bool)>, name: &str, size: u64) -> Vec<(String, u64)> {
    let mut seen_self = false;
    man.into_iter()
        .filter(|(n, s, excluded)| {
            if *excluded {
                return false;
            }
            if !seen_self && n == name && *s == size {
                seen_self = true; // 지금 결정한 파일 자신은 제외(1회만)
                return false;
            }
            true
        })
        .map(|(n, s, _)| (n, s))
        .collect()
}

/// 요청 단위 결정 소비(M4-2e) — 잔여 목록에서 이름+크기 일치 항목 하나를 꺼낸다.
/// `Some(비었는가)` = 이 오퍼는 그 결정의 잔여분 · `None` = 수동 폴백(fail-closed).
fn batch_take(rem: &mut Vec<(String, u64)>, name: &str, size: u64) -> Option<bool> {
    rem.iter()
        .position(|(n, s)| n == name && *s == size)
        .map(|pos| {
            rem.remove(pos);
            rem.is_empty()
        })
}

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

/// 격리 아카이브 내용 목록 본문(M4-4 ⓐ · 08-21) — **해제 없이** 중앙 디렉터리
/// 목록만 그린다. 정책 판정 한 줄 동반(위반·판정 불가 = ⚠ 사유 — 통과 표기는
/// 하지 않는다: "통과 = 안전"이 아니다 NFR-S-5).
fn archive_listing_body(bytes: &[u8]) -> String {
    use nbeep_safe::zip::{inspect_zip, parse_zip_entries, ZipInspect};
    let entries = match parse_zip_entries(bytes) {
        Err(why) => {
            return format!(
                "{}: {why}",
                nbeep_core::t(nbeep_core::Msg::ArchiveUnreadable)
            )
        }
        Ok(e) => e,
    };
    let files = entries.iter().filter(|e| !e.is_dir).count();
    let total: u64 = entries.iter().map(|e| e.uncompressed).sum();
    let mut out = nbeep_core::tf(
        nbeep_core::Msg::ArchiveSummary,
        &[&files.to_string(), &human_size(total)],
    );
    match inspect_zip(bytes, &nbeep_safe::ArchivePolicy::default()) {
        ZipInspect::Ok(_) => {}
        ZipInspect::Reject(why) => {
            out.push('\n');
            out.push_str(&format!("! {why}"));
        }
        ZipInspect::Malformed(why) => {
            out.push('\n');
            out.push_str(&format!("! {why}"));
        }
    }
    out.push('\n');
    const MAX_LIST: usize = 12; // 경고 모달 높이 상한 안쪽(110+22/줄 clamp 460)
    for e in entries.iter().take(MAX_LIST) {
        out.push('\n');
        if e.is_dir {
            out.push_str(&e.name);
        } else if e.is_link {
            out.push_str(&format!(
                "{} {}",
                e.name,
                nbeep_core::t(nbeep_core::Msg::ArchiveLinkTag)
            ));
        } else {
            out.push_str(&format!("{} — {}", e.name, human_size(e.uncompressed)));
        }
    }
    if entries.len() > MAX_LIST {
        out.push('\n');
        out.push_str(&nbeep_core::tf(
            nbeep_core::Msg::ArchiveMore,
            &[&(entries.len() - MAX_LIST).to_string()],
        ));
    }
    out
}

/// 파일 스트림 한 프레임 처리(액터 스레드) — 오류 = 세션 종료 신호.
#[allow(clippy::too_many_arguments)] // 액터 한 프레임의 협력자 — 묶으면 오히려 흐릿해진다
fn xfer_step(
    bytes: &[u8],
    peer: PeerId,
    session: &mut LiveSession,
    inbox: &mut nbeep_core::XferInbox,
    outgoing: &mut HashMap<nbeep_core::XferId, (std::path::PathBuf, u64)>,
    sending: &mut Option<ActiveSend>,
    recv_started: &mut HashMap<nbeep_core::XferId, u64>,
    recv_names: &mut HashMap<nbeep_core::XferId, String>,
    sent_names: &mut HashMap<nbeep_core::XferId, (String, u64)>,
    canceled: &mut std::collections::HashSet<nbeep_core::XferId>,
    proxy: &winit::event_loop::EventLoopProxy<AppEvent>,
    send_rate: nbeep_core::RateLimit,
    meter: &mut nbeep_core::RateMeter,
    seal_secret: &[u8; 32],
) -> Result<(), ()> {
    use nbeep_core::mux::StreamId;
    use nbeep_core::XferMsg;
    let fail = |why: String| {
        let _ = proxy.send_event(AppEvent::XferFailed { peer, why });
    };
    match XferMsg::decode(bytes) {
        Ok(XferMsg::Offer {
            id,
            size,
            name,
            sha256,
        }) => {
            let m = XferMsg::decode(bytes).map_err(|_| ())?;
            match inbox.offer(&m) {
                Ok(()) => {
                    recv_names.insert(id, String::from_utf8_lossy(&name).into_owned());
                    let _ = proxy.send_event(AppEvent::XferOffer {
                        peer,
                        id,
                        name: String::from_utf8_lossy(&name).into_owned(),
                        size,
                        sha256,
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
                    fail(nbeep_core::tf(
                        nbeep_core::Msg::XfRecvRefused,
                        &[&e.to_string()],
                    ));
                }
            }
        }
        Ok(XferMsg::Accept {
            id,
            rate_cap,
            resume_offset,
            prefix_sha,
        }) => {
            if let Some((path, total)) = outgoing.remove(&id) {
                let _ = proxy.send_event(AppEvent::XferAccepted { peer });
                // 스트리밍 발신(08-18) — 파일을 메모리에 올리지 않는다.
                let Ok(mut file) = std::fs::File::open(&path) else {
                    fail(nbeep_core::tf(
                        nbeep_core::Msg::XfOpenFailed,
                        &[&path.display().to_string()],
                    ));
                    return Ok(());
                };
                // ★ 재개(M4-10b) — 요청 오프셋의 내 원본 프리픽스 해시가 상대의
                //   보존분 해시와 **일치할 때만** 그 지점부터. 불일치·범위 초과 =
                //   처음부터(fail-closed). 같은 패스에서 **전체 해시 시딩**도 한다
                //   (지연 선언 — 검증 해시와 계속 해시가 같은 프리픽스를 읽는다).
                let mut hasher = nbeep_crypto::Sha256Stream::new();
                let start = if resume_offset > 0 && resume_offset < total {
                    use std::io::{Read as _, Seek as _};
                    let mut verify = nbeep_crypto::Sha256Stream::new();
                    let mut left = resume_offset;
                    let mut buf = vec![0u8; 1024 * 1024];
                    let mut ok = file.rewind().is_ok();
                    while ok && left > 0 {
                        let want = buf.len().min(usize::try_from(left).unwrap_or(buf.len()));
                        match file.read(&mut buf[..want]) {
                            Ok(0) | Err(_) => ok = false,
                            Ok(n) => {
                                verify.update(&buf[..n]);
                                hasher.update(&buf[..n]);
                                left -= n as u64;
                            }
                        }
                    }
                    if ok && left == 0 && verify.finalize() == prefix_sha {
                        resume_offset
                    } else {
                        hasher = nbeep_crypto::Sha256Stream::new(); // 0부터 = 새 해시
                        0
                    }
                } else {
                    0
                };
                // ★ 쌍방 협상 — 내 상한과 상대가 공지한 상한 중 **낮은 쪽**으로 창을 잡는다.
                // ★ 08-16: 여기서 전량을 쏟지 않는다 — 상태로 등록만 하고 액터 루프
                //   틱이 조금씩 보낸다(그전엔 이 블로킹 루프 동안 취소 명령도 상대
                //   Cancel도 못 봤다 — 취소 UX의 구조적 선행).
                let local = send_rate.target_bps(meter);
                let mtime0 = std::fs::metadata(&path)
                    .ok()
                    .and_then(|m| m.modified().ok());
                *sending = Some(ActiveSend {
                    id,
                    file,
                    hasher,
                    src: (path, mtime0),
                    next: start,
                    from: start,
                    total,
                    pacer: nbeep_core::Pacer::new(nbeep_core::negotiate(local, rate_cap)),
                    rate_cap,
                    probed: false,
                    started_ms: xfer_now_ms(),
                    next_probe: u64::MAX,
                    bursting: None,
                });
            }
        }
        Ok(XferMsg::Reject { id, why, limit }) => {
            outgoing.remove(&id);
            let extra = if limit > 0 {
                nbeep_core::tf(
                    nbeep_core::Msg::PeerCapSuffix,
                    &[&format!("{}MiB", limit / (1024 * 1024))],
                )
            } else {
                String::new()
            };
            fail(nbeep_core::tf(
                nbeep_core::Msg::XferPeerRejected,
                &[&format!("{why:?}"), &extra],
            ));
        }
        Ok(XferMsg::Chunk { id, offset, data }) => {
            if canceled.contains(&id) {
                return Ok(()); // 취소분의 잔류 청크 — 오류 아님(08-18)
            }
            recv_started.entry(id).or_insert_with(xfer_now_ms); // 평균 속도 기준점
            match inbox.chunk(&id, offset, &data) {
                Ok(()) => {
                    if let Some((got, total)) = inbox.progress(&id) {
                        let _ = proxy.send_event(AppEvent::XferProgress {
                            peer,
                            got,
                            total,
                            sending: false,
                            name: recv_names.get(&id).cloned().unwrap_or_default(),
                        });
                    }
                }
                Err(e) => fail(nbeep_core::tf(
                    nbeep_core::Msg::XfRecvError,
                    &[&e.to_string()],
                )),
            }
        }
        Ok(XferMsg::Done { id, .. }) if canceled.contains(&id) => {} // 취소분 꼬리(08-18)
        Ok(XferMsg::Done { id, sha256 }) => match inbox.done(&id) {
            Ok(mut got) => {
                // 지연 선언(08-18) — Offer가 0이었으면 Done 동봉 해시가 선언이다.
                // 둘 다 0 = 선언 부재(fail-closed — 검증 없는 수신물은 없다).
                if got.declared_sha256 == [0u8; 32] {
                    if sha256 == [0u8; 32] {
                        let _ = session.send(StreamId::File, &XferMsg::Failed { id }.encode());
                        fail("무결성 선언 부재 — 폐기".into());
                        return Ok(());
                    }
                    got.declared_sha256 = sha256;
                }
                match crate::gate::quarantine_received(&got, peer, crate::gate::CH_GUI, seal_secret)
                {
                    Ok(q) => {
                        // 평균 수신 속도(08-16) — 첫 청크~Done. 표기는 스레드 완료 항목.
                        let avg_bps = {
                            let dur = xfer_now_ms()
                                .saturating_sub(
                                    recv_started.remove(&id).unwrap_or_else(xfer_now_ms),
                                )
                                .max(1);
                            (got.bytes.len() as u64).saturating_mul(1000) / dur
                        };
                        // ★ 종단 확인(M4-9) — 격리까지 성공했으니 발신자에게 Received를 돌려준다.
                        //    이걸 받아야 상대의 "완료"가 참이 된다("보냈다"≠"닿았다").
                        let _ = session.send(StreamId::File, &XferMsg::Received { id }.encode());
                        let _ = proxy.send_event(AppEvent::XferDone {
                            peer,
                            name: q.name,
                            risk: q.risk,
                            mismatch: q.mismatch,
                            qpath: q.path.to_string_lossy().into_owned(),
                            avg_bps,
                            scan: q.scan,
                        });
                    }
                    Err(e) => {
                        // 수신측 실패 — 발신자가 거짓 완료를 남기지 않게 Failed를 되돌린다.
                        let _ = session.send(StreamId::File, &XferMsg::Failed { id }.encode());
                        fail(format!("{e}"));
                    }
                }
            }
            Err(e) => {
                let _ = session.send(StreamId::File, &XferMsg::Failed { id }.encode());
                fail(nbeep_core::tf(
                    nbeep_core::Msg::XfDoneFailed,
                    &[&e.to_string()],
                ));
            }
        },
        // ★ 발신측이 받는 종단 확인(M4-9) — 확인 대기 항목을 완료/실패로 닫는다.
        Ok(XferMsg::Received { id }) => {
            let (name, size) = sent_names.remove(&id).unwrap_or_default();
            let _ = proxy.send_event(AppEvent::XferAcked {
                peer,
                ok: true,
                name,
                size,
            });
        }
        Ok(XferMsg::Failed { id }) => {
            let (name, size) = sent_names.remove(&id).unwrap_or_default();
            let _ = proxy.send_event(AppEvent::XferAcked {
                peer,
                ok: false,
                name,
                size,
            });
        }
        Ok(XferMsg::Cancel { id }) => {
            inbox.drop_xfer(&id);
            outgoing.remove(&id);
            recv_started.remove(&id);
            canceled.insert(id); // 대칭 — 이후 잔류 프레임 조용히 폐기(08-18)
            if sending.as_ref().is_some_and(|st| st.id == id) {
                *sending = None; // 상대(수신측)가 진행 중 취소 — 펌프 즉시 중단(08-16)
            }
            fail(nbeep_core::t(nbeep_core::Msg::XferPeerCanceled).into());
        }
        Err(e) => fail(nbeep_core::tf(
            nbeep_core::Msg::XfWireError,
            &[&e.to_string()],
        )),
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
    /// 수신 이미지 **확대 미리보기**(08-16 · M4-5 잔여) — 단일 창(peer_info 패턴 ·
    /// 내용은 [`App::image_view`]가 든다 · Role은 Copy라 경로는 상태에).
    ImageView,
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
    /// 대화함 — 대화 기록 관리(M3-23 · 목록/열기/삭제/백업/복원).
    Convbox,
    /// 수신 승인 — 제안 정보를 보여 주고 결정을 받는 창(타임아웃 = 취소).
    Approve(PeerId),
    /// 주소 직접 입력 모달(DR-19 · M3-16 — `⌘/Ctrl+K`·툴바 +).
    AddEndpoint,
    /// 경고 모달(08-13 — 상태바 한 줄로는 지나치는 실패를 눈앞에 세운다).
    Alert,
    /// 한 줄 이름 입력 모달(M5-1 — 그룹 생성·개명).
    NamePrompt,
    /// 그룹 스레드 창(M5-1 · Separate 모드 — 팬아웃 발신을 하나의 스레드로 · FR-G-3).
    GroupChat(nbeep_core::group::GroupId),
}

/// 선택 모달(Role::Alert 2버튼)의 문맥 — 결과를 어디에 적용할지(M5-1g).
#[derive(Clone, Debug)]
enum AlertCtx {
    /// 그룹 초대 — 수락/거절 대상.
    GroupInvite {
        uid: nbeep_core::GroupUid,
        owner: PeerId,
    },
    /// 설정 초기화 확인(08-15 · 고급) — 긍정 = 표시 설정 전부 기본값.
    SettingsReset,
    /// 구성원 제외 확인(G4 · 08-15) — 긍정 = roster에서 제외·재배포(소유자만).
    GroupKick {
        gid: nbeep_core::group::GroupId,
        peer: PeerId,
    },
    /// 원격 인바운드 요청 대기(M5-3b · ADR-0006 §6 — FR-S-25) — 긍정 = 수락·등록.
    /// 세션 실물은 `pending_remote` 슬롯에 있다(AlertCtx는 Clone이라 못 담는다).
    RemoteInbound { peer: PeerId },
}

/// 이름 입력 모달의 용도(M5-1) — 제출된 이름을 어디에 쓸지.
#[derive(Clone, Debug)]
enum NamePurpose {
    /// 선택한 구성원으로 그룹 생성.
    CreateGroup(Vec<PeerId>),
    /// 기존 그룹 개명.
    RenameGroup(nbeep_core::group::GroupId),
    /// 공지 보내기(④ 08-20 · FR-M-6) — 입력 = 공지 본문(발견된 전체에게
    /// **Notice 등급** 팬아웃 · Urgent 공지는 만들지 않는다 — docs/24 "팬아웃
    /// Urgent 1단계 강등"의 정신을 발신에서 지킨다).
    Broadcast,
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
                    importance: nbeep_core::Importance::Normal,
                    broadcast: false,
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
                // 경로 등급은 핸드셰이크 **전** 실소켓 주소로 판정(M5-3c) — 세션이
                // 링크를 삼키기 전 마지막 관측 지점이다. 소켓 아님(None) = Local.
                let path = link
                    .remote_ip()
                    .map_or(nbeep_core::PathClass::Local, nbeep_core::class_of_ip);
                // 핸드셰이크(블로킹) — 실패(자기 키 복제 U-P2 포함)면 조용히 버린다.
                let Ok(session) = nbeep_crypto::NoiseSession::accept(link, &identity) else {
                    return;
                };
                let _ = proxy.send_event(AppEvent::Inbound {
                    session: Box::new(InboundSession { session, path }),
                });
            });
        }
    });
}

/// 릴레이 인바운드 수락 루프(X-2b ②) — 서버 랑데부로 걸어온 상대를 사다리
/// ([`nbeep_relay::accept_via`] — 펀치 병행 · 첫 프레임 링크 추종)로 성립시키고,
/// LAN 인바운드와 **같은 깔때기**([`AppEvent::Inbound`] — TOFU 판정·원격 요청 대기
/// FR-S-25·중복 세션 가드)에 합류시킨다. 경로 등급도 사다리 결과(`via.path` —
/// 성립 소켓 실주소)가 이미 들고 온다(§5-1-5).
///
/// 수명 = 클라이언트 `Arc`: App이 내려놓으면(해제·재접속) Weak 승격 실패로 끝난다.
fn spawn_relay_accept(
    client: &std::sync::Arc<nbeep_relay::RelayClient>,
    identity: std::sync::Arc<nbeep_crypto::Identity>,
    proxy: winit::event_loop::EventLoopProxy<AppEvent>,
) {
    let weak = std::sync::Arc::downgrade(client);
    std::thread::spawn(move || loop {
        let Some(client) = weak.upgrade() else {
            break; // App이 접속을 내려놓았다 — 이 루프의 소임 끝
        };
        let Some(inc) = client.accept_incoming(std::time::Duration::from_secs(1)) else {
            continue; // 타임아웃(1s) — Weak를 다시 보고 돈다
        };
        // 성립(펀치→릴레이 · 최대 12s)은 인바운드별 스레드 — 동시 인바운드가 서로를
        // 기다리지 않는다(LAN spawn_inbound_accept가 세션별 스레드를 쓰는 이유와 동일).
        let identity = std::sync::Arc::clone(&identity);
        let proxy = proxy.clone();
        std::thread::spawn(move || {
            // 실패는 조용히 — 상대가 보는 것은 Closed뿐(ADR-0006 §2 규칙 4 · LAN
            // 인바운드 핸드셰이크 실패와 같은 결).
            if let Ok(via) = nbeep_relay::accept_via(
                &client,
                inc,
                &identity,
                true,
                std::time::Duration::from_secs(12),
            ) {
                let _ = proxy.send_event(AppEvent::Inbound {
                    session: Box::new(InboundSession {
                        session: via.session,
                        path: via.path,
                    }),
                });
            }
        });
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
    /// 설정 **백업 폴더** 선택(08-15) — 폴더 탐색 + "여기에 저장".
    SettingsBackupDir,
    /// 설정 **백업 파일** 선택(복원 · 08-15) — 폴더 탐색 + .cfg 파일.
    SettingsRestoreFile,
    /// 대화 기록 **백업 폴더** 선택(M3-23) — 폴더 탐색 + "여기에 저장".
    HistoryBackupDir,
    /// 대화 기록 **복원 위치** 선택(M3-23) — 폴더 탐색 + "이 폴더에서 복원"
    /// (개별 .seg 파일 클릭 = 그 파일만 복원).
    HistoryRestoreDir,
}

/// 대화창 입력을 명령으로 가른 결과(08-15).
enum CmdOutcome {
    /// 보낼 것이 남았다(명령이 아니었다 — 첫 글자가 `/`가 아닌 입력 · 08-16 규칙).
    Send(Option<nbeep_core::SafeText>),
    /// 등급 명령(④ — /notice·/urgent)의 본문을 보낸다(0 일반 · 1 알림 · 2 긴급).
    SendGraded(nbeep_core::SafeText, u8),
    /// 명령으로 처리했다 — **상대에게 보내지 않는다**.
    Handled,
}

/// 신뢰 등급의 사람 말 라벨(`/trust` 출력 · 카드·목록 문구와 같은 어휘).
fn trust_label(lv: nbeep_core::TrustLevel) -> &'static str {
    use nbeep_core::TrustLevel as L;
    match lv {
        L::Unverified => "미검증(핸드셰이크 전)",
        L::Pinned => "고정됨(TOFU — 첫 접촉 키 기억)",
        L::FingerprintVerified => "지문 대조 완료(사람이 확인)",
    }
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
        nbeep_core::tf(
            nbeep_core::Msg::TitleFilePick,
            &[&self.dir.display().to_string()],
        )
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
    /// Managed 릴레이 서버 접속(X-2b · ADR-0013 §12-1) — 원천은 설정(`net.server.*`).
    /// Drop = 수락 스레드·클라 액터 정리(Weak 승격 실패). None = Unmanaged/미접속.
    relay: Option<std::sync::Arc<nbeep_relay::RelayClient>>,
    /// 서버 접속 워커 가동 중(중복 시도 가드 — 13 §12-1).
    relay_connecting: bool,
    /// 서버 재접속 백오프 — (단계, 다음 시도 at_ms). 핀 불일치 = `u64::MAX`(수동만).
    relay_backoff: (u8, u64),
    /// 설정 세대(늦은 성공 가드) — `net.server.*` 변경마다 +1 · 워커 결과 gen과 대조.
    relay_gen: u64,
    /// 다음 서버 생존 점검 시각(ms) — `is_alive` 폴링 페이싱(2s).
    relay_check_at: u64,
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
    /// 세션 액터 join 핸들(M4-10a · 08-18) — **정상 종료 시 부분 수신물 보존을
    /// 기다린다**: main 리턴 = 전 스레드 즉사라, 기다리지 않으면 수신자 쪽 종료
    /// 경로에서 `.part`가 안 남는다(발신자 쪽 끊김만 보존되던 반쪽).
    actor_joins: Vec<std::thread::JoinHandle<()>>,
    /// 상태 로거(M3-22 · `log.enabled` hot-swap) — None = 끔.
    statuslog: Option<crate::statuslog::StatusLog>,
    /// 네트워크 점검 로거(netmon · 08-21 — `netmon.enabled` 옵트인) — None = 끔.
    netmon_log: Option<crate::statuslog::StatusLog>,
    /// netmon 직전 스냅숏(델타의 기준점).
    netmon_prev: nbeep_net::netmon::NetSnapshot,
    /// netmon 마지막 기록 시각(unix 초).
    netmon_last_sec: u64,
    /// 마지막 공지(브로드캐스트) 발신 시각(ms) — 3초 1회 제한(08-21 사용자 확정).
    last_broadcast_ms: u64,
    /// 대화별 데이터 키 테이블(크립토 셰레딩 · D-18 §7 · 08-21) — 기록 봉인 키의
    /// 단일 원천. 삭제 = 키 폐기([`crate::keytable::KeyTable::destroy`]).
    datakeys: crate::keytable::KeyTable,
    /// 이 시각 전까지 메인 목록의 Enter 활성화를 무시(08-21 — 모달 Enter 제출
    /// 잔향이 캐럿 행 대화를 열던 것 · `ENTER_GUARD_MS`).
    enter_guard_until_ms: u64,
    /// 이어받기로 수락한 수신의 원본 sha(M4-10) — 완료·취소 때 `.part` 정리.
    resumed_recv: HashMap<PeerId, [u8; 32]>,
    /// 마지막으로 **적용한** 프로필 수신 내용의 지문(RL-1 · 08-18) — 동일 내용
    /// 재수신이면 하류 전부(사진 재봉인·imgdec 자식·trust 재암호화·목록 재조립)를
    /// 건너뛴다. 08-16 수정은 트리거 빈도만 낮췄고, 비용 측은 이 한 겹이 막는다.
    peer_profile_fp: HashMap<PeerId, u64>,
    /// 와이어 축소본 세대(08-16) — `image_path`가 바뀔 때마다 +1. 워커 완료
    /// 이벤트가 이 값과 다르면 낡은 세대(그 사이 사진이 또 바뀜) — 조용히 폐기.
    wire_gen: u64,
    /// 축소본 워커가 도는 중 — 이 동안 [`Self::push_profile`]을 보류한다(2단계
    /// 깜빡임 방지 · 08-16 실기: 상대가 "옛 내장 그림 → 실사진" 순서로 봤다).
    wire_pending: bool,
    /// [`Self::wire_pending`]이 선 시각(RL-11 watchdog 기준점 — 15초 무응답이면
    /// 래치를 풀고 사진 없이 전파한다 · 유실 = 전파 영구 침묵 방지).
    wire_pending_ms: u64,
    /// 내장 12간지 아바타(키 → 그림 · 08-14) — 기동 시 1회 해석(NBAV1 fail-soft).
    builtin_avatars: HashMap<String, Rc<nbeep_ui::IconImage>>,
    /// 내 사진(imgdec 디코드 완료본 · 08-14) — 툴바 프로필 버튼·프로필 프리뷰 공용.
    my_avatar: Option<Rc<nbeep_ui::IconImage>>,
    /// 시스템 트레이(M3-2a · Windows — 비지원 OS는 None). 아이콘 = 내 아바타.
    tray: Option<nbeep_plat::tray::TrayHandle>,
    /// 상대 프로필 보기 뷰(우클릭 ▸ 프로필 보기 — 열려 있을 때만 Some).
    peer_info_view: Option<nbeep_ui::PeerInfoWidget>,
    /// 주소 입력 모달 뷰(DR-19 · M3-16 — 열려 있을 때만 Some).
    addr_view: Option<nbeep_ui::AddrPromptWidget>,
    /// About 뷰(열려 있을 때만 Some).
    about_view: Option<AboutWidget>,
    /// 경고 모달 뷰(열려 있을 때만 Some).
    alert_view: Option<nbeep_ui::AlertWidget>,
    /// 열어야 할 경고(제목, 본문, 진원 창) — 이벤트 루프 참조가 없는 지점에서
    /// 요청되면 `about_to_wait`가 연다(pending_picker와 같은 패턴). 진원 창이
    /// 소유·위치 기준(08-20 — 메인 소유면 경고가 뜰 때 메인 묶음이 대화창
    /// 위로 부상하던 실기).
    pending_alert: Option<(String, String, Option<WindowId>)>,
    /// DnD 수집 버퍼(08-20 4차) — winit은 파일당 이벤트라 한 번의 드롭을
    /// 여기 모았다가 about_to_wait에서 **요청 단위로 선판정**한다.
    pending_drops: Vec<(WindowId, std::path::PathBuf)>,
    quarantine_view: Option<nbeep_ui::QuarantineWidget>,
    /// 대화함 뷰(M3-23 — 열려 있을 때만 Some).
    convbox_view: Option<nbeep_ui::ConvboxWidget>,
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
    /// ★ base 슬롯(08-18 실기) — 종전엔 아예 미배선이라 Base UI 글꼴명이
    /// 조용히 무시됐다(FontSet.base가 항상 내장 기본).
    face_base: Option<nbeep_gfx::Font>,
    face_peerlist: Option<nbeep_gfx::Font>,
    face_message: Option<nbeep_gfx::Font>,
    face_status: Option<nbeep_gfx::Font>,
    /// 고정폭 얼굴 — 지정 없으면 OS 기본.
    face_mono: Option<nbeep_gfx::Font>,
    /// 상대별 수락 대기 큐 — **오퍼 1건당 승인 1번**(2번 보내면 2번 물어본다).
    pending_offers: HashMap<PeerId, VecDeque<PendingOffer>>,
    /// 상대별 발신 대기 파일 큐(다중 드롭 — 한 번에 하나씩 협상한다).
    send_queue: HashMap<PeerId, VecDeque<std::path::PathBuf>>,
    /// 격리함 스캔 세대·원시 캐시(08-18 — 대용량 개봉을 메인에서 몰아내고,
    /// 썸네일 도착 재렌더는 캐시로 · RL-15ⓑ 전량 재스캔 제거).
    qscan_gen: u64,
    qrows_raw: Vec<QRowRaw>,
    /// 상대가 공지한 수신 파일 상한(08-18) — (상한, 수신 시각 ms). 송신 직전
    /// 신선도(3초) 안이면 질의 생략, 아니면 CapRequest 후 응답·타임아웃에 진행.
    peer_recv_cap: HashMap<PeerId, (u64, u64)>,
    /// 송신 직전 상한 질의 대기(08-18 — 마감 ms · 지나면 미상으로 진행).
    cap_req_deadline: HashMap<PeerId, u64>,
    /// 해시 워커가 도는 발신(08-18 스트리밍) — 도는 동안 펌프 중복 진입 금지.
    preparing_send: std::collections::HashSet<PeerId>,
    /// 협상·전송 중인 발신 파일의 경로(M4-10c) — 세션이 끊기면 재-Offer 원료.
    current_send: HashMap<PeerId, std::path::PathBuf>,
    /// 끊김으로 중단된 발신(M4-10c) — 세션 재성립 시 **능동 재-Offer 1회**.
    /// 같은 파일 재-Offer = 수신측 `.part` 매치 = 이어받기 성립.
    resend_offers: HashMap<PeerId, VecDeque<std::path::PathBuf>>,
    /// 상대별 배치 집계 (보낸 파일 수, 총 파일 수, 보낸 바이트, 총 바이트).
    send_batch: HashMap<PeerId, (u32, u32, u64, u64)>,
    /// 발신 협상 대기 중인가(수락 전) — 큐 진행 판단.
    awaiting_accept: HashMap<PeerId, nbeep_core::XferId>,
    /// 진행 중(수락 후) 발신 — 취소 라우팅용(08-16 · 배너 "취소" → CancelXfer).
    active_send: HashMap<PeerId, nbeep_core::XferId>,
    /// 진행 중(수락 후) 수신 — 위와 대칭.
    active_recv: HashMap<PeerId, nbeep_core::XferId>,
    /// 상대에게서 받은 마지막 메시지 seq(N-2 읽음 up-to · 읽음 ack 대상).
    last_recv_seq: HashMap<PeerId, u64>,
    /// 확대 미리보기 창 내용(08-16 — 단일 창 · [`Role::ImageView`]의 짝).
    image_view: Option<ImageViewState>,
    /// 완료 대기 중 발신 평균 속도(B/s · 08-16) — 종단 ack 도착 시 완료 문구에.
    send_avg: HashMap<PeerId, u64>,
    /// 마지막 LinkChanged 생사 프로브 시각(ms · 08-16) — 쿨다운 10초(13 §12-1
    /// 중복 가드: 감시 오작동·이벤트 폭주가 Request 폭주로 번지지 않게 — 실기:
    /// 1초 루프 × 프로필 Full(이미지 동반) 양방향 = 쓰기-쓰기 교착의 방아쇠).
    last_link_probe_ms: u64,
    /// 발신했으나 **수신 종단 확인(ack) 대기 중**인 건수(상대별 · M4-9). 종료 가드가 본다.
    awaiting_ack: HashMap<PeerId, u32>,
    /// 종료 가드 — 미확인 전송이 있을 때 첫 닫기는 경고만, 두 번째로 확정(파괴적 확인 문법).
    close_armed: bool,
    /// 발신 대기 창의 **승인 타임아웃 타이머**(상대별 · 렌더 안 함 — M4-2d는
    /// 창을 배치 패널로 바꿨다. 이건 순수 타이머로 남아 승인 무응답 시 취소한다).
    send_wait: HashMap<PeerId, nbeep_ui::TimeoutButton>,
    /// 발신 배치 패널(M4-2d · 상대별) — 개수·총량·스크롤 목록·파일별 아이콘 제어.
    /// 일시정지된 발신 파일(M4-2d · 상대별 경로 집합) — 큐 펌프가 건너뛴다(대기
    /// 파일 pass) · 활성 전송은 P2에서 액터 pump gate로 확장.
    send_paused: HashMap<PeerId, std::collections::HashSet<std::path::PathBuf>>,
    /// 수신 대기 배치 목록(M4-2e · 상대별 manifest) — 요청 단위 승인의 원료.
    /// 첫 오퍼 승인 시 남은 항목이 `batch_approved`로 넘어간다.
    recv_manifest: HashMap<PeerId, Vec<(String, u64, bool)>>,
    /// 요청 단위 승인 잔여(M4-2e) — 승인 1회 후 이후 오퍼를 **이름+크기 대조**로
    /// 자동 수락(불일치 = 수동 폴백 · fail-closed). 소진·거절·종료 때 비운다.
    batch_approved: HashMap<PeerId, Vec<(String, u64)>>,
    /// 요청 단위 **거절** 잔여(M4-2e · 08-20 — 승인의 대칭): 거절 1회 후 같은
    /// 배치의 이후 오퍼는 재확인 없이 자동 거절·라인 종결(2파일 실기 구멍).
    batch_declined: HashMap<PeerId, Vec<(String, u64)>>,
    /// 수신 전송의 이름→xid(M4-2e — 수신측 ⏸▶✕가 1슬롯(active_recv)에 안 갇히게).
    recv_xids: HashMap<PeerId, Vec<(String, nbeep_core::XferId)>>,
    /// 발신측이 정지시킨 수신 파일 이름(M4-2e — 수신 배너 유지·전체취소 포함 판단).
    recv_paused: HashMap<PeerId, std::collections::HashSet<String>>,
    /// 수신 배치 집계(M4-2e — 배너 합산 표기): (완료 수, 총 수, 완료 바이트, 총 바이트).
    recv_batch: HashMap<PeerId, (u32, u32, u64, u64)>,
    /// 수신 배치 파일 크기 장부(manifest 비제외분 — 완료 시 크기 가산용).
    recv_batch_sizes: HashMap<PeerId, Vec<(String, u64)>>,
    /// 정지 잔존의 최근 상태 변화 시각(M4-2e ⓓ) — 한도 넘게 그대로면 양쪽 자동
    /// 전체 취소(cancel → CANCEL_ALL 전파). 활동(진행·완료·정지/재개)마다 갱신.
    xfer_pause_since: HashMap<PeerId, u64>,
    /// 정지 방치 자동 취소 한도 ms(설정 `xfer.auto_cancel_min` · 기본 2분 —
    /// 사용자 확정 08-20 · 유효 범위 1~10분).
    auto_cancel_limit_ms: u64,
    /// 일시정지로 **보관 중인 발신**(M4-2e ⓐ · 상대별) — (이름, 크기, 경로, xid).
    /// 액터 parked 슬롯의 앱측 그림자: 다음 파일 펌프·재개 요청·취소·Closed
    /// 재-Offer 원료가 여기서 나온다.
    paused_sends: HashMap<PeerId, Vec<(String, u64, std::path::PathBuf, nbeep_core::XferId)>>,
    /// 이번 배치에서 **용량 초과로 제외**된 파일(M4-2e · 상대별 이름·크기) —
    /// 전송하지 않지만 목록(스레드 라인·manifest)에는 보인다(사용자 확정 08-19
    /// "파일 단위 검사 · 제외 상태로 목록에"). 배치 종료 때 함께 비운다.
    send_excluded: HashMap<PeerId, Vec<(String, u64)>>,
    /// 다음 루프에서 만들 발신 대기 창(창 생성은 ActiveEventLoop가 필요하다).
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
    /// 목록 정렬 드롭다운(08-15 — 툴바 행 좌측 끝 슬롯 · IconDropdown 첫 사용처).
    /// 값 = `ui.list_sort`(영속 · 설정 화면 Radio와 상호 동기화).
    sort_drop: nbeep_ui::IconDropdown,
    /// 세션이 끊긴 상대(AppEvent::Closed) — 목록 상태 점 Lost 근거. 재수립 시 제거.
    closed_peers: std::collections::HashSet<PeerId>,
    /// 끊긴 대화의 **스레드 대피소**(DR-26 — 대화 상태는 상대별 유지 · 08-13 실기).
    /// `Closed`가 `Conversation`(세션 채널)을 지울 때 lines만 여기로 옮기고, 재수립
    /// (`install_conversation`)이 되찾아 간다 — 그전엔 끊김 = 스레드 통째 소실이었다.
    /// (영속 기록은 M2-5b — 이건 프로세스 수명 안의 보존이다.)
    parked_lines: HashMap<PeerId, Vec<ChatLine>>,
    /// 격리함 썸네일 캐시(`.beepq` 경로 → 디코드 결과 · `None` = 요청 중/실패·비이미지).
    /// 채움은 워커(`Decoded(QThumb)`) — 행 재적재가 imgdec를 재호출하지 않게 한다.
    qthumbs: HashMap<String, Option<std::rc::Rc<nbeep_ui::IconImage>>>,
    /// **비발견 상대**(수동 등록·다른 서브넷 인바운드 — 사용자 실기 08-13) — 발견
    /// 테이블에 ANNOUNCE가 안 닿아도 목록에 유지한다(대화창을 닫으면 사라지던 버그).
    /// 이름은 성립 시 스냅샷(프로필 이름은 2줄째로 따로 표시된다).
    extra_peers: HashMap<PeerId, nbeep_core::DisplayName>,
    /// 성공한 수동 등록 주소(DR-19) — 세션이 끊긴 뒤 목록에서 다시 클릭하면
    /// 발견 주소록에 없어도 이 주소로 재연결한다.
    manual_addrs: HashMap<PeerId, String>,
    /// 읽지 않은 수신 메시지 수(③ — 대화 뷰가 닫혀 있는 동안 도착한 것).
    unread: HashMap<PeerId, u32>,
    /// 알림 스로틀(M3-8) — 키(상대/방)별 마지막 알림 시각 ms(3초 안 중복 억제).
    last_notify: HashMap<String, u64>,
    /// 알림 대상 맵(08-15 — 클릭 = 해당 대화 열기): 토큰(=알림 키) → 대상.
    /// 토큰은 OS를 불투명하게 왕복하고, 해석은 여기서만 한다(봉투 원리).
    notify_targets: HashMap<String, NotifyTarget>,
    /// 그 대화를 마지막으로 확인한 시각(뷰가 열려 있던 마지막 순간 — 목록에 표시).
    last_read: HashMap<PeerId, nbeep_ui::WallTime>,
    /// 자동 재연결 스케줄(사용자 확정 08-13 ⓑ) — `(백오프 단계, 다음 시도 at_ms)`.
    /// 끊김·연결 실패 시 해제, 성공·수동 클릭·상한 도달 시 해제.
    reconnect: HashMap<PeerId, (u8, u64)>,
    /// 발견발 목록 재조립 대기(08-14 — 목록 갱신 주기 설정). 발견 이벤트는 표시만
    /// 바꾸므로 즉시 재조립하지 않고 주기에 맞춰 묶는다(그 외 경로는 즉시).
    rows_dirty: bool,
    /// 마지막 목록 재조립 시각(ms) — 어떤 경로든 재조립하면 갱신된다.
    last_rows_ms: u64,
    /// 목록 갱신 최소 간격(ms · `ui.list_refresh_ms` — 기본 1500 · 사용자 확정 08-14).
    list_refresh_ms: u64,
    /// 그룹 저장(M5-1 · FR-G-1) — `groups.seg` 암호화 영속(트러스트와 같은 결 —
    /// 그룹 이름·구성원은 인간관계 목록이다).
    groups: nbeep_store::FileGroupStore,
    /// 그룹 스레드(FR-G-3 — 팬아웃 발신을 하나의 스레드로) — 뷰와 분리(DR-26 동형).
    group_threads: HashMap<nbeep_core::group::GroupId, Vec<ChatLine>>,
    /// 그룹 스레드 뷰(단일 모드 주 창 전환용) — 열려 있는 그룹.
    single_open_group: Option<nbeep_core::group::GroupId>,
    /// 그룹 스레드 뷰들(그룹별 · 단일/별도 창 공용 — chats와 동형).
    gchats: HashMap<nbeep_core::group::GroupId, ChatViewWidget>,
    /// 이름 입력 모달 뷰(열려 있을 때만 Some) + 용도.
    name_prompt: Option<nbeep_ui::TextPromptWidget>,
    name_prompt_for: Option<NamePurpose>,
    /// ★ 1:1 **오프라인 대기 큐**(M4-6 · C-3 · 08-20 사용자 확정 — **재시작 유지**):
    /// 세션이 없을 때의 발신을 상대별로 보관했다가 성립 합류점에서 flush한다.
    /// 영속 = `data/pending/{short}.seg`(SEAL_PENDING · 원자적 쓰기) · 상한 =
    /// [`PENDING_DIRECT_MAX`](오래된 것부터 정리). ⚠ 전달은 **내 PC가 켜져 있고
    /// 상대가 나타날 때**만(Q-25-2 — 한계는 상태바 문구로 명시).
    pending_direct: HashMap<PeerId, Vec<PendingDirect>>,
    /// 그룹 발신 중 미연결 구성원에게 이어 보낼 본문(성립 시 flush — 사용자 확정
    /// "자동 연결 시도 후 전송" · 보관 주체 = 송신자). 백오프 소진 시 실패 라인으로 종결.
    pending_group_sends: HashMap<PeerId, Vec<(nbeep_core::group::GroupId, String)>>,
    /// 미연결 구성원에게 이어 보낼 그룹 파일 경로(M5-1g 파일 팬아웃 — 08-13).
    pending_group_files: HashMap<PeerId, Vec<(nbeep_core::group::GroupId, std::path::PathBuf)>>,
    /// 미연결 구성원에게 이어 보낼 그룹 제어 프레임(초대·명부 — M5-1g).
    pending_invites: HashMap<PeerId, Vec<Vec<u8>>>,
    /// 내가 소유한 방에서 **수락을 확인한 구성원**(uid → 수락자 집합 · 08-19).
    /// 초대는 세션 미성립·네트워크 churn으로 첫 전달이 실패할 수 있다(소유자가
    /// 대화하면 세션 성립 → `flush_group_sends` resync가 재전달). 이 집합으로
    /// 구성원 목록에 **"초대 대기"** 를 표시해 미수락(=미전달 포함)을 눈에 보이게
    /// 한다. 재시작으로 비워져도 재초대 수신 측이 Accept를 재통지해 자가 치유한다.
    group_accepts: HashMap<nbeep_core::GroupUid, std::collections::HashSet<PeerId>>,
    /// 방 미확인 메시지 수(M5-1g — 그룹판 unread).
    gunread: HashMap<nbeep_core::group::GroupId, u32>,
    /// 열린 선택 모달의 문맥(초대 수락/거절 등) — Role::Alert 결과 라우팅.
    alert_ctx: Option<AlertCtx>,
    /// 요청 대기 중인 원격 인바운드 세션(M5-3b · §6) — 슬롯 1개(모달 1개 규칙).
    /// 점유 중 추가 원격 인바운드는 드롭(fail-closed). 거절·불일치 = 드롭 = 소켓 닫힘.
    pending_remote: Option<PendingRemote>,
    /// 구성원 모달 열기 대기(08-15 — el은 about_to_wait에 있다 · 내용은 열 때 계산).
    pending_members: Option<nbeep_core::group::GroupId>,
    /// 상대 카드 열기 대기(`/verify` — el은 about_to_wait에 있다 · pending_members와 같은 문법).
    pending_peer_info: Option<PeerId>,
    /// 지문 대조를 이미 권한 상대(대화당 1회 — 반복되면 소음이 되고 안 읽힌다).
    verify_hinted: std::collections::HashSet<PeerId>,
    /// 열려 있는 구성원 모달의 문맥(G4) — (방, 표시 순서의 구성원, 내가 소유자인가).
    /// 행 클릭(제외)의 인덱스 해석 근거 — 모달 내용과 같은 순서로 만든다.
    members_ctx: Option<(nbeep_core::group::GroupId, Vec<PeerId>, bool)>,
    /// 설정 초기화 확인 대기(08-15 · 고급 — 확인 모달은 about_to_wait에서 연다).
    pending_reset: bool,
    /// IME 중재 상태기계(M3-1e G3 — [`crate::ime_gate`]): 조합 게이트·보류-판정·
    /// 유출 조합기·잔향 억제·이동 키 재생·프리에딧 보존을 **한 타입**으로. 실측
    /// 이벤트 순서는 ime_gate의 재생 테스트 10종이 지킨다(H-1~H-24 계보는 그쪽 문서).
    ime: crate::ime_gate::ImeGate<WindowId>,
    /// IME 이벤트 트레이스(`NEXA_IME_TRACE=1`) — 조합 경합은 추정 금지·실측 필수라
    /// 이벤트 순서를 stderr로 남긴다(개인 입력이 찍히므로 opt-in 전용).
    ime_trace: bool,
    /// 지금 OS 포커스를 가진 창 — 캐럿 깜빡임(포커스 창만 점멸)·비포커스 캐럿 소등.
    os_focused: Option<WindowId>,
    /// 캐럿 깜빡임 기준 시각(ms) — 입력(키·IME·클릭)마다 리셋해 타이핑 중엔 항상 밝다.
    blink_anchor_ms: u64,
    /// 마지막으로 그린 깜빡임 위상 — 5Hz 틱이 위상 변화 프레임에만 다시 그린다.
    blink_phase_seen: bool,
    /// OS 주 수식키(⌘/Ctrl) 눌림 상태 — `Cmd/Ctrl+,` 판정.
    primary_down: bool,
    /// Shift 눌림 상태 — Shift+Enter 줄바꿈·Shift+이동 선택(08-10).
    shift_down: bool,
    /// Windows 목록 타입어헤드 한/영 모드([docs/27 §8]). 목록 창은 IME를 끊는데
    /// Windows 한글 입력은 IME 조합에서만 나오므로(레이아웃은 US) 한/영 키가 무력해진다
    /// — 앱이 모드를 직접 들고 라틴 키를 두벌식 자모로 번역한다. macOS/Linux 미사용.
    hangul_mode: bool,
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
    /// ImeGate 명령 적용(M3-1e G3) — 게이트는 판단만, 라우팅·표시는 여기서.
    fn apply_ime(&mut self, outs: Vec<crate::ime_gate::Out<WindowId>>, el: &ActiveEventLoop) {
        for o in outs {
            match o {
                crate::ime_gate::Out::Char(wid, c) => {
                    let now_ms = self.now_ms();
                    self.route(wid, InputEvent::Char { c, now_ms }, el);
                }
                crate::ime_gate::Out::Key(wid, key, shift, primary) => {
                    self.route(
                        wid,
                        InputEvent::Key {
                            key,
                            shift,
                            primary,
                        },
                        el,
                    );
                }
                crate::ime_gate::Out::Preedit(wid, t) => self.set_chat_preedit(wid, t),
            }
        }
    }

    /// 이 창이 목록 모드인가(IME off — 직접 조합 경로 B). 게이트 `ime_on`의 반대편.
    fn is_list_mode(&self, id: WindowId) -> bool {
        Some(id) == self.main_id && self.single_open.is_none() && self.single_open_group.is_none()
    }

    /// IME 켠 창의 프리에딧 표시를 갱신한다(조합 중 밑줄 — 대화·프로필·이름/주소
    /// 프롬프트. 08-13: "확정 전엔 안 보인다"가 모든 입력창 공통 문제였다).
    fn set_chat_preedit(&mut self, id: WindowId, text: String) {
        let mut inv = Invalidations::default();
        if let Some(peer) = self.chat_peer_for(id) {
            if let Some(chat) = self.chats.get_mut(&peer) {
                chat.set_preedit(text, &mut inv);
            }
        } else if let Some(gid) = self.group_chat_for(id) {
            if let Some(chat) = self.gchats.get_mut(&gid) {
                chat.set_preedit(text, &mut inv);
            }
        } else {
            match self.windows.get(&id).map(|e| e.role) {
                Some(Role::Profile) => {
                    if let Some(v) = self.profile_view.as_mut() {
                        v.set_preedit(&text, &mut inv);
                    }
                }
                Some(Role::NamePrompt) => {
                    if let Some(v) = self.name_prompt.as_mut() {
                        v.set_preedit(&text, &mut inv);
                    }
                }
                Some(Role::Convbox) => {
                    if let Some(v) = self.convbox_view.as_mut() {
                        v.set_preedit(&text, &mut inv);
                    }
                }
                Some(Role::AddEndpoint) => {
                    if let Some(v) = self.addr_view.as_mut() {
                        v.set_preedit(&text, &mut inv);
                    }
                }
                _ => {}
            }
        }
        self.request_redraw(id);
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
        let attrs = self.modal_attrs(attrs, false); // 메인 소유(08-15 — 창 묶음 부상)
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
            tagline: nbeep_core::t(nbeep_core::Msg::AboutTagline).into(),
            links: vec![
                ("SosomLab".into(), "https://sosomlab.com".into()),
                (
                    nbeep_core::t(nbeep_core::Msg::AboutHomepage).into(),
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
    /// 소유·위치 기준 = **진원 창**(08-20 — 종전 메인 소유는 경고가 뜰 때 메인
    /// 묶음이 대화창 위로 부상 · 위치도 진원 창 중앙 부근이라야 시선이 잇닿는다).
    fn open_alert(
        &mut self,
        el: &ActiveEventLoop,
        title: &str,
        message: &str,
        anchor: Option<WindowId>,
    ) {
        // 본문 줄 수에 맞춰 높이 산정(08-14 — 그룹 구성원 목록처럼 여러 줄 본문이
        // 고정 170에서 잘리던 것). 워드랩 줄은 근사에서 빠지지만 명시 줄바꿈이 기준.
        let lines = message.lines().count().max(1) as f64;
        let win_h = (110.0 + lines * 22.0).clamp(170.0, 460.0);
        if let Some((aid, _)) = self.windows.iter().find(|(_, e)| e.role == Role::Alert) {
            let aid = *aid;
            let mut inv = Invalidations::default();
            if let Some(av) = &mut self.alert_view {
                av.set_content(title, message, &mut inv);
            }
            if let Some(e) = self.windows.get(&aid) {
                let _ = e
                    .window
                    .request_inner_size(winit::dpi::LogicalSize::new(400.0, win_h));
                e.window.focus_window();
            }
            self.request_redraw(aid);
            return;
        }
        let mut attrs = Window::default_attributes()
            .with_title(format!(
                "Nexa Beep — {}",
                nbeep_core::t(nbeep_core::Msg::WinAlert)
            ))
            .with_inner_size(winit::dpi::LogicalSize::new(400.0, win_h))
            .with_resizable(false)
            .with_window_icon(self.icon.clone());
        // 진원 창 중앙 부근에 배치(승인 창과 같은 문법 — 08-20).
        if let Some(e) = anchor
            .and_then(|a| self.windows.get(&a))
            .or_else(|| self.main_id.and_then(|m| self.windows.get(&m)))
        {
            if let Ok(pos) = e.window.outer_position() {
                let sz = e.window.inner_size();
                let sf = e.window.scale_factor();
                let (mw, mh) = ((400.0 * sf) as i32, (win_h * sf) as i32);
                attrs = attrs.with_position(winit::dpi::PhysicalPosition::new(
                    pos.x + (sz.width as i32 - mw) / 2,
                    pos.y + (sz.height as i32 - mh) / 2,
                ));
            }
        }
        let attrs = self.modal_attrs_from(anchor, attrs, false); // 진원 창 소유(부상·복귀 축)
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

    /// 지금 열린 **앱 모달** 창(08-14 표준 재정리) — 겹치면 더 안쪽 층이 입력을
    /// 갖는다(알림 > 피커 > 이름/주소 프롬프트 > 프로필 > About). 모달이 떠 있는
    /// 동안 다른 앱 창은 입력 불가·클릭 시 모달이 앞으로 온다.
    fn modal_id(&self) -> Option<WindowId> {
        let picks: [fn(Role) -> bool; 6] = [
            |r| matches!(r, Role::Alert),
            |r| matches!(r, Role::Picker),
            |r| matches!(r, Role::NamePrompt),
            |r| matches!(r, Role::AddEndpoint),
            |r| matches!(r, Role::Profile),
            |r| matches!(r, Role::About),
        ];
        for pick in picks {
            if let Some((wid, _)) = self.windows.iter().find(|(_, e)| pick(e.role)) {
                return Some(*wid);
            }
        }
        None
    }

    /// 앱 모달 창 속성(08-15 사용자 요청 2건) — ① `at_cursor`면 **현재 마우스
    /// 위치가 창 좌상단**(기본 위치로 멀리 열려 커서 왕복이 번거롭다) ②
    /// **Windows는 메인 창 소유(owned)** — 소유 창을 클릭하면 OS가 소유자(메인)까지
    /// 함께 앞으로 올린다. 2-앱 실기: B 모달을 선택해도 B 메인이 A 창들 뒤에
    /// 남던 문제의 처방 — AlwaysOnTop 없이 "모달은 메인 위 + 같은 앱 창 묶음
    /// 부상"이 성립하고 다른 앱을 가리지도 않는다(창 전환 관례).
    fn modal_attrs(
        &self,
        attrs: winit::window::WindowAttributes,
        at_cursor: bool,
    ) -> winit::window::WindowAttributes {
        self.modal_attrs_from(None, attrs, at_cursor)
    }

    /// [`Self::modal_attrs`]의 진원 창 지정판(08-18 실기) — 소유자·커서 기준을
    /// **연 창**으로 잡는다. 격리함에서 연 미리보기가 메인 소유면 메인이 격리함
    /// 위로 부상하고, 닫아도 격리함으로 안 돌아온다(소유 관계 = 부상·복귀 축).
    /// `anchor`가 없거나 죽었으면 종전대로 메인.
    fn modal_attrs_from(
        &self,
        anchor: Option<WindowId>,
        attrs: winit::window::WindowAttributes,
        at_cursor: bool,
    ) -> winit::window::WindowAttributes {
        let Some(e) = anchor
            .and_then(|a| self.windows.get(&a))
            .or_else(|| self.main_id.and_then(|m| self.windows.get(&m)))
        else {
            return attrs;
        };
        let mut attrs = attrs;
        if at_cursor {
            if let Ok(p) = e.window.inner_position() {
                attrs = attrs.with_position(winit::dpi::PhysicalPosition::new(
                    p.x + e.cursor.0,
                    p.y + e.cursor.1,
                ));
            }
        }
        #[cfg(windows)]
        {
            use winit::platform::windows::WindowAttributesExtWindows;
            use winit::raw_window_handle::{HasWindowHandle as _, RawWindowHandle};
            if let Ok(h) = e.window.window_handle() {
                if let RawWindowHandle::Win32(w) = h.as_raw() {
                    attrs = attrs.with_owner_window(w.hwnd.get());
                }
            }
        }
        attrs
    }

    /// 이 창에서 열려 있는 그룹 방(M5-1g) — `chat_peer_for`의 그룹판.
    fn group_chat_for(&self, id: WindowId) -> Option<nbeep_core::group::GroupId> {
        match self.windows.get(&id).map(|e| e.role) {
            Some(Role::GroupChat(gid)) => Some(gid),
            Some(Role::Main) => self.single_open_group,
            _ => None,
        }
    }

    /// 드롭된 파일을 **큐에 넣는다**(다중 드롭 = 여러 번 호출된다 · winit는 파일마다
    /// 이벤트를 준다). 협상은 한 번에 하나씩 — 승인도 파일마다 받아야 하기 때문이다.
    /// 요청당 파일 수 상한(설정 `xfer.batch_max` · 1~5 · 기본 5).
    fn batch_max(&self) -> usize {
        self.settings
            .get("xfer.batch_max")
            .parse::<usize>()
            .unwrap_or(5)
            .clamp(1, 5)
    }

    /// 한 번의 드롭 묶음을 요청 단위로 선판정(08-20 4차 확정) — 기존 배치
    /// 시도분(제외 포함)과 이번 드롭 수의 합이 상한을 넘으면 **한 파일도
    /// 시도하지 않고** 안내 모달만 띄운다(부분 전송 금지).
    /// 클립보드 **이미지** 붙여넣기 시도(③ 08-20 사용자 확정 3-OS) — 텍스트가 없을
    /// 때의 폴백. 대화(1:1·그룹) 창에서만 발화한다. 읽기+PNG 인코딩(Windows DIB →
    /// imgdec 워커)은 **별도 스레드** — imgdec 스폰 1~2초가 UI를 얼리지 않게
    /// (디코드 워커화와 같은 이유 · 08-13). 완료는 [`AppEvent::ClipImage`]로 복귀.
    /// 반환 = 시도했는가(대화 창이 아니면 false — 호출자가 종전 안내).
    fn try_clipboard_image_paste(&mut self, id: WindowId) -> bool {
        if self.chat_peer_for(id).is_none() && self.group_chat_for(id).is_none() {
            return false;
        }
        let proxy = self.proxy.clone();
        std::thread::spawn(move || {
            let png = match nbeep_plat::clipboard::get_image() {
                Some(nbeep_plat::clipboard::ClipImage::Png(b)) => Some(b),
                Some(nbeep_plat::clipboard::ClipImage::Rgba { w, h, data }) => {
                    crate::imgdec::encode_raw_isolated(w, h, &data)
                }
                None => None,
            };
            let _ = proxy.send_event(AppEvent::ClipImage { win: id, png });
        });
        true
    }

    fn offer_dropped(&mut self, drops: Vec<(WindowId, std::path::PathBuf)>) {
        let mut by_win: Vec<(WindowId, Vec<std::path::PathBuf>)> = Vec::new();
        for (id, p) in drops {
            if let Some(e) = by_win.iter_mut().find(|(w, _)| *w == id) {
                e.1.push(p);
            } else {
                by_win.push((id, vec![p]));
            }
        }
        let max = self.batch_max();
        for (id, paths) in by_win {
            let attempted = self.chat_peer_for(id).map_or(0, |peer| {
                self.send_batch.get(&peer).map_or(0, |b| b.1 as usize)
                    + self.send_excluded.get(&peer).map_or(0, Vec::len)
            });
            if attempted + paths.len() > max {
                self.pending_alert = Some((
                    nbeep_core::t(nbeep_core::Msg::WarnBatchLimitTitle).to_string(),
                    nbeep_core::tf(nbeep_core::Msg::WarnBatchLimitBody, &[&max.to_string()]),
                    Some(id),
                ));
                self.request_redraw(id);
                continue;
            }
            for p in paths {
                self.offer_file(id, &p);
            }
        }
    }

    fn offer_file(&mut self, id: WindowId, path: &std::path::Path) {
        // 그룹 방 — 명부 팬아웃. 대화 여부와 무관하게 시도 가능, 게이트는 수신자
        // 승인이다(사용자 확정 08-13).
        if let Some(gid) = self.group_chat_for(id) {
            self.offer_file_group(gid, id, path);
            return;
        }
        let Some(peer) = self.chat_peer_for(id) else {
            self.set_status(nbeep_core::t(nbeep_core::Msg::StXferNeedPeer));
            self.request_redraw(id);
            return;
        };
        // 사전 점검 — 핀 미고정·차단은 여전히 막는다. **상호 미왕래는 경고 후 진행**
        // (08-13 확정: 수신측이 수동 승인으로 강등해 받으므로 발신을 막을 이유가 없다).
        {
            use nbeep_core::TrustStore as _;
            if let Err(reason) =
                nbeep_core::check_send_eligibility(self.trust.level(peer), self.ledger.get(peer))
            {
                if matches!(reason, nbeep_core::DenyReason::NoMutualConversation) {
                    self.push_peer_note(peer, nbeep_core::t(nbeep_core::Msg::NoticeFirstContact));
                } else {
                    // 모달로 세운다(08-13 사용자 실기 — 상태바 한 줄은 지나쳐서 "그냥
                    // 전송이 안 된다"로 보였다). 창 생성은 about_to_wait 몫.
                    self.status =
                        nbeep_core::tf(nbeep_core::Msg::StfCannotSend, &[reason.message()]);
                    let how = match reason {
                        nbeep_core::DenyReason::NotPinned => {
                            "\n\n상대와 연결(세션)이 성립하면 신원이 고정됩니다 — 목록에서 상대를 열어 연결부터 하세요."
                        }
                        _ => "",
                    };
                    self.pending_alert = Some((
                        "파일을 보낼 수 없습니다".into(),
                        format!("{}{how}", reason.message()),
                        Some(id),
                    ));
                    self.request_redraw(id);
                    return;
                }
            }
        }
        // ★ 원격 × 미대조 = 파일 발신 금지(M5-3b · §5-1-3 FR-S-24) — 인터넷 경유
        //   상대는 지문(SAS) 대조 전엔 파일을 보내지 않는다. 메시지는 허용(등급 곱).
        if self.remote_file_blocked(peer) {
            self.status = nbeep_core::t(nbeep_core::Msg::StRemoteFileBlocked).to_string();
            self.pending_alert = Some((
                nbeep_core::t(nbeep_core::Msg::AlertCannotSendFile).to_string(),
                nbeep_core::t(nbeep_core::Msg::RemoteFileNeedVerify).to_string(),
                Some(id),
            ));
            self.request_redraw(id);
            return;
        }
        let size = std::fs::metadata(path).map(|m| m.len()).unwrap_or(0);
        let name = path
            .file_name()
            .map_or_else(|| "file".to_string(), |n| n.to_string_lossy().into_owned());
        // ★ 폴더 전송 불가(08-20 확정) — 제외 대상으로 목록에 남긴다(상한 합산 포함).
        if path.is_dir() {
            self.push_excluded_line(peer, true, &name, 0);
            self.set_status(nbeep_core::tf(nbeep_core::Msg::StfFolderExcluded, &[&name]));
            self.send_manifest(peer);
            self.request_redraw(id);
            return;
        }
        // ★ 파일 단위 용량 검사(M4-2e · 사용자 확정 08-19) — 발신 상한과 **상대
        //   수신 상한**(CapAdvert 기억값) 중 낮은 쪽을 넘는 파일은 **전송 제외**:
        //   큐에 넣지 않고, 스레드 라인에는 "전송 제외" 상태로 남긴다(배치는 계속).
        let send_cap = cap_from_setting(self.settings.get("xfer.send_max_mb"));
        let peer_cap = self
            .peer_recv_cap
            .get(&peer)
            .map(|&(c, _)| c)
            .filter(|c| *c < u64::MAX);
        let eff_cap = match (send_cap, peer_cap) {
            (Some(a), Some(b)) => Some(a.min(b)),
            (a, b) => a.or(b),
        };
        if eff_cap.is_some_and(|cap| size > cap) {
            self.push_excluded_line(peer, true, &name, size);
            self.send_manifest(peer); // 목록 변화 즉시 공지(수신측 목록에도 제외 표시)
            self.request_redraw(id);
            return;
        }
        // ★ 요청당 파일 수 상한(M4-2e 규격 · 08-20 3차 확정) — **제외 포함
        //   합산**이 기준: 시도한 전체(전송분 b.1 + 제외분)가 상한이면 이후
        //   드롭은 **보내지 않고 안내 모달만**(닫기 버튼 하나 — 사용자 확정:
        //   목록·manifest에 남기지 않는다). 상한 = `xfer.batch_max`(1~5 · 기본 5).
        {
            let max = self.batch_max();
            let attempted = self.send_batch.get(&peer).map_or(0, |b| b.1 as usize)
                + self.send_excluded.get(&peer).map_or(0, Vec::len);
            if attempted >= max {
                self.pending_alert = Some((
                    nbeep_core::t(nbeep_core::Msg::WarnBatchLimitTitle).to_string(),
                    nbeep_core::tf(nbeep_core::Msg::WarnBatchLimitBody, &[&max.to_string()]),
                    Some(id),
                ));
                self.request_redraw(id);
                return;
            }
        }
        // 스레드 라인 = **드롭 즉시**(M4-2e — 종전엔 오퍼 시점에만 라인이라 큐
        // 파일이 대화창에 안 보였다. push_xfer_line은 재활성 가드가 있어 오퍼
        // 시점의 재호출과 중복되지 않는다).
        self.push_xfer_line(peer, true, &name, size);
        self.send_queue
            .entry(peer)
            .or_default()
            .push_back(path.to_path_buf());
        let b = self.send_batch.entry(peer).or_insert((0, 0, 0, 0));
        b.1 += 1;
        b.3 += size;
        let total = b.3;
        self.set_status(nbeep_core::tf(
            nbeep_core::Msg::StfSendQueued,
            &[
                &self
                    .send_queue
                    .get(&peer)
                    .map_or(0, VecDeque::len)
                    .to_string(),
                &human_size(total),
            ],
        ));
        self.pump_send_queue(peer);
        self.send_manifest(peer); // 배치 목록 공지(M4-2e — 요청 단위 승인 원료)
        self.refresh_send_batch(peer); // 대기 중 더 드롭한 파일도 패널에 반영(M4-2d)
        self.request_redraw(id);
    }

    /// 그룹 방 파일 전송 — 명부 팬아웃(M5-1g · 08-13 확정: 대화 여부 무관 시도 가능,
    /// 게이트 = 수신자 승인). 미연결 구성원은 경로를 대기시키고 자동 연결한다.
    fn offer_file_group(
        &mut self,
        gid: nbeep_core::group::GroupId,
        id: WindowId,
        path: &std::path::Path,
    ) {
        let me = self.identity.peer_id();
        let members: Vec<PeerId> = match self.groups.shared_by_id(gid) {
            Some(s) => s
                .roster
                .members
                .iter()
                .copied()
                .filter(|m| *m != me)
                .collect(),
            None => {
                self.set_status(nbeep_core::t(nbeep_core::Msg::StGroupGone));
                self.request_redraw(id);
                return;
            }
        };
        let size = std::fs::metadata(path).map(|m| m.len()).unwrap_or(0);
        let mut inv = Invalidations::default();
        let mut offered = 0usize;
        let mut waiting: Vec<PeerId> = Vec::new();
        let mut first_contact: Vec<String> = Vec::new();
        for m in &members {
            if !self.ledger.get(*m).is_mutual() {
                first_contact.push(self.peer_title(*m));
            }
            if self.conversations.contains_key(m) {
                self.send_queue
                    .entry(*m)
                    .or_default()
                    .push_back(path.to_path_buf());
                let b = self.send_batch.entry(*m).or_insert((0, 0, 0, 0));
                b.1 += 1;
                b.3 += size;
                self.pump_send_queue(*m);
                offered += 1;
            } else {
                // 미연결 — 경로를 대기시키고 자동 연결(성립 시 flush_group_sends 합류).
                let q = self.pending_group_files.entry(*m).or_default();
                q.push((gid, path.to_path_buf()));
                if q.len() > GROUP_FILE_KEEP {
                    let n = q.len() - GROUP_FILE_KEEP;
                    q.drain(..n);
                }
                waiting.push(*m);
            }
        }
        for m in &waiting {
            self.reconnect.remove(m); // 발신 의사 = 백오프 리셋(즉시 시도)
            self.start_connect(*m, true); // 자동 — 창을 열지 않는다
        }
        // 첫 왕래 상대 1줄 경고(사용자 확정 08-13) — 왜 바로 안 가는지 보이게.
        if !first_contact.is_empty() {
            self.push_group_note(
                gid,
                &format!(
                    "! 아직 서로 메시지를 주고받은 적 없는 상대: {} — 수신 승인을 눌러야 전송이 진행됩니다",
                    first_contact.join(", ")
                ),
                &mut inv,
            );
        }
        self.set_status(nbeep_core::tf(
            nbeep_core::Msg::StfGroupFileOffer,
            &[&offered.to_string(), &waiting.len().to_string()],
        ));
        self.request_redraw(id);
    }

    /// 1:1 스레드에 시스템 안내 라인(그룹판 `push_group_note`와 동형).
    fn push_peer_note(&mut self, peer: PeerId, note: &str) {
        let (at_ms, wall) = now_stamp();
        let line = ChatLine::text(false, nbeep_core::sanitize_message(note), at_ms, wall);
        if let Some(conv) = self.conversations.get_mut(&peer) {
            conv.lines.push(line.clone());
        }
        self.record_history(peer); // 대화 기록 영속(M2-5b)
        let mut inv = Invalidations::default();
        if let Some(chat) = self.chats.get_mut(&peer) {
            chat.push_line(line, &mut inv);
        }
        self.redraw_conversation(peer);
    }

    /// 큐에서 다음 파일을 꺼내 오퍼를 보낸다(협상 중이면 대기).
    fn pump_send_queue(&mut self, peer: PeerId) {
        if self.awaiting_accept.contains_key(&peer) || self.preparing_send.contains(&peer) {
            return; // 앞 파일 협상·해시 준비 중
        }
        // 일시정지된 대기 파일은 건너뛴다(M4-2d — "앞 전송이 끝나면 pass 후 다음
        // 파일"): 앞에서부터 꺼내 정지 파일은 모아 두고, 첫 비정지 파일을 고른다.
        // 고른 뒤 정지 파일을 **원래 순서로 큐 앞에 되돌려** 재개 때 자리를 지킨다.
        let picked = {
            let mut skipped: Vec<std::path::PathBuf> = Vec::new();
            let chosen = loop {
                let Some(p) = self.send_queue.get_mut(&peer).and_then(VecDeque::pop_front) else {
                    break None;
                };
                if self.send_paused.get(&peer).is_some_and(|s| s.contains(&p)) {
                    skipped.push(p);
                } else {
                    break Some(p);
                }
            };
            if !skipped.is_empty() {
                let q = self.send_queue.entry(peer).or_default();
                for p in skipped.into_iter().rev() {
                    q.push_front(p);
                }
            }
            chosen
        };
        let Some(path) = picked else {
            // 보낼 게 없다 — 큐가 완전히 비었으면 배치 종료, 전부 일시정지면 유지.
            let has_paused = self.send_queue.get(&peer).is_some_and(|q| !q.is_empty());
            if !has_paused {
                self.send_batch.remove(&peer);
                self.send_excluded.remove(&peer); // 배치 종료 = 제외 목록도 마감(M4-2e)
            }
            self.refresh_send_batch(peer); // 창 닫기(종료) 또는 상태 갱신(전부 정지)
            return;
        };
        // ★ 크기 사전 검사(08-18 실기 — 10.7GB ISO DnD가 앱을 얼렸다): 설정 상한
        //   (`xfer.send_max_mb` · 무제한 가능)을 **읽기 전에** 검사한다. 수신측
        //   상한 협상(Reject·TooLarge)과는 별개의 발신측 관문.
        let fsize = std::fs::metadata(&path).map(|m| m.len()).unwrap_or(0);
        let cap = cap_from_setting(self.settings.get("xfer.send_max_mb"));
        if let Some(cap) = cap {
            if fsize > cap {
                let name = path
                    .file_name()
                    .map_or_else(|| "file".to_string(), |n| n.to_string_lossy().into_owned());
                self.push_xfer_line(peer, true, &name, fsize);
                self.set_xfer_line(
                    peer,
                    true,
                    nbeep_ui::XferLineState::Failed {
                        why: nbeep_core::tf(nbeep_core::Msg::XferTooBigWhy, &[&human_size(cap)]),
                    },
                );
                self.set_status(nbeep_core::tf(
                    nbeep_core::Msg::XferTooBigLocal,
                    &[&name, &human_size(cap)],
                ));
                self.redraw_conversation(peer);
                self.pump_send_queue(peer); // 다음 파일(배치 계속)
                return;
            }
        }
        self.current_send.insert(peer, path.clone()); // 끊김 대비(M4-10c)
                                                      // ★ 지연 해시(08-18 — "수신 확인이 2~3초 늦게 뜬다"): 오퍼 전 전체 해시를
                                                      //   없앤다. Offer sha=0(선언 유예) · 발신이 전송하며 증분 계산해 Done에
                                                      //   동봉 · 수신은 완료 시 그 값과 대조(무결성 검증 지점 불변 — FR-X-6).
        let name = path
            .file_name()
            .map_or_else(|| "file".to_string(), |n| n.to_string_lossy().into_owned());
        let mut xid = [0u8; 16];
        xid.copy_from_slice(&nbeep_crypto::Identity::generate().peer_id().as_bytes()[..16]);
        let sent = self.conversations.get(&peer).is_some_and(|c| {
            c.out_tx
                .send(SessionCmd::OfferFile {
                    id: xid,
                    name: name.clone(),
                    sha: [0u8; 32],
                    path,
                    size: fsize,
                })
                .is_ok()
        });
        if !sent {
            self.set_status(nbeep_core::t(nbeep_core::Msg::StSessionDropSend));
            self.send_queue.remove(&peer);
            self.send_batch.remove(&peer);
            self.send_excluded.remove(&peer); // 배치 종료 = 제외 목록도 마감(M4-2e)
            return;
        }
        self.awaiting_accept.insert(peer, xid);
        self.set_status(nbeep_core::tf(
            nbeep_core::Msg::StfFileOffer,
            &[&name, &human_size(fsize)],
        ));
        self.push_xfer_line(peer, true, &name, fsize);
        self.open_send_wait(peer, &name);
    }

    #[allow(dead_code)]
    fn dead_prehash_removed(&mut self, peer: PeerId, path: std::path::PathBuf, fsize: u64) {
        // ★ 스트리밍 발신(08-18 — "Unlimited에서 10GB도 무정지"): 전량 fs::read를
        //   버리고 ① 전체 해시는 **워커 스레드**에서 스트리밍으로(메인 무정지)
        //   ② 청크는 액터가 파일에서 직접 읽는다(메모리 O(청크)).
        // 준비 표시(08-18 실기 — 10GB 해시가 수십 초라 "무반응"으로 보였다):
        // 스레드 항목을 먼저 세우고 상태바로 준비 중임을 알린다.
        // ★ 상대 수신 상한 확인(08-18 사용자 요청 — "송신 처리 전에 수신 제한
        //   정보를 확인"): 파일 크기는 메타데이터로 이미 아니(읽기 0), 상한이
        //   신선(3초 이내 공지)하면 즉시 대조, 아니면 질의를 보내고 응답(또는
        //   2초 타임아웃 = 구버전 상대)에 이어서 진행한다.
        let now = self.now_ms();
        let fresh = self
            .peer_recv_cap
            .get(&peer)
            .is_some_and(|(_, at)| now.saturating_sub(*at) < 3_000);
        if !fresh && self.conversations.contains_key(&peer) {
            // 큐 앞으로 되돌리고 질의 1회 — 응답·타임아웃이 다시 펌프한다.
            self.send_queue.entry(peer).or_default().push_front(path);
            if !self.cap_req_deadline.contains_key(&peer) {
                if let Some(conv) = self.conversations.get(&peer) {
                    let _ = conv.out_tx.send(SessionCmd::Control(vec![
                        nbeep_core::xfer::encode_cap_request(),
                    ]));
                }
                self.cap_req_deadline.insert(peer, now + 2_000);
            }
            return;
        }
        if let Some(&(pcap, _)) = self.peer_recv_cap.get(&peer) {
            if pcap < u64::MAX && fsize > pcap {
                let name = path
                    .file_name()
                    .map_or_else(|| "file".to_string(), |n| n.to_string_lossy().into_owned());
                self.push_xfer_line(peer, true, &name, fsize);
                self.set_xfer_line(
                    peer,
                    true,
                    nbeep_ui::XferLineState::Failed {
                        why: nbeep_core::tf(
                            nbeep_core::Msg::XferPeerCapBlock,
                            &[&human_size(pcap)],
                        ),
                    },
                );
                self.set_status(nbeep_core::tf(
                    nbeep_core::Msg::XferPeerCapBlockStatus,
                    &[&name, &human_size(pcap)],
                ));
                self.redraw_conversation(peer);
                self.pump_send_queue(peer); // 다음 파일(배치 계속)
                return;
            }
        }
        {
            let name = path
                .file_name()
                .map_or_else(|| "file".to_string(), |n| n.to_string_lossy().into_owned());
            self.push_xfer_line(peer, true, &name, fsize);
            self.set_status(nbeep_core::tf(nbeep_core::Msg::XferHashing, &[&name]));
            self.redraw_conversation(peer);
        }
        self.preparing_send.insert(peer);
        let proxy = self.proxy.clone();
        std::thread::spawn(move || {
            let name = path
                .file_name()
                .map_or_else(|| "file".to_string(), |n| n.to_string_lossy().into_owned());
            // 증분 해시 + 진행 이벤트(≈5%마다) — "무반응"을 없앤다.
            let sha = (|| {
                use std::io::Read as _;
                let mut f = std::fs::File::open(&path).ok()?;
                let mut h = nbeep_crypto::Sha256Stream::new();
                let mut buf = vec![0u8; 1024 * 1024];
                let mut done = 0u64;
                let mut last_pct = 0u8;
                loop {
                    let n = f.read(&mut buf).ok()?;
                    if n == 0 {
                        break;
                    }
                    h.update(&buf[..n]);
                    done += n as u64;
                    #[allow(clippy::cast_possible_truncation)]
                    let pct = (done.saturating_mul(100) / fsize.max(1)).min(100) as u8;
                    if pct >= last_pct.saturating_add(5) {
                        last_pct = pct;
                        let _ = proxy.send_event(AppEvent::HashProgress {
                            peer,
                            name: name.clone(),
                            pct,
                        });
                    }
                }
                Some(h.finalize())
            })();
            let _ = proxy.send_event(AppEvent::SendHashed {
                peer,
                path,
                size: fsize,
                sha,
            });
        });
    }

    /// 발신 대기 시작(한 파일 오퍼마다) — **승인 타임아웃 타이머**를 (재)무장하고,
    /// 배치 패널을 갱신한다(M4-2d). 타이머는 렌더하지 않는다(패널이 화면을 갖는다).
    fn open_send_wait(&mut self, peer: PeerId, _name: &str) {
        let ms = self.wait_timeout_sec.saturating_mul(1000);
        let mut tb = nbeep_ui::TimeoutButton::new(String::new(), ms);
        tb.start(self.now_ms());
        self.send_wait.insert(peer, tb);
        self.refresh_send_batch(peer);
    }

    /// 발신 대기 강제 종료 — 승인 타임아웃 타이머·정지 표식을 정리한다
    /// (취소·실패·배치 종료 · 별도 창은 M4-2e에서 제거 — 인챗 라인이 화면).
    fn close_send_wait(&mut self, peer: PeerId) {
        self.send_wait.remove(&peer);
        self.send_paused.remove(&peer);
    }

    /// 발신 배치 상태 정리(M4-2e — 별도 창 제거 후 정리 판단만 남았다):
    /// 배치가 전부 끝났으면 타이머·정지 표식을 정리한다. 진행 표시는 인챗 라인.
    fn refresh_send_batch(&mut self, peer: PeerId) {
        let busy = self.current_send.contains_key(&peer)
            || self.awaiting_accept.contains_key(&peer)
            || self.active_send.contains_key(&peer)
            || self.preparing_send.contains(&peer)
            || self.paused_sends.contains_key(&peer) // 정지만 남아도 배치 유지(ⓑ)
            || self.send_queue.get(&peer).is_some_and(|q| !q.is_empty());
        if !busy {
            self.close_send_wait(peer); // 배치 종료 → 타이머·표식 정리
        }
    }

    /// 전송 제외 라인(M4-2e) — 큐에 넣지 않고 스레드에 "전송 제외" 종결 라인으로
    /// 남긴다(사용자 확정: 초과 파일도 목록에 보이게 · 배치는 계속). manifest에도
    /// excluded로 실려 수신측 목록에 같은 상태가 보인다.
    fn push_excluded_line(&mut self, peer: PeerId, mine: bool, name: &str, size: u64) {
        let (at_ms, wall) = now_stamp();
        let safe = nbeep_core::sanitize_message(name);
        let mut line = ChatLine::xfer(mine, safe, size, at_ms, wall);
        if let nbeep_ui::ChatBody::Xfer(x) = &mut line.body {
            x.state = nbeep_ui::XferLineState::Failed {
                why: nbeep_core::t(nbeep_core::Msg::XferExcluded).into(),
            };
        }
        if let Some(conv) = self.conversations.get_mut(&peer) {
            conv.lines.push(line.clone());
        }
        let mut inv = Invalidations::default();
        if let Some(chat) = self.chats.get_mut(&peer) {
            chat.push_line(line, &mut inv);
        }
        self.record_history(peer);
        if mine {
            self.send_excluded
                .entry(peer)
                .or_default()
                .push((name.to_string(), size));
        }
        self.set_status(format!(
            "{name} — {}",
            nbeep_core::t(nbeep_core::Msg::XferExcluded)
        ));
        self.redraw_conversation(peer);
    }

    /// 배치 목록(manifest) 발신(M4-2e) — 현재+큐+제외 전 파일을 Control 프레임으로
    /// 알린다(수신측 요청 단위 승인·목록 표시의 원료). 세션 없으면 조용히 생략 —
    /// 구버전 수신자는 미지 태그 무시(전방 호환 · 파일 단위 승인으로 강등).
    fn send_manifest(&mut self, peer: PeerId) {
        let name_of = |p: &std::path::Path| {
            p.file_name()
                .map_or_else(|| "file".to_string(), |n| n.to_string_lossy().into_owned())
        };
        let size_of = |p: &std::path::Path| std::fs::metadata(p).map(|m| m.len()).unwrap_or(0);
        let mut entries: Vec<(String, u64, bool)> = Vec::new();
        if let Some(p) = self.current_send.get(&peer) {
            entries.push((name_of(p), size_of(p), false));
        }
        if let Some(q) = self.send_queue.get(&peer) {
            for p in q {
                entries.push((name_of(p), size_of(p), false));
            }
        }
        if let Some(ex) = self.send_excluded.get(&peer) {
            for (n, s) in ex {
                entries.push((n.clone(), *s, true));
            }
        }
        if let Some(conv) = self.conversations.get(&peer) {
            let _ = conv.out_tx.send(SessionCmd::Control(vec![
                nbeep_core::xfer::encode_batch_manifest(&entries),
            ]));
        }
    }

    /// 배치의 모든 파일 경로(현재 + 큐).
    fn batch_paths(&self, peer: PeerId) -> Vec<std::path::PathBuf> {
        let mut v = Vec::new();
        if let Some(p) = self.current_send.get(&peer) {
            v.push(p.clone());
        }
        if let Some(q) = self.send_queue.get(&peer) {
            v.extend(q.iter().cloned());
        }
        v
    }

    /// 목록 i번째 파일 경로(0 = 현재 파일 · 있으면).
    fn nth_batch_path(&self, peer: PeerId, i: usize) -> Option<std::path::PathBuf> {
        self.batch_paths(peer).into_iter().nth(i)
    }

    /// 그 파일만 취소(M4-2d 행 취소) — 대기 파일은 큐에서 제거, 현재 파일은
    /// Cancel 통지 후 다음 파일로 넘어간다(배치 전체를 비우지 않는다).
    fn cancel_one_send(&mut self, peer: PeerId, i: usize) {
        let Some(path) = self.nth_batch_path(peer, i) else {
            return;
        };
        let is_current = self.current_send.get(&peer) == Some(&path);
        if is_current {
            // 현재 파일만 취소 — xid Cancel 통지 + 상태 해제 후 다음으로.
            if let Some(xid) = self
                .awaiting_accept
                .remove(&peer)
                .or_else(|| self.active_send.remove(&peer))
            {
                if let Some(c) = self.conversations.get(&peer) {
                    let _ = c.out_tx.send(SessionCmd::CancelXfer(xid));
                }
            }
            self.current_send.remove(&peer);
            self.preparing_send.remove(&peer);
            if let Some(set) = self.send_paused.get_mut(&peer) {
                set.remove(&path);
            }
            self.set_xfer_line(
                peer,
                true,
                nbeep_ui::XferLineState::Failed {
                    why: nbeep_core::t(nbeep_core::Msg::XferCanceled).into(),
                },
            );
            self.pump_send_queue(peer); // 다음 파일(배치 계속)
        } else {
            // 대기 파일 — 큐에서만 제거.
            if let Some(q) = self.send_queue.get_mut(&peer) {
                if let Some(pos) = q.iter().position(|p| p == &path) {
                    q.remove(pos);
                }
            }
            if let Some(set) = self.send_paused.get_mut(&peer) {
                set.remove(&path);
            }
        }
    }

    /// 활성 전송 일시정지/재개(P2 — 액터 pump gate). 세션은 유지한 채 청크 펌프만
    /// 멈췄다 잇는다(와이어 무변경 — 수신측은 그저 다음 청크를 기다린다).
    fn pause_active_send(&mut self, peer: PeerId, pause: bool) {
        if let Some(xid) = self.active_send.get(&peer).copied() {
            if let Some(c) = self.conversations.get(&peer) {
                let _ = c.out_tx.send(if pause {
                    SessionCmd::PauseXfer(xid)
                } else {
                    SessionCmd::ResumeXfer(xid)
                });
            }
        }
    }

    /// 발신 취소(타임아웃·사용자) — 상대에게 Cancel을 보내고 큐를 비운다.
    fn cancel_send(&mut self, peer: PeerId, by_timeout: bool) {
        self.send_cancel_all_notice(peer); // 상대도 자기 쪽 전체취소 루틴(M4-2e)
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
                    nbeep_core::t(nbeep_core::Msg::XferTimeoutCancel).into()
                } else {
                    nbeep_core::t(nbeep_core::Msg::XferCanceled).into()
                },
            },
        );
        // 보관 정지분(ⓐ)도 전체 취소에 포함(ⓒ — 어느 쪽에서든 전체 중지).
        if let Some(list) = self.paused_sends.remove(&peer) {
            if let Some(c) = self.conversations.get(&peer) {
                for (_, _, _, xid) in list {
                    let _ = c.out_tx.send(SessionCmd::CancelXfer(xid));
                }
            }
        }
        self.send_queue.remove(&peer);
        self.send_batch.remove(&peer);
        self.send_excluded.remove(&peer); // 배치 종료 = 제외 목록도 마감(M4-2e)
        self.send_paused.remove(&peer);
        self.fail_pending_xfer_lines(
            peer,
            true,
            nbeep_core::t(nbeep_core::Msg::XferCanceled).to_string(),
        ); // 대기·정지 라인 일괄 종결(M4-2e)
        self.close_send_wait(peer);
        self.clear_xfer(peer);
        self.set_status(if by_timeout {
            nbeep_core::tf(
                nbeep_core::Msg::StfXferTimeoutCanceled,
                &[&self.wait_timeout_sec.to_string()],
            )
        } else {
            nbeep_core::t(nbeep_core::Msg::StXferCanceled).to_string()
        });
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
        let Some((xid, name, size, sha)) = self
            .pending_offers
            .get_mut(&peer)
            .and_then(VecDeque::pop_front)
        else {
            self.set_status(nbeep_core::t(nbeep_core::Msg::StNoPendingOffer));
            self.request_redraw(id);
            return;
        };
        let resume = if accept {
            self.load_resume(peer, &sha, size)
        } else {
            None
        };
        let resumed = resume.as_ref().map(Vec::len);
        let ok =
            self.send_xfer_decision(peer, xid, accept, nbeep_core::RejectWhy::Declined, resume);
        if accept && ok {
            self.arm_batch_approval(peer, &name, size); // 요청 단위 승인(M4-2e)
        } else if !accept {
            // 거절 = 요청(배치) 전체 결정(M4-2e · 08-20) — 잔여 라인 즉시 종결 포함.
            self.arm_batch_decline(peer, &name, size);
            self.fail_pending_xfer_lines(
                peer,
                false,
                nbeep_core::t(nbeep_core::Msg::XferDeclined).to_string(),
            );
            self.recv_batch.remove(&peer);
            self.recv_batch_sizes.remove(&peer);
            self.clear_xfer(peer);
        }
        if let Some(got) = resumed {
            self.set_status(nbeep_core::tf(
                nbeep_core::Msg::StfResumeFrom,
                &[&name, &(got as u64 * 100 / size.max(1)).to_string()],
            ));
        }
        if !accept {
            self.set_xfer_line(
                peer,
                false,
                nbeep_ui::XferLineState::Failed {
                    why: nbeep_core::t(nbeep_core::Msg::XferDeclined).into(),
                },
            );
        }
        let left = self.pending_offers.get(&peer).map_or(0, VecDeque::len);
        self.set_status(if ok {
            let head = if accept {
                nbeep_core::tf(nbeep_core::Msg::StfAcceptStart, &[&name])
            } else {
                nbeep_core::tf(nbeep_core::Msg::StfDeclineName, &[&name])
            };
            if left > 0 {
                nbeep_core::tf(nbeep_core::Msg::StfMoreOffers, &[&head, &left.to_string()])
            } else {
                head
            }
        } else {
            "세션이 끊겨 응답하지 못했습니다".into()
        });
        self.request_redraw(id);
    }

    /// 대기열 맨 앞 제안으로 승인 화면 내용을 만든다(없으면 `None`).
    fn front_offer_info(&self, peer: PeerId) -> Option<nbeep_ui::OfferInfo> {
        let secret = self.identity.wrap_secret();
        let q = self.pending_offers.get(&peer)?;
        let (_, name, size, sha) = q.front()?;
        let sender = self
            .table
            .list()
            .into_iter()
            .find(|e| e.peer == peer)
            .map_or_else(
                || peer.short(),
                |e| format!("{} ({})", e.name.as_str(), peer.short()),
            );
        // 첫 왕래 경고(08-13 확정 — 미교환 상대는 거절 대신 수동 승인 강등이므로,
        // "왜 물어보는지"를 승인 카드에서 바로 보이게 한다).
        let sender = if self.ledger.get(peer).is_mutual() {
            sender
        } else {
            format!("{sender} · ⚠ 서로 주고받은 메시지가 아직 없습니다")
        };
        // 강등 안내(08-16 실기 — "자동승인 켰는데 왜 묻지?"가 버그로 오인됐다):
        // 자동(즉시·기간)이 켜져 있는데 이 창이 떴다 = 첫 왕래 전 강등이 이유다.
        let auto_on = matches!(
            self.approval.tick(self.now_ms()).0,
            nbeep_core::ApprovalPolicy::Basic(nbeep_core::BasicApproval::Auto)
                | nbeep_core::ApprovalPolicy::TimedAuto { .. }
        );
        let downgrade_note = if auto_on && !self.ledger.get(peer).is_mutual() {
            "자동 승인 기간 중이지만 첫 왕래 전이라 확인이 필요합니다".to_string()
        } else {
            String::new()
        };
        // 이어받기 후보(M4-10c) — 보존율을 승인 창 2택으로 보여 준다.
        let resume_pct = crate::part::partial_len(crate::gate::CH_GUI, &secret, sha, *size)
            .map(|got| u8::try_from(got * 100 / (*size).max(1)).unwrap_or(99));
        // ★ 요청 단위 승인(M4-2e · 08-20 사용자 확정 재요청) — 창의 파일명·크기를
        //   **첫 파일이 아니라 요청 전체**로: manifest(비제외분)가 원료다.
        //   파일명 = ", " 구분 순수 파일명 목록(위젯이 폭 실측 줄바꿈) ·
        //   크기 = 총합 · 파일 수 줄 추가. manifest 미도착(구버전 발신)이면 단건.
        // 파일 수 = **시도 전체(제외 포함)** · 제외 목록 별도 행(08-20 확정 —
        // 상한 초과로 빠진 파일도 요청의 일부로 식별돼야 한다). 크기 총합은
        // 실제 전송분(비제외)만 — 안 오는 바이트를 합치면 거짓말이다.
        let (bname, bsize, bcount, bexcluded) = self
            .recv_manifest
            .get(&peer)
            .map(|m| {
                let inc: Vec<&(String, u64, bool)> = m.iter().filter(|e| !e.2).collect();
                let exc: Vec<&str> = m.iter().filter(|e| e.2).map(|e| e.0.as_str()).collect();
                (
                    inc.iter()
                        .map(|e| e.0.as_str())
                        .collect::<Vec<_>>()
                        .join(", "),
                    inc.iter().map(|e| e.1).sum::<u64>(),
                    m.len(),
                    exc.join(", "),
                )
            })
            .filter(|(_, _, n, _)| *n >= 1)
            .unwrap_or_else(|| (name.clone(), *size, 1, String::new()));
        Some(nbeep_ui::OfferInfo {
            sender,
            when: nbeep_plat::clock::local_hms(unix_now()).hms(),
            name: bname,
            size: bsize,
            count: bcount,
            excluded: bexcluded,
            queued: q.len(),
            downgrade_note,
            resume_pct,
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
    /// 요청 단위 승인 무장(M4-2e) — 사용자가 배치의 한 파일을 승인하면, 대기
    /// manifest의 **나머지 비제외 항목**을 자동 수락 목록으로 옮긴다(이후 오퍼는
    /// 이름+크기 대조로 무확인 수락 · 불일치 = 수동 폴백 fail-closed).
    fn arm_batch_approval(&mut self, peer: PeerId, name: &str, size: u64) {
        let Some(man) = self.recv_manifest.remove(&peer) else {
            return;
        };
        let rem = batch_remainder(man, name, size);
        if !rem.is_empty() {
            self.batch_approved.insert(peer, rem);
        }
    }

    /// 요청 단위 **거절** 무장(M4-2e · 08-20 — 승인의 대칭): 프롬프트 거절/타임아웃
    /// 1회가 배치 전체를 마감한다 — 잔여 파일의 오퍼가 오는 대로 자동 거절되고
    /// 대기 라인도 그때 종결된다(2.sna·3.hsh 실기: 파일마다 다시 묻던 구멍).
    fn arm_batch_decline(&mut self, peer: PeerId, name: &str, size: u64) {
        self.batch_approved.remove(&peer); // 결정 교체 — 두 장부 동시 무장 금지
        let Some(man) = self.recv_manifest.remove(&peer) else {
            return;
        };
        let rem = batch_remainder(man, name, size);
        if !rem.is_empty() {
            self.batch_declined.insert(peer, rem);
        }
    }

    /// 요청 단위 승인 해제(M4-2e) — 거절·종료 시 배치 전체 마감.
    fn clear_batch_approval(&mut self, peer: PeerId) {
        self.recv_manifest.remove(&peer);
        self.batch_approved.remove(&peer);
        self.batch_declined.remove(&peer);
    }

    fn run_offer_choice(&mut self, peer: PeerId, choice: nbeep_ui::OfferChoice) {
        use nbeep_ui::OfferChoice;
        let front = self
            .pending_offers
            .get_mut(&peer)
            .and_then(VecDeque::pop_front);
        let Some((xid, name, size, sha)) = front else {
            self.close_approve(peer);
            return;
        };
        match choice {
            OfferChoice::ApproveFresh => {
                // 처음부터(M4-10c) — 보존분은 의사 표시로 폐기하고 새로 받는다.
                crate::part::remove_partial(crate::gate::CH_GUI, &sha);
                self.send_xfer_decision(peer, xid, true, nbeep_core::RejectWhy::Declined, None);
                self.arm_batch_approval(peer, &name, size); // 요청 단위 승인(M4-2e)
                self.set_status(nbeep_core::tf(nbeep_core::Msg::StfAcceptRecv, &[&name]));
            }
            OfferChoice::Approve => {
                let resume = self.load_resume(peer, &sha, size);
                let resumed = resume.as_ref().map(Vec::len);
                self.send_xfer_decision(peer, xid, true, nbeep_core::RejectWhy::Declined, resume);
                self.arm_batch_approval(peer, &name, size); // 요청 단위 승인(M4-2e)
                self.set_status(if let Some(got) = resumed {
                    nbeep_core::tf(
                        nbeep_core::Msg::StfResumeFrom,
                        &[&name, &(got as u64 * 100 / size.max(1)).to_string()],
                    )
                } else {
                    nbeep_core::tf(nbeep_core::Msg::StfAcceptRecv, &[&name])
                });
            }
            OfferChoice::Cancel { by_timeout } => {
                self.send_xfer_decision(peer, xid, false, nbeep_core::RejectWhy::Declined, None);
                // 거절 = 요청(배치) 전체 결정(M4-2e · 08-20) — 잔여 오퍼 자동 거절
                // 무장(경합 안전망) + **잔여 수신 라인 즉시 종결**(발신자가 남은
                // 파일을 다시 오퍼하지 않으므로 라인을 지금 닫아야 한다).
                self.arm_batch_decline(peer, &name, size);
                self.fail_pending_xfer_lines(
                    peer,
                    false,
                    nbeep_core::t(nbeep_core::Msg::XferDeclined).to_string(),
                );
                self.recv_batch.remove(&peer);
                self.recv_batch_sizes.remove(&peer);
                self.clear_xfer(peer);
                self.set_xfer_line(
                    peer,
                    false,
                    nbeep_ui::XferLineState::Failed {
                        why: if by_timeout {
                            nbeep_core::t(nbeep_core::Msg::XferTimeoutReject).into()
                        } else {
                            nbeep_core::t(nbeep_core::Msg::XferDeclined).into()
                        },
                    },
                );
                self.set_status(if by_timeout {
                    nbeep_core::tf(
                        nbeep_core::Msg::StfTimeoutDeclined,
                        &[&self.wait_timeout_sec.to_string(), &name],
                    )
                } else {
                    nbeep_core::tf(nbeep_core::Msg::StfDeclineName, &[&name])
                });
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
                let resume = self.load_resume(peer, &sha, size);
                self.send_xfer_decision(peer, xid, true, nbeep_core::RejectWhy::Declined, resume);
                self.set_status(nbeep_core::tf(
                    nbeep_core::Msg::StfAutoAcceptStart,
                    &[&name],
                ));
            }
        }
        // 다음 제안이 있으면 이어서 묻는다(오퍼 1건당 승인 1번).
        self.refresh_approve_view(peer);
        if let Some(mid) = self.main_id {
            self.request_redraw(mid);
        }
    }

    /// `.part` 재개 후보 조회(M4-10) — 있으면 (프리픽스, 보존율%). 조회 시점에
    /// 크기 불일치·손상은 part 모듈이 폐기한다(fail-closed).
    fn load_resume(&mut self, peer: PeerId, sha: &[u8; 32], size: u64) -> Option<Vec<u8>> {
        let secret = self.identity.wrap_secret();
        let prefix = crate::part::load_partial(crate::gate::CH_GUI, &secret, sha, size)?;
        self.resumed_recv.insert(peer, *sha); // 완료·취소 시 .part 정리용
        Some(prefix)
    }

    /// 액터에 수락/거절 명령을 보낸다(성공 여부).
    fn send_xfer_decision(
        &mut self,
        peer: PeerId,
        xid: nbeep_core::XferId,
        accept: bool,
        why: nbeep_core::RejectWhy,
        resume: Option<Vec<u8>>,
    ) -> bool {
        let cmd = if accept {
            // 수신 상한 공지(M4-11 개정) — Auto는 **상한 무주장(0)**: 종전
            // target_bps 공지는 관측 없는 수신측이 하한(256KiB/s)을 주장해
            // 쌍방 협상이 하한에 고착됐다(로컬 0.25MB/s 실기). 명시 설정만 주장.
            let rate_cap = self.recv_rate.advertised_cap();
            SessionCmd::AcceptXfer {
                id: xid,
                rate_cap,
                resume,
            }
        } else {
            SessionCmd::RejectXfer { id: xid, why }
        };
        let sent = self
            .conversations
            .get(&peer)
            .is_some_and(|c| c.out_tx.send(cmd).is_ok());
        if sent && accept {
            // 수락 후 취소 UX(08-16) — 등록은 **여기 한 곳**: 수락 경로가 둘이라
            // (⌘Y answer_offer · 승인 창 run_offer_choice) 호출부에 두면 빠뜨린다
            // (실기 08-16: 승인 창 수락은 취소 버튼이 무반응이었다).
            self.active_recv.insert(peer, xid);
        }
        sent
    }

    /// 진행률 반영의 **청크 주기판**(RL-6 · 08-18) — 대화 위젯(배너·진행 바)은
    /// 즉시, **목록 재조립은 주기 스로틀**에 태운다. 종전엔 32KiB 청크마다
    /// `refresh_rows` 직행 = 발견 경로가 일부러 건 `ui.list_refresh_ms`(1500ms)
    /// 스로틀 우회(100MB 파일 = 3,200회 전 피어 순회+2중 정렬 — 최대 CPU 낭비).
    /// dirty만 세우면 주기 도래 시 발견 폴이 한 번에 반영한다(같은 깔때기).
    fn apply_xfer_view_throttled(&mut self, peer: PeerId) {
        let xp = self.xfer_progress.get(&peer).copied();
        let mut inv = Invalidations::default();
        if let Some(chat) = self.chats.get_mut(&peer) {
            chat.set_xfer(xp, &mut inv);
        }
        self.rows_dirty = true;
        if self.now_ms().saturating_sub(self.last_rows_ms) >= self.list_refresh_ms {
            self.refresh_rows(&mut inv);
        }
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
    /// 정지 방치 타이머 갱신(M4-2e ⓓ) — 정지 항목이 있으면 "최근 상태 변화
    /// 시각"을 지금으로, 없으면 타이머 해제. 전송 상태가 바뀌는 지점마다 부른다.
    fn note_xfer_state_change(&mut self, peer: PeerId) {
        let has_pause = self.paused_sends.contains_key(&peer)
            || self.send_paused.get(&peer).is_some_and(|s| !s.is_empty())
            || self.recv_paused.contains_key(&peer);
        if has_pause {
            self.xfer_pause_since.insert(peer, self.now_ms());
        } else {
            self.xfer_pause_since.remove(&peer);
        }
    }

    /// **로컬 전체취소 루틴**(M4-2e — 진행 상태 관리의 단일 기준): 발신·수신
    /// 양 역할의 배치 상태를 전부 마감한다. 와이어 통지는 하지 않는다(호출자
    /// 몫 — 내가 누른 취소는 통지 후 이걸 부르고, 상대 통지 수신은 이것만).
    fn apply_local_cancel_all(&mut self, peer: PeerId) {
        // 발신 역할 정리.
        self.awaiting_accept.remove(&peer);
        self.active_send.remove(&peer);
        self.preparing_send.remove(&peer);
        self.current_send.remove(&peer);
        self.send_queue.remove(&peer);
        self.send_batch.remove(&peer);
        self.send_excluded.remove(&peer);
        self.send_paused.remove(&peer);
        self.paused_sends.remove(&peer); // 상대가 이미 취소 — 개별 통지 불요
        self.resend_offers.remove(&peer);
        self.fail_pending_xfer_lines(
            peer,
            true,
            nbeep_core::t(nbeep_core::Msg::XferCanceled).to_string(),
        );
        // 수신 역할 정리.
        self.active_recv.remove(&peer);
        self.recv_xids.remove(&peer);
        self.recv_paused.remove(&peer);
        self.recv_batch.remove(&peer);
        self.recv_batch_sizes.remove(&peer);
        self.xfer_pause_since.remove(&peer);
        self.clear_batch_approval(peer);
        if let Some(sha) = self.resumed_recv.remove(&peer) {
            crate::part::remove_partial(crate::gate::CH_GUI, &sha);
        }
        self.fail_pending_xfer_lines(
            peer,
            false,
            nbeep_core::t(nbeep_core::Msg::XferCanceled).to_string(),
        );
        // 공통 마감.
        self.close_send_wait(peer);
        self.clear_xfer(peer);
        self.refresh_send_batch(peer);
    }

    /// 전체 취소 와이어 통지(M4-2e · Control 16) — 상대도 자기 쪽 루틴을 돌린다.
    /// 큐 단계(오퍼 전) 정지/재개를 수신측에 통지(M4-2e ①ⓐ — 이름 기반 태그 17).
    fn send_qpause_notice(&mut self, peer: PeerId, name: &str, pause: bool) {
        if let Some(c) = self.conversations.get(&peer) {
            let _ = c
                .out_tx
                .send(SessionCmd::Control(vec![nbeep_core::xfer::encode_qpause(
                    name, pause,
                )]));
        }
    }

    fn send_cancel_all_notice(&mut self, peer: PeerId) {
        if let Some(c) = self.conversations.get(&peer) {
            let _ = c.out_tx.send(SessionCmd::Control(vec![
                nbeep_core::xfer::encode_cancel_all(),
            ]));
        }
    }

    /// 미종결 전송 라인 일괄 종결(M4-2e — 전체취소 공용): FIFO(대기·활성)는
    /// 반복 갱신으로, Paused(FIFO가 건너뜀)는 직접 순회로 Failed 처리한다.
    fn fail_pending_xfer_lines(&mut self, peer: PeerId, mine: bool, why: String) {
        while self.conversations.get_mut(&peer).is_some_and(|conv| {
            nbeep_ui::update_xfer_in(
                &mut conv.lines,
                mine,
                nbeep_ui::XferLineState::Failed { why: why.clone() },
            )
        }) {}
        if let Some(conv) = self.conversations.get_mut(&peer) {
            for line in conv.lines.iter_mut() {
                if line.mine == mine {
                    if let nbeep_ui::ChatBody::Xfer(x) = &mut line.body {
                        if matches!(x.state, nbeep_ui::XferLineState::Paused { .. }) {
                            x.state = nbeep_ui::XferLineState::Failed { why: why.clone() };
                        }
                    }
                }
            }
        }
        let mut inv = Invalidations::default();
        if let Some(chat) = self.chats.get_mut(&peer) {
            while chat.update_xfer_line(
                mine,
                nbeep_ui::XferLineState::Failed { why: why.clone() },
                &mut inv,
            ) {}
            chat.fail_paused_lines(mine, &why, &mut inv);
        }
        self.record_history(peer);
        self.redraw_conversation(peer);
    }

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
        let safe = nbeep_core::sanitize_message(name);
        // ★ 재활성화 우선(M4-10c · 08-18 실기) — 끊김 후 재-Offer는 같은 전송의
        //   연속이다: 미종결 동일 항목이 있으면 **항목 하나로 유지**(승인 대기로
        //   되돌림). 종전엔 새 항목이 추가되고, 진행률 갱신(미종결 첫 항목 대상)이
        //   옛 항목에 붙어 두 항목이 서로 어긋났다.
        let mut inv = Invalidations::default();
        let re_store = self.conversations.get_mut(&peer).is_some_and(|conv| {
            nbeep_ui::reactivate_xfer_in(&mut conv.lines, mine, safe.as_str(), size)
        });
        let re_view = self
            .chats
            .get_mut(&peer)
            .is_some_and(|chat| chat.reactivate_xfer(mine, safe.as_str(), size, &mut inv));
        if re_store || re_view {
            if !mine {
                self.note_incoming(peer);
            }
            self.redraw_conversation(peer);
            return;
        }
        let line = ChatLine::xfer(mine, safe, size, at_ms, wall);
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
        // 종결(Done/Failed) 갱신이면 기록 영속(M2-5b — 진행 중은 이력 아님).
        if matches!(
            state,
            nbeep_ui::XferLineState::Done { .. } | nbeep_ui::XferLineState::Failed { .. }
        ) {
            self.record_history(peer);
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
        self.record_history(peer); // 종단 ack로 종결 = 기록 영속(M2-5b)
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
            self.set_status(nbeep_core::t(nbeep_core::Msg::StAutoRevert));
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
        self.face_base = load(self.settings.get("font.base.family"));
        self.face_peerlist = load(self.settings.get("font.peerlist.family"));
        self.face_message = load(self.settings.get("font.message.family"));
        self.face_status = load(self.settings.get("font.status.family"));
        // 고정폭: 지정이 있으면 그것, 없으면 **OS 기본 고정폭**(사용자 확정 08-09).
        self.face_mono = load(self.settings.get("font.mono.family")).or_else(|| {
            let (bytes, idx) = nbeep_plat::font::system_mono_font()?;
            nbeep_gfx::Font::from_static(bytes, idx).ok()
        });
    }

    /// 상태바 문구의 **단일 통로**(M3-22 — 산재 대입은 곧 빠뜨린 대입: 로깅 훅은
    /// 여기 한 곳뿐이다). 로그는 상태 문구만 싣는다(봉투 원리 — 이미 사용자 노출
    /// 텍스트 · 키·전문 경로가 새로 들어오지 않게 이 관문이 지킨다).
    fn set_status(&mut self, s: impl Into<String>) {
        let s = s.into();
        if let Some(l) = &self.statuslog {
            l.log(&s); // 논블로킹 — 큐 포화 = 드롭(본 기능 무영향 1급 요구)
        }
        self.status = s;
    }

    /// 로거 기동/정지(M3-22 hot-swap) — 설정 3키 변경·부팅에서 부른다.
    /// 재시작 방식(파라미터 변경 = stop→start)이 가장 단순하고, 호출 빈도가
    /// 설정 조작뿐이라 비용이 없다.
    fn refresh_statuslog(&mut self) {
        if let Some(l) = self.statuslog.take() {
            l.stop(); // flush 후 정지
        }
        if self.settings.get("log.enabled") == "on" {
            let days = self.settings.get("log.retain_days").parse().unwrap_or(7);
            let mb = self.settings.get("log.max_total_mb").parse().unwrap_or(20);
            self.statuslog =
                crate::statuslog::StatusLog::start(self.data_dir.join("logs"), "beep", days, mb);
            if self.statuslog.is_none() {
                self.set_status("⚠ 로그 폴더를 만들 수 없어 기록을 켜지 못했습니다");
            }
        }
    }

    /// 네트워크 점검 로거 재기동(netmon · 08-21) — `netmon.enabled` hot-swap.
    /// 켜지면 그 시점 누적을 기준점으로 삼아 **켠 이후의 델타**만 기록한다.
    /// 보존·총량은 로그 설정을 공유한다(같은 진단 폴더 · 프리픽스만 다르다).
    fn refresh_netmon(&mut self) {
        if let Some(l) = self.netmon_log.take() {
            l.stop(); // flush 후 정지
        }
        if self.settings.get("netmon.enabled") == "on" {
            let days = self.settings.get("log.retain_days").parse().unwrap_or(7);
            let mb = self.settings.get("log.max_total_mb").parse().unwrap_or(20);
            self.netmon_log =
                crate::statuslog::StatusLog::start(self.data_dir.join("logs"), "netmon", days, mb);
            match &self.netmon_log {
                Some(l) => {
                    self.netmon_prev = nbeep_net::netmon::snapshot();
                    self.netmon_last_sec = unix_now();
                    l.log(&format!(
                        "netmon start interval_s={}",
                        self.netmon_interval_s()
                    ));
                }
                None => {
                    self.set_status("⚠ 로그 폴더를 만들 수 없어 네트워크 점검을 켜지 못했습니다");
                }
            }
        }
    }

    /// netmon 점검 주기(초) — 설정값(하한 2·상한 3600 · 파싱 실패 = 10).
    fn netmon_interval_s(&self) -> u64 {
        self.settings
            .get("netmon.interval_s")
            .parse::<u64>()
            .map_or(10, |v| v.clamp(2, 3600))
    }

    /// netmon 주기 틱 — 켜져 있고 주기가 찼으면 요약 한 줄 기록 + 과다 경고는
    /// 상태바에도 띄운다(로그를 안 열어도 폭주를 알 수 있게).
    fn tick_netmon(&mut self) {
        if self.netmon_log.is_none() {
            return;
        }
        let now = unix_now();
        let interval = self.netmon_interval_s();
        if now.saturating_sub(self.netmon_last_sec) < interval {
            return;
        }
        let cur = nbeep_net::netmon::snapshot();
        let dt_ms = now
            .saturating_sub(self.netmon_last_sec)
            .saturating_mul(1000);
        let (line, warns) = nbeep_net::netmon::report_line(&self.netmon_prev, &cur, dt_ms);
        if let Some(l) = &self.netmon_log {
            l.log(&line);
        }
        self.netmon_prev = cur;
        self.netmon_last_sec = now;
        if !warns.is_empty() {
            self.set_status(nbeep_core::tf(
                nbeep_core::Msg::StfNetmonWarn,
                &[&warns.join(",")],
            ));
        }
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
            &nbeep_core::tf(
                nbeep_core::Msg::AutoAcceptCountdown,
                &[&start_s, &elapsed_s, &remain_s, &end_s],
            ),
            &mut inv,
        );
        // Auto 속도 실측 표시(08-16 사용자 요청) — 값이 안 바뀌면 `set_row_note`가
        // 무효화를 건너뛰므로 1초 주기에 태워도 비용이 없다(peak는 전송 완료 때만 변한다).
        sv.set_row_note(
            "xfer.send_rate",
            &auto_rate_note(self.send_rate, &self.send_meter, true),
            &mut inv,
        );
        sv.set_row_note(
            "xfer.recv_rate",
            &auto_rate_note(self.recv_rate, &self.recv_meter, false),
            &mut inv,
        );
        // ★ 잠금은 **여기 한 곳에서 전량 계산**한다 — set_disabled가 전체 교체라
        //   다른 지점에서 부분만 넣으면 1초 주기인 이 깔때기가 도로 지운다.
        let mut locked: Vec<&'static str> = Vec::new();
        if remain.is_none() {
            // 기간 자동이 아니면 기간 설정은 쓰이지 않는다 — 잠근다.
            locked.push("xfer.approval_window");
        }
        if self.settings.get("net.server.mode") != "managed" {
            // Unmanaged(LAN) = 서버 접속 정보가 쓰이지 않는다(08-18 사용자 요청).
            locked.extend(["net.server.address", "net.server.port", "net.server.type"]);
        }
        sv.set_disabled(&locked, &mut inv);
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
                    // 최근 접속 관측(08-15 — 아는 상대만 · 60초 스로틀 영속).
                    self.trust.note_seen(hint.peer, unix_now_ms());
                    // 무변화 관측(생존 비컨)은 재조립 사유가 아니다(RL-16ⓑ ·
                    // 08-18) — observe의 None을 버리고 무조건 dirty를 세우면
                    // 상대가 광고하는 한 1.5초마다 목록 재조립·재도색이 영구다.
                    if self
                        .table
                        .observe(hint.peer, hint.name, nbeep_core::SourceId(0), now)
                        .is_some()
                    {
                        changed = true;
                    }
                    // ★ 오프라인 대기 상대가 나타났다(M4-6) — 즉시 연결 후보로.
                    //   observe의 무변화 스로틀(RL-16ⓑ)과 무관하게 여기서 건다:
                    //   대기분이 있는데 세션이 없으면 백오프를 "지금"으로 당긴다
                    //   (실제 시도는 reconnect 틱이 — 중복 시도는 ConnectLatch가 막는다).
                    if self.pending_direct.contains_key(&hint.peer)
                        && !self.conversations.contains_key(&hint.peer)
                    {
                        // or_insert — 진행 중 표식(u64::MAX)·기존 백오프를 덮지 않는다
                        // (Appeared는 비컨마다 온다 — 덮으면 1.5초마다 재시도 폭주).
                        self.reconnect.entry(hint.peer).or_insert((0, 0));
                    }
                }
                DiscoveryEvent::Vanished(peer) => {
                    if self.table.goodbye(peer, nbeep_core::SourceId(0)).is_some() {
                        changed = true;
                    }
                }
            }
        }
        // 무응답 만료(RL-5 · 08-18) — 정의만 있고 배선이 없어 goodbye 없이 죽은
        // 상대가 영원히 "발견됨"이었다(→ `ConnectFailed`의 reachable 영구 참 =
        // 무의미 재연결 사다리 반복). 60초(비컨 800ms의 75배) 무관측 = 이탈.
        // 항목 수 = LAN 피어 수라 5Hz 순회는 실질 0비용.
        for ev in self.table.sweep(now) {
            changed = true;
            let nbeep_core::PeerEvent::Departed(p, _) = ev else {
                continue;
            };
            // 아는 상대(핀·대화·수동 주소)는 **오프라인 행으로 강등해 유지** —
            // 발견에서 빠졌다고 목록에서 증발하면 "비발견 상대 목록 유지"(08-13
            // 사용자 확정)가 깨진다. 모르는 상대(스쳐간 발견)만 행이 사라진다.
            use nbeep_core::TrustStore as _;
            let known = self.trust.level(p) != nbeep_core::TrustLevel::Unverified
                || self.conversations.contains_key(&p)
                || self.manual_addrs.contains_key(&p);
            if known {
                let name = self
                    .peer_profiles
                    .get(&p)
                    .and_then(|pr| pr.name.clone())
                    .unwrap_or_else(|| nbeep_core::default_display_name(None, &p));
                self.extra_peers.entry(p).or_insert(name);
            }
        }
        if changed {
            // 즉시 재조립하지 않는다(08-14 사용자 실기 — 800ms 비컨마다 목록이
            // 재조립되며 스크롤이 튀었다). 주기 도래 시 아래에서 한 번에 반영.
            self.rows_dirty = true;
        }
        // 60초 심장박동(RL-16ⓑ의 짝) — 무변화 생략으로 dirty가 안 서도 상대
        // 시각 라벨("방금/N분")은 분 단위로 늙는다. 느린 강제 재조립 한 번이
        // 라벨을 신선하게 유지한다(비용 = 분당 1회 — 종전 1.5초마다의 1/40).
        let since = self.now_ms().saturating_sub(self.last_rows_ms);
        if (self.rows_dirty && since >= self.list_refresh_ms) || since >= 60_000 {
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
        // 어떤 경로로든 재조립했으면 발견발 대기분도 함께 소화된 것(주기 스로틀 기준점).
        self.rows_dirty = false;
        self.last_rows_ms = self.now_ms();
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
        // 정렬(08-15 사용자 확정) — 고정(★) 구획이 먼저, 각 구획은 설정 모드
        // (`ui.list_sort` — 최근 접속/최근 대화/접속 우선/이름)의 속성 사슬로.
        let mode = self.settings.get("ui.list_sort").to_string();
        entries.sort_by(|a, b| {
            let key = |e: &nbeep_core::PeerEntry| {
                // 접속 계층 — 세션 중(0) > 발견됨(1) > 오프라인(2).
                let tier = if self.conversations.contains_key(&e.peer) {
                    0
                } else if self.table.get(e.peer).is_some() {
                    1
                } else {
                    2
                };
                let (seen, chat) = self.trust.meta(e.peer);
                peer_order_key(&mode, self.trust.fav(e.peer), tier, seen, chat)
            };
            key(a)
                .cmp(&key(b))
                .then_with(|| a.name.as_str().cmp(b.name.as_str()))
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
                // 소개글(08-17) — 목록 2번째 줄(줄바꿈은 위젯이 접는다).
                let bio = self
                    .peer_profiles
                    .get(&entry.peer)
                    .and_then(|p| p.bio.clone());
                let avatar = self
                    .peer_profiles
                    .get(&entry.peer)
                    .and_then(|p| p.avatar.clone());
                let border = self.peer_profiles.get(&entry.peer).and_then(|p| p.border);
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
                let fav = self.trust.fav(entry.peer);
                // 신뢰 배지 확장(M3-14) — 차단·이름 충돌은 도메인엔 있었는데
                // 화면에 안 나오던 상태다(충돌 = v1 사칭 유일 가시 신호).
                let blocked = self.trust.is_blocked(entry.peer);
                let conflict = self.trust.name_conflict(entry.peer, &entry.name).is_some();
                // 최근 접속 상대 시각(08-17 — 삭제 메뉴가 시각만 표시).
                let last_seen_label = ago_label(self.trust.meta(entry.peer).0);
                PeerRow {
                    entry,
                    trust,
                    link,
                    xfer,
                    profile_name,
                    bio,
                    avatar,
                    border,
                    unread,
                    last_read,
                    fav,
                    blocked,
                    conflict,
                    last_seen_label,
                }
            })
            .collect();
        self.list.set_rows(rows, inv);
        // 그룹 섹션(M5-1g) — **공유 그룹만** 노출(동보 그룹은 숨김 · 사용자 확정 08-13).
        // ★ Invited(수락 전)도 표시한다(08-19 개정 — 종전엔 숨겨 재시작 후 초대
        //   카드가 사라지면 수락할 길이 없었다). "초대됨" 표식 + 클릭 = 수락 모달.
        let me = self.identity.peer_id();
        let grows: Vec<nbeep_ui::GroupRow> = self
            .groups
            .shared_list()
            .iter()
            .map(|s| nbeep_ui::GroupRow {
                id: s.local_id,
                name: if s.mine == nbeep_store::MineState::Invited {
                    format!(
                        "{} · {}",
                        s.roster.name.as_str(),
                        nbeep_core::t(nbeep_core::Msg::GroupInvitedTag)
                    )
                } else {
                    s.roster.name.as_str().to_string()
                },
                members: u32::try_from(s.roster.members.len()).unwrap_or(u32::MAX),
                online: u32::try_from(
                    s.roster
                        .members
                        .iter()
                        .filter(|p| **p == me || self.conversations.contains_key(p))
                        .count(),
                )
                .unwrap_or(u32::MAX),
                unread: self.gunread.get(&s.local_id).copied().unwrap_or(0),
                fav: s.pinned,
                owned: s.roster.owner == me,
                member_invite: s.roster.member_invite,
            })
            .collect();
        let mut grows = grows;
        // 그룹도 고정 먼저(08-15) — 각 구획은 이름순.
        grows.sort_by(|a, b| (!a.fav, &a.name).cmp(&(!b.fav, &b.name)));
        self.list.set_groups(grows, inv);
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
    ///
    /// `auto` = **조용한 연결**: 성립해도 대화 뷰를 열지 않는다(자동 재연결 ⓑ ·
    /// 프로필 pull 08-14 — 카드와 대화는 분리). `false` = 사용자가 대화를 열려는
    /// 연결(성립 시 `activate`가 뷰를 연다).
    /// Managed 서버 접속 틱(X-2b) — `about_to_wait`에서 2초 페이스로 돈다.
    /// 목표(설정) 대비 현 상태를 수렴시키는 **단일 경로**: 붙기·재접속은 여기,
    /// 목표 변경은 [`Self::server_settings_changed`]가 상태를 지워 여기로 모은다.
    /// (13 §12-1 — 중복 가드 `relay_connecting` · 종료 조건 = Unmanaged · 실패
    ///  복귀 = 백오프 · 수동 우선 = 설정 변경 즉시 · 포커스 안 뺏음 = 상태바만.)
    fn server_tick(&mut self) {
        if !self.live {
            return; // 데모 전송(InMemory) — 서버 축 없음
        }
        let now = self.now_ms();
        if now < self.relay_check_at {
            return;
        }
        self.relay_check_at = now + 2_000;
        let want = server_target(
            self.settings.get("net.server.mode"),
            self.settings.get("net.server.address"),
            self.settings.get("net.server.port"),
        );
        let Some(raw) = want else {
            if self.relay.take().is_some() {
                self.set_status(nbeep_core::t(nbeep_core::Msg::StServerDetached));
            }
            return;
        };
        if self.relay_connecting {
            return;
        }
        if let Some(c) = &self.relay {
            if c.is_alive() {
                return; // 붙어 있고 살아 있다 — 할 일 없음
            }
            // 서버 세션 사망 관측 — 내려놓고 첫 단계부터 재시도(백오프는 실패가 올린다).
            self.relay = None;
            self.relay_backoff = (0, now);
            self.set_status(nbeep_core::t(nbeep_core::Msg::StServerLost));
        }
        if now < self.relay_backoff.1 {
            return;
        }
        self.relay_connecting = true;
        let gen = self.relay_gen;
        let identity = std::sync::Arc::clone(&self.identity);
        let pin = self.data_dir.join("server.pin");
        let proxy = self.proxy.clone();
        // 접속·Noise·DNS는 워커에서(수 초 블로킹 — UI 불가침). 결과는 이벤트로.
        std::thread::spawn(move || {
            let outcome = match nbeep_relay::attach(&raw, &identity, &pin) {
                Ok(at) => Ok(Box::new(at)),
                Err(nbeep_relay::AttachError::Resolve) => Err(ServerAttachFail::Resolve),
                Err(nbeep_relay::AttachError::Relay(nbeep_relay::RelayError::PinMismatch {
                    ..
                })) => Err(ServerAttachFail::PinMismatch),
                Err(nbeep_relay::AttachError::Relay(e)) => {
                    Err(ServerAttachFail::Other(format!("{e:?}")))
                }
            };
            let _ = proxy.send_event(AppEvent::ServerAttach { gen, outcome });
        });
    }

    /// `net.server.*` 변경(설정 hot-swap) — 현 접속을 내려놓고 다음 틱이 새 목표로
    /// 붙는다(핀 불일치 정지도 여기서 풀린다 — "설정을 다시 저장하면 재접속").
    fn server_settings_changed(&mut self) {
        self.relay_gen = self.relay_gen.wrapping_add(1);
        self.relay = None;
        self.relay_backoff = (0, 0);
        self.relay_check_at = 0;
    }

    fn start_connect(&mut self, peer: PeerId, auto: bool) {
        if !self.connecting.begin(peer) {
            if !auto {
                self.set_status(nbeep_core::tf(
                    nbeep_core::Msg::StfConnectingBusy,
                    &[&self.peer_title(peer)],
                ));
            }
            return; // 중복 시도 가드(수동 클릭·자동 재연결 공용 — 워커는 하나만)
        }
        self.set_status(if auto {
            format!("자동 재연결 시도… {}", self.peer_title(peer))
        } else {
            format!("연결 중… {}", self.peer_title(peer))
        });
        // 목록 행 점을 즉시 "연결 중"(강조색)으로(M2-8 잔여).
        let mut inv = Invalidations::default();
        self.refresh_rows(&mut inv);
        self.refresh_chat_link(peer); // 헤더 아이콘 = 연결 중(M3-20)
        let transport = std::sync::Arc::clone(&self.transport);
        let identity = std::sync::Arc::clone(&self.identity);
        let proxy = self.proxy.clone();
        // 비발견 상대(수동 등록 이력)는 발견 주소록이 비어 connect가 실패한다 —
        // 성공했던 수동 주소로 폴백(08-13 실기: 대화창 닫은 뒤 재연결이 이 경로).
        let manual = self.manual_addrs.get(&peer).cloned();
        // 서버 사다리 폴백 재료(X-2b ③) — LAN·수동 주소가 모두 닿지 않을 때만 쓴다
        // (S-1 LAN 우선 · docs/32 §0 경로 사다리).
        let relay = self.relay.clone();
        std::thread::spawn(move || {
            let conn = match transport.connect(peer) {
                Ok(link) => Ok(link),
                Err(e) => match &manual {
                    Some(addr) => transport.add_endpoint(addr).map_err(|e2| format!("{e2:?}")),
                    None => Err(format!("{e:?}")),
                },
            };
            let r = match conn {
                Ok(link) => {
                    // 경로 등급 = 성립 소켓의 실주소(M5-3c) — 수동 폴백이면 공인망일 수 있다.
                    let path = link
                        .remote_ip()
                        .map_or(nbeep_core::PathClass::Local, nbeep_core::class_of_ip);
                    nbeep_crypto::NoiseSession::initiate(link, &identity)
                        .map(|s| (s, path))
                        .map_err(|e| e.to_string())
                }
                // ★ 서버 사다리(X-2b ③ — 펀치→릴레이): 발견·수동이 다 실패한 상대를
                //   Managed 서버 랑데부로 시도. 성립 세션은 상대 키 인증 완료 상태고
                //   경로 등급도 사다리 결과(성립 실주소)가 들고 온다(§5-1-5).
                Err(why) => match relay.filter(|c| c.is_alive()) {
                    Some(client) => nbeep_relay::connect_via(
                        &client,
                        &identity,
                        &peer,
                        true,
                        std::time::Duration::from_secs(10),
                    )
                    .map(|via| (via.session, via.path))
                    .map_err(|e| format!("{why} · 서버 사다리 {e:?}")),
                    None => Err(why),
                },
            };
            let _ = match r {
                Ok((session, path)) => proxy.send_event(AppEvent::Outbound {
                    session: Box::new(InboundSession { session, path }),
                    via_addr: None, // 수동 주소는 이미 기억돼 있다(성공 시 갱신 불요)
                    intent: Some(peer), // ★ 래치는 **넣은 키로** 뺀다
                    auto,
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
        // 64자리 지문 = 서버 랑데부 대상(X-2b ③ — CLI `/connect <지문>` 미러):
        // 주소가 아니라 **키**로 상대를 찾는다(사다리 = 펀치→릴레이 · WrongPeer =
        // 암호학적 근거 fail-closed). Managed 서버가 붙어 있어야 성립한다.
        if let Some(peer) = nbeep_relay::parse_peer_hex(&addr) {
            let Some(client) = self.relay.clone().filter(|c| c.is_alive()) else {
                self.set_status(nbeep_core::t(nbeep_core::Msg::StFingerprintNeedsServer));
                if let Some(mid) = self.main_id {
                    self.request_redraw(mid);
                }
                return;
            };
            self.set_status(nbeep_core::tf(nbeep_core::Msg::StfConnecting, &[&addr]));
            let identity = std::sync::Arc::clone(&self.identity);
            let proxy = self.proxy.clone();
            std::thread::spawn(move || {
                let r = nbeep_relay::connect_via(
                    &client,
                    &identity,
                    &peer,
                    true,
                    std::time::Duration::from_secs(10),
                );
                let _ = match r {
                    Ok(via) => proxy.send_event(AppEvent::Outbound {
                        session: Box::new(InboundSession {
                            session: via.session,
                            path: via.path,
                        }),
                        via_addr: None, // 주소가 아니라 키로 찾았다 — 재연결도 사다리
                        intent: None,   // 수동 등록과 같은 결(래치 없음)
                        auto: false,
                    }),
                    Err(e) => proxy.send_event(AppEvent::AddFailed {
                        addr,
                        why: format!("{e:?}"),
                    }),
                };
            });
            if let Some(mid) = self.main_id {
                self.request_redraw(mid);
            }
            return;
        }
        self.set_status(nbeep_core::tf(nbeep_core::Msg::StfConnecting, &[&addr]));
        let transport = std::sync::Arc::clone(&self.transport);
        let identity = std::sync::Arc::clone(&self.identity);
        let proxy = self.proxy.clone();
        std::thread::spawn(move || {
            let r = transport
                .add_endpoint(&addr)
                .map_err(|e| format!("{e:?}"))
                .and_then(|link| {
                    // 수동 등록 = 원격일 확률이 높은 경로 — 등급은 실주소가 정한다(M5-3c).
                    let path = link
                        .remote_ip()
                        .map_or(nbeep_core::PathClass::Local, nbeep_core::class_of_ip);
                    nbeep_crypto::NoiseSession::initiate(link, &identity)
                        .map(|s| (s, path))
                        .map_err(|e| e.to_string())
                });
            let _ = match r {
                Ok((session, path)) => proxy.send_event(AppEvent::Outbound {
                    session: Box::new(InboundSession { session, path }),
                    via_addr: Some(addr), // 성공한 수동 주소 — 재연결용 기억(④)
                    intent: None,         // 수동 등록은 래치를 쓰지 않는다
                    auto: false,          // 사용자가 직접 입력한 연결
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
        // 그룹 방 입력(M5-1g — 08-13 전수 검사: 이 분기가 없어 ⌘C가 무반응이었다).
        if let Some(t) = self
            .group_chat_for(id)
            .and_then(|g| self.gchats.get(&g))
            .and_then(ChatViewWidget::copy_selection)
        {
            return Some(t);
        }
        match self.windows.get(&id).map(|e| e.role)? {
            Role::AddEndpoint => self.addr_view.as_ref()?.clipboard_copy(),
            Role::Profile => self.profile_view.as_ref()?.clipboard_copy(),
            Role::Settings => self.settings_view.as_ref()?.clipboard_copy(),
            Role::NamePrompt => self.name_prompt.as_ref()?.clipboard_copy(),
            Role::Convbox => self.convbox_view.as_ref()?.clipboard_copy(),
            Role::Gallery => self.gallery_view.as_ref()?.clipboard_copy(),
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
        if let Some(t) = self
            .group_chat_for(id)
            .and_then(|g| self.gchats.get_mut(&g))
            .and_then(|c| c.cut_selection(&mut inv))
        {
            return Some(t);
        }
        match self.windows.get(&id).map(|e| e.role)? {
            Role::AddEndpoint => self.addr_view.as_mut()?.clipboard_cut(&mut inv),
            Role::Profile => self.profile_view.as_mut()?.clipboard_cut(&mut inv),
            Role::Settings => self.settings_view.as_mut()?.clipboard_cut(&mut inv),
            Role::NamePrompt => self.name_prompt.as_mut()?.clipboard_cut(&mut inv),
            Role::Convbox => self.convbox_view.as_mut()?.clipboard_cut(&mut inv),
            Role::Gallery => self.gallery_view.as_mut()?.clipboard_cut(&mut inv),
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
        if let Some(gid) = self.group_chat_for(id) {
            if let Some(c) = self.gchats.get_mut(&gid) {
                c.paste(text, &mut inv);
                return;
            }
        }
        match self.windows.get(&id).map(|e| e.role) {
            Some(Role::NamePrompt) => {
                if let Some(v) = self.name_prompt.as_mut() {
                    v.clipboard_paste(text, &mut inv);
                }
            }
            Some(Role::Convbox) => {
                if let Some(v) = self.convbox_view.as_mut() {
                    v.clipboard_paste(text, &mut inv);
                }
            }
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
            Some(Role::Gallery) => {
                if let Some(v) = self.gallery_view.as_mut() {
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
    /// 읽음 확인 되쏘기(N-2 · 수신자가 대화창에서 봤을 때). `chat.send_read`(기본
    /// on · 수신자 제어) AND 검증 상대일 때만(프라이버시 게이트 · 전달과 **독립**).
    fn send_read_ack(&self, peer: PeerId) {
        use nbeep_core::TrustStore as _;
        if self.settings.get("chat.send_read") != "on" {
            return;
        }
        if self.trust.level(peer) == nbeep_core::TrustLevel::Unverified {
            return;
        }
        let Some(&seq) = self.last_recv_seq.get(&peer) else {
            return;
        };
        if seq == 0 {
            return;
        }
        if let Some(conv) = self.conversations.get(&peer) {
            let ack = nbeep_core::ChatAck {
                target_seq: seq,
                kind: nbeep_core::AckKind::Read,
            };
            let _ = conv.out_tx.send(SessionCmd::Control(vec![ack.encode()]));
        }
    }

    fn note_incoming(&mut self, peer: PeerId) {
        if self.chat_visible(peer) {
            let (_, wall) = now_stamp();
            self.last_read.insert(peer, wall);
            self.send_read_ack(peer); // 보이는 중 도착 = 즉시 읽음(N-2)
            return;
        }
        let n = {
            let e = self.unread.entry(peer).or_insert(0);
            *e = e.saturating_add(1);
            *e
        };
        self.set_status(nbeep_core::tf(
            nbeep_core::Msg::StfNewMsgUnread,
            &[&self.peer_title(peer), &n.to_string()],
        ));
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
        let total: u32 = self.unread.values().sum::<u32>() + self.gunread.values().sum::<u32>();
        if total > 0 {
            e.window.set_title(&format!(
                "Nexa Beep — {}",
                nbeep_core::tf(nbeep_core::Msg::WinNewMessages, &[&total.to_string()])
            ));
        } else {
            e.window.set_title("Nexa Beep");
        }
    }

    /// 내 프로필 응답 프레임(M3-17) — **공개 정책 판단은 여기 한 곳**(메인 스레드).
    /// 켠 필드만 싣고, 이미지는 기본정보 공개가 켜져 있고 상한 이내일 때만 청크로 잇는다.
    /// 전체 프로필 프레임(Info + 사진 청크) — 성립 프리페치 응답·Request 응답·
    /// 사진에 영향 주는 변경(M3-21 [`ProfileScope::Full`])이 쓴다.
    fn my_profile_frames(&self) -> Vec<Vec<u8>> {
        self.my_profile_frames_scoped(ProfileScope::Full)
    }

    /// scope별 프로필 프레임(M3-21 — "되도록 묶어 한 번에, 필요하면 유형 분화").
    /// [`ProfileScope::Info`]는 Info 한 프레임만 — 공유 사진이 있으면 `image_keep`
    /// 마커로 "네 캐시 유지"를 알린다(256KiB 사진을 텍스트 변경마다 재전송하지 않게).
    fn my_profile_frames_scoped(&self, scope: ProfileScope) -> Vec<Vec<u8>> {
        use nbeep_core::{ProfileMsg, PROFILE_IMAGE_CHUNK, PROFILE_IMAGE_MAX};
        let on = |k: &str| self.settings.get(k) == "on";
        let share_basic = on("profile.share.basic");
        let name = share_basic.then(|| {
            effective_display_name(&self.settings, &self.identity.peer_id())
                .as_str()
                .to_string()
        });
        // 소개글(08-17) — 기본정보 공개 시 · 빈 값은 안 싣는다(name과 같은 결).
        let bio = share_basic
            .then(|| self.settings.get("profile.bio").to_string())
            .filter(|s| !s.is_empty());
        let email = (on("profile.share.email"))
            .then(|| self.settings.get("profile.email").to_string())
            .filter(|s| !s.is_empty());
        let phone = (on("profile.share.phone"))
            .then(|| self.settings.get("profile.phone").to_string())
            .filter(|s| !s.is_empty());
        // 이미지 — 기본정보 공개 + 파일 존재 + 상한 이내. ★ 원본이 상한(256KiB)을
        // 넘으면 imgdec가 구운 **와이어 축소본**(me.wire.png)이 대신 나간다(08-16 —
        // 종전엔 초과가 조용히 생략돼 "본인은 사진, 상대는 옛 내장 그림"이 됐다).
        // Info scope는 바이트를 읽지 않는다(존재·크기만 — image_keep 판정용 · M3-21).
        let image_path = share_basic
            .then(|| self.settings.get("profile.image_path").to_string())
            .filter(|p| !p.is_empty());
        let wire_file = self.wire_avatar_path();
        let eff_path = image_path.as_ref().map(|orig| {
            if wire_file.is_file() {
                wire_file.to_string_lossy().into_owned()
            } else {
                orig.clone()
            }
        });
        let image = (scope == ProfileScope::Full)
            .then(|| eff_path.as_ref().and_then(|p| std::fs::read(p).ok()))
            .flatten()
            .filter(|b| !b.is_empty() && b.len() <= PROFILE_IMAGE_MAX);
        let image_shared = match scope {
            ProfileScope::Full => image.is_some(),
            ProfileScope::Info => eff_path
                .as_ref()
                .and_then(|p| std::fs::metadata(p).ok())
                .is_some_and(|m| {
                    let len = usize::try_from(m.len()).unwrap_or(usize::MAX);
                    len > 0 && len <= PROFILE_IMAGE_MAX
                }),
        };
        let image_len = image.as_ref().map_or(0u32, |b| {
            u32::try_from(b.len()).unwrap_or(0) // MAX 256KiB라 실패 불가
        });
        // 내장 아바타 키 — ★ 사진과 **동시에** 싣는다(08-16 확정: 두 필드 동시
        // 송수신). 종전 `!image_shared` 조건은 사진이 어떤 이유로든 빠지는 순간
        // (상한 초과·읽기 실패) 옛 내장 키를 광고하는 통로였다. 수신측은 사진
        // 우선·키는 폴백으로 쓴다(PeerProfile 핸들러의 짝 규칙).
        let avatar = share_basic
            .then(|| {
                nbeep_core::avatar::AvatarChoice::parse(self.settings.get("profile.avatar"))
                    .builtin_key()
                    .map(str::to_string)
            })
            .flatten();
        // 보더 색(08-14) — 기본정보 공개 시 유효 값만(어느 얼굴이든 링은 함께 간다).
        let border = share_basic
            .then(|| self.settings.get("profile.avatar_border").to_string())
            .filter(|s| nbeep_core::avatar::parse_border(s).is_some());
        let mut frames = vec![ProfileMsg::Info {
            name,
            email,
            phone,
            image_len,
            avatar,
            border,
            // Info scope + 공유 사진 존재 = "네 캐시 유지"(사진 재전송 생략 · M3-21).
            // Full은 언제나 false — 사진은 청크로 실려 가거나(존재) 철회다(부재).
            image_keep: scope == ProfileScope::Info && image_shared,
            bio,
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
    /// 인바운드 세션 수락 합류점(§6 · M5-3b) — 즉시 수락(LAN·아는 상대)과 요청 대기
    /// 승인 둘 다 여기로 온다: TOFU 판정 → 다중화 → 대화 등록 → 대기 발신 flush.
    fn accept_inbound(
        &mut self,
        session: nbeep_crypto::NoiseSession<Box<dyn nbeep_core::Link>>,
        path: nbeep_core::PathClass,
    ) {
        use nbeep_core::Session as _;
        let peer = session.peer();
        let est = match nbeep_core::TrustedSession::wrap(session, &mut self.trust) {
            Ok(est) => est,
            Err(_) => return, // 차단 상대 등 — fail-closed
        };
        self.install_conversation(nbeep_core::MuxSession::new(est.session), path);
        let title = self.peer_title(peer);
        let mut inv = Invalidations::default();
        self.refresh_rows(&mut inv);
        // ② 자동 열림 금지(사용자 확정 08-13) — 인바운드는 창을 뺏지 않는다.
        // 목록에 행이 뜨고(비발견 상대는 extra_peers ④), 메시지가 오면
        // 배지·제목 카운트(③)로 알린다. 여는 것은 언제나 사용자.
        // ★ 재동기는 인바운드에서도(G4) — "접속하면 발신자가 밀어준다"의
        // 세션 성립에는 상대가 나에게 걸어온 경우도 포함된다(종전엔 Outbound만).
        self.flush_group_sends(peer);
        self.flush_direct_sends(peer); // 1:1 오프라인 대기(M4-6)
        self.refresh_chat_link(peer); // 헤더 아이콘 = 연결됨(M3-20 — 인바운드도)
        self.set_status(nbeep_core::tf(nbeep_core::Msg::StfConnectedOpen, &[&title]));
        if let Some(mid) = self.main_id {
            self.request_redraw(mid);
        }
    }

    fn install_conversation(&mut self, session: LiveSession, path: nbeep_core::PathClass) {
        let peer = session.peer();
        let (out_tx, out_rx) = std::sync::mpsc::channel();
        let join = spawn_session_actor(
            session,
            out_rx,
            self.proxy.clone(),
            self.send_rate,
            self.identity.wrap_secret(),
            cap_from_setting(self.settings.get("xfer.recv_max_mb")).unwrap_or(u64::MAX),
        );
        self.actor_joins.push(join);
        // 프로필 자동 프리페치(M3-17 · ADR-0008) — 세션이 섰으니 요청 1회.
        // 상대가 전부 비공개면 빈 응답이 온다(그래도 요청은 무해).
        let _ = out_tx.send(SessionCmd::Control(vec![
            nbeep_core::ProfileMsg::Request.encode()
        ]));
        // ★ 스레드 보존(DR-26 · 08-13 실기) — 끊겼다 재수립돼도 대화 기록은 이어진다.
        //   대피소(parked) 우선, 살아 있는 기존 대화를 갈아끼우는 경우(경합)도 이력 유지.
        let lines = self
            .parked_lines
            .remove(&peer)
            .or_else(|| self.conversations.remove(&peer).map(|c| c.lines))
            .unwrap_or_default();
        self.conversations.insert(
            peer,
            Conversation {
                out_tx,
                lines,
                path,
            },
        );
        // ★ 원격 경로 고지(M5-3b — 조용히, 그러나 보이게): 인터넷 경유 세션은 스레드에
        //   1줄 남긴다. 지문 대조 전엔 파일이 막히는 이유가 여기서 설명된다(§5-1-3).
        if path == nbeep_core::PathClass::Remote {
            self.push_peer_note(peer, nbeep_core::t(nbeep_core::Msg::NoticeRemotePath));
        }
        // 최근 접속(08-15) — 세션 성립도 접속 관측이다(발견 없는 수동 등록 포함).
        self.trust.note_seen(peer, unix_now_ms());
        // ★ 새 세션 = 새 seq 공간(08-13 실기 — 상대가 재시작·재대화로 seq를 처음부터
        //   다시 발급하면 옛 기억이 새 메시지를 조용히 폐기했다). 옛 세션 재생은 Noise
        //   키가 원천 차단하므로 세션 경계 리셋은 안전하다(nbeep-core reset_device 문서).
        self.dedup.reset_device(peer);
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

    /// 이 상대의 **경로 등급**(M5-3c) — 살아 있는 세션의 성립 경로. 세션이 없으면
    /// Local(기본값) — 파일 정책 판정은 어차피 세션이 있어야 도달한다.
    fn peer_path(&self, peer: PeerId) -> nbeep_core::PathClass {
        self.conversations
            .get(&peer)
            .map_or(nbeep_core::PathClass::Local, |c| c.path)
    }

    /// 원격 × 미대조 = 파일 금지(M5-3b · ADR-0006 §5-1-3 — FR-S-24). 정책 판정
    /// **단일 지점** — 발신·수신 게이트가 같은 함수를 부른다(두 벌 금지). 곱 행렬
    /// 자체는 core([`nbeep_core::file_allowed`])에 있고 여기는 재료만 모은다.
    fn remote_file_blocked(&self, peer: PeerId) -> bool {
        use nbeep_core::TrustStore as _;
        !nbeep_core::file_allowed(self.peer_path(peer), self.trust.level(peer))
    }

    /// 대화 뷰 생성(스레드 복원 — 상태-뷰 분리).
    /// 이 상대의 세션 연결 상태(M3-20 — 헤더 아이콘의 단일 산출점).
    /// 우선순위 = 살아 있는 세션 > 연결 시도 중 > 끊김 관측 > 유휴.
    fn peer_link_state(&self, peer: PeerId) -> nbeep_ui::LinkState {
        if self.conversations.contains_key(&peer) {
            nbeep_ui::LinkState::Active
        } else if self.connecting.contains(peer) {
            nbeep_ui::LinkState::Connecting
        } else if self.closed_peers.contains(&peer) {
            nbeep_ui::LinkState::Lost
        } else {
            nbeep_ui::LinkState::Idle
        }
    }

    /// 열린 1:1 대화 뷰의 헤더 연결 아이콘 갱신(M3-20) — 세션 이벤트 합류점들이
    /// 부른다(성립·끊김·시도·실패). 뷰가 없으면 no-op.
    fn refresh_chat_link(&mut self, peer: PeerId) {
        let link = self.peer_link_state(peer);
        let remote = self.peer_path(peer) == nbeep_core::PathClass::Remote;
        let mut inv = Invalidations::default();
        if let Some(chat) = self.chats.get_mut(&peer) {
            chat.set_link(link, &mut inv);
            chat.set_remote(remote, &mut inv); // 인터넷 경유 상시 배지(M5-3c)
            self.redraw_conversation(peer);
        }
    }

    fn build_chat_view(&self, peer: PeerId) -> ChatViewWidget {
        let mut chat = ChatViewWidget::new(self.peer_title(peer));
        let mut inv = Invalidations::default();
        chat.set_link(self.peer_link_state(peer), &mut inv); // 헤더 아이콘 초기값(M3-20)
        chat.set_remote(
            self.peer_path(peer) == nbeep_core::PathClass::Remote,
            &mut inv,
        ); // 인터넷 경유 상시 배지 초기값(M5-3c)
           // 시각 표시 형식(설정 — 08-10).
        chat.set_time_format(
            self.settings.get("chat.time_24h") != "off",
            self.settings.get("chat.date_format") == "short",
            &mut inv,
        );
        // 세션 있으면 conv, 없으면 대피/복원(parked) — 재시작 후 열어도 뜬다(M2-5b).
        let lines = self
            .conversations
            .get(&peer)
            .map(|c| &c.lines)
            .or_else(|| self.parked_lines.get(&peer));
        if let Some(lines) = lines {
            for line in lines {
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
        // 대화 창은 모달이 아니다 — 메인에 종속시키지 않는다(자유로운 독립 창).
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
    /// 썸네일은 **캐시만 본다**(&mut = 미보유분 워커 요청·선점 기록) — 그전엔 행마다
    /// imgdec 자식 프로세스를 동기로 돌려 이미지 N개면 열기가 N배 얼었다(08-13).
    /// 격리함 재스캔을 **워커로** 요청(08-18 — 실기: 1.5GB 항목 하나에 창이
    /// "응답 없음": 종전엔 메인이 항목마다 전량 읽기+개봉+이중 읽기+lazy 재봉인
    /// 쓰기까지 했다). 완료 = `QuarantineScanned`(gen 대조 · 원시 캐시 갱신).
    fn spawn_quarantine_scan(&mut self) {
        self.qscan_gen = self.qscan_gen.wrapping_add(1);
        let gen = self.qscan_gen;
        let proxy = self.proxy.clone();
        let secret = self.identity.wrap_secret();
        std::thread::spawn(move || {
            use nbeep_safe::{Beepq, QuarantineDir};
            let rows = (|| -> Vec<QRowRaw> {
                let Ok(dir) =
                    QuarantineDir::open(crate::gate::quarantine_root(crate::gate::CH_GUI))
                else {
                    return Vec::new();
                };
                let Ok(paths) = dir.list() else {
                    return Vec::new();
                };
                paths
                    .into_iter()
                    .filter_map(|p| {
                        // ★ 사이드카 우선(08-18) — 있으면 페이로드(512MB)를 **안 읽는다**.
                        //   목록이 즉시 뜬다. 무결성 검증은 별도 백그라운드(verified=false).
                        if let Some(m) = crate::gate::read_qmeta(&p, &secret) {
                            return Some(QRowRaw {
                                path: p.to_string_lossy().into_owned(),
                                name: m.name,
                                size: m.size,
                                risk: m.risk,
                                mismatch: m.mismatch,
                                sender: m.sender,
                                received_at: m.received_at,
                                scan: m.scan,
                                archive_viol: m.archive_viol,
                                verified: false,
                            });
                        }
                        // 폴백 — 사이드카 없는 구본: 한 번 읽어 메타를 얻고(개봉·이관)
                        //   **lazy 사이드카 생성**(다음 스캔부터 즉시). 이 경로만 전체 읽기.
                        let raw = std::fs::read(&p).ok()?;
                        let sealed_already = nbeep_store::sealed::is_sealed(&raw);
                        let bytes = if sealed_already {
                            nbeep_store::sealed::open(crate::gate::SEAL_QUARANTINE, &secret, &raw)?
                        } else {
                            raw
                        };
                        if !sealed_already {
                            if let Ok(sealed) = nbeep_store::sealed::seal(
                                crate::gate::SEAL_QUARANTINE,
                                &secret,
                                &bytes,
                            ) {
                                let _ = std::fs::write(&p, sealed);
                            }
                        }
                        let bq = Beepq::open(&bytes).ok()?;
                        let mismatch = bq.meta.detected_kind != "unknown"
                            && !bq.meta.declared_ext.is_empty()
                            && !bq.meta.detected_kind.contains(&bq.meta.declared_ext);
                        let name = String::from_utf8_lossy(&bq.meta.orig_name).into_owned();
                        crate::gate::write_qmeta(
                            &p,
                            &crate::gate::QMeta {
                                name: name.clone(),
                                size: bq.original_size,
                                risk: bq.meta.risk,
                                mismatch,
                                sender: bq.meta.sender,
                                received_at: bq.meta.received_at,
                                scan: bq.meta.scan,
                                // 구본(08-21 이전 수신) = 아카이브 미점검으로 정직 표기
                                // (재계산은 prefix+body 재조립 = 대용량 이중 복사라 비채택).
                                archive_viol: false,
                            },
                            &secret,
                        );
                        Some(QRowRaw {
                            path: p.to_string_lossy().into_owned(),
                            name,
                            size: bq.original_size,
                            risk: bq.meta.risk,
                            mismatch,
                            sender: bq.meta.sender,
                            received_at: bq.meta.received_at,
                            scan: bq.meta.scan,
                            archive_viol: false,
                            verified: false,
                        })
                    })
                    .collect()
            })();
            let _ = proxy.send_event(AppEvent::QuarantineScanned { gen, rows });
        });
    }

    /// 격리물 무결성 검증(08-18 · 백그라운드) — 목록이 뜬 뒤 행마다 **전체 개봉 +
    /// AEAD 태그 + Beepq 파싱**을 확인해 `QVerified`로 그 행 Approve를 활성화한다.
    /// **작은 파일 먼저**(1 워커 순차 — 스레드 폭주 없이 512MB가 소형을 막지 않게).
    /// 손상·미완(태그 실패)은 `ok=false` → 승인 차단 유지(fail-closed).
    fn spawn_quarantine_verify(&mut self, gen: u64) {
        let proxy = self.proxy.clone();
        let mut items: Vec<(u64, String)> = self
            .qrows_raw
            .iter()
            .map(|r| (r.size, r.path.clone()))
            .collect();
        items.sort_by_key(|(s, _)| *s); // 작은 것부터 = 즉시 활성
        let secret = self.identity.wrap_secret();
        std::thread::spawn(move || {
            for (_, path) in items {
                let ok = crate::gate::read_beepq_bytes(std::path::Path::new(&path), &secret)
                    .and_then(|b| nbeep_safe::Beepq::open(&b).ok())
                    .is_some();
                let _ = proxy.send_event(AppEvent::QVerified { gen, path, ok });
            }
        });
    }

    /// 원시 캐시 → 표시 행(값싼 가공만 — 이름 조회·시각 라벨·썸네일 캐시).
    fn quarantine_rows(&mut self) -> Vec<nbeep_ui::QRow> {
        use nbeep_core::TrustStore as _;
        let secret = self.identity.wrap_secret();
        let raws = self.qrows_raw.clone();
        raws.into_iter()
            .map(|r| {
                let from = self
                    .table
                    .list()
                    .into_iter()
                    .find(|e| e.peer == r.sender)
                    .map_or_else(
                        || r.sender.short(),
                        |e| format!("{} ({})", e.name.as_str(), r.sender.short()),
                    );
                let age = unix_now().saturating_sub(r.received_at);
                let when = if age >= 86_400 {
                    nbeep_core::tf(nbeep_core::Msg::AgoDays, &[&(age / 86_400).to_string()])
                } else {
                    nbeep_plat::clock::local_hms(r.received_at).hms()
                };
                let path_s = r.path.clone();
                // 이미지 미리보기(M4-5ⓑ) — 캐시 조회. 미보유면 `None` 선점 기록 후
                // 워커 요청(중복 요청·실패 재시도 방지 겸용) → `Decoded(QThumb)`가
                // 캐시를 채우고 행을 다시 그린다(팝인). 이미지가 아니면 조용히 없음.
                let thumb = match self.qthumbs.get(&path_s) {
                    Some(t) => t.clone(),
                    None => {
                        self.qthumbs.insert(path_s.clone(), None);
                        let qp = path_s.clone();
                        spawn_decode(
                            self.proxy.clone(),
                            DecodeTarget::QThumb(path_s.clone()),
                            move || {
                                crate::imgdec::thumb_raw_from_beepq(
                                    std::path::Path::new(&qp),
                                    64,
                                    &secret,
                                )
                            },
                        );
                        None
                    }
                };
                nbeep_ui::QRow {
                    name: r.name,
                    risk: r.risk,
                    mismatch: r.mismatch,
                    size: r.size,
                    trust: self.trust.level(r.sender),
                    from,
                    when,
                    thumb,
                    path: path_s,
                    ready: r.verified,
                    scan: r.scan,
                    archive_viol: r.archive_viol,
                }
            })
            .collect()
    }

    /// 격리함 창(메뉴 → 격리함) — 승인 = 실체화, 삭제 = `.beepq` 제거.
    /// 대화함 열기(M3-23 · 사용자 확정 08-20) — 툴바 서랍 아이콘. 단일 창.
    fn open_convbox(&mut self, el: &ActiveEventLoop) {
        if let Some((cid, _)) = self.windows.iter().find(|(_, e)| e.role == Role::Convbox) {
            if let Some(e) = self.windows.get(cid) {
                e.window.focus_window();
            }
            return;
        }
        let attrs = Window::default_attributes()
            .with_title(format!(
                "Nexa Beep — {}",
                nbeep_core::t(nbeep_core::Msg::ConvboxTitle)
            ))
            .with_inner_size(winit::dpi::LogicalSize::new(560.0, 460.0))
            .with_window_icon(self.icon.clone());
        let window = Rc::new(el.create_window(attrs).unwrap());
        window.set_ime_allowed(true); // 이름 필터 = 한글 입력 대상
        let scale = window.scale_factor() as f32;
        let context = softbuffer::Context::new(window.clone()).unwrap();
        let surface = SbSurface::new(&context, window.clone()).unwrap();
        let id = window.id();
        self.windows.insert(
            id,
            WinEntry {
                role: Role::Convbox,
                window,
                surface,
                cursor: (0, 0),
                scale,
            },
        );
        let rows = self.build_convbox_rows();
        self.convbox_view = Some(nbeep_ui::ConvboxWidget::new(rows));
        self.layout_window(id);
        self.request_redraw(id);
    }

    /// 대화함 행 구성 — `data/history/*.seg` 스캔 → 핀/그룹 매핑(등재 사양 ①).
    /// 미리보기·시각은 메모리 스레드가 우선, 없으면 세그 개봉(작은 파일 — 값싸다).
    /// 개봉도 실패(다른 신원 봉인)하면 크기만 보인다(fail-closed 표시).
    fn build_convbox_rows(&self) -> Vec<nbeep_ui::CRow> {
        let dir = self.data_dir.join("history");
        let Ok(rd) = std::fs::read_dir(&dir) else {
            return Vec::new();
        };
        let recs = self.trust.export();
        let mut rows: Vec<(u64, nbeep_ui::CRow)> = Vec::new();
        for e in rd.flatten() {
            let path = e.path();
            if path.extension().and_then(|x| x.to_str()) != Some("seg") {
                continue;
            }
            let Some(stem) = path.file_stem().and_then(|s| s.to_str()).map(String::from) else {
                continue;
            };
            let size = std::fs::metadata(&path).map(|m| m.len()).unwrap_or(0);
            // 메모리 스레드 우선(부팅 복원분·활성 대화) — 없으면 세그 개봉.
            struct Src<'a> {
                is_group: bool,
                name: String,
                avatar: Option<std::rc::Rc<nbeep_ui::theme::IconImage>>,
                border: Option<(u8, u8, u8)>,
                seed: Vec<u8>,
                lines: Option<&'a [ChatLine]>,
            }
            let decoded: Option<Vec<ChatLine>>;
            let src: Src = if let Some(gshort) = stem.strip_prefix("g-") {
                let sg = self
                    .groups
                    .shared_list()
                    .iter()
                    .find(|s| s.roster.uid.short() == gshort);
                let (name, seed, mem) = sg.map_or_else(
                    || (gshort.to_string(), stem.as_bytes().to_vec(), None),
                    |s| {
                        (
                            s.roster.name.as_str().to_string(),
                            s.roster.uid.0.to_vec(),
                            self.group_threads.get(&s.local_id).map(Vec::as_slice),
                        )
                    },
                );
                Src {
                    is_group: true,
                    name,
                    avatar: None,
                    border: None,
                    seed,
                    lines: mem,
                }
            } else {
                let peer = recs.iter().find(|r| r.peer.short() == stem).map(|r| r.peer);
                let (name, seed) = peer.map_or_else(
                    || (stem.clone(), stem.as_bytes().to_vec()),
                    |p| (self.peer_title(p), p.as_bytes().to_vec()),
                );
                let (avatar, border) = peer
                    .and_then(|p| self.peer_profiles.get(&p))
                    .map_or((None, None), |pr| (pr.avatar.clone(), pr.border));
                let mem = peer.and_then(|p| {
                    self.conversations
                        .get(&p)
                        .map(|c| c.lines.as_slice())
                        .or_else(|| self.parked_lines.get(&p).map(Vec::as_slice))
                });
                Src {
                    is_group: false,
                    name,
                    avatar,
                    border,
                    seed,
                    lines: mem,
                }
            };
            let lines = if src.lines.is_some() {
                src.lines
            } else {
                decoded = std::fs::read(&path)
                    .ok()
                    .and_then(|raw| self.history_open_bytes(&stem, &raw))
                    .map(|b| decode_history(&b));
                decoded.as_deref()
            };
            let (preview, when, at_ms) = lines.and_then(|l| l.last()).map_or_else(
                || (String::new(), String::new(), 0),
                |l| {
                    let pv = match &l.body {
                        nbeep_ui::ChatBody::Text(t) => {
                            t.as_str().lines().next().unwrap_or("").to_string()
                        }
                        nbeep_ui::ChatBody::Xfer(x) => {
                            format!(
                                "[{}] {}",
                                nbeep_core::t(nbeep_core::Msg::WordFile),
                                x.name.as_str()
                            )
                        }
                    };
                    let w = l.wall;
                    (
                        pv,
                        format!("{:02}-{:02} {:02}:{:02}", w.mo, w.d, w.h, w.m),
                        l.at_ms,
                    )
                },
            );
            rows.push((
                at_ms,
                nbeep_ui::CRow {
                    key: stem,
                    is_group: src.is_group,
                    name: src.name,
                    when,
                    preview,
                    size,
                    avatar: src.avatar,
                    border: src.border,
                    seed: src.seed,
                },
            ));
        }
        rows.sort_by_key(|r| std::cmp::Reverse(r.0)); // 최신이 위
        rows.into_iter().map(|(_, r)| r).collect()
    }

    /// 대화함 목록 재구성 + 재도색(삭제·복원 뒤).
    fn refresh_convbox(&mut self) {
        let rows = self.build_convbox_rows();
        let mut inv = Invalidations::default();
        if let Some(cv) = &mut self.convbox_view {
            cv.set_rows(rows, &mut inv);
        }
        if let Some((cid, _)) = self.windows.iter().find(|(_, e)| e.role == Role::Convbox) {
            let cid = *cid;
            self.request_redraw(cid);
        }
    }

    /// 대화함 행 하나 삭제 — sealed 파일 삭제 + **메모리 스레드 비움**(parked/
    /// conv/group_threads) + 열린 대화창·목록 즉시 반영(등재 사양 ②).
    fn convbox_delete_one(&mut self, key: &str) {
        let path = self.data_dir.join("history").join(format!("{key}.seg"));
        let _ = std::fs::remove_file(&path);
        // ★ 셰레딩(D-18 §7 · 08-21) — 파일 삭제 + **키 폐기**: 디스크 잔존
        //   바이트가 있어도(웨어 레벨링·백업 사본) 복호 불가가 된다.
        //   (백업해 둔 기록은 백업의 keys.seg가 키를 지녀 복원 시 되살아난다.)
        self.datakeys.destroy(key);
        let mut inv = Invalidations::default();
        if let Some(gshort) = key.strip_prefix("g-") {
            if let Some(gid) = self
                .groups
                .shared_list()
                .iter()
                .find(|s| s.roster.uid.short() == gshort)
                .map(|s| s.local_id)
            {
                self.group_threads.remove(&gid);
                if let Some(c) = self.gchats.get_mut(&gid) {
                    c.clear_lines(&mut inv);
                }
                if let Some((wid, _)) = self
                    .windows
                    .iter()
                    .find(|(_, e)| e.role == Role::GroupChat(gid))
                {
                    let wid = *wid;
                    self.request_redraw(wid);
                }
            }
        } else if let Some(peer) = self
            .trust
            .export()
            .iter()
            .find(|r| r.peer.short() == key)
            .map(|r| r.peer)
        {
            self.parked_lines.remove(&peer);
            if let Some(conv) = self.conversations.get_mut(&peer) {
                conv.lines.clear();
            }
            if let Some(c) = self.chats.get_mut(&peer) {
                c.clear_lines(&mut inv);
            }
            self.redraw_conversation(peer);
        }
    }

    /// 대화함 행위 실행 — 열기/삭제/전체 삭제/백업/복원(위젯은 의도만 안다).
    fn run_convbox_action(&mut self, act: nbeep_ui::CvAction, id: WindowId, el: &ActiveEventLoop) {
        match act {
            nbeep_ui::CvAction::Open(key) => {
                // 행 클릭 = 그 대화 열기(목록 클릭과 동일 경로 · 사용자 확정 08-20).
                if let Some(gshort) = key.strip_prefix("g-") {
                    if let Some(gid) = self
                        .groups
                        .shared_list()
                        .iter()
                        .find(|s| s.roster.uid.short() == gshort)
                        .map(|s| s.local_id)
                    {
                        self.open_group_thread(gid, el);
                    }
                } else if let Some(peer) = self
                    .trust
                    .export()
                    .iter()
                    .find(|r| r.peer.short() == key)
                    .map(|r| r.peer)
                {
                    self.activate(peer, el);
                }
            }
            nbeep_ui::CvAction::Delete(key) => {
                self.convbox_delete_one(&key);
                self.set_status(nbeep_core::tf(nbeep_core::Msg::StfCvDeleted, &[&key]));
                self.refresh_convbox();
            }
            nbeep_ui::CvAction::DeleteAll => {
                let keys: Vec<String> = self
                    .build_convbox_rows()
                    .into_iter()
                    .map(|r| r.key)
                    .collect();
                for k in keys {
                    self.convbox_delete_one(&k);
                }
                self.set_status(nbeep_core::t(nbeep_core::Msg::StCvCleared).to_string());
                self.refresh_convbox();
            }
            nbeep_ui::CvAction::Backup => {
                if self.build_convbox_rows().is_empty() {
                    let mut inv = Invalidations::default();
                    if let Some(cv) = &mut self.convbox_view {
                        cv.set_message(nbeep_core::t(nbeep_core::Msg::StCvNone), &mut inv);
                    }
                    self.request_redraw(id);
                } else {
                    self.pending_picker = Some(PickerPurpose::HistoryBackupDir);
                }
            }
            nbeep_ui::CvAction::Restore => {
                self.pending_picker = Some(PickerPurpose::HistoryRestoreDir);
            }
        }
    }

    /// 대화 기록 전체 백업(M3-23 · 사용자 확정 = **sealed 그대로 · 전체만**) —
    /// 대상지에 하위 폴더를 만들고 `*.seg`를 복사한다(기록 암호화 철학 유지 —
    /// 복원은 같은 신원에서만 열린다).
    fn do_backup_history(&mut self, dir: &std::path::Path) -> String {
        let src = self.data_dir.join("history");
        let dst = dir.join(self.picker_save_name(PickerPurpose::HistoryBackupDir));
        if std::fs::create_dir_all(&dst).is_err() {
            return nbeep_core::t(nbeep_core::Msg::StHistorySealFail).to_string();
        }
        let mut n = 0usize;
        if let Ok(rd) = std::fs::read_dir(&src) {
            for e in rd.flatten() {
                let p = e.path();
                if p.extension().and_then(|x| x.to_str()) == Some("seg")
                    && std::fs::copy(&p, dst.join(e.file_name())).is_ok()
                {
                    n += 1;
                }
            }
        }
        if n == 0 {
            return nbeep_core::t(nbeep_core::Msg::StCvNone).to_string();
        }
        // ★ 키 동반(셰레딩 의미론 · 08-21) — 데이터 키 테이블을 백업에 포함해야
        //   복원이 세그를 열 수 있다(없으면 08-21 이후 세그는 복원 불가).
        let _ = std::fs::copy(self.data_dir.join("keys.seg"), dst.join("keys.seg"));
        let m = nbeep_core::tf(
            nbeep_core::Msg::StfCvBackupDone,
            &[&n.to_string(), &dst.display().to_string()],
        );
        let mut inv = Invalidations::default();
        if let Some(cv) = &mut self.convbox_view {
            cv.set_message(m.clone(), &mut inv);
        }
        self.refresh_convbox();
        m
    }

    /// 대화 기록 복원 — 폴더의 `*.seg` 전부(사용자 확정: **중복 = 덮어쓰기 ·
    /// 없으면 추가 · 기존 유지**). 파일 반입 후 메모리 스레드에 즉시 반영.
    fn do_restore_history_dir(&mut self, dir: &std::path::Path) -> String {
        let files: Vec<std::path::PathBuf> = std::fs::read_dir(dir)
            .map(|rd| {
                rd.flatten()
                    .map(|e| e.path())
                    .filter(|p| p.extension().and_then(|x| x.to_str()) == Some("seg"))
                    .collect()
            })
            .unwrap_or_default();
        self.do_restore_history_files(&files)
    }

    /// 복원 실행(파일 목록판 — 폴더 복원·개별 파일 클릭 공용).
    fn do_restore_history_files(&mut self, files: &[std::path::PathBuf]) -> String {
        if files.is_empty() {
            return nbeep_core::t(nbeep_core::Msg::StCvNone).to_string();
        }
        let dst_dir = self.data_dir.join("history");
        let _ = std::fs::create_dir_all(&dst_dir);
        let mut n = 0usize;
        let mut restored: Vec<String> = Vec::new();
        for p in files {
            let Some(name) = p.file_name() else { continue };
            if std::fs::copy(p, dst_dir.join(name)).is_ok() {
                n += 1;
                if let Some(stem) = p.file_stem().and_then(|s| s.to_str()) {
                    restored.push(stem.to_string());
                }
            }
        }
        // ★ 백업 키 병합(셰레딩 짝 · 08-21) — 복원 원천 폴더의 keys.seg를 합쳐야
        //   그 키로 봉인된 세그가 열린다(같은 stem = 백업 키가 이긴다 · 복원 우선).
        {
            let mut dirs: Vec<&std::path::Path> = files.iter().filter_map(|p| p.parent()).collect();
            dirs.dedup();
            for d in dirs {
                let _ = self.datakeys.merge_from(&d.join("keys.seg"));
            }
        }
        self.apply_restored_history(&restored);
        let m = nbeep_core::tf(nbeep_core::Msg::StfCvRestoreDone, &[&n.to_string()]);
        let mut inv = Invalidations::default();
        if let Some(cv) = &mut self.convbox_view {
            cv.set_message(m.clone(), &mut inv);
        }
        self.refresh_convbox();
        m
    }

    /// 복원 파일을 메모리 스레드에 반영 — 복원이 이긴다(덮어쓰기 의미론을
    /// 메모리에도 · 안 그러면 활성 대화의 다음 record_history가 복원본을 되덮는다).
    fn apply_restored_history(&mut self, stems: &[String]) {
        let dir = self.data_dir.join("history");
        let recs = self.trust.export();
        let mut inv = Invalidations::default();
        for stem in stems {
            let path = dir.join(format!("{stem}.seg"));
            let Some(lines) = std::fs::read(&path)
                .ok()
                .and_then(|raw| self.history_open_bytes(stem, &raw))
                .map(|b| decode_history(&b))
            else {
                continue; // 다른 신원 봉인 — fail-closed(파일은 반입돼 있음)
            };
            if let Some(gshort) = stem.strip_prefix("g-") {
                if let Some(gid) = self
                    .groups
                    .shared_list()
                    .iter()
                    .find(|s| s.roster.uid.short() == gshort)
                    .map(|s| s.local_id)
                {
                    if let Some(c) = self.gchats.get_mut(&gid) {
                        c.clear_lines(&mut inv);
                        for l in &lines {
                            c.push_line(l.clone(), &mut inv);
                        }
                    }
                    self.group_threads.insert(gid, lines);
                    if let Some((wid, _)) = self
                        .windows
                        .iter()
                        .find(|(_, e)| e.role == Role::GroupChat(gid))
                    {
                        let wid = *wid;
                        self.request_redraw(wid);
                    }
                }
            } else if let Some(peer) = recs
                .iter()
                .find(|r| r.peer.short() == stem.as_str())
                .map(|r| r.peer)
            {
                if let Some(conv) = self.conversations.get_mut(&peer) {
                    conv.lines = lines.clone();
                }
                if let Some(c) = self.chats.get_mut(&peer) {
                    c.clear_lines(&mut inv);
                    for l in &lines {
                        c.push_line(l.clone(), &mut inv);
                    }
                }
                self.parked_lines.insert(peer, lines);
                self.redraw_conversation(peer);
            }
        }
    }

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
        // 캐시로 즉시 열고(빈 캐시면 빈 목록) 워커 스캔이 채운다(08-18 —
        // 대용량 항목 개봉이 메인을 얼리던 것 · 완료 = QuarantineScanned).
        let rows = self.quarantine_rows();
        let mut qv = nbeep_ui::QuarantineWidget::new(rows);
        let mut inv = Invalidations::default();
        qv.set_loading(true, &mut inv); // 스캔 완료(QuarantineScanned)가 끈다
        self.quarantine_view = Some(qv);
        self.layout_window(id);
        self.request_redraw(id);
        self.spawn_quarantine_scan();
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
                let out = crate::gate::read_beepq_bytes(
                    std::path::Path::new(&path),
                    &self.identity.wrap_secret(),
                )
                .ok_or_else(|| "봉인 개봉 실패(다른 신원·손상)".to_string())
                .and_then(|b| Beepq::open(&b).map_err(|e| format!("{e:?}")))
                .and_then(|bq| {
                    QuarantineDir::materialize(&bq, &dest, &CryptoHash, &OsMark)
                        .map_err(|e| e.to_string())
                });
                self.set_status(match out {
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
                });
            }
            nbeep_ui::QAction::Reject(path) => {
                crate::gate::remove_qmeta(std::path::Path::new(&path)); // 사이드카 동반 삭제
                self.set_status(match std::fs::remove_file(&path) {
                    Ok(()) => "격리물을 삭제했습니다".into(),
                    Err(e) => {
                        msg_err = true;
                        format!("삭제 실패: {e}")
                    }
                });
            }
            nbeep_ui::QAction::Clear => {
                // 비우기 — 위젯이 2단계 확인을 통과시킨 상태(사용자 요청 08-10).
                let (mut n, mut failed) = (0u32, 0u32);
                if let Ok(dir) =
                    QuarantineDir::open(crate::gate::quarantine_root(crate::gate::CH_GUI))
                {
                    if let Ok(paths) = dir.list() {
                        for p in paths {
                            crate::gate::remove_qmeta(&p); // 사이드카 동반 삭제
                            match std::fs::remove_file(&p) {
                                Ok(()) => n += 1,
                                Err(_) => failed += 1,
                            }
                        }
                    }
                }
                self.set_status(if failed == 0 {
                    format!("격리함을 비웠습니다 — {n}건 삭제")
                } else {
                    msg_err = true;
                    format!("격리함 비우기 — {n}건 삭제 · {failed}건 실패")
                });
            }
        }
        // 액션이 디스크를 바꿨다 — 사라진 경로만 캐시에서 즉시 걷어내고
        // (값싼 exists 검사) 진실은 워커 재스캔이 맞춘다.
        self.qrows_raw
            .retain(|r| std::path::Path::new(&r.path).is_file());
        self.spawn_quarantine_scan();
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

    /// 피커 "여기에 저장" 파일명 — 용도별(신원 키 vs 설정 · 08-15).
    fn picker_save_name(&self, purpose: PickerPurpose) -> String {
        match purpose {
            PickerPurpose::SettingsBackupDir => {
                format!("nexa-beep-settings-{}.cfg", self.identity.peer_id().short())
            }
            PickerPurpose::HistoryBackupDir => {
                // 백업은 하위 폴더로 묶는다(파일 여러 개 — 대상지 오염 방지).
                format!("nexa-beep-history-{}", self.identity.peer_id().short())
            }
            _ => self.default_backup_name(),
        }
    }

    /// 탐색형 피커 목록 구성 — (창 제목, 트리 행, 라벨→행위). 라벨 접두로 종류를
    /// 구분한다(글리프 폴백만으로 충분한 문자 — 이모지 금지).
    fn picker_listing(
        purpose: PickerPurpose,
        dir: &std::path::Path,
        save_name: &str,
    ) -> (String, Vec<nbeep_ui::TreeNode>, Vec<(String, PickEntry)>) {
        let mut entries: Vec<(String, PickEntry)> = Vec::new();
        if matches!(
            purpose,
            PickerPurpose::BackupDir
                | PickerPurpose::SettingsBackupDir
                | PickerPurpose::HistoryBackupDir
        ) {
            entries.push((format!("[여기에 저장] {save_name}"), PickEntry::SaveHere));
        }
        if purpose == PickerPurpose::HistoryRestoreDir {
            // 폴더 단위 복원(중복 = 덮어쓰기 · 사용자 확정) — 개별 파일 클릭도 허용.
            entries.push((
                nbeep_core::t(nbeep_core::Msg::PickRestoreHere).to_string(),
                PickEntry::SaveHere,
            ));
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
                            PickerPurpose::SettingsRestoreFile => {
                                name.to_ascii_lowercase().ends_with(".cfg")
                            }
                            PickerPurpose::HistoryRestoreDir => {
                                name.to_ascii_lowercase().ends_with(".seg")
                            }
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
            entries.push((
                nbeep_core::tf(nbeep_core::Msg::PickDirPrefix, &[&name]),
                PickEntry::Dir(p),
            ));
        }
        for (name, p) in files {
            entries.push((name, PickEntry::File(p)));
        }
        let dir_s = dir.display().to_string();
        let title = match purpose {
            PickerPurpose::BackupDir => {
                nbeep_core::tf(nbeep_core::Msg::TitlePickBackupDir, &[&dir_s])
            }
            PickerPurpose::RestoreKey => {
                nbeep_core::tf(nbeep_core::Msg::TitlePickBackup, &[&dir_s])
            }
            PickerPurpose::ProfileImage => {
                nbeep_core::tf(nbeep_core::Msg::TitlePickProfileImage, &[&dir_s])
            }
            PickerPurpose::SettingsBackupDir => {
                nbeep_core::tf(nbeep_core::Msg::TitlePickSettingsBackupDir, &[&dir_s])
            }
            PickerPurpose::SettingsRestoreFile => {
                nbeep_core::tf(nbeep_core::Msg::TitlePickSettingsBackup, &[&dir_s])
            }
            PickerPurpose::HistoryBackupDir => {
                nbeep_core::tf(nbeep_core::Msg::TitlePickCvBackupDir, &[&dir_s])
            }
            PickerPurpose::HistoryRestoreDir => {
                nbeep_core::tf(nbeep_core::Msg::TitlePickCvRestoreDir, &[&dir_s])
            }
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
                Self::picker_listing(purpose, &dir, &self.picker_save_name(purpose));
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
        let attrs = self.modal_attrs(attrs, false); // 메인 소유(08-15 — 창 묶음 부상)
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
            Self::picker_listing(ctx.purpose, &ctx.dir, &self.picker_save_name(ctx.purpose));
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

    /// 설정 백업(08-15 · 고급) — 대기 중 스냅샷까지 flush한 뒤 settings.cfg 복사.
    fn do_backup_settings(&mut self, dir: &std::path::Path) -> String {
        self.conf_save(false); // 스케줄 대기분 포함 최신본을 파일에
        let src = self.data_dir.join("settings.cfg");
        let dst = dir.join(self.picker_save_name(PickerPurpose::SettingsBackupDir));
        match std::fs::copy(&src, &dst) {
            Ok(_) => format!("설정 백업 완료 — {}", dst.display()),
            Err(e) => format!("설정 백업 실패: {e}"),
        }
    }

    /// 설정 복원(08-15 · 고급) — 백업 파일의 **아는 키 전부**를 정식 깔때기
    /// (apply_settings)로 적용한다(hot-swap 일습 — 테마·IME·목록·프로필 전파까지).
    /// 모르는 키는 보존 저장(F-1 관용).
    fn do_restore_settings(&mut self, file: &std::path::Path) -> String {
        let Ok(text) = std::fs::read_to_string(file) else {
            return "설정 복원 실패 — 파일을 읽을 수 없습니다".into();
        };
        let doc = nexa_conf::parse(&text);
        let mut changes: Vec<(&'static str, String)> = Vec::new();
        let mut unknown = 0usize;
        for (k, v) in doc.pairs {
            // &'static 키로 정규화(레지스트리가 원천 — set_by_name과 같은 대조).
            // 행위 키는 값이 아니라 제외(손편집 파일이 피커를 열게 두지 않는다).
            if let Some(entry) = nbeep_ui::settings::registry().iter().find(|e| {
                e.key == k && !matches!(e.kind, nbeep_ui::settings::SettingKind::Action { .. })
            }) {
                // 동일값 생략(RL-2ⓒ) — do_reset_settings의 필터와 대칭. 없으면
                // 무변경 복원 1회가 프로필 9벌 push·전 화면 재적용을 냈다.
                if self.settings.get(entry.key) != v {
                    changes.push((entry.key, v));
                }
            } else {
                self.conf.keep_unknown(k, v);
                unknown += 1;
            }
        }
        let n = changes.len();
        // 행위 키(backup/restore/reset)는 파일에 없다(값이 아니라서 저장 안 됨).
        self.apply_settings(changes);
        format!("설정 복원 완료 — {n}개 적용(모르는 키 {unknown}개 보존)")
    }

    /// 설정 초기화(08-15 · 고급) — 레지스트리(표시 항목) 전부 기본값으로. 숨김 키
    /// (창 위치·최근 목록)와 신원·핀·그룹은 건드리지 않는다.
    /// OS 알림(M3-8 최소 슬라이스 · 08-15) — 조건: 설정 on · **앱의 어느 창도
    /// 포커스가 아님**(포커스 중엔 배지·제목이 맡는다) · 키별 3초 스로틀.
    /// `silent` = DR-25 신뢰 게이트(미검증 발신자 = 소리 없음). 내용 정책은
    /// 호출자가 끝낸다(미리보기 무해화·파일명 금지 — FR-S-41).
    fn notify_user(
        &mut self,
        key: &str,
        title: &str,
        body: &str,
        silent: bool,
        force: bool,
        target: NotifyTarget,
    ) {
        if self.settings.get("notify.enabled") != "on" {
            return;
        }
        // ④ Urgent(force) = 앱이 앞에 있어도 알림("지금 당장"의 요청 — docs/24).
        if !force && self.windows.values().any(|e| e.window.has_focus()) {
            return;
        }
        let now = self.now_ms();
        if now.saturating_sub(self.last_notify.get(key).copied().unwrap_or(0)) < 3000 {
            return;
        }
        self.last_notify.insert(key.to_string(), now);
        // 클릭 왕복(08-15 — 알림 클릭 = 해당 대화): 토큰(=키)을 OS에 실어 보내고
        // 되돌아오면 이 맵으로만 해석한다(OS에는 불투명 — 봉투 원리).
        self.notify_targets.insert(key.to_string(), target);
        // Windows = 트레이 풍선(있을 때) · 그 외/트레이 부재 = plat 어댑터(fail-soft).
        #[cfg(windows)]
        if let Some(t) = &self.tray {
            t.notify(title, body, silent, key);
            return;
        }
        let _ = nbeep_plat::notify::notify(&nbeep_plat::notify::Note {
            title,
            body,
            silent,
            target: key,
        });
    }

    /// 알림 본문(M3-8) — 미리보기 설정 on이면 무해화된 본문 80자, off면 일반 문구.
    fn notify_body(&self, text: &str) -> String {
        if self.settings.get("notify.preview") == "on" {
            let mut b: String = text.chars().take(80).collect();
            if text.chars().count() > 80 {
                b.push('…');
            }
            b
        } else {
            nbeep_core::t(nbeep_core::Msg::NotifyNewMessage).to_string()
        }
    }

    fn do_reset_settings(&mut self) -> String {
        // 항목의 **기본값 쌍 전체**를 돌린다(08-15 점검) — `e.key` 하나만 돌면
        // 값 키가 여럿인 항목(FontSection = family+size)의 짝 키가 초기화에서 샜다.
        let changes: Vec<(&'static str, String)> = nbeep_ui::settings::registry()
            .iter()
            .filter(|e| !matches!(e.kind, nbeep_ui::settings::SettingKind::Action { .. }))
            .flat_map(nbeep_ui::settings::Entry::default_values)
            .filter(|(k, d)| self.settings.get(k) != d)
            .collect();
        let n = changes.len();
        self.apply_settings(changes);
        format!("설정 초기화 완료 — {n}개 항목을 기본값으로")
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
        // 그룹 세그먼트도 래핑 원료가 바뀐다(M5-1 — trust와 같은 결).
        let (groups, _gload) = nbeep_store::FileGroupStore::open(
            self.data_dir.join("groups.seg"),
            self.identity.wrap_secret(),
        );
        self.groups = groups;
        self.conversations.clear();
        self.chats.clear();
        self.single_open = None;
        self.single_open_group = None;
        self.gchats.clear();
        self.pending_group_sends.clear();
        self.connecting.clear();
        self.reconnect.clear(); // 신원 교체 — 옛 신원의 재연결 스케줄은 무효(ⓑ)
        let chat_wins: Vec<WindowId> = self
            .windows
            .iter()
            .filter(|(_, e)| matches!(e.role, Role::Chat(_) | Role::GroupChat(_)))
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
            bio: self.settings.get("profile.bio").to_string(),
            image_path: self.settings.get("profile.image_path").to_string(),
            share_basic: self.settings.get("profile.share.basic") == "on",
            share_email: self.settings.get("profile.share.email") == "on",
            share_phone: self.settings.get("profile.share.phone") == "on",
            resolved_name: effective_display_name(&self.settings, &self.identity.peer_id())
                .as_str()
                .to_string(),
            seed: self.identity.peer_id().as_bytes().to_vec(),
            // 내 키 지문(08-17 사용자 요청 — 상대가 내 카드에서 보는 "키 지문"과
            // 같은 값을 나도 볼 수 있게. 대조의 기준점).
            fingerprint: self.identity.peer_id().short(),
            avatar_choice: self.settings.get("profile.avatar").to_string(),
            avatar_border: self.settings.get("profile.avatar_border").to_string(),
            // 툴팁 대기(ms · 08-14) — 무효는 위젯이 기본 2000으로 본다(관용 파싱).
            tooltip_ms: self
                .settings
                .get("ui.tooltip_ms")
                .parse::<u64>()
                .unwrap_or(2000),
            avatar: {
                // 내 사진 미리보기(M4-5) — 격리 디코드는 워커로(창 열림이 1~2초 얼지
                // 않게 · 08-13). ★ 초기값 = **툴바용 캐시(my_avatar)**(08-19 실기 —
                // None으로 열면 기본 아바타가 먼저 보였다가 사진으로 바뀌는 깜빡임).
                // 256px 정본은 도착 시 `Decoded(MyAvatar)`가 갈아끼운다(같은 사진이라
                // 선명해질 뿐 화면 전환으로 보이지 않는다).
                let p = self.settings.get("profile.image_path").to_string();
                if !p.is_empty() {
                    spawn_decode(self.proxy.clone(), DecodeTarget::MyAvatar, move || {
                        std::fs::read(&p)
                            .ok()
                            .and_then(|b| crate::imgdec::avatar_raw_from_bytes(&b, 256))
                    });
                    self.my_avatar.clone()
                } else {
                    None
                }
            },
            recent: {
                // 최근 프로필 이미지(08-14) — 탭 구분 목록. 썸네일은 워커 디코드로
                // 뒤따라온다(Decoded(RecentThumb) — 그전엔 자리표시 원).
                let list: Vec<String> = self
                    .settings
                    .get("profile.image_recent")
                    .split('\t')
                    .filter(|s| !s.is_empty())
                    .take(10)
                    .map(str::to_string)
                    .collect();
                for p in &list {
                    let key = p.clone();
                    let jp = p.clone();
                    spawn_decode(
                        self.proxy.clone(),
                        DecodeTarget::RecentThumb(key),
                        move || {
                            std::fs::read(&jp)
                                .ok()
                                .and_then(|b| crate::imgdec::avatar_raw_from_bytes(&b, 64))
                        },
                    );
                }
                list
            },
        };
        let attrs = Window::default_attributes()
            .with_title(format!(
                "Nexa Beep — {}",
                nbeep_core::t(nbeep_core::Msg::ProfileTitle)
            ))
            // 창 높이 770 — bio 고정 3줄 + 토글·안내·버튼이 다 들어간다(08-18 확정 ·
            // 창 크기 조정 없음 · 중간영역 스크롤 재설계는 후속 TODO).
            .with_inner_size(winit::dpi::LogicalSize::new(440.0, 770.0))
            .with_resizable(false)
            // 앱 모달(08-14 표준 재정리) — 앱 창 입력은 모달이 흡수하되, 다른 앱은
            // 자유롭게 위로 온다(AlwaysOnTop 금지 — OS 창 전환 관례).
            .with_window_icon(self.icon.clone());
        let attrs = self.modal_attrs(attrs, true); // 마우스 위치 + 메인 소유(08-15)
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
        let mut pv = nbeep_ui::ProfileWidget::new(&values);
        pv.set_carousel_inverted(carousel_inverted(&self.settings));
        self.profile_view = Some(pv);
        self.layout_window(id);
        self.request_redraw(id);
    }

    /// 상대 프로필 보기 카드(M3-17 — 목록 우클릭). 이미 있으면 갱신·포커스.
    /// 그룹 구성원 목록 모달 내용(08-14 사용자 요청) — **두 진입점이 같은 모달**:
    /// 목록의 그룹 아이콘 클릭 + 방 헤더 클릭. 온라인 판정은 목록 행과 같은 기준.
    fn group_members_summary(
        &self,
        gid: nbeep_core::group::GroupId,
    ) -> Option<(String, Vec<(LinkState, String)>)> {
        let s = self.groups.shared_by_id(gid)?;
        let me = self.identity.peer_id();
        // 소유자 먼저, 나머지는 명부 순서(정렬돼 있다). 상태 점은 **목록 행과 같은
        // 판정·같은 팔레트**(08-15 사용자 확정 "아바타 옆 점과 색을 맞춰줘").
        let mut members: Vec<PeerId> = s.roster.members.clone();
        members.sort_by_key(|m| *m != s.roster.owner);
        // 내가 소유자면 미수락 구성원에 "초대 대기"를 붙인다(08-19 — 첫 전달 실패를
        // 눈에 보이게). 수락은 `group_accepts`가 기록(재초대 시 자가 치유).
        let owner_view = s.roster.owner == me;
        let accepted = self.group_accepts.get(&s.roster.uid);
        let lines: Vec<(LinkState, String)> = members
            .iter()
            .map(|m| {
                // 목록 행과 같은 4상태 판정(refresh_rows와 동일 기준).
                let link = if *m == me || self.conversations.contains_key(m) {
                    LinkState::Active
                } else if self.connecting.contains(*m) {
                    LinkState::Connecting
                } else if self.closed_peers.contains(m) {
                    LinkState::Lost
                } else {
                    LinkState::Idle
                };
                // 통일 표기(08-17 사용자 확정): `표시 이름 — 지문 · 설명(나·소유자)`.
                let name = if *m == me {
                    self.my_display_name()
                } else {
                    self.peer_title(*m)
                };
                let mut line = format!("{name} — {}", m.short());
                let mut descs: Vec<&str> = Vec::new();
                if *m == me {
                    descs.push(nbeep_core::t(nbeep_core::Msg::MemberSelf));
                }
                if *m == s.roster.owner {
                    descs.push(nbeep_core::t(nbeep_core::Msg::MemberOwner));
                }
                // 소유자 시점 · 나·소유자 아닌 구성원이 아직 수락 안 했으면 초대 대기.
                if owner_view
                    && *m != me
                    && *m != s.roster.owner
                    && !accepted.is_some_and(|a| a.contains(m))
                {
                    descs.push(nbeep_core::t(nbeep_core::Msg::MemberPending));
                }
                if !descs.is_empty() {
                    line.push_str(" · ");
                    line.push_str(&descs.join(" · "));
                }
                (link, line)
            })
            .collect();
        let online_n = lines
            .iter()
            .filter(|(l, _)| *l == LinkState::Active)
            .count();
        Some((
            format!(
                "{} — {}",
                s.roster.name.as_str(),
                nbeep_core::tf(
                    nbeep_core::Msg::ListGroupMembers,
                    &[&members.len().to_string(), &online_n.to_string()],
                )
            ),
            lines,
        ))
    }

    /// 내 표시 이름(구성원 목록 등 — M1-10 규칙 그대로).
    fn my_display_name(&self) -> String {
        effective_display_name(&self.settings, &self.identity.peer_id())
            .as_str()
            .to_string()
    }

    /// 구성원 목록 모달 열기 요청 — 실제 열기는 `about_to_wait`(el이 거기 있다).
    /// 내용은 열리는 순간 계산한다(그 사이 상태 변화 반영 — "확인할 때"의 상태).
    fn open_group_members(&mut self, gid: nbeep_core::group::GroupId) {
        self.pending_members = Some(gid);
        self.nudge_pending_members(gid); // "누가 들어왔나" 확인 = 미전달 재시도 계기
    }

    /// 미수락 구성원에게 재연결을 시도한다(08-19) — 소유자만. 세션이 성립하면
    /// [`Self::flush_group_sends`]의 resync가 초대를 재전달한다(첫 전달이 세션
    /// 미성립·네트워크 churn으로 실패한 경우의 명시적 복구 계기 · 저churn = 사용자가
    /// 구성원 목록을 열 때만). 이미 연결된·수락한 구성원은 건드리지 않는다.
    fn nudge_pending_members(&mut self, gid: nbeep_core::group::GroupId) {
        let me = self.identity.peer_id();
        let (uid, members) = match self.groups.shared_by_id(gid) {
            Some(s) if s.roster.owner == me => (s.roster.uid, s.roster.members.clone()),
            _ => return, // 소유자만 재전달 주체(ADR §4 — 명부 단일 진실)
        };
        let accepted = self.group_accepts.get(&uid).cloned().unwrap_or_default();
        let pending: Vec<PeerId> = members
            .into_iter()
            .filter(|m| *m != me && !accepted.contains(m) && !self.conversations.contains_key(m))
            .collect();
        for m in pending {
            self.reconnect.remove(&m); // 백오프 리셋 = 즉시 재시도
            self.start_connect(m, true); // 자동(창 안 열음) · ConnectLatch가 중복 거른다
        }
    }

    /// 구성원 목록 모달 실제 열기(08-15) — 알림 모달 창 재사용 + 상태 목록 모드.
    fn open_members_alert(&mut self, el: &ActiveEventLoop, gid: nbeep_core::group::GroupId) {
        let Some((mut title, lines)) = self.group_members_summary(gid) else {
            return;
        };
        // G4(08-15) — 소유자면 행 클릭 = 제외 진입. 문맥 = 모달과 같은 순서.
        let me = self.identity.peer_id();
        let (order, owned) = self
            .groups
            .shared_by_id(gid)
            .map(|s| {
                let mut m = s.roster.members.clone();
                m.sort_by_key(|p| *p != s.roster.owner); // summary와 같은 정렬
                (m, s.roster.owner == me)
            })
            .unwrap_or_default();
        if owned {
            title.push_str(" · 행 클릭 = 제외");
        }
        self.members_ctx = Some((gid, order, owned));
        let win_h = (110.0 + lines.len() as f64 * 26.0).clamp(170.0, 460.0);
        if let Some((aid, _)) = self.windows.iter().find(|(_, e)| e.role == Role::Alert) {
            let aid = *aid;
            let mut inv = Invalidations::default();
            if let Some(av) = &mut self.alert_view {
                av.set_content(&title, "", &mut inv);
                av.set_status_list(lines, &mut inv);
                av.set_rows_clickable(owned);
            }
            if let Some(e) = self.windows.get(&aid) {
                let _ = e
                    .window
                    .request_inner_size(winit::dpi::LogicalSize::new(400.0, win_h));
                e.window.focus_window();
            }
            self.request_redraw(aid);
            return;
        }
        let attrs = Window::default_attributes()
            .with_title(format!(
                "Nexa Beep — {}",
                nbeep_core::t(nbeep_core::Msg::WinMembers)
            ))
            .with_inner_size(winit::dpi::LogicalSize::new(400.0, win_h))
            .with_resizable(false)
            .with_window_icon(self.icon.clone());
        let attrs = self.modal_attrs(attrs, false); // 메인 소유(08-15 — 창 묶음 부상)
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
        let mut av = nbeep_ui::AlertWidget::new(title, "");
        let mut inv = Invalidations::default();
        av.set_status_list(lines, &mut inv);
        av.set_rows_clickable(owned);
        av.set_badge_shape(self.settings.get("ui.link_badge_shape") == "on");
        self.alert_view = Some(av);
        self.layout_window(id);
        self.request_redraw(id);
    }

    /// 상대 프로필 카드 내용 조립(열기·갱신 공용 — M3-21).
    fn build_peer_info(&self, peer: PeerId) -> nbeep_ui::PeerInfo {
        let p = self.peer_profiles.get(&peer);
        nbeep_ui::PeerInfo {
            name: self
                .table
                .get(peer)
                .map_or_else(|| format!("{peer:?}"), |e| e.name.as_str().to_string()),
            profile_name: p
                .and_then(|p| p.name.as_ref())
                .map(|n| n.as_str().to_string())
                .unwrap_or_default(),
            bio: p.and_then(|p| p.bio.clone()).unwrap_or_default(),
            email: p.and_then(|p| p.email.clone()).unwrap_or_default(),
            phone: p.and_then(|p| p.phone.clone()).unwrap_or_default(),
            has_image: p.is_some_and(|p| p.image_file.is_some()),
            fingerprint: peer.short(),
            seed: peer.as_bytes().to_vec(),
            avatar: p.and_then(|p| p.avatar.clone()),
            last_seen: ago_label(self.trust.meta(peer).0),
            last_chat: ago_label(self.trust.meta(peer).1),
            received: ago_label(p.map_or(0, |p| p.received_ms)),
            border: p.and_then(|p| p.border),
            // 안전 번호(M3-6 · SAS) — 두 키에서 결정적으로 파생(개시자 무관 정렬)
            // 이라 세션 없이도 계산된다. 대조는 사람 몫(전화·대면).
            safety_number: nbeep_crypto::safety_number(self.identity.peer_id(), peer),
            verified: {
                use nbeep_core::TrustStore as _;
                self.trust.level(peer) == nbeep_core::TrustLevel::FingerprintVerified
            },
        }
    }

    /// 열려 있는 상대 프로필 카드 갱신(M3-21 — push/pull 응답·아바타 디코드 도착).
    /// 그 상대의 카드가 열려 있을 때만 — 창은 두고 내용만 다시 짓는다.
    fn refresh_peer_info_card(&mut self, peer: PeerId) {
        let Some(wid) = self
            .windows
            .iter()
            .find(|(_, e)| e.role == Role::PeerInfo(peer))
            .map(|(wid, _)| *wid)
        else {
            return;
        };
        self.peer_info_view = Some(nbeep_ui::PeerInfoWidget::new(self.build_peer_info(peer)));
        self.layout_window(wid);
        self.request_redraw(wid);
    }

    /// 대화창 명령 실행 결과 — 보낼 것이 남았는가.
    ///
    /// 명령은 **로컬 지시**라 와이어로 나가지 않는다([`nbeep_core::command`] 문서).
    /// `peer`가 `None`이면 그룹 방 — 상대가 하나로 정해지지 않는 명령(`/verify`)은
    /// 거기서 쓸 수 없다고 알린다(조용히 아무 일도 안 하면 사용자는 실행된 줄 안다).
    fn run_chat_command(
        &mut self,
        input: Option<&nbeep_core::SafeText>,
        peer: Option<PeerId>,
    ) -> CmdOutcome {
        use nbeep_core::command::{parse, ChatCommand, Parsed};
        let Some(text) = input else {
            return CmdOutcome::Send(None);
        };
        match parse(text.as_str()) {
            // parse가 앞뒤 공백을 떨어낸 본문이 원본과 다르면 그 본문을 보낸다.
            // (escape(`//…`)는 08-16 2차 확정으로 폐지 — `/` 시작은 전부 비전송.)
            Parsed::Text(t) => {
                if t == text.as_str() {
                    return CmdOutcome::Send(Some(text.clone()));
                }
                CmdOutcome::Send(Some(nbeep_core::sanitize_message(&t)))
            }
            Parsed::Empty => CmdOutcome::Send(None),
            Parsed::Unknown(name) => {
                self.set_status(nbeep_core::tf(nbeep_core::Msg::CmdUnknown, &[&name]));
                CmdOutcome::Handled
            }
            Parsed::Command(cmd) => {
                match cmd {
                    // 등급 명령(④ 08-20) — 본문을 그 등급으로 보낸다. 빈 본문·
                    // 멀티라인 = 사용법 안내(비전송 — 뒷줄이 몰래 사라지지 않게).
                    ChatCommand::Notice(body) | ChatCommand::Urgent(body)
                        if peer.is_none()
                            || body.trim().is_empty()
                            || text.as_str().contains('\n') =>
                    {
                        if peer.is_none() {
                            self.set_status(nbeep_core::t(
                                nbeep_core::Msg::StGradeGroupUnsupported,
                            ));
                        } else {
                            let name = if matches!(
                                parse(text.as_str()),
                                Parsed::Command(ChatCommand::Urgent(_))
                            ) {
                                "/urgent"
                            } else {
                                "/notice"
                            };
                            self.set_status(nbeep_core::tf(
                                nbeep_core::Msg::StfGradeUsage,
                                &[name],
                            ));
                        }
                    }
                    ChatCommand::Notice(body) => {
                        return CmdOutcome::SendGraded(nbeep_core::sanitize_message(&body), 1);
                    }
                    ChatCommand::Urgent(body) => {
                        return CmdOutcome::SendGraded(nbeep_core::sanitize_message(&body), 2);
                    }
                    ChatCommand::Help => {
                        self.push_chat_notice(peer, &nbeep_core::command::help_text());
                    }
                    ChatCommand::Trust => {
                        let line = peer.map_or_else(
                            || nbeep_core::t(nbeep_core::Msg::CmdTrustGroup).to_string(),
                            |p| {
                                use nbeep_core::TrustStore as _;
                                let lv = self.trust.level(p);
                                nbeep_core::tf(
                                    nbeep_core::Msg::CmdTrustStatus,
                                    &[&self.peer_title(p), trust_label(lv)],
                                )
                            },
                        );
                        self.push_chat_notice(peer, &line);
                    }
                    ChatCommand::Verify => match peer {
                        // 08-18 사용자 확정 — 카드+버튼 대신 **직접 대조 완료**. 사람이
                        // 먼저 /fingerprint 로 지문을 다른 채널로 맞춘 뒤 부르는 전제.
                        Some(p) => {
                            use nbeep_core::TrustStore as _;
                            if self.trust.level(p) == nbeep_core::TrustLevel::FingerprintVerified {
                                self.push_chat_notice(
                                    peer,
                                    nbeep_core::t(nbeep_core::Msg::CmdVerifyAlready),
                                );
                            } else {
                                self.verify_peer(p);
                                self.push_chat_notice(
                                    peer,
                                    nbeep_core::t(nbeep_core::Msg::CmdVerifiedNow),
                                );
                            }
                        }
                        None => self
                            .push_chat_notice(peer, nbeep_core::t(nbeep_core::Msg::CmdVerify1to1)),
                    },
                    ChatCommand::Fingerprint => match peer {
                        // 상대·내 키 지문 출력(08-18) — 비교용(다른 채널로 맞춘 뒤 /verify).
                        Some(p) => {
                            let line = nbeep_core::tf(
                                nbeep_core::Msg::CmdFingerprint,
                                &[
                                    &self.identity.peer_id().short(),
                                    &self.peer_title(p),
                                    &p.short(),
                                ],
                            );
                            self.push_chat_notice(peer, &line);
                        }
                        None => self
                            .push_chat_notice(peer, nbeep_core::t(nbeep_core::Msg::CmdTrustGroup)),
                    },
                    ChatCommand::Unverify => match peer {
                        // 인증 취소 — SAS 승격을 **실제로 되돌린다**(파란 배지 강등).
                        // 카드가 열려 있으면 함께 닫는다. 1:1에서만(대상이 하나여야).
                        Some(p) => {
                            use nbeep_core::TrustStore as _;
                            if self.trust.level(p) == nbeep_core::TrustLevel::FingerprintVerified {
                                self.unverify_peer(p);
                                self.close_peer_info_card();
                                self.push_chat_notice(
                                    peer,
                                    nbeep_core::t(nbeep_core::Msg::CmdUnverifyDone),
                                );
                            } else {
                                self.close_peer_info_card();
                                self.push_chat_notice(
                                    peer,
                                    nbeep_core::t(nbeep_core::Msg::CmdUnverifyNone),
                                );
                            }
                        }
                        None => self.push_chat_notice(
                            peer,
                            nbeep_core::t(nbeep_core::Msg::CmdUnverify1to1),
                        ),
                    },
                    ChatCommand::Close => {
                        self.close_chat_view(peer);
                    }
                }
                CmdOutcome::Handled
            }
        }
    }

    /// 지문 대조 완료로 승격(08-18 · `/verify`·카드 버튼 공통) — 신뢰를
    /// `FingerprintVerified`로 올리고(write-through 영속) 목록·카드 배지를 즉시 갱신.
    /// 사람이 다른 채널로 지문을 맞춘 뒤 부르는 것이 전제(SAS 카드를 여는 대신
    /// `/fingerprint` 비교 → `/verify` 흐름 · 사용자 확정 08-18).
    fn verify_peer(&mut self, peer: PeerId) {
        use nbeep_core::TrustStore as _;
        self.trust.verify(peer);
        self.set_status(nbeep_core::tf(
            nbeep_core::Msg::StfVerified,
            &[&self.peer_title(peer)],
        ));
        let mut rinv = Invalidations::default();
        self.refresh_rows(&mut rinv);
        self.refresh_peer_info_card(peer);
    }

    /// 인증 취소(08-17 · `/unverify`·카드 버튼 공통) — SAS 승격을 되돌려 신뢰를
    /// `Pinned`로 강등(write-through 영속)하고 목록 배지를 즉시 갱신한다. verify와
    /// 대칭(승격의 역).
    fn unverify_peer(&mut self, peer: PeerId) {
        use nbeep_core::TrustStore as _;
        self.trust.unverify(peer);
        self.set_status(nbeep_core::tf(
            nbeep_core::Msg::StfUnverified,
            &[&self.peer_title(peer)],
        ));
        let mut rinv = Invalidations::default();
        self.refresh_rows(&mut rinv);
    }

    /// 목록에서 삭제(08-17 · 사용자 요청) — 핀(trust.seg)·프로필 캐시 파일·목록/미읽음
    /// 상태를 통째로 지운다. **차단이 아니다** — 상대가 다시 세션을 맺으면 처음처럼
    /// 새로 뜬다(되돌릴 수 있으니 안전한 기본). 열린 대화 세션은 건드리지 않는다
    /// (지금 대화 중인 사람을 목록에서 지워도 그 창은 그대로 — 다음 부팅에 안 뜬다).
    fn forget_peer(&mut self, peer: PeerId) {
        use nbeep_core::TrustStore as _;
        let title = self.peer_title(peer);
        self.trust.forget(peer); // 핀·이름 이력·즐겨찾기 삭제(write-through)
                                 // 프로필 캐시 파일(부팅 복원의 근거) — 지워야 다음 부팅에 안 되살아난다.
        let dir = self.data_dir.join("profiles");
        let _ = std::fs::remove_file(dir.join(format!("{}.img", peer.short())));
        let _ = std::fs::remove_file(dir.join(format!("{}.meta", peer.short())));
        self.peer_profiles.remove(&peer);
        self.extra_peers.remove(&peer); // 비발견 시드 제거(재부팅 재시드 방지)
        self.closed_peers.remove(&peer);
        self.unread.remove(&peer);
        self.last_read.remove(&peer);
        self.table.forget(peer); // 발견 테이블에서도 즉시(다시 비컨하면 새로 뜬다)
        self.set_status(nbeep_core::tf(nbeep_core::Msg::StfPeerRemoved, &[&title]));
        self.refresh_and_redraw();
    }

    /// SAS(안전 번호) 카드를 닫는다(08-17) — 열려 있으면 창·뷰·대기 요청을 지우고
    /// true. 신뢰 등급은 건드리지 않는다(카드는 표시 UI일 뿐). 열린 게 없으면 false.
    fn close_peer_info_card(&mut self) -> bool {
        self.pending_peer_info = None;
        let wid = self
            .windows
            .iter()
            .find(|(_, e)| matches!(e.role, Role::PeerInfo(_)))
            .map(|(w, _)| *w);
        let had_view = self.peer_info_view.is_some();
        self.peer_info_view = None;
        if let Some(w) = wid {
            self.windows.remove(&w);
            true
        } else {
            had_view
        }
    }

    /// 명령 결과를 **그 대화 스레드에 로컬 줄로** 남긴다(상대에게 가지 않는다).
    /// 상태바는 한 줄뿐이라 여러 줄 안내(`/help`)를 담지 못한다.
    fn push_chat_notice(&mut self, peer: Option<PeerId>, text: &str) {
        let (at_ms, wall) = now_stamp();
        let safe = nbeep_core::sanitize_message(text);
        let mut inv = Invalidations::default();
        match peer {
            Some(p) => {
                if let Some(chat) = self.chats.get_mut(&p) {
                    chat.push_line(ChatLine::text(false, safe.clone(), at_ms, wall), &mut inv);
                }
                if let Some(conv) = self.conversations.get_mut(&p) {
                    conv.lines.push(ChatLine::text(false, safe, at_ms, wall));
                }
                self.record_history(p); // 대화 기록 영속(M2-5b)
            }
            None => {
                if let Some(gid) = self.single_open_group {
                    if let Some(gc) = self.gchats.get_mut(&gid) {
                        gc.push_line(ChatLine::text(false, safe, at_ms, wall), &mut inv);
                    }
                }
            }
        }
    }

    /// `/close` — 열려 있는 대화 뷰를 닫는다(단일 모드는 목록 복귀 · 별도 창은 창 닫기).
    fn close_chat_view(&mut self, peer: Option<PeerId>) {
        let _ = peer;
        self.single_open = None;
        self.single_open_group = None;
        self.set_status(nbeep_core::t(nbeep_core::Msg::StChatClosed));
        self.refresh_and_redraw();
    }

    fn open_peer_info(&mut self, peer: PeerId, el: &ActiveEventLoop) {
        // ★ Pull(M3-21 · 사용자 확정 08-14) — **카드를 여는 순간이 가장 신선해야
        //   하는 순간**이라 세션이 살아 있으면 최신을 1회 요청한다(자동 반복 아님 —
        //   사용자 행위 트리거 · 13 §12-1 무관). 캐시를 먼저 보여주고(아래) 응답이
        //   오면 `refresh_peer_info_card`가 갱신한다. 구버전 발신자(push 미배선)의
        //   낡은 캐시도 이 경로로 회복된다.
        if let Some(conv) = self.conversations.get(&peer) {
            let _ = conv.out_tx.send(SessionCmd::Control(vec![
                nbeep_core::ProfileMsg::Request.encode()
            ]));
        } else if self.live {
            // 세션 없음(점이 녹색 아님 · 08-14 사용자 요청) — **조용한 연결**을 시도
            // 한다(`auto=true` = 성립해도 대화 뷰를 열지 않는다 — 08-14 실기: false로
            // 걸었더니 프로필 보기가 대화창을 자동으로 열었다. 카드와 대화는 분리).
            // 성립하면 자동 프리페치(install_conversation)가 돌고, 응답 도착이
            // `refresh_peer_info_card`로 열린 카드를 채운다. 실패는 기존 연결 실패
            // 경로 그대로(상태바) — 카드는 캐시로 남는다.
            self.start_connect(peer, true);
            self.set_status(nbeep_core::tf(
                nbeep_core::Msg::StfConnectingProfile,
                &[&self.peer_title(peer)],
            ));
        }
        let info = self.build_peer_info(peer);
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
            .with_title(format!(
                "Nexa Beep — {}",
                nbeep_core::t(nbeep_core::Msg::WinPeerProfile)
            ))
            // 높이 500(M3-6 — 안전 번호 2줄+버튼이 380에선 지문·안내와 겹쳤다 · 실기).
            .with_inner_size(winit::dpi::LogicalSize::new(360.0, 500.0))
            .with_resizable(false)
            .with_window_icon(self.icon.clone());
        // 마우스 위치 + 메인 소유(08-15 — 내 프로필과 같은 카드 규약. 우클릭한
        // 그 자리에서 열린다).
        let attrs = self.modal_attrs(attrs, true);
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

    /// 확대 미리보기 열기(08-16 · M4-5 잔여) — 단일 창(peer_info 패턴): 이미
    /// 열려 있으면 내용 교체·포커스. 디코드는 워커(imgdec 격리 · 1024px 상한).
    fn open_image_view(
        &mut self,
        el: &ActiveEventLoop,
        src: WindowId,
        qpath: String,
        title: String,
    ) {
        // 진원이 바뀌면 창을 버리고 새로 만든다(08-18 — 소유자 불변 제약).
        let owner_ok = self.image_view.as_ref().is_some_and(|v| v.owner == src);
        if !owner_ok {
            if let Some((wid, _)) = self.windows.iter().find(|(_, e)| e.role == Role::ImageView) {
                let wid = *wid;
                self.windows.remove(&wid);
            }
        }
        let same = self
            .image_view
            .as_ref()
            .is_some_and(|v| v.qpath == qpath && !matches!(v.img, ImgLoad::Failed));
        if same && owner_ok {
            // 같은 이미지가 이미 열려 있다 — 재디코드 없이 창만 앞으로.
            if let Some((_, e)) = self.windows.iter().find(|(_, e)| e.role == Role::ImageView) {
                e.window.focus_window();
            }
            return;
        }
        let keep_img = if same {
            // 같은 이미지·다른 진원 — 디코드 결과는 재사용하고 창만 다시 만든다.
            self.image_view.take().map(|v| v.img)
        } else {
            None
        };
        self.image_view = Some(ImageViewState {
            qpath: qpath.clone(),
            img: keep_img.unwrap_or(ImgLoad::Loading),
            owner: src,
        });
        if matches!(
            self.image_view.as_ref().map(|v| &v.img),
            Some(ImgLoad::Loading)
        ) {
            let qp = qpath.clone();
            let secret = self.identity.wrap_secret();
            spawn_decode(
                self.proxy.clone(),
                DecodeTarget::FullImage(qpath),
                move || {
                    crate::imgdec::thumb_raw_from_beepq(std::path::Path::new(&qp), 1024, &secret)
                },
            );
        }
        if let Some((wid, _)) = self.windows.iter().find(|(_, e)| e.role == Role::ImageView) {
            let wid = *wid;
            if let Some(e) = self.windows.get_mut(&wid) {
                e.window
                    .set_title(&nbeep_core::tf(nbeep_core::Msg::WinPreview, &[&title]));
                e.window.focus_window();
            }
            self.request_redraw(wid);
            return;
        }
        let attrs = Window::default_attributes()
            .with_title(nbeep_core::tf(nbeep_core::Msg::WinPreview, &[&title]))
            .with_inner_size(winit::dpi::LogicalSize::new(640.0, 560.0))
            .with_window_icon(self.icon.clone());
        // 커서 자리 + **연 창 소유**(08-18 — 격리함에서 열면 격리함 소유:
        // 메인이 위로 튀지 않고, 닫으면 격리함으로 복귀).
        let attrs = self.modal_attrs_from(Some(src), attrs, true);
        let window = Rc::new(el.create_window(attrs).unwrap());
        let scale = window.scale_factor() as f32;
        let context = softbuffer::Context::new(window.clone()).unwrap();
        let surface = SbSurface::new(&context, window.clone()).unwrap();
        let id = window.id();
        self.windows.insert(
            id,
            WinEntry {
                role: Role::ImageView,
                window,
                surface,
                cursor: (0, 0),
                scale,
            },
        );
        self.request_redraw(id);
    }

    /// 확대 미리보기 닫기(뷰·창 동시 정리 — close_profile과 같은 문법).
    fn close_image_view(&mut self) {
        self.image_view = None;
        if let Some((wid, _)) = self.windows.iter().find(|(_, e)| e.role == Role::ImageView) {
            let wid = *wid;
            self.windows.remove(&wid);
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
            .with_title(format!(
                "Nexa Beep — {}",
                nbeep_core::t(nbeep_core::Msg::WinConnectAddr)
            ))
            .with_inner_size(winit::dpi::LogicalSize::new(380.0, 150.0))
            .with_resizable(false)
            // 앱 모달(08-14 표준 재정리) — 앱 창 입력은 모달이 흡수하되, 다른 앱은
            // 자유롭게 위로 온다(AlwaysOnTop 금지 — OS 창 전환 관례).
            .with_window_icon(self.icon.clone());
        let attrs = self.modal_attrs(attrs, false); // 메인 소유(08-15 — 창 묶음 부상)
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
            let _ = base; // 절대화(08-18 2차) — 슬롯 상대 기준 폐지(전 슬롯 Normal 16)
            let raw = settings.get(&format!("font.{region}.size"));
            // 숫자 = **절대 논리 px**. 구 코드(s/m/l/xl)는 부팅 이관이 바꾸지만
            // 이관 전 경로 대비 관용 해석을 남긴다(같은 표).
            let size = if let Ok(px) = raw.parse::<f32>() {
                px.clamp(8.0, 48.0)
            } else {
                match raw {
                    "s" => 14.0,
                    "l" => 18.0,
                    "xl" => 22.0,
                    _ => 16.0, // "m"·미설정 = Normal
                }
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
    /// 내 아바타 RGBA 합성(트레이 아이콘 · M3-2a) — 투명 바탕 → 시드 색 원(1px AA)
    /// → (사진 > 내장) **박스 평균 축소** 알파 블렌드 → 보더 링(소형 규약).
    /// 이니셜 글자는 생략(트레이 크기에서 판독성 낮음 — 툴팁·메뉴가 이름을 보인다).
    #[allow(clippy::cast_possible_truncation, clippy::cast_precision_loss)]
    #[allow(clippy::cast_sign_loss)]
    fn my_avatar_rgba(&self, side: u32) -> Vec<u8> {
        use nbeep_core::avatar::{parse_border, AvatarChoice};
        let n = side as usize;
        let mut out = vec![0u8; n * n * 4];
        let seed = self.identity.peer_id().as_bytes().to_vec();
        let base = nbeep_ui::avatar::avatar_color(&seed).0;
        let (cr, cg, cb) = (
            ((base >> 16) & 0xFF) as u8,
            ((base >> 8) & 0xFF) as u8,
            (base & 0xFF) as u8,
        );
        let c = side as f32 / 2.0;
        // ① 시드 색 원.
        for y in 0..n {
            for x in 0..n {
                let d = (((x as f32 + 0.5) - c).powi(2) + ((y as f32 + 0.5) - c).powi(2)).sqrt();
                let cov = (c - d + 0.5).clamp(0.0, 1.0);
                if cov > 0.0 {
                    let i = (y * n + x) * 4;
                    out[i] = cr;
                    out[i + 1] = cg;
                    out[i + 2] = cb;
                    out[i + 3] = (cov * 255.0).round() as u8;
                }
            }
        }
        // ② 그림(사진 > 내장) — 박스 평균 축소 + over 블렌드.
        let img = self.my_avatar.clone().or_else(|| {
            match AvatarChoice::parse(self.settings.get("profile.avatar")) {
                AvatarChoice::Builtin(k) => self.builtin_avatars.get(&k).cloned(),
                _ => None,
            }
        });
        if let Some(img) = img {
            let (sw, sh) = (img.w as usize, img.h as usize);
            if sw > 0 && sh > 0 {
                for y in 0..n {
                    for x in 0..n {
                        let sx0 = x * sw / n;
                        let sx1 = ((x + 1) * sw).div_ceil(n).clamp(sx0 + 1, sw);
                        let sy0 = y * sh / n;
                        let sy1 = ((y + 1) * sh).div_ceil(n).clamp(sy0 + 1, sh);
                        let (mut r, mut g, mut b, mut a, mut cnt) = (0f32, 0f32, 0f32, 0f32, 0f32);
                        for sy in sy0..sy1 {
                            for sx in sx0..sx1 {
                                let si = (sy * sw + sx) * 4;
                                let av = f32::from(img.rgba[si + 3]) / 255.0;
                                r += f32::from(img.rgba[si]) * av;
                                g += f32::from(img.rgba[si + 1]) * av;
                                b += f32::from(img.rgba[si + 2]) * av;
                                a += av;
                                cnt += 1.0;
                            }
                        }
                        let aa = if cnt > 0.0 { a / cnt } else { 0.0 };
                        if aa <= 0.0 {
                            continue;
                        }
                        let inv = 1.0 / a;
                        let (pr, pg, pb) = (r * inv, g * inv, b * inv);
                        let i = (y * n + x) * 4;
                        let da = f32::from(out[i + 3]) / 255.0;
                        let oa = aa + da * (1.0 - aa);
                        if oa > 0.0 {
                            out[i] = ((pr * aa + f32::from(out[i]) * da * (1.0 - aa)) / oa)
                                .round()
                                .clamp(0.0, 255.0) as u8;
                            out[i + 1] = ((pg * aa + f32::from(out[i + 1]) * da * (1.0 - aa)) / oa)
                                .round()
                                .clamp(0.0, 255.0) as u8;
                            out[i + 2] = ((pb * aa + f32::from(out[i + 2]) * da * (1.0 - aa)) / oa)
                                .round()
                                .clamp(0.0, 255.0) as u8;
                            out[i + 3] = (oa * 255.0).round() as u8;
                        }
                    }
                }
            }
        }
        // ③ 보더 링 — 가장자리 안쪽 밴드(소형 2px 상당 · 32px 기준).
        if let Some((rr, rg, rb)) = parse_border(self.settings.get("profile.avatar_border")) {
            let w = (side as f32 / 16.0).max(2.0);
            for y in 0..n {
                for x in 0..n {
                    let d = (((x as f32 + 0.5) - c).powi(2) + ((y as f32 + 0.5) - c).powi(2))
                        .sqrt()
                        - c; // 음수 = 안쪽
                    let ring = (d + w / 2.0).abs() - w / 2.0;
                    let cov = (0.5 - ring).clamp(0.0, 1.0);
                    if cov > 0.0 {
                        let i = (y * n + x) * 4;
                        let da = f32::from(out[i + 3]) / 255.0;
                        let oa = cov + da * (1.0 - cov);
                        if oa > 0.0 {
                            out[i] = ((f32::from(rr) * cov + f32::from(out[i]) * da * (1.0 - cov))
                                / oa)
                                .round()
                                .clamp(0.0, 255.0) as u8;
                            out[i + 1] = ((f32::from(rg) * cov
                                + f32::from(out[i + 1]) * da * (1.0 - cov))
                                / oa)
                                .round()
                                .clamp(0.0, 255.0) as u8;
                            out[i + 2] = ((f32::from(rb) * cov
                                + f32::from(out[i + 2]) * da * (1.0 - cov))
                                / oa)
                                .round()
                                .clamp(0.0, 255.0) as u8;
                            out[i + 3] = (oa * 255.0).round() as u8;
                        }
                    }
                }
            }
        }
        out
    }

    /// 트레이 표시 내용(M3-2a) — 아이콘 32px 아바타 · 툴팁/헤더 = 표시 이름 ·
    /// 라벨 = i18n(언어 전환 시 refresh_tray로 재주입).
    fn tray_content(&self) -> nbeep_plat::tray::TrayContent {
        use nbeep_core::{t, Msg};
        let name = effective_display_name(&self.settings, &self.identity.peer_id())
            .as_str()
            .to_string();
        nbeep_plat::tray::TrayContent {
            rgba: self.my_avatar_rgba(32),
            side: 32,
            tooltip: format!("Nexa Beep — {name}"),
            name,
            open_label: t(Msg::TrayOpen).to_string(),
            quit_label: t(Msg::TrayQuit).to_string(),
        }
    }

    /// 트레이 갱신(아바타·보더·표시 이름 변경 동기 — refresh_toolbar_avatar 깔때기).
    fn refresh_tray(&self) {
        if let Some(t) = &self.tray {
            t.update(self.tray_content());
        }
    }

    /// 툴바 프로필 버튼 = **지금의 내 얼굴**(08-14 사용자 요청) — 우선순위는 프로필
    /// 프리뷰와 동일(사진 > 내장 > 이니셜/빈 원) + 보더 링(소형 2px).
    fn refresh_toolbar_avatar(&mut self) {
        use nbeep_core::avatar::{parse_border, AvatarChoice};
        let choice = AvatarChoice::parse(self.settings.get("profile.avatar"));
        let img = self.my_avatar.clone().or_else(|| match &choice {
            AvatarChoice::Builtin(k) => self.builtin_avatars.get(k.as_str()).cloned(),
            _ => None,
        });
        let initials = if matches!(choice, AvatarChoice::None) && img.is_none() {
            String::new() // 없음 = 빈 원
        } else {
            nbeep_ui::avatar::initials(
                effective_display_name(&self.settings, &self.identity.peer_id()).as_str(),
            )
        };
        let border = parse_border(self.settings.get("profile.avatar_border")).map(|(r, g, b)| {
            nbeep_ui::Color((u32::from(r) << 16) | (u32::from(g) << 8) | u32::from(b))
        });
        let mut inv = Invalidations::default();
        self.toolbar.set_item_icon(
            "profile",
            ToolIcon::Avatar {
                img,
                initials,
                seed: self.identity.peer_id().as_bytes().to_vec(),
                border,
            },
            &mut inv,
        );
        self.refresh_tray(); // 트레이 아이콘·툴팁도 같은 깔때기로(M3-2a)
        if let Some(mid) = self.main_id {
            self.request_redraw(mid);
        }
    }

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
        // ★ PII는 cfg에 안 쓴다(08-17 평문 3면 ② — 이메일·전화는 사람이 읽는
        //   설정 파일에 평문으로 남길 값이 아니다). 값은 메모리 Entry에 그대로
        //   있고(화면·전파 경로 무변경), 영속은 봉인 사이드카(profile.sec)가 맡는다.
        //   known에서 빠지므로 cfg에서 사라진다(구본 평문의 제거 = 이 저장 1회).
        pairs.retain(|(k, _)| !crate::gate::PII_KEYS.contains(k));
        if let Err(e) = self.conf.save(&pairs) {
            let path = self.conf.path().display();
            if exiting {
                eprintln!("설정 저장 실패(종료 경로) — {path}: {e}");
            } else {
                self.status =
                    nbeep_core::tf(nbeep_core::Msg::StfSettingsSaveFail, &[&e.to_string()]);
            }
        }
        // 사이드카는 pairs(설정 빌림) 소비 후 — cfg와 같은 저장 사이클에 봉인 기록.
        self.save_pii_sidecar();
    }

    /// PII 봉인 사이드카 저장(08-17 평문 3면 ②) — `data/profile.sec`.
    /// 형식: 봉인 평문 = `key\tvalue` 줄들(빈 값 키는 생략 · 전부 비면 파일 삭제).
    fn save_pii_sidecar(&mut self) {
        let path = self.data_dir.join("profile.sec");
        let mut body = String::new();
        for k in crate::gate::PII_KEYS {
            let v = self.settings.get(k).to_string();
            if !v.is_empty() {
                body.push_str(k);
                body.push('\t');
                body.push_str(&v);
                body.push('\n');
            }
        }
        if body.is_empty() {
            let _ = std::fs::remove_file(&path); // 철회 대칭 — 빈 봉투를 남기지 않는다
            return;
        }
        match nbeep_store::sealed::seal(
            crate::gate::SEAL_PII,
            &self.identity.wrap_secret(),
            body.as_bytes(),
        ) {
            Ok(env) => {
                if std::fs::write(&path, env).is_err() {
                    self.set_status(nbeep_core::t(nbeep_core::Msg::StContactSaveFail));
                }
            }
            Err(_) => {
                // 봉인 실패 = 평문 폴백 없음(원칙) — 소리 내어 알린다.
                self.set_status(nbeep_core::t(nbeep_core::Msg::StContactSealFail));
            }
        }
    }

    /// PII 사이드카 로드(부팅 · 08-17) — 열리면 메모리 Entry에 주입(사이드카가
    /// cfg 구본보다 우선). cfg에 남은 구본 평문은 다음 저장에서 제거된다(known
    /// 제외 — 이관은 저장 1회로 완결).
    fn load_pii_sidecar(&mut self) {
        let path = self.data_dir.join("profile.sec");
        let Ok(raw) = std::fs::read(&path) else {
            return;
        };
        let Some(body) =
            nbeep_store::sealed::open(crate::gate::SEAL_PII, &self.identity.wrap_secret(), &raw)
        else {
            // 다른 신원·손상 — fail-closed(평문인 척 읽지 않는다). 값은 비워 둔다.
            self.set_status(nbeep_core::t(nbeep_core::Msg::StContactOpenFail));
            return;
        };
        if let Ok(text) = String::from_utf8(body) {
            for line in text.lines() {
                if let Some((k, v)) = line.split_once('\t') {
                    // &'static 키로 매칭해 주입(set의 키 수명 요구).
                    for &sk in crate::gate::PII_KEYS {
                        if sk == k {
                            self.settings.set(sk, v.to_string());
                        }
                    }
                }
            }
        }
    }

    /// 영속 설정의 부팅 반영(M3-15) — 파생 런타임 상태를 가진 키만.
    /// [`Self::apply_settings`]의 부팅판: status 문구·redraw 없이 상태만 맞춘다
    /// (`ui.language`·`ui.scrollbar_hide`는 run()이 창 생성 전에 이미 반영).
    /// 부팅 시 프로필 캐시 복원(08-14 사용자 요청 — 재시작하면 아는 상대가 빈 지문
    /// 행으로 시작하던 것). **핀 = "연결됐던 기록"**이 열쇠다: trust.seg 핀별로 마지막
    /// 이름(`record_name` 이력)·내장 아바타/보더(`profiles/{peer}.meta`)·이미지
    /// (`{peer}.img`)를 미리 채우고, live 모드면 목록 행도 시드한다(비발견 상대 유지
    /// ④의 부팅판 — 발견이 닿으면 테이블 항목이 이긴다 · refresh_rows 병합 규칙).
    /// 갱신·철회는 기존 규칙 그대로 — 세션이 서면 자동 프리페치가 최신으로 덮는다.
    /// **부팅 자동 연결은 하지 않는다**: 핀 N개 전원 아웃바운드는 전원이 켤 때마다
    /// 서로를 부르는 부팅 풀메시(N² 상시 세션)가 된다(13 §12-1 — 캐시로 충분).
    /// 사용자 사진 **관리 복사**(M3-17 잔여 `c:` 이관 · 08-16) — 선택한 원본을
    /// `data/profiles/custom/`으로 복사하고 그 사본 경로를 쓴다. 원본을 옮기거나
    /// 지워도 프로필·최근 목록·재전송이 깨지지 않는다(종전엔 원본 절대 경로를
    /// 매번 읽었다). 이름 = 내용 해시 8자리 + 정제된 원본 이름 — **같은 사진
    /// 재선택 = 같은 사본 재사용**(중복 누적 없음). 실패 = 원본 경로 그대로
    /// (fail-soft — 종전 동작과 동일 · 포터블 이동 시에도 data/가 통째로 간다).
    fn manage_profile_image(&self, path: &str) -> String {
        let dir = self.data_dir.join("profiles").join("custom");
        let p = std::path::Path::new(path);
        if p.starts_with(&dir) {
            return path.to_string(); // 이미 관리 사본(최근 목록 재선택)
        }
        let Ok(bytes) = std::fs::read(p) else {
            return path.to_string();
        };
        let h = nbeep_crypto::sha256(&bytes);
        let tag: String = h[..4].iter().map(|b| format!("{b:02x}")).collect();
        let orig = p
            .file_name()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_default();
        let safe: String = orig
            .chars()
            .filter(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_'))
            .take(48)
            .collect();
        let name = if safe.is_empty() {
            format!("{tag}.img")
        } else {
            format!("{tag}-{safe}")
        };
        let dst = dir.join(name);
        if !dst.is_file()
            && (std::fs::create_dir_all(&dir).is_err() || std::fs::write(&dst, &bytes).is_err())
        {
            return path.to_string();
        }
        dst.to_string_lossy().into_owned()
    }

    /// 내 프로필 사진의 와이어 축소본 경로(`data/profiles/me.wire.png` · 08-16).
    fn wire_avatar_path(&self) -> std::path::PathBuf {
        self.data_dir.join("profiles").join("me.wire.png")
    }

    /// 원본 사진이 와이어 상한(256KiB)을 넘으면 **워커에서** imgdec 축소본을 굽는다
    /// (08-16 — 자식 프로세스 왕복은 메인에서 돌리지 않는다 · M4-5 결). 완료는
    /// [`AppEvent::WireAvatar`]로 돌아와 Full push(실사진 전파)로 이어진다.
    /// 상한 이내 원본·이미 준비된 축소본은 아무것도 하지 않는다.
    fn ensure_wire_avatar(&mut self) {
        use nbeep_core::PROFILE_IMAGE_MAX;
        let path = self.settings.get("profile.image_path").to_string();
        if path.is_empty() {
            return;
        }
        let over = std::fs::metadata(&path)
            .is_ok_and(|m| usize::try_from(m.len()).unwrap_or(usize::MAX) > PROFILE_IMAGE_MAX);
        if !over {
            return; // 원본이 그대로 실려 나간다 — 축소본 불필요
        }
        if self.wire_avatar_path().is_file() {
            return; // 이미 준비됨(부팅 경로 재진입)
        }
        if self.wire_pending {
            return; // 워커가 이미 도는 중(중복 스폰 금지 — 13 §12-1)
        }
        self.wire_pending = true;
        self.wire_pending_ms = self.now_ms(); // watchdog 기준점(RL-11)
        let gen = self.wire_gen;
        let proxy = self.proxy.clone();
        std::thread::spawn(move || {
            // 파일 쓰기는 메인 몫 — 워커가 직접 쓰면 그 사이 사진이 바뀐 낡은
            // 세대가 새 축소본을 덮을 수 있다(세대 판정은 메인 단일 지점).
            let png = std::fs::read(&path)
                .ok()
                .and_then(|b| crate::imgdec::wire_png_from_bytes(&b, 256));
            let _ = proxy.send_event(AppEvent::WireAvatar { gen, png });
        });
    }

    /// 기록 세그먼트 개봉(&self — 셰레딩 · 08-21): **데이터 키 우선**, 실패 시
    /// 레거시(신원 키 파생 — 08-21 이전 세그). 마이그레이션은 쓰기 경로가 자연히
    /// 한다(record가 항상 데이터 키로 재봉인 — 메시지 하나만 오가면 승격).
    fn history_open_bytes(&self, stem: &str, raw: &[u8]) -> Option<Vec<u8>> {
        if let Some(k) = self.datakeys.get(stem) {
            if let Some(p) = nbeep_store::sealed::open(crate::gate::SEAL_HISTORY, &k, raw) {
                return Some(p);
            }
        }
        nbeep_store::sealed::open(crate::gate::SEAL_HISTORY, &self.identity.wrap_secret(), raw)
    }

    /// 대화 기록 봉인 저장(M2-5b) — sealed(history-v1 · **대화별 데이터 키** —
    /// 셰레딩 D-18 §7 08-21) → 원자적 `data/history/{short}.seg`.
    /// 빈 스레드 = 파일 삭제 **+ 키 폐기**(지우기 = 셰레딩). 봉인 실패 = 저장 포기.
    fn record_history(&mut self, peer: PeerId) {
        let Some(conv) = self.conversations.get(&peer) else {
            return;
        };
        let dir = self.data_dir.join("history");
        let stem = peer.short();
        let path = dir.join(format!("{stem}.seg"));
        let plain = encode_history(&conv.lines);
        if plain.is_empty() {
            let _ = std::fs::remove_file(&path);
            self.datakeys.destroy(&stem);
            return;
        }
        let key = self.datakeys.get_or_create(&stem);
        let Ok(env) = nbeep_store::sealed::seal(crate::gate::SEAL_HISTORY, &key, &plain) else {
            self.set_status(nbeep_core::t(nbeep_core::Msg::StHistorySealFail));
            return;
        };
        if std::fs::create_dir_all(&dir).is_ok() {
            let tmp = path.with_extension(format!("tmp.{}", std::process::id()));
            if std::fs::write(&tmp, &env).is_ok() {
                let _ = std::fs::rename(&tmp, &path);
            }
        }
    }

    /// 1:1 오프라인 대기 큐 영속(M4-6 · 08-20 — 재시작 유지 사용자 확정).
    /// history와 같은 문법: SEAL_PENDING 봉인 · 원자적 쓰기 · 빈 큐 = 파일 제거.
    fn save_pending(&mut self, peer: PeerId) {
        let dir = self.data_dir.join("pending");
        let path = dir.join(format!("{}.seg", peer.short()));
        let plain = self
            .pending_direct
            .get(&peer)
            .map(|q| encode_pending(q))
            .unwrap_or_default();
        if plain.is_empty() {
            let _ = std::fs::remove_file(&path);
            return;
        }
        let Ok(env) = nbeep_store::sealed::seal(
            crate::gate::SEAL_PENDING,
            &self.identity.wrap_secret(),
            &plain,
        ) else {
            return; // 봉인 실패 — 평문 저장은 하지 않는다(다음 변경에서 재시도)
        };
        if std::fs::create_dir_all(&dir).is_ok() {
            let tmp = path.with_extension(format!("tmp.{}", std::process::id()));
            if std::fs::write(&tmp, &env).is_ok() {
                let _ = std::fs::rename(&tmp, &path);
            }
        }
    }

    /// 부팅 시 대기 큐 복원(M4-6) — restore_history와 같은 지문 매핑(핀 상대만).
    /// 복원분은 대기 풍선으로도 살아난다(`parked_lines`에 queued 줄 append).
    fn restore_pending(&mut self) {
        let secret = self.identity.wrap_secret();
        let dir = self.data_dir.join("pending");
        let Ok(rd) = std::fs::read_dir(&dir) else {
            return;
        };
        let recs = self.trust.export();
        for ent in rd.flatten() {
            let path = ent.path();
            if path.extension().and_then(|x| x.to_str()) != Some("seg") {
                continue;
            }
            let Some(stem) = path.file_stem().and_then(|s| s.to_str()) else {
                continue;
            };
            let Some(peer) = recs
                .iter()
                .find(|r| !r.blocked && r.peer.short() == stem)
                .map(|r| r.peer)
            else {
                continue; // 핀 없는/차단된 상대 — 매핑 불가(파일은 남긴다 · fail-soft)
            };
            let Some(plain) = std::fs::read(&path).ok().and_then(|raw| {
                nbeep_store::sealed::open(crate::gate::SEAL_PENDING, &secret, &raw)
            }) else {
                continue; // 다른 신원의 봉투 — 열 수 없다(fail-closed)
            };
            let q = decode_pending(&plain);
            if q.is_empty() {
                continue;
            }
            // 대기 풍선 복원 — 기록(parked)에 이어 붙인다(대기분은 항상 최신이다).
            let parked = self.parked_lines.entry(peer).or_default();
            for m in &q {
                parked.push(
                    ChatLine::text(
                        true,
                        nbeep_core::sanitize_message(&m.text),
                        m.at_ms,
                        wall_from_ms(m.at_ms),
                    )
                    .with_queued(true),
                );
            }
            self.pending_direct.insert(peer, q);
            // 상대가 보이면 자동 전달 — 백오프를 즉시 후보로 걸어 둔다(발견 시 재시도).
            self.reconnect.entry(peer).or_insert((0, 0));
        }
    }

    /// 1:1 대기 flush(M4-6) — 세션 성립 합류점에서 호출(그룹 flush와 같은 자리).
    /// fresh seq로 실제 발신하고, 열린 뷰의 대기 풍선을 "전송됨"으로 푼다.
    fn flush_direct_sends(&mut self, peer: PeerId) {
        let Some(q) = self.pending_direct.remove(&peer) else {
            return;
        };
        if q.is_empty() {
            return;
        }
        let mut sent = 0usize;
        let mut inv = Invalidations::default();
        for m in &q {
            let seq = self.seq.issue();
            let msg = nbeep_core::ChatMessage {
                sender_device: self.identity.peer_id(),
                seq,
                body: nbeep_core::MessageBody::Text(m.text.clone()),
                importance: match m.importance & 0x3 {
                    2 => nbeep_core::Importance::Urgent,
                    1 => nbeep_core::Importance::Notice,
                    _ => nbeep_core::Importance::Normal,
                },
                broadcast: m.importance & 0x4 != 0, // 대기 편입 공지 표식(08-21)
            };
            let Some(conv) = self.conversations.get_mut(&peer) else {
                break;
            };
            if conv.out_tx.send(SessionCmd::Chat(msg.encode())).is_err() {
                break; // 세션이 곧바로 죽음 — 남은 것은 아래에서 도로 대기
            }
            // conv.lines의 대기 줄을 확정으로(뷰가 닫혔다 열려도 기록이 맞도록).
            if let Some(l) = conv
                .lines
                .iter_mut()
                .find(|l| l.mine && l.queued && l.at_ms == m.at_ms)
            {
                l.queued = false;
                l.seq = seq;
            } else {
                conv.lines.push(
                    ChatLine::text(
                        true,
                        nbeep_core::sanitize_message(&m.text),
                        m.at_ms,
                        wall_from_ms(m.at_ms),
                    )
                    .with_seq(seq),
                );
            }
            if let Some(chat) = self.chats.get_mut(&peer) {
                chat.resolve_queued(m.at_ms, seq, &mut inv);
            }
            self.ledger.note_sent(peer);
            sent += 1;
        }
        // 못 보낸 잔여(세션 급사)는 도로 대기 — 유실 금지.
        if sent < q.len() {
            self.pending_direct.insert(peer, q[sent..].to_vec());
        }
        self.save_pending(peer);
        if sent > 0 {
            self.record_history(peer);
            self.set_status(nbeep_core::tf(
                nbeep_core::Msg::StfQueuedFlushed,
                &[&sent.to_string()],
            ));
            self.redraw_conversation(peer);
        }
    }

    /// 그룹 대화 기록 영속(08-19 — M3-23의 `g-*.seg` 축 · 사용자 실기 "재시작하면
    /// 기록이 사라진다"). 1:1 [`Self::record_history`]와 같은 문법: 같은 봉인 도메인
    /// (`SEAL_HISTORY`) · 원자적 쓰기 · 파일명 = `g-{uid.short()}.seg`(uid = 전역
    /// 진실 — 로컬 gid는 표시용 키라 파일명에 쓰지 않는다).
    fn record_group_history(&mut self, gid: nbeep_core::group::GroupId) {
        let Some(uid) = self.groups.shared_by_id(gid).map(|s| s.roster.uid) else {
            return; // 로컬(동보) 그룹 — 공유 방만 영속(스레드도 공유 방에만 있다)
        };
        let dir = self.data_dir.join("history");
        let stem = format!("g-{}", uid.short());
        let path = dir.join(format!("{stem}.seg"));
        let plain = self
            .group_threads
            .get(&gid)
            .map(|l| encode_history(l))
            .unwrap_or_default();
        if plain.is_empty() {
            let _ = std::fs::remove_file(&path);
            self.datakeys.destroy(&stem); // 지우기 = 셰레딩(1:1과 동일 규약)
            return;
        }
        let key = self.datakeys.get_or_create(&stem);
        let Ok(env) = nbeep_store::sealed::seal(crate::gate::SEAL_HISTORY, &key, &plain) else {
            self.set_status(nbeep_core::t(nbeep_core::Msg::StHistorySealFail));
            return;
        };
        if std::fs::create_dir_all(&dir).is_ok() {
            let tmp = path.with_extension(format!("tmp.{}", std::process::id()));
            if std::fs::write(&tmp, &env).is_ok() {
                let _ = std::fs::rename(&tmp, &path);
            }
        }
    }

    /// 부팅 그룹 기록 복원(08-19) — 공유 그룹의 `g-{uid}.seg`를 개봉해
    /// `group_threads`에 넣는다(방 열면 바로 보인다). **소속 없는 고아 세그먼트는
    /// 정리**한다(탈퇴·해산 뒤 남은 파일 — 방이 없으면 열 수도 없다).
    fn restore_group_history(&mut self) {
        let dir = self.data_dir.join("history");
        // uid.short() → local_id 매핑(공유 그룹 전수 — Invited도 복원해 두면 수락
        // 직후 바로 보인다).
        let by_short: HashMap<String, nbeep_core::group::GroupId> = self
            .groups
            .shared_list()
            .iter()
            .map(|s| (s.roster.uid.short(), s.local_id))
            .collect();
        let Ok(rd) = std::fs::read_dir(&dir) else {
            return;
        };
        for e in rd.flatten() {
            let path = e.path();
            if path.extension().and_then(|x| x.to_str()) != Some("seg") {
                continue;
            }
            let Some(stem) = path.file_stem().and_then(|s| s.to_str()) else {
                continue;
            };
            let Some(short) = stem.strip_prefix("g-") else {
                continue; // 1:1 기록 — restore_history 몫
            };
            let Some(&gid) = by_short.get(short) else {
                let _ = std::fs::remove_file(&path); // 고아(탈퇴·해산 잔재) 정리
                continue;
            };
            let Some(bytes) = std::fs::read(&path)
                .ok()
                .and_then(|raw| self.history_open_bytes(stem, &raw))
            else {
                continue; // 개봉 실패(다른 신원 봉인) — fail-closed(보존·미표시)
            };
            let lines = decode_history(&bytes);
            if !lines.is_empty() {
                self.group_threads.insert(gid, lines);
            }
        }
    }

    /// 부팅 대화 기록 복원(M2-5b) — 개봉해 parked_lines에(대화창 열면 뜨고,
    /// 재연결하면 install_conversation이 이어받는다). 차단 상대·개봉 실패는 건너뜀.
    fn restore_history(&mut self) {
        let dir = self.data_dir.join("history");
        let Ok(rd) = std::fs::read_dir(&dir) else {
            return;
        };
        let recs = self.trust.export();
        for e in rd.flatten() {
            let path = e.path();
            if path.extension().and_then(|x| x.to_str()) != Some("seg") {
                continue;
            }
            let Some(short) = path.file_stem().and_then(|s| s.to_str()) else {
                continue;
            };
            let Some(peer) = recs
                .iter()
                .find(|r| !r.blocked && r.peer.short() == short)
                .map(|r| r.peer)
            else {
                continue; // 핀 없는/차단된 기록 — 매핑 불가 or 제외
            };
            let Some(bytes) = std::fs::read(&path)
                .ok()
                .and_then(|raw| self.history_open_bytes(short, &raw))
            else {
                continue;
            };
            let mut lines = decode_history(&bytes);
            if lines.is_empty() {
                continue;
            }
            // 1:1 수신 줄 발신자 라벨 소급(08-19 사용자 요청 — 수신 풍선 위 이름).
            // 1:1은 수신 발신자가 항상 그 상대라 라벨 없는 옛 기록에도 안전하게
            // 붙일 수 있다(그룹은 줄별 발신자를 몰라 소급 불가 — tag 3 저장분만).
            let title = self.peer_title(peer);
            for l in &mut lines {
                if !l.mine && l.from.is_none() {
                    l.from = Some(title.clone());
                }
            }
            self.parked_lines.insert(peer, lines);
            if self.live {
                self.extra_peers
                    .entry(peer)
                    .or_insert_with(|| nbeep_core::default_display_name(None, &peer));
            }
        }
    }

    fn restore_cached_profiles(&mut self) {
        let dir = self.data_dir.join("profiles");
        for rec in self.trust.export() {
            if rec.blocked {
                continue; // 차단 상대는 복원 대상이 아니다(목록·캐시 모두)
            }
            let peer = rec.peer;
            let (key, border) = std::fs::read_to_string(dir.join(format!("{}.meta", peer.short())))
                .map(|s| parse_profile_meta(&s))
                .unwrap_or((None, None));
            let avatar = key
                .as_deref()
                .and_then(|k| self.builtin_avatars.get(k).cloned());
            let img_path = dir.join(format!("{}.img", peer.short()));
            let image_file = img_path.exists().then(|| img_path.clone());
            if image_file.is_some() {
                // 파일 읽기·격리 디코드 모두 워커에서(M4-5 결 — 부팅 무정지).
                // 봉인 개봉 + 구본(평문) 관용 — 구본이면 여기서 lazy 재봉인(이관).
                let secret = self.identity.wrap_secret();
                spawn_decode(
                    self.proxy.clone(),
                    DecodeTarget::PeerAvatar(peer),
                    move || {
                        let raw = std::fs::read(&img_path).ok()?;
                        let bytes = if nbeep_store::sealed::is_sealed(&raw) {
                            nbeep_store::sealed::open(
                                crate::gate::SEAL_PROFILE_CACHE,
                                &secret,
                                &raw,
                            )?
                        } else {
                            if let Ok(env) = nbeep_store::sealed::seal(
                                crate::gate::SEAL_PROFILE_CACHE,
                                &secret,
                                &raw,
                            ) {
                                let _ = std::fs::write(&img_path, env);
                            }
                            raw
                        };
                        crate::imgdec::avatar_raw_from_bytes(&bytes, 256)
                    },
                );
            }
            if self.live {
                // 데모(에코 봇) 모드는 시드하지 않는다 — 실기 핀이 봇 목록에 섞인다.
                self.extra_peers
                    .entry(peer)
                    .or_insert_with(|| nbeep_core::default_display_name(None, &peer));
            }
            let name = rec.names.last().cloned();
            if name.is_some() || avatar.is_some() || border.is_some() || image_file.is_some() {
                // 수신 시각(M3-21 ③) — 캐시 파일 mtime으로 근사(meta 우선 · 없으면
                // img · 둘 다 없으면 0 = 미상). .meta에 시각을 넣지 않는 이유 =
                // 그 파일 규약은 "와이어 검증 통과 표시값"뿐(encode_profile_meta).
                let received_ms = std::iter::once(dir.join(format!("{}.meta", peer.short())))
                    .chain(image_file.clone())
                    .filter_map(|f| std::fs::metadata(f).ok()?.modified().ok())
                    .filter_map(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                    .map(|d| u64::try_from(d.as_millis()).unwrap_or(0))
                    .max()
                    .unwrap_or(0);
                self.peer_profiles.insert(
                    peer,
                    PeerProfile {
                        name,
                        email: None,
                        phone: None,
                        // 소개글은 이메일·전화처럼 부팅 캐시하지 않는다(재연결
                        // 프리페치가 다시 채운다 · at-rest 최소화 결).
                        bio: None,
                        image_file,
                        avatar,
                        border,
                        received_ms,
                    },
                );
            }
        }
    }

    /// 한글 입력(IME) 기준값 반영(08-15 사용자 요청 — 하드코딩이던 판정 창 전부를
    /// 설정으로 · hot-swap). 기본값은 macOS 실측(H-27) — 잘못 넣은 값은 관용
    /// 파싱으로 기본값 폴백(ADR-0011 자세).
    fn apply_ime_tuning(&mut self) {
        let d = crate::ime_gate::ImeTuning::default();
        let ms = |k: &str, dv: u64| self.settings.get(k).parse().unwrap_or(dv);
        self.ime.set_tuning(crate::ime_gate::ImeTuning {
            inject: self.settings.get("ime.inject") != "off",
            leak: self.settings.get("ime.leak") != "off",
            stale_ms: ms("ime.stale_ms", d.stale_ms),
            same_key_ms: ms("ime.same_key_ms", d.same_key_ms),
            pending_ms: ms("ime.pending_ms", d.pending_ms),
            echo_ms: ms("ime.echo_ms", d.echo_ms),
            stash_ms: ms("ime.stash_ms", d.stash_ms),
            owed_ms: ms("ime.owed_ms", d.owed_ms),
            pre_clear_ms: ms("ime.pre_clear_ms", d.pre_clear_ms),
            swallow_ms: ms("ime.swallow_ms", d.swallow_ms),
            selfcommit_ms: ms("ime.selfcommit_ms", d.selfcommit_ms),
        });
    }

    /// 정렬 드롭다운 항목(08-15 — 아이콘·라벨은 언어에 따라 재구성).
    fn sort_drop_items() -> Vec<nbeep_ui::IconDropItem> {
        use nbeep_core::{t, Msg};
        use nbeep_ui::icons::sort;
        vec![
            nbeep_ui::IconDropItem {
                value: "chat",
                label: t(Msg::SortChat).to_string(),
                alpha: sort::RECENT_ALPHA,
                size: sort::SIZE,
            },
            nbeep_ui::IconDropItem {
                value: "name",
                label: t(Msg::SortName).to_string(),
                alpha: sort::NAME_ALPHA,
                size: sort::SIZE,
            },
            nbeep_ui::IconDropItem {
                value: "seen",
                label: t(Msg::SortSeen).to_string(),
                alpha: sort::SEEN_ALPHA,
                size: sort::SIZE,
            },
            nbeep_ui::IconDropItem {
                value: "online",
                label: t(Msg::SortOnline).to_string(),
                alpha: sort::ONLINE_ALPHA,
                size: sort::SIZE,
            },
        ]
    }

    fn apply_boot_settings(&mut self) {
        use nbeep_core::{ApprovalPolicy, BasicApproval};
        // 기본 아바타 — 미설정이면 **12간지 중 무작위 배정 후 저장**(사용자 확정
        // 08-21 — 종전 키 지문 안정 배정에서 변경). 첫 부팅 1회 CSPRNG로 뽑아
        // **저장하므로 이후 실행에서 요동치지 않는다**(상대 화면 얼굴 안정 —
        // 08-14의 우려는 "저장 없는 매 실행 랜덤"에 대한 것이었다). 보더 색도
        // 같은 무작위 시드에서 유도(아바타와 한 몸의 첫인상).
        if self.settings.get("profile.avatar").is_empty() {
            // CSPRNG 32B — 그룹 uid·데이터 키와 같은 관례(Identity = getrandom 유래).
            let rnd = *nbeep_crypto::Identity::generate().peer_id().as_bytes();
            let v = nbeep_core::avatar::default_for_seed(&rnd).to_setting();
            self.settings.set("profile.avatar", v);
            if self.settings.get("profile.avatar_border").is_empty() {
                let rgb = nbeep_core::avatar::default_border_for_seed(&rnd);
                self.settings.set(
                    "profile.avatar_border",
                    nbeep_core::avatar::border_to_setting(rgb),
                );
            }
            self.conf_mark();
        }
        // 보더만 비어 있는 구설정(아바타는 있음) — 종전대로 지문 유도 유지.
        if self.settings.get("profile.avatar_border").is_empty() {
            let rgb =
                nbeep_core::avatar::default_border_for_seed(self.identity.peer_id().as_bytes());
            self.settings.set(
                "profile.avatar_border",
                nbeep_core::avatar::border_to_setting(rgb),
            );
            self.conf_mark();
        }
        self.rebuild_theme(); // ui.theme + theme.* 색 오버라이드
        if let Ok(ms) = self.settings.get("ui.typeahead_timeout").parse::<u64>() {
            self.list.set_typeahead_timeout(ms);
        }
        self.apply_ime_tuning(); // 한글 입력(IME) 기준값(08-15 · H-27) — 부팅 반영
                                 // 목록 보기(08-14 사용자 확정) — 갱신 주기 + 갱신 시 스크롤 동작.
        self.list_refresh_ms = self
            .settings
            .get("ui.list_refresh_ms")
            .parse()
            .unwrap_or(1500);
        self.list
            .set_refresh_scroll(nbeep_ui::RefreshScroll::from_code(
                self.settings.get("ui.list_refresh_scroll"),
            ));
        let mut inv = Invalidations::default();
        let badge_shape = self.settings.get("ui.link_badge_shape") == "on";
        self.list.set_badge_shape(badge_shape, &mut inv);
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
        if let Ok(v) = self.settings.get("xfer.auto_cancel_min").parse::<u64>() {
            self.auto_cancel_limit_ms = v.clamp(1, 10) * 60_000;
        }
        // 시스템 시작 시 자동 실행(08-20 · 기본 on) — **부팅마다 재동기화**: 포터블
        // (DR-4)은 실행 파일 위치가 옮겨질 수 있어 등록 경로가 낡는다. 켜져 있으면
        // 현재 경로로 재등록, 꺼져 있으면 등록 제거(수기 잔재 치유 · 멱등).
        // 실패 = 상태바 고지(기능은 계속 — 설정값 유지로 다음 기회에 재시도).
        if let Err(e) = nbeep_plat::autostart::apply(self.settings.get("app.autostart") != "off") {
            self.set_status(nbeep_core::tf(
                nbeep_core::Msg::StfAutostartFail,
                &[&e.to_string()],
            ));
        }
        // 트레이 상주(M3-2a · Windows — 비지원 OS는 None): 아이콘 = 내 아바타 ·
        // 이벤트는 프록시로 메인에 복귀(좌클릭/열기·종료).
        if self.tray.is_none() {
            let proxy = self.proxy.clone();
            self.tray = nbeep_plat::tray::spawn(self.tray_content(), move |ev| {
                let _ = proxy.send_event(AppEvent::Tray(ev));
            });
        }
        // 툴바 프로필 버튼 = 내 얼굴(08-14) — 부팅 반영 + 사진이 있으면 워커 디코드
        // (도착 시 Decoded(MyAvatar)가 버튼을 갱신한다).
        self.refresh_toolbar_avatar();
        let p = self.settings.get("profile.image_path").to_string();
        if !p.is_empty() {
            spawn_decode(self.proxy.clone(), DecodeTarget::MyAvatar, move || {
                std::fs::read(&p)
                    .ok()
                    .and_then(|b| crate::imgdec::avatar_raw_from_bytes(&b, 256))
            });
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
        let degraded = local.discovery_recv_degraded();
        self.discovery = local.discovery();
        spawn_inbound_accept(
            local.incoming(),
            std::sync::Arc::clone(&self.identity),
            self.proxy.clone(),
        );
        self.transport = std::sync::Arc::new(local);
        self.table = nbeep_core::PeerTable::new(60_000);
        self.listen_port = Some(bound);
        if degraded {
            // M1-13ⓔ — 조용한 비호환 금지: 듣지 못하는 상태는 반드시 화면에.
            self.status =
                "⚠ 발견 수신 불가(포트 47100 점유) — 발신 전용(상대 목록에서 나를 선택)".into();
        }
        Ok(bound)
    }

    fn apply_settings(&mut self, changes: Vec<(&'static str, String)>) {
        if !changes.is_empty() {
            self.conf_mark();
        }
        // 프로필 push 배치(RL-2ⓑ · 08-18) — 키 단위로 밀면 적용 1회에 N벌
        // (Full 키 포함 시 256KiB 사진 N회). 루프에서 유형만 모아 끝에 1회.
        let mut profile_push: Option<ProfileScope> = None;
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
                // 설정 백업·복원·초기화(08-15 · 고급) — 값이 아니라 행위.
                "settings.backup" => {
                    self.pending_picker = Some(PickerPurpose::SettingsBackupDir);
                    continue;
                }
                "settings.restore" => {
                    self.pending_picker = Some(PickerPurpose::SettingsRestoreFile);
                    continue;
                }
                "settings.reset" => {
                    self.pending_reset = true; // 확인 모달은 about_to_wait(el)에서
                    continue;
                }
                // 로그 보기(M3-22 Action) — 오늘 파일 우선, 없으면 폴더(그것도
                // 없으면 안내 — off 상태 접근성).
                "log.view" => {
                    let dir = self.data_dir.join("logs");
                    let t = nbeep_plat::clock::local_time(unix_now());
                    let f = dir.join(format!("beep-{:04}{:02}{:02}.log", t.y, t.mo, t.d));
                    let target = if f.is_file() { f } else { dir.clone() };
                    if target.exists() && nbeep_plat::launch::open_path(&target) {
                        self.set_status(nbeep_core::t(nbeep_core::Msg::LogView).to_string());
                    } else {
                        self.set_status(nbeep_core::t(nbeep_core::Msg::StNoLogs));
                    }
                    continue;
                }
                _ => {}
            }
            self.settings.set(key, value.clone());
            // ★ 열려 있는 설정 화면에 역반영(08-15 사용자 실기 — 툴바 정렬처럼
            //   다른 경로의 변경이 설정 화면 표시와 어긋났다 · 쌍방 동기화).
            //   설정 화면 자신이 낸 변경이면 같은 값이라 무해(no-op 표시 갱신).
            if let Some(sv) = &mut self.settings_view {
                let mut sinv = Invalidations::default();
                sv.set_value(key, &value, &mut sinv);
                if let Some(sid) = self
                    .windows
                    .iter()
                    .find(|(_, e)| e.role == Role::Settings)
                    .map(|(id, _)| *id)
                {
                    self.request_redraw(sid);
                }
            }
            match key {
                "chat.window_mode" => {
                    // 새 대화부터 적용(DR-26 — 열린 창은 유지·소급 강제 없음).
                    self.mode = if value == "separate" {
                        WindowMode::Separate
                    } else {
                        WindowMode::Single
                    };
                    self.set_status(nbeep_core::tf(nbeep_core::Msg::StfWindowMode, &[&value]));
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
                    self.set_status(nbeep_core::tf(
                        nbeep_core::Msg::StfColorApplied,
                        &[k, &value],
                    ));
                    for e in self.windows.values() {
                        e.window.request_redraw();
                    }
                }
                "ui.language" => {
                    // 현재 언어 전환 — 전 위젯이 다음 렌더에서 새 언어로 그린다.
                    nbeep_core::set_lang(nbeep_core::Lang::from_code(&value).unwrap_or_default());
                    // 메뉴 라벨은 생성 시 고정이라 재구성.
                    self.menu.set_menus(build_menus());
                    // 정렬 드롭다운 라벨도 생성 시 고정 — 값 유지한 채 재구성(08-15).
                    let keep = self.sort_drop.value().to_string();
                    self.sort_drop = nbeep_ui::IconDropdown::new(Self::sort_drop_items(), &keep);
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
                    self.set_status(match self.approval.remaining_ms(now) {
                        Some(ms) => format!("파일 수신: {}분간 자동 수락", ms / 60_000),
                        None => format!("파일 수신 승인 = {value}"),
                    });
                    self.refresh_approval_ui();
                }
                "xfer.send_rate" => {
                    self.send_rate = nbeep_core::RateLimit::from_code(&value);
                    self.set_status(format!(
                        "보내기 제한 = {}",
                        rate_label(self.send_rate.target_bps(&self.send_meter))
                    ));
                }
                "xfer.recv_rate" => {
                    self.recv_rate = nbeep_core::RateLimit::from_code(&value);
                    self.set_status(format!(
                        "받기 제한 = {} (상대에게 공지됨)",
                        rate_label(self.recv_rate.target_bps(&self.recv_meter))
                    ));
                }
                "xfer.timeout_sec" => {
                    if let Ok(v) = value.parse::<u64>() {
                        self.wait_timeout_sec = v.clamp(5, 3600);
                    }
                }
                "xfer.auto_cancel_min" => {
                    if let Ok(v) = value.parse::<u64>() {
                        self.auto_cancel_limit_ms = v.clamp(1, 10) * 60_000;
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
                            self.set_status(nbeep_core::t(nbeep_core::Msg::StAutoRestart));
                        }
                    }
                    self.refresh_approval_ui();
                }
                // 캐러셀 스크롤 방향(08-14) — 열린 프로필에 즉시 적용(hot-swap).
                "ui.carousel_scroll" => {
                    let inv = carousel_inverted(&self.settings);
                    if let Some(pv) = &mut self.profile_view {
                        pv.set_carousel_inverted(inv);
                    }
                    self.set_status(format!(
                        "캐러셀 스크롤 = {}",
                        if inv {
                            "내추럴(반전)"
                        } else {
                            "정방향"
                        }
                    ));
                }
                // 한글 입력(IME) 기준값(08-15 · H-27) — 어느 키든 일습 재적용(hot-swap).
                k if k.starts_with("ime.") => {
                    self.apply_ime_tuning();
                    self.set_status(nbeep_core::t(nbeep_core::Msg::StImeApplied));
                }
                // 목록 정렬 모드(08-15) — 즉시 재조립 + 드롭다운 동기화.
                "ui.list_sort" => {
                    let mut dinv = Invalidations::default();
                    self.sort_drop.set_value(&value, &mut dinv);
                    self.refresh_and_redraw();
                }
                // 목록 갱신 주기(08-14) — 발견발 재조립을 이 간격으로 묶는다.
                "ui.list_refresh_ms" => {
                    self.list_refresh_ms = value.parse().unwrap_or(1500);
                    self.set_status(nbeep_core::tf(
                        nbeep_core::Msg::StfListRefresh,
                        &[&self.list_refresh_ms.to_string()],
                    ));
                }
                // 목록 갱신 시 스크롤 동작(08-14 사용자 확정 3택).
                "ui.list_refresh_scroll" => {
                    self.list
                        .set_refresh_scroll(nbeep_ui::RefreshScroll::from_code(&value));
                }
                // 세션 배지 실루엣(M3-19) — 목록 + 열린 구성원 모달까지 즉시(hot-swap 원칙).
                "ui.link_badge_shape" => {
                    let on = value == "on";
                    let mut binv = Invalidations::default();
                    self.list.set_badge_shape(on, &mut binv);
                    if let Some(al) = &mut self.alert_view {
                        al.set_badge_shape(on);
                    }
                    if let Some(mid) = self.main_id {
                        self.request_redraw(mid);
                    }
                }
                // 툴팁 대기(08-14) — 열린 프로필 화면에 즉시 적용(hot-swap 원칙).
                "ui.tooltip_ms" => {
                    let ms = value.parse::<u64>().unwrap_or(2000);
                    if let Some(pv) = &mut self.profile_view {
                        pv.set_tooltip_ms(ms);
                    }
                    self.status =
                        nbeep_core::tf(nbeep_core::Msg::StfTooltipDelay, &[&ms.to_string()]);
                }
                // 아바타 선택·보더(08-14) — 재전송은 아래 프로필 전파 깔때기가
                // 맡는다(M3-21에서 키별 arm → 단일 깔때기로 이관).
                "profile.avatar" | "profile.avatar_border" => {
                    self.refresh_toolbar_avatar(); // 프로필 버튼 = 내 얼굴
                    self.set_status(nbeep_core::t(nbeep_core::Msg::StAvatarChanged));
                }
                // 사진 경로(08-14) — 스와치 선택이 비우면 툴바 버튼도 즉시 따라간다.
                // 값이 차면(최근 캐러셀 선택 등) 그 사진을 디코드해 미리보기·툴바 갱신.
                "profile.image_path" => {
                    // 경로가 바뀌면 옛 와이어 축소본은 무효(NULL 엄격 — 08-16).
                    // 비움(철회)도 같은 규칙: 축소본이 남으면 철회가 안 나간다.
                    // 세대를 올려 도는 중이던 워커의 결과도 함께 무효화한다.
                    self.wire_gen = self.wire_gen.wrapping_add(1);
                    self.wire_pending = false;
                    let _ = std::fs::remove_file(self.wire_avatar_path());
                    if value.is_empty() {
                        self.my_avatar = None;
                    } else {
                        let jp = value.clone();
                        spawn_decode(self.proxy.clone(), DecodeTarget::MyAvatar, move || {
                            std::fs::read(&jp)
                                .ok()
                                .and_then(|b| crate::imgdec::avatar_raw_from_bytes(&b, 256))
                        });
                        // 상한 초과 원본이면 워커가 축소본을 굽고 완료 시 Full 재전파.
                        self.ensure_wire_avatar();
                    }
                    self.refresh_toolbar_avatar();
                }
                // 표시 이름(M1-10) — 즉시 재공지(사용자 확정 08-11). 상대 목록은
                // PeerTable Renamed 경로로 갱신된다.
                "profile.display_name" => {
                    let name = effective_display_name(&self.settings, &self.identity.peer_id());
                    self.transport.set_display_name(name.clone());
                    self.refresh_toolbar_avatar(); // 이니셜이 이름을 따라간다(08-14)
                    self.set_status(nbeep_core::tf(
                        nbeep_core::Msg::StfDisplayName,
                        &[name.as_str()],
                    ));
                }
                // 수신 포트(08-13 ⓐ — 듣는 포트 = 거는 기본 포트). 즉시 적용 = 전송 재시작
                // (성립한 대화 세션은 개별 소켓이라 유지 · 발견 목록은 재공지로 회복).
                "net.session_port" => {
                    let want = session_port_from(&self.settings);
                    if !self.live {
                        self.status =
                            format!("수신 포트 = {want} (데모 모드 — 실물 전송에서 적용)");
                    } else if self.listen_port == Some(want) {
                        self.status =
                            nbeep_core::tf(nbeep_core::Msg::StfPortListening, &[&want.to_string()]);
                    } else {
                        match self.respawn_transport() {
                            Ok(bound) if bound == want => {
                                self.set_status(nbeep_core::tf(
                                    nbeep_core::Msg::StfPortApplied,
                                    &[&bound.to_string()],
                                ));
                            }
                            // 폴백을 조용히 하지 않는다 — 상대에게 알려줄 값은 실제 포트다.
                            Ok(bound) => {
                                self.set_status(format!(
                                    "포트 {want} 점유 — 임의 포트 {bound}로 듣는 중(설정값은 유지)"
                                ));
                            }
                            Err(e) => {
                                self.set_status(nbeep_core::tf(
                                    nbeep_core::Msg::StfRestartFail,
                                    &[&e.to_string()],
                                ));
                            }
                        }
                    }
                }
                // 서버 모드(08-18) — Unmanaged면 주소·포트·타입을 잠근다(계산은
                // refresh_approval_ui 깔때기 한 곳 — set_disabled 전체 교체 규약).
                // Managed 배선(X-2b) — 접속을 내려놓고 서버 틱이 새 목표로 수렴한다.
                "net.server.mode" => {
                    self.refresh_approval_ui();
                    self.server_settings_changed();
                }
                "net.server.address" | "net.server.port" => self.server_settings_changed(),
                // 수신 파일 상한(08-18) — 연결된 전 세션 hot-swap(새 오퍼부터 적용).
                "xfer.recv_max_mb" => {
                    let cap = cap_from_setting(&value).unwrap_or(u64::MAX);
                    for conv in self.conversations.values() {
                        let _ = conv.out_tx.send(SessionCmd::SetRecvMax(cap));
                    }
                }
                // 로그 설정(M3-22) — 켜기/끄기·보존·상한 전부 hot-swap(재시작 없음).
                // 보존·상한은 netmon도 공유하므로 함께 재기동한다.
                "log.enabled" | "log.retain_days" | "log.max_total_mb" => {
                    self.refresh_statuslog();
                    if self.netmon_log.is_some() {
                        self.refresh_netmon();
                    }
                }
                // 네트워크 점검(netmon · 08-21) — 옵트인 켜기/끄기 hot-swap.
                // 주기는 다음 틱부터 자연 반영(재기동 불요).
                "netmon.enabled" => self.refresh_netmon(),
                // 시스템 시작 시 자동 실행(08-20 — 기본 on) — 토글 즉시 OS 등록/해제.
                // 실패를 조용히 넘기지 않는다(상태바 고지) — 설정값은 유지되므로
                // 다음 부팅 재동기화·재토글에서 재시도된다.
                "app.autostart" => {
                    let on = value == "on";
                    match nbeep_plat::autostart::apply(on) {
                        Ok(()) => self.set_status(nbeep_core::t(if on {
                            nbeep_core::Msg::StAutostartOn
                        } else {
                            nbeep_core::Msg::StAutostartOff
                        })),
                        Err(e) => self.set_status(nbeep_core::tf(
                            nbeep_core::Msg::StfAutostartFail,
                            &[&e.to_string()],
                        )),
                    }
                }
                "ui.typeahead_space" => self.list.set_typeahead_space(value == "on"),
                "ui.typeahead_special" => self.list.set_typeahead_special(value == "on"),
                k if k.starts_with("font.") => {
                    self.fonts = Self::fonts_from_settings(&self.settings);
                    self.reload_faces(); // 글꼴명 → 실제 얼굴 로드(Enter 확정 시 도달)
                    self.set_status(nbeep_core::t(nbeep_core::Msg::StFontApplied));
                    for e in self.windows.values() {
                        e.window.request_redraw();
                    }
                }
                _ => {}
            }
            // ★ 프로필 전파 깔때기(M3-21 · 사용자 확정 08-14) — 응답 구성이 읽는
            // 키의 변경은 **연결된 전 세션**(수동 등록 상대 포함)에 push. 추가·변경뿐
            // 아니라 **끄는 변경도** 같은 응답에 "필드 부재"로 실려 상대 캐시가
            // 지워진다(철회 대칭). 키별 arm에 재전송을 두지 않는 이유 = 빠뜨린 키가
            // 곧 전파 안 되는 키가 되는 구조였다(이메일 공유 실기 구멍의 원인).
            if let Some(scope) = profile_push_scope(key) {
                // Full이 하나라도 있으면 Full(사진 동반) — Info는 그에 포함된다.
                profile_push = Some(match (profile_push, scope) {
                    (Some(ProfileScope::Full), _) | (_, ProfileScope::Full) => ProfileScope::Full,
                    _ => ProfileScope::Info,
                });
            }
        }
        if let Some(scope) = profile_push {
            self.push_profile(scope); // 배치 1회(RL-2ⓑ)
        }
        if let Some(id) = self.main_id {
            self.request_redraw(id);
        }
    }

    /// 내 프로필을 연결된 전 세션에 push(M3-21) — scope가 프레임 무게를 정한다.
    fn push_profile(&mut self, scope: ProfileScope) {
        if self.wire_pending {
            // ★ 한 번에 밀기(08-16 실기) — 축소본 워커가 도는 동안 밀면 축소본이
            // 없는 프레임(내장 키만)이 먼저 나가 상대가 "옛 내장 그림 → 실사진"
            // 2단계를 본다. 완료 이벤트(WireAvatar)가 Full을 민다(Info 상위집합).
            return;
        }
        if self.conversations.is_empty() {
            return;
        }
        let frames = self.my_profile_frames_scoped(scope);
        for conv in self.conversations.values() {
            let _ = conv.out_tx.send(SessionCmd::Control(frames.clone()));
        }
    }

    /// 그룹 행동 처리(M5-1g — 공유 그룹 기준) — 멤버십 변경은 소유자만(ADR G-6).
    fn handle_group_action(&mut self, action: nbeep_ui::GroupAction, el: &ActiveEventLoop) {
        use nbeep_ui::GroupAction as GA;
        let me = self.identity.peer_id();
        match action {
            GA::Create { members } => {
                if members.is_empty() {
                    self.set_status(nbeep_core::t(nbeep_core::Msg::StGroupNeedSelect));
                    return;
                }
                self.open_name_prompt(
                    el,
                    NamePurpose::CreateGroup(members),
                    "그룹 대화 만들기 — 이름",
                    "",
                );
            }
            GA::Rename(gid) => {
                let cur = self
                    .groups
                    .shared_by_id(gid)
                    .map(|s| s.roster.name.as_str().to_string())
                    .unwrap_or_default();
                self.open_name_prompt(el, NamePurpose::RenameGroup(gid), "그룹 이름 변경", &cur);
            }
            GA::AddMembers(gid, peers) => {
                let Some(s) = self.groups.shared_by_id(gid) else {
                    return;
                };
                if s.roster.owner != me {
                    // 구성원 초대(08-13 사용자 확정 — 기본 허용): 명부 갱신은 소유자
                    // 단일 진실을 유지해야 하므로 **요청(Suggest)을 소유자에게** 보낸다.
                    // 소유자가 정책 확인 후 명부에 반영·전원 배포 — 부분 가시성 분기 없음.
                    if !s.roster.member_invite {
                        self.set_status(nbeep_core::t(nbeep_core::Msg::StGroupOwnerInvite));
                        return;
                    }
                    let uid = s.roster.uid;
                    let owner = s.roster.owner;
                    let frame = nbeep_core::SGroupMsg::Suggest {
                        uid,
                        members: peers.clone(),
                    }
                    .encode();
                    self.send_group_frames(owner, vec![frame]);
                    self.list.clear_selection();
                    self.set_status(format!(
                        "{}명 초대 요청 발송 — 소유자가 명부에 반영하면 초대됩니다",
                        peers.len()
                    ));
                    return;
                }
                let mut roster = s.roster.clone();
                let new: Vec<PeerId> = peers
                    .into_iter()
                    .filter(|p| !roster.members.contains(p))
                    .collect();
                if new.is_empty() {
                    self.set_status(nbeep_core::t(nbeep_core::Msg::StGroupAllMembers));
                    return;
                }
                roster.members.extend(new.iter().copied());
                roster.members.sort();
                roster.version += 1;
                // 신규자에겐 초대(수락제 — G-4), 기존 구성원에겐 명부 갱신.
                let invite = nbeep_core::SGroupMsg::Invite {
                    roster: roster.clone(),
                }
                .encode();
                self.broadcast_roster(roster);
                for p in &new {
                    self.send_group_frames(*p, vec![invite.clone()]);
                }
                self.status =
                    nbeep_core::tf(nbeep_core::Msg::StfInvitesSent, &[&new.len().to_string()]);
                self.refresh_and_redraw();
            }
            GA::RemoveMembers(gid, peers) => {
                let Some(s) = self.groups.shared_by_id(gid) else {
                    return;
                };
                if s.roster.owner != me {
                    self.set_status(nbeep_core::t(nbeep_core::Msg::StGroupOwnerRemove));
                    return;
                }
                let mut roster = s.roster.clone();
                let before = roster.members.len();
                roster.members.retain(|m| !peers.contains(m) || *m == me);
                if roster.members.len() == before {
                    return;
                }
                roster.version += 1;
                // 제외자에게도 마지막 1회 배포(방 닫힘을 알게 — ADR §4).
                let frame = nbeep_core::SGroupMsg::Roster {
                    roster: roster.clone(),
                }
                .encode();
                for p in peers.iter().filter(|p| **p != me) {
                    self.send_group_frames(*p, vec![frame.clone()]);
                }
                self.broadcast_roster(roster);
                self.set_status(nbeep_core::t(nbeep_core::Msg::StGroupRemoved));
                self.refresh_and_redraw();
            }
            GA::TogglePolicy(gid) => {
                let Some(s) = self.groups.shared_by_id(gid) else {
                    return;
                };
                if s.roster.owner != me {
                    self.set_status(nbeep_core::t(nbeep_core::Msg::StGroupOwnerPolicy));
                    return;
                }
                let mut roster = s.roster.clone();
                roster.member_invite = !roster.member_invite;
                roster.version += 1;
                let on = roster.member_invite;
                self.broadcast_roster(roster);
                self.set_status(if on {
                    "이 방: 구성원 초대 허용 — 구성원에게 배포".to_string()
                } else {
                    "이 방: 소유자만 초대 — 구성원에게 배포".to_string()
                });
                self.refresh_and_redraw();
            }
            GA::Members(gid) => {
                // 그룹 아이콘 클릭(08-14) — 방 헤더 클릭과 같은 구성원 모달.
                self.open_group_members(gid);
                let _ = el;
            }
            GA::ToggleFav(gid) => {
                // 그룹 목록 고정(08-15) — groups.seg v3에 영속.
                let cur = self.groups.shared_by_id(gid).is_some_and(|s| s.pinned);
                if self.groups.set_shared_pinned(gid, !cur) {
                    self.refresh_and_redraw();
                    self.set_status(if cur {
                        "그룹 — 목록 고정 해제".to_string()
                    } else {
                        "그룹 — 목록 상단에 고정".into()
                    });
                }
            }
            GA::Delete(gid) => {
                let Some(s) = self.groups.shared_by_id(gid) else {
                    return;
                };
                let uid = s.roster.uid;
                if s.roster.owner == me {
                    // 해산 = 구성원이 나뿐인 명부를 마지막 배포(전원이 자기 제외를 안다).
                    let mut roster = s.roster.clone();
                    let old: Vec<PeerId> = roster
                        .members
                        .iter()
                        .copied()
                        .filter(|m| *m != me)
                        .collect();
                    roster.members = vec![me];
                    roster.version += 1;
                    let frame = nbeep_core::SGroupMsg::Roster { roster }.encode();
                    for p in old {
                        self.send_group_frames(p, vec![frame.clone()]);
                    }
                    self.set_status(nbeep_core::t(nbeep_core::Msg::StGroupDisband));
                } else {
                    // 탈퇴 — 소유자에게 통지(명부 갱신은 소유자 몫).
                    let leave = nbeep_core::SGroupMsg::Leave { uid }.encode();
                    let owner = s.roster.owner;
                    self.send_group_frames(owner, vec![leave]);
                    self.set_status(nbeep_core::t(nbeep_core::Msg::StGroupLeave));
                }
                let _ = self.groups.remove_shared(uid);
                self.close_group_views(gid);
                self.refresh_and_redraw();
            }
        }
    }

    /// 그룹 뷰·스레드 정리(해산·탈퇴·제외 공용).
    fn close_group_views(&mut self, gid: nbeep_core::group::GroupId) {
        // 기록 세그먼트도 정리(08-19 — 방이 없으면 열 수 없다). uid를 못 찾는
        // 호출 순서(저장소 선삭제)는 부팅 복원의 고아 정리가 처리한다.
        if let Some(uid) = self.groups.shared_by_id(gid).map(|s| s.roster.uid) {
            let _ = std::fs::remove_file(
                self.data_dir
                    .join("history")
                    .join(format!("g-{}.seg", uid.short())),
            );
        }
        self.gchats.remove(&gid);
        self.group_threads.remove(&gid);
        self.gunread.remove(&gid);
        if self.single_open_group == Some(gid) {
            self.single_open_group = None;
            self.set_main_ime(false);
            if let Some(mid) = self.main_id {
                self.layout_window(mid);
            }
        }
        let wins: Vec<WindowId> = self
            .windows
            .iter()
            .filter(|(_, e)| e.role == Role::GroupChat(gid))
            .map(|(id, _)| *id)
            .collect();
        for wid in wins {
            self.windows.remove(&wid);
        }
    }

    /// 목록 갱신 + 주 창 재도(그룹 편집 후 공용 짧은 손).
    fn refresh_and_redraw(&mut self) {
        let mut inv = Invalidations::default();
        self.refresh_rows(&mut inv);
        if let Some(mid) = self.main_id {
            self.request_redraw(mid);
        }
    }

    /// 이름 입력 모달을 연다(M5-1 — 그룹 생성·개명 공용 · 항상 위).
    fn open_name_prompt(
        &mut self,
        el: &ActiveEventLoop,
        purpose: NamePurpose,
        title: &str,
        initial: &str,
    ) {
        // 이미 열려 있으면 새 용도로 교체(중복 창 금지 — 13 §12-1 중복 가드).
        if let Some((pid, _)) = self
            .windows
            .iter()
            .find(|(_, e)| e.role == Role::NamePrompt)
        {
            let pid = *pid;
            self.windows.remove(&pid);
        }
        // 창 제목·플레이스홀더 = 용도별 i18n(08-21 승격 — 공지인데 "Group" 제목,
        // "공지 내용" 한글 고정이던 것).
        let broadcast = matches!(purpose, NamePurpose::Broadcast);
        self.name_prompt_for = Some(purpose);
        let mut attrs = Window::default_attributes()
            .with_title(format!(
                "Nexa Beep — {}",
                nbeep_core::t(if broadcast {
                    nbeep_core::Msg::WinBroadcast
                } else {
                    nbeep_core::Msg::WinGroup
                })
            ))
            .with_inner_size(winit::dpi::LogicalSize::new(420.0, 150.0))
            .with_resizable(false)
            .with_window_icon(self.icon.clone());
        // 메인(목록) 창 중앙 부근에 배치(08-21 사용자 요청 — OS 기본 위치는 목록과
        // 멀리 떨어져 뜬다 · 경고 모달과 같은 문법 08-20). 높이는 첫 paint의 제목
        // word-wrap 실측으로 재조정된다(desired_height — 잘림 방지).
        if let Some(e) = self.main_id.and_then(|m| self.windows.get(&m)) {
            if let Ok(pos) = e.window.outer_position() {
                let sz = e.window.inner_size();
                let sf = e.window.scale_factor();
                let (mw, mh) = ((420.0 * sf) as i32, (150.0 * sf) as i32);
                attrs = attrs.with_position(winit::dpi::PhysicalPosition::new(
                    pos.x + (sz.width as i32 - mw) / 2,
                    pos.y + (sz.height as i32 - mh) / 2,
                ));
            }
        }
        let attrs = self.modal_attrs(attrs, false); // 메인 소유(08-15 — 창 묶음 부상)
        let window = Rc::new(el.create_window(attrs).unwrap());
        window.set_ime_allowed(true); // 그룹 이름 한글 입력
        let scale = window.scale_factor() as f32;
        let context = softbuffer::Context::new(window.clone()).unwrap();
        let surface = SbSurface::new(&context, window.clone()).unwrap();
        let id = window.id();
        self.windows.insert(
            id,
            WinEntry {
                role: Role::NamePrompt,
                window,
                surface,
                cursor: (0, 0),
                scale,
            },
        );
        let ph = nbeep_core::t(if broadcast {
            nbeep_core::Msg::PhBroadcastBody // ④ — 이름이 아니라 본문 입력
        } else {
            nbeep_core::Msg::PhGroupName
        });
        self.name_prompt = Some(nbeep_ui::TextPromptWidget::new(title, ph, initial));
        self.layout_window(id);
        self.request_redraw(id);
    }

    /// 이름 모달 제출 적용(M5-1g) — 무해화(DisplayName)는 여기서.
    /// 생성 = **공유 그룹**(ADR-0012): roster v1 서명석(세션 인증) + 전 구성원 초대 발송.
    fn apply_name_prompt(&mut self, name: &str) {
        // 공지(④ 08-20)는 이름이 아니라 **본문** — DisplayName 제약(길이·문자)을
        // 태우지 않고 메시지 무해화만 거친다(send_broadcast 안에서).
        if matches!(self.name_prompt_for, Some(NamePurpose::Broadcast)) {
            self.name_prompt_for = None;
            self.send_broadcast(name);
            self.refresh_and_redraw();
            return;
        }
        let Ok(dn) = nbeep_core::DisplayName::parse(name) else {
            self.set_status(nbeep_core::t(nbeep_core::Msg::StGroupBadName));
            return;
        };
        match self.name_prompt_for.take() {
            Some(NamePurpose::CreateGroup(members)) => {
                // uid = 무작위 32B(기존 instance 생성 관례 — CSPRNG 유래).
                let uid =
                    nbeep_core::GroupUid(*nbeep_crypto::Identity::generate().peer_id().as_bytes());
                let me = self.identity.peer_id();
                let mut all = members.clone();
                if !all.contains(&me) {
                    all.push(me);
                }
                all.sort();
                let roster = nbeep_core::Roster {
                    uid,
                    name: dn,
                    owner: me,
                    members: all,
                    version: 1,
                    // 새 방 정책 = 전역 설정 상속(사용자 확정 08-13 — 이후 방별 변경).
                    member_invite: self.settings.get("group.member_invite") != "off",
                };
                let invite = nbeep_core::SGroupMsg::Invite {
                    roster: roster.clone(),
                }
                .encode();
                let _ = self
                    .groups
                    .upsert_shared(roster, nbeep_store::MineState::Owner);
                for m in &members {
                    self.send_group_frames(*m, vec![invite.clone()]);
                }
                self.list.clear_selection();
                self.set_status(format!(
                    "그룹 대화 생성 — {}명에게 초대 발송(수락하면 방에 들어옵니다)",
                    members.len()
                ));
            }
            Some(NamePurpose::RenameGroup(gid)) => self.rename_shared(gid, dn),
            // 공지는 위에서 조기 처리(본문 = DisplayName 제약 밖) — 방어적 무동작.
            Some(NamePurpose::Broadcast) => {}
            None => {}
        }
        if self.groups.write_failed() {
            self.status =
                nbeep_core::tf(nbeep_core::Msg::StfGroupSaveFail, &[&self.status.clone()]);
        }
        self.refresh_and_redraw();
    }

    /// 공지 발송(④ 08-20 · FR-M-6 — 사용자 확정 "발견된 전체"): 목록의 모든
    /// 상대에게 **Notice 등급** 1:1 팬아웃. 연결된 상대 = 즉시, 미연결 = 오프라인
    /// 대기(M4-6) 편입 + 자동 연결(그룹 팬아웃 규약). Urgent 공지는 만들지 않는다
    /// (docs/24 — 팬아웃 Urgent는 수신측 1단계 강등 · 발신에서부터 그 정신을 지킨다).
    fn send_broadcast(&mut self, body: &str) {
        let text = nbeep_core::sanitize_message(body);
        if text.as_str().trim().is_empty() {
            return;
        }
        // ★ 발신 빈도 제한(08-21 사용자 확정 — 3초에 1번): 공지는 발견 전체에게
        //   1:1 팬아웃이라, 연타 한 번이 N명 × 세션/큐 비용이 된다. 막을 때는
        //   조용히 버리지 않고 상태바로 이유를 말한다(조용한 생략 금지).
        const BROADCAST_MIN_GAP_MS: u64 = 3_000;
        let now = self.now_ms();
        let since = now.saturating_sub(self.last_broadcast_ms);
        if self.last_broadcast_ms != 0 && since < BROADCAST_MIN_GAP_MS {
            self.set_status(nbeep_core::t(nbeep_core::Msg::StBroadcastRateLimit));
            return;
        }
        self.last_broadcast_ms = now;
        // 대상 = 발견 목록 + 비발견 유지 상대(refresh_rows와 같은 병합 · 나 제외).
        let me = self.identity.peer_id();
        let mut targets: Vec<PeerId> = self.table.list().into_iter().map(|e| e.peer).collect();
        for &peer in self.extra_peers.keys() {
            if !targets.contains(&peer) {
                targets.push(peer);
            }
        }
        targets.retain(|p| *p != me);
        let mut now_n = 0usize;
        let mut queued_n = 0usize;
        let mut inv = Invalidations::default();
        for peer in targets {
            let (at_ms, wall) = now_stamp();
            if self.conversations.contains_key(&peer) {
                let msg = nbeep_core::ChatMessage {
                    sender_device: me,
                    seq: self.seq.issue(),
                    body: nbeep_core::MessageBody::Text(text.as_str().to_string()),
                    importance: nbeep_core::Importance::Notice,
                    broadcast: true, // 공지 표식(08-21 — 수신측 "받지 않기"의 근거)
                };
                if let Some(chat) = self.chats.get_mut(&peer) {
                    chat.push_line(
                        ChatLine::text(true, text.clone(), at_ms, wall)
                            .with_seq(msg.seq)
                            .with_importance(1),
                        &mut inv,
                    );
                }
                self.ledger.note_sent(peer);
                if let Some(conv) = self.conversations.get_mut(&peer) {
                    conv.lines.push(
                        ChatLine::text(true, text.clone(), at_ms, wall)
                            .with_seq(msg.seq)
                            .with_importance(1),
                    );
                    let _ = conv.out_tx.send(SessionCmd::Chat(msg.encode()));
                }
                self.record_history(peer);
                now_n += 1;
            } else {
                // 미연결 = 오프라인 대기 편입(M4-6 — 상대가 나타나면 자동 전달).
                let q = self.pending_direct.entry(peer).or_default();
                q.push(PendingDirect {
                    text: text.as_str().to_string(),
                    at_ms,
                    importance: 1 | 0x4, // Notice + 공지 표식(08-21)
                });
                if q.len() > PENDING_DIRECT_MAX {
                    let drop_n = q.len() - PENDING_DIRECT_MAX;
                    q.drain(..drop_n);
                }
                if let Some(chat) = self.chats.get_mut(&peer) {
                    chat.push_line(
                        ChatLine::text(true, text.clone(), at_ms, wall)
                            .with_queued(true)
                            .with_importance(1),
                        &mut inv,
                    );
                }
                self.save_pending(peer);
                self.reconnect.remove(&peer);
                self.start_connect(peer, true);
                queued_n += 1;
            }
        }
        self.set_status(nbeep_core::tf(
            nbeep_core::Msg::StfBroadcastSent,
            &[&now_n.to_string(), &queued_n.to_string()],
        ));
    }

    /// 그룹 제어 프레임 발송 — 세션 있으면 즉시, 없으면 대기 + 자동 연결(M5-1g).
    fn send_group_frames(&mut self, peer: PeerId, frames: Vec<Vec<u8>>) {
        if let Some(conv) = self.conversations.get(&peer) {
            match conv.out_tx.send(SessionCmd::Group(frames)) {
                Ok(()) => return,
                Err(e) => {
                    // 액터 사망 — 되찾아 대기 경로로(아래).
                    let SessionCmd::Group(frames) = e.0 else {
                        return;
                    };
                    self.queue_group_frames(peer, frames);
                    return;
                }
            }
        }
        self.queue_group_frames(peer, frames);
    }

    /// 미연결 상대의 그룹 프레임 대기 + 자동 연결(상한 = `group.resync_keep`).
    fn queue_group_frames(&mut self, peer: PeerId, frames: Vec<Vec<u8>>) {
        let keep = group_resync_keep(&self.settings);
        let q = self.pending_invites.entry(peer).or_default();
        q.extend(frames);
        if q.len() > keep {
            let n = q.len() - keep;
            q.drain(..n);
        }
        self.reconnect.remove(&peer);
        self.start_connect(peer, true);
    }

    /// 로컬(동보) 그룹 → 그룹 대화 승격(G4 · 08-15) — 부팅 1회 마이그레이션.
    ///
    /// 동보 그룹 UI는 숨겨져(ADR-0012 §7 · 08-13 확정) 구버전에서 만든 로컬 그룹이
    /// **보이지도 지워지지도 않는 채** 남는다 → 같은 이름·구성원의 공유 그룹(내가
    /// 소유자·roster v1)으로 올리고 로컬 기록은 지운다. 초대는 즉시 쏘지 않는다 —
    /// 부팅 시점엔 주소가 없어 연결이 전부 실패한다. [`Self::flush_group_sends`]의
    /// 재동기 push(소유 방 Invite 재송)가 **세션이 성립하는 순간** 전달을 맡는다.
    fn promote_local_groups(&mut self) {
        let locals: Vec<(
            nbeep_core::group::GroupId,
            nbeep_core::DisplayName,
            Vec<PeerId>,
        )> = self
            .groups
            .list()
            .iter()
            .map(|(id, g)| (*id, g.name.clone(), g.members()))
            .collect();
        if locals.is_empty() {
            return;
        }
        let me = self.identity.peer_id();
        let n = locals.len();
        for (id, name, members) in locals {
            let mut all = members;
            if !all.contains(&me) {
                all.push(me);
            }
            all.sort();
            let roster = nbeep_core::Roster {
                // uid = 무작위 32B(생성 플로와 같은 관례 — CSPRNG 유래).
                uid: nbeep_core::GroupUid(*nbeep_crypto::Identity::generate().peer_id().as_bytes()),
                name,
                owner: me,
                members: all,
                version: 1,
                member_invite: self.settings.get("group.member_invite") != "off",
            };
            let _ = self
                .groups
                .upsert_shared(roster, nbeep_store::MineState::Owner);
            let _ = self.groups.delete(id);
        }
        self.set_status(format!(
            "로컬 그룹 {n}개를 그룹 대화로 승격 — 구성원에게는 연결될 때 초대가 전달됩니다"
        ));
    }

    /// 공유 그룹 개명(소유자만) — roster v+1 재배포.
    fn rename_shared(&mut self, gid: nbeep_core::group::GroupId, dn: nbeep_core::DisplayName) {
        let Some(s) = self.groups.shared_by_id(gid) else {
            return;
        };
        if s.roster.owner != self.identity.peer_id() {
            self.set_status(nbeep_core::t(nbeep_core::Msg::StGroupOwnerRename));
            return;
        }
        let mut roster = s.roster.clone();
        roster.name = dn;
        roster.version += 1;
        self.broadcast_roster(roster);
        self.set_status(nbeep_core::t(nbeep_core::Msg::StGroupRenamed));
    }

    /// 새 roster를 저장하고 전 구성원(나 제외)에게 배포(소유자 전용 경로).
    fn broadcast_roster(&mut self, roster: nbeep_core::Roster) {
        let me = self.identity.peer_id();
        let frame = nbeep_core::SGroupMsg::Roster {
            roster: roster.clone(),
        }
        .encode();
        let members = roster.members.clone();
        let _ = self
            .groups
            .upsert_shared(roster, nbeep_store::MineState::Owner);
        for m in members.iter().filter(|m| **m != me) {
            self.send_group_frames(*m, vec![frame.clone()]);
        }
    }

    /// 그룹 스레드를 연다(M5-1 · FR-G-3 — 팬아웃을 하나의 스레드로).
    fn open_group_thread(&mut self, gid: nbeep_core::group::GroupId, el: &ActiveEventLoop) {
        let Some(s) = self.groups.shared_by_id(gid) else {
            return;
        };
        // 초대받은(미수락) 방 클릭 = 대화가 아니라 **수락/거절 모달**(08-19 —
        // 재시작 후 초대 카드가 사라져도 목록에서 언제든 수락할 수 있게).
        if s.mine == nbeep_store::MineState::Invited {
            let uid = s.roster.uid;
            let owner = s.roster.owner;
            let name = s.roster.name.as_str().to_string();
            let n = s.roster.members.len();
            let from = self.peer_title(owner);
            self.open_choice(
                el,
                "그룹 대화 초대",
                &format!(
                    "{from} 님이 '{name}' 그룹(구성원 {n}명)에 초대했습니다.\n수락하면 방이 목록에 생기고 구성원과 함께 대화합니다."
                ),
                "수락",
                "거절",
                AlertCtx::GroupInvite { uid, owner },
            );
            return;
        }
        let title = format!(
            "{} (그룹 {}명)",
            s.roster.name.as_str(),
            s.roster.members.len()
        );
        // 방을 열었다 = 확인했다(M5-1g unread) — 배지 해제.
        if self.gunread.remove(&gid).is_some() {
            let mut inv2 = Invalidations::default();
            self.refresh_rows(&mut inv2);
            self.update_main_title();
        }
        let mut chat = ChatViewWidget::new(title.clone());
        let mut inv = Invalidations::default();
        chat.set_time_format(
            self.settings.get("chat.time_24h") != "off",
            self.settings.get("chat.date_format") == "short",
            &mut inv,
        );
        for line in self.group_threads.get(&gid).into_iter().flatten() {
            chat.push_line(line.clone(), &mut inv);
        }
        match self.mode {
            WindowMode::Single => {
                self.gchats.insert(gid, chat);
                self.single_open = None; // 1:1 뷰가 열려 있었다면 목록 상태로 접고 그룹으로
                self.single_open_group = Some(gid);
                self.set_main_ime(true);
                if let Some(mid) = self.main_id {
                    self.layout_window(mid);
                    self.request_redraw(mid);
                }
            }
            WindowMode::Separate => {
                // 이미 열려 있으면 포커스(재활성화 = 기존 창 — DR-26 관례).
                if let Some((wid, _)) = self
                    .windows
                    .iter()
                    .find(|(_, e)| e.role == Role::GroupChat(gid))
                {
                    if let Some(e) = self.windows.get(wid) {
                        e.window.focus_window();
                    }
                    return;
                }
                self.gchats.insert(gid, chat);
                let attrs = Window::default_attributes()
                    .with_title(format!("Nexa Beep — {title}"))
                    .with_inner_size(winit::dpi::LogicalSize::new(520.0, 560.0))
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
                        role: Role::GroupChat(gid),
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

    /// 그룹 스레드 발신·복귀 처리(M5-1 · FR-G-2·G-6) — 구성원별 팬아웃.
    /// 세션 있는 구성원 = 즉시 · 없는 구성원 = 자동 연결 + 성립 시 이어 전달(사용자 확정).
    fn drain_group_effects(&mut self, gid: nbeep_core::group::GroupId, id: WindowId) {
        let mut inv = Invalidations::default();
        // 방 헤더 클릭 = 구성원 목록(08-14 — 목록 그룹 아이콘 클릭과 같은 모달).
        if self
            .gchats
            .get_mut(&gid)
            .is_some_and(ChatViewWidget::take_header_click)
        {
            self.open_group_members(gid);
        }
        // 풍선 우클릭 복사·붙여넣기 — 1:1 `drain_chat_effects`와 동형(위젯은 OS를
        // 모른다 · 08-13 실기: 이 배선이 없어 그룹 방 Copy Message가 무반응이었다).
        if let Some(t) = self
            .gchats
            .get_mut(&gid)
            .and_then(ChatViewWidget::take_copy_text)
        {
            self.set_status(if nbeep_plat::clipboard::set_text(&t) {
                "메시지 복사됨".to_string()
            } else {
                "복사 실패 — 클립보드를 열 수 없습니다".to_string()
            });
            self.request_redraw(id);
            if let Some(mid) = self.main_id {
                self.request_redraw(mid);
            }
        }
        if self
            .gchats
            .get_mut(&gid)
            .is_some_and(ChatViewWidget::take_paste_request)
        {
            if let Some(t) = nbeep_plat::clipboard::get_text() {
                if let Some(c) = self.gchats.get_mut(&gid) {
                    c.paste(&t, &mut inv);
                }
            } else if !self.try_clipboard_image_paste(id) {
                // 텍스트도 이미지도 없다(③ 08-20 — 이미지는 파일 전송으로 폴백).
                self.set_status(nbeep_core::t(nbeep_core::Msg::StPasteFail));
                if let Some(mid) = self.main_id {
                    self.request_redraw(mid);
                }
            }
            self.request_redraw(id);
        }
        // 뷰 닫기(단일 모드 ← 버튼) — 뷰만 닫힌다(스레드 유지 · DR-26 동형).
        if self
            .gchats
            .get_mut(&gid)
            .is_some_and(ChatViewWidget::take_back)
        {
            self.gchats.remove(&gid);
            match self.mode {
                WindowMode::Single => {
                    self.single_open_group = None;
                    self.set_main_ime(false);
                    self.layout_window(id);
                    self.request_redraw(id);
                }
                WindowMode::Separate => {
                    self.windows.remove(&id);
                }
            }
            return;
        }
        let outgoing = self
            .gchats
            .get_mut(&gid)
            .and_then(ChatViewWidget::take_outgoing);
        // 명령 가름(08-15) — 그룹 방도 같은 문법. 단일 상대가 필요한 명령은 거절된다.
        let outgoing = match self.run_chat_command(outgoing.as_ref(), None) {
            CmdOutcome::Send(t) => t,
            // 방에서는 등급 미지원(그룹 와이어에 자리 없음 · ④) — run_chat_command가
            // peer=None에서 안내 후 Handled를 주므로 여기는 방어적 평문 폴백.
            CmdOutcome::SendGraded(t, _) => Some(t),
            CmdOutcome::Handled => {
                self.request_redraw(id);
                return;
            }
        };
        if let Some(text) = outgoing {
            // 방 발신(M5-1g) — 명부 기준 팬아웃(나 제외 · Group 스트림 · ADR §4).
            let Some(s) = self.groups.shared_by_id(gid) else {
                self.set_status(nbeep_core::t(nbeep_core::Msg::StGroupGone));
                self.request_redraw(id);
                return;
            };
            let uid = s.roster.uid;
            let me = self.identity.peer_id();
            let members: Vec<PeerId> = s
                .roster
                .members
                .iter()
                .copied()
                .filter(|m| *m != me)
                .collect();
            if members.is_empty() {
                self.set_status(nbeep_core::t(nbeep_core::Msg::StGroupNoMembers));
                self.request_redraw(id);
                return;
            }
            let (at_ms, wall) = now_stamp();
            // 내 말풍선은 스레드에 한 번(팬아웃과 무관 — "하나의 방").
            let line = ChatLine::text(true, text.clone(), at_ms, wall);
            self.group_threads
                .entry(gid)
                .or_default()
                .push(line.clone());
            self.record_group_history(gid); // 그룹 기록 영속(08-19)
            if let Some(chat) = self.gchats.get_mut(&gid) {
                chat.push_line(line, &mut inv);
            }
            let mut sent = 0usize;
            let mut queued: Vec<PeerId> = Vec::new();
            for m in &members {
                if let Some(conv) = self.conversations.get(m) {
                    let frame = nbeep_core::SGroupMsg::Msg {
                        uid,
                        seq: self.seq.issue(),
                        text: text.as_str().to_string(),
                    }
                    .encode();
                    if conv.out_tx.send(SessionCmd::Group(vec![frame])).is_ok() {
                        self.ledger.note_sent(*m);
                        sent += 1;
                        continue;
                    }
                }
                // 미연결 — 본문을 상대별로 대기시키고 자동 연결(성립 시 flush).
                // 보관 상한 = 설정 `group.resync_keep`(사용 시점 읽기 — 즉시 반영) ·
                // 초과분은 오래된 것부터 폐기(큐 상한 필수 — NFR-B-6 · 13 §12-1).
                let keep = group_resync_keep(&self.settings);
                let q = self.pending_group_sends.entry(*m).or_default();
                q.push((gid, text.as_str().to_string()));
                if q.len() > keep {
                    let drop_n = q.len() - keep;
                    q.drain(..drop_n);
                }
                queued.push(*m);
            }
            for m in &queued {
                self.reconnect.remove(m); // 발신 의사 = 백오프 리셋(즉시 시도)
                self.start_connect(*m, true); // 자동 — 창을 열지 않는다
            }
            // FR-G-4 — 전달 경과는 **상태 바**에만(정보성 라인이 스레드에 섞이면
            // 대화를 가린다 — 08-13 실기 피드백). 스레드에는 ⚠ 실패만 남긴다.
            self.set_status(if queued.is_empty() {
                format!("그룹 발신 — 즉시 {sent} · 연결 대기 0")
            } else {
                let names: Vec<String> = queued.iter().map(|p| self.peer_title(*p)).collect();
                format!(
                    "그룹 발신 — 즉시 {sent} · 연결 대기 {}: {}",
                    queued.len(),
                    names.join(", ")
                )
            });
            self.refresh_and_redraw();
            self.request_redraw(id);
        }
    }

    /// 그룹 스레드에 시스템 안내 라인(FR-G-4 — 전달 상태는 스레드에서 보인다).
    fn push_group_note(
        &mut self,
        gid: nbeep_core::group::GroupId,
        note: &str,
        inv: &mut Invalidations,
    ) {
        let (at_ms, wall) = now_stamp();
        let line = ChatLine::text(false, nbeep_core::sanitize_message(note), at_ms, wall);
        self.group_threads
            .entry(gid)
            .or_default()
            .push(line.clone());
        self.record_group_history(gid); // 그룹 기록 영속(08-19)
        if let Some(chat) = self.gchats.get_mut(&gid) {
            chat.push_line(line, inv);
        }
        // 이 그룹 뷰가 보이는 창 재도.
        if self.single_open_group == Some(gid) {
            if let Some(mid) = self.main_id {
                self.request_redraw(mid);
            }
        }
        let wins: Vec<WindowId> = self
            .windows
            .iter()
            .filter(|(_, e)| e.role == Role::GroupChat(gid))
            .map(|(i, _)| *i)
            .collect();
        for w in wins {
            self.request_redraw(w);
        }
    }

    /// 성립한 상대에게 대기 중이던 그룹 본문·제어 프레임을 이어 보낸다(M5-1g —
    /// **Outbound·Inbound 공용** 합류점 · 재동기 주체 = 송신자).
    fn flush_group_sends(&mut self, peer: PeerId) {
        // ★ 재동기 확장(G4 · 08-15) — 내가 소유한 방에 이 상대가 있으면 **세션 성립마다
        // 현행 명부를 Invite로 1회 재송**한다. 소유자는 상대의 수락·수신 여부를 모르므로
        // Invite가 세 경우를 전부 닫는다: ① 초대가 유실된 상대(자동 연결 소진으로
        // pending이 폐기됐던) = 초대 카드 ② 배포를 놓쳐 명부가 낡은 구성원 = 명부
        // 갱신(수신측이 카드 재노출 없이 처리) ③ 최신 상대 = 같은 버전이라 조용히 무시.
        // 이벤트 구동(세션 성립)·소유 방 수 상한·멱등이라 [13 §12-1] 자동 반복 아님.
        let me = self.identity.peer_id();
        let resync: Vec<Vec<u8>> = self
            .groups
            .shared_list()
            .iter()
            .filter(|s| s.roster.owner == me && s.roster.has_member(peer) && peer != me)
            .map(|s| {
                nbeep_core::SGroupMsg::Invite {
                    roster: s.roster.clone(),
                }
                .encode()
            })
            .collect();
        if !resync.is_empty() {
            if let Some(conv) = self.conversations.get(&peer) {
                let _ = conv.out_tx.send(SessionCmd::Group(resync));
            }
        }
        // 제어 프레임(초대·명부) 먼저 — 본문보다 명부가 앞서야 수신측이 방을 안다.
        if let Some(frames) = self.pending_invites.remove(&peer) {
            if let Some(conv) = self.conversations.get(&peer) {
                let _ = conv.out_tx.send(SessionCmd::Group(frames));
            }
        }
        // 중단 발신 재-Offer(M4-10c · 1회) — 같은 파일을 다시 제안하면 수신측
        // `.part`가 매치돼 "이어받기 N%" 2택이 뜬다. 큐 합류 = 기존 배치 문법.
        if let Some(files) = self.resend_offers.remove(&peer) {
            let mut n = 0u32;
            for path in files {
                if !path.is_file() {
                    continue; // 그 사이 지워진 원본 — 조용히 제외
                }
                let size = std::fs::metadata(&path).map(|m| m.len()).unwrap_or(0);
                self.send_queue.entry(peer).or_default().push_back(path);
                let b = self.send_batch.entry(peer).or_insert((0, 0, 0, 0));
                b.1 += 1;
                b.3 += size;
                n += 1;
            }
            if n > 0 {
                self.set_status(nbeep_core::tf(
                    nbeep_core::Msg::XferReofferN,
                    &[&n.to_string()],
                ));
                self.pump_send_queue(peer);
            }
        }
        // 대기 파일(그룹 팬아웃 — 08-13): 세션이 성립했으니 일반 오퍼 큐로 합류.
        if let Some(files) = self.pending_group_files.remove(&peer) {
            for (_gid, path) in files {
                let size = std::fs::metadata(&path).map(|m| m.len()).unwrap_or(0);
                self.send_queue.entry(peer).or_default().push_back(path);
                let b = self.send_batch.entry(peer).or_insert((0, 0, 0, 0));
                b.1 += 1;
                b.3 += size;
            }
            self.pump_send_queue(peer);
        }
        let Some(pends) = self.pending_group_sends.remove(&peer) else {
            return;
        };
        let mut inv = Invalidations::default();
        let title = self.peer_title(peer);
        for (gid, text) in pends {
            let uid = self.groups.shared_by_id(gid).map(|s| s.roster.uid);
            let sent = uid.is_some_and(|uid| {
                self.conversations.get(&peer).is_some_and(|conv| {
                    let frame = nbeep_core::SGroupMsg::Msg {
                        uid,
                        seq: self.seq.issue(),
                        text: text.clone(),
                    }
                    .encode();
                    conv.out_tx.send(SessionCmd::Group(vec![frame])).is_ok()
                })
            });
            if sent {
                self.ledger.note_sent(peer);
                // 성공 종결은 상태 바만 — 스레드는 대화 몫(08-13 실기 피드백).
                self.set_status(nbeep_core::tf(
                    nbeep_core::Msg::StfGroupPendingSent,
                    &[&title],
                ));
                if let Some(mid) = self.main_id {
                    self.request_redraw(mid);
                }
            } else {
                self.push_group_note(gid, &format!("⚠ 전달 실패: {title}"), &mut inv);
            }
        }
    }

    /// 자동 연결이 끝내 실패한 상대의 그룹 대기분을 실패로 종결(FR-G-4 — 조용히 버리지 않는다).
    fn fail_group_sends(&mut self, peer: PeerId) {
        self.pending_invites.remove(&peer); // 초대·명부도 폐기(다음 편집 때 재배포된다)
        let mut inv = Invalidations::default();
        // 대기 파일도 실패로 종결(조용히 버리지 않는다 — FR-G-4).
        if let Some(files) = self.pending_group_files.remove(&peer) {
            let title = self.peer_title(peer);
            for (gid, path) in files {
                let name = path
                    .file_name()
                    .map_or_else(|| "?".into(), |n| n.to_string_lossy().into_owned());
                self.push_group_note(
                    gid,
                    &format!("⚠ 파일 전달 실패(연결 안 됨): {title} — {name}"),
                    &mut inv,
                );
            }
        }
        let Some(pends) = self.pending_group_sends.remove(&peer) else {
            return;
        };
        let title = self.peer_title(peer);
        for (gid, _) in pends {
            self.push_group_note(gid, &format!("⚠ 전달 실패(연결 안 됨): {title}"), &mut inv);
        }
    }

    /// 이 방이 지금 화면에 있는가(M5-1g unread 기준 — 1:1 `chat_visible`과 동형).
    fn group_visible(&self, gid: nbeep_core::group::GroupId) -> bool {
        match self.mode {
            WindowMode::Single => self.single_open_group == Some(gid),
            WindowMode::Separate => self
                .windows
                .values()
                .any(|e| matches!(e.role, Role::GroupChat(g) if g == gid)),
        }
    }

    /// 선택 모달(2버튼)을 연다 — 결과는 `alert_ctx`로 라우팅(M5-1g 초대 카드).
    fn open_choice(
        &mut self,
        el: &ActiveEventLoop,
        title: &str,
        message: &str,
        yes: &str,
        no: &str,
        ctx: AlertCtx,
    ) {
        // 열려 있던 경고·선택은 대체(모달 1개 규칙 — 13 §12-1 중복 가드).
        if let Some((aid, _)) = self.windows.iter().find(|(_, e)| e.role == Role::Alert) {
            let aid = *aid;
            self.windows.remove(&aid);
        }
        self.alert_ctx = Some(ctx);
        let attrs = Window::default_attributes()
            .with_title(format!(
                "Nexa Beep — {}",
                nbeep_core::t(nbeep_core::Msg::WinConfirm)
            ))
            .with_inner_size(winit::dpi::LogicalSize::new(400.0, 170.0))
            .with_resizable(false)
            .with_window_icon(self.icon.clone());
        let attrs = self.modal_attrs(attrs, false); // 메인 소유(08-15 — 창 묶음 부상)
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
        self.alert_view = Some(nbeep_ui::AlertWidget::new(title, message).with_choice(yes, no));
        self.layout_window(id);
        self.request_redraw(id);
    }

    /// 선택 모달 결과 적용(M5-1g) — 초대 수락 = 방 합류 + 소유자에 통지.
    fn apply_alert_choice(&mut self, yes: bool, ctx: AlertCtx, el: &ActiveEventLoop) {
        match ctx {
            AlertCtx::RemoteInbound { peer } => {
                // 요청 대기 판정(§6 · M5-3b) — 세션 실물은 pending 슬롯에 살아 있다.
                // 거절·키 불일치 = 드롭 = 소켓 닫힘(상대는 Closed만 관측 — 정보 최소).
                if let Some((p, session, path)) = self.pending_remote.take() {
                    if yes && p == peer {
                        self.accept_inbound(session, path);
                    } else {
                        drop(session);
                        self.set_status(nbeep_core::t(nbeep_core::Msg::StRemoteInboundDropped));
                        if let Some(mid) = self.main_id {
                            self.request_redraw(mid);
                        }
                    }
                }
            }
            AlertCtx::GroupKick { gid, peer } => {
                if yes {
                    // 기존 제외 경로 재사용(소유자 검증·마지막 명부 배포 포함 — G4).
                    self.handle_group_action(
                        nbeep_ui::GroupAction::RemoveMembers(gid, vec![peer]),
                        el,
                    );
                    // 편집 후 구성원 모달을 새 명부로 다시 연다(이어서 편집).
                    self.open_group_members(gid);
                } else {
                    self.set_status(nbeep_core::t(nbeep_core::Msg::StRemoveCancel));
                }
            }
            AlertCtx::SettingsReset => {
                if yes {
                    {
                        let m = self.do_reset_settings();
                        self.set_status(m);
                    }
                } else {
                    self.set_status(nbeep_core::t(nbeep_core::Msg::StResetCancel));
                }
            }
            AlertCtx::GroupInvite { uid, owner } => {
                if yes {
                    if self.groups.set_mine(uid, nbeep_store::MineState::Joined) {
                        let frame = nbeep_core::SGroupMsg::Accept { uid }.encode();
                        self.send_group_frames(owner, vec![frame]);
                        let name = self
                            .groups
                            .shared_by_uid(uid)
                            .map(|s| s.roster.name.as_str().to_string())
                            .unwrap_or_default();
                        self.set_status(nbeep_core::tf(nbeep_core::Msg::StfJoinedGroup, &[&name]));
                        // 합류 = 구성원들과 연결(아바타·프로필은 세션 경유 · 08-14).
                        self.connect_group_members(uid);
                    }
                } else {
                    let frame = nbeep_core::SGroupMsg::Decline { uid }.encode();
                    self.send_group_frames(owner, vec![frame]);
                    let _ = self.groups.remove_shared(uid);
                    self.set_status(nbeep_core::t(nbeep_core::Msg::StInviteDeclined));
                }
                self.refresh_and_redraw();
            }
        }
    }

    /// 그룹 구성원 중 **미연결 상대에게 자동 연결**(M5-1g · 08-14 실기: 세션이 없으면
    /// 프로필(아바타)이 안 온다 — 프로필은 세션 경유가 원칙(DR-22)이라 브로드캐스트로
    /// 풀지 않고 연결로 푼다. 방 발신 즉시 도달률도 함께 오른다). 자동(창 안 열음) ·
    /// 중복은 ConnectLatch가 거른다(13 §12-1).
    fn connect_group_members(&mut self, uid: nbeep_core::GroupUid) {
        let me = self.identity.peer_id();
        let members: Vec<PeerId> = match self.groups.shared_by_uid(uid) {
            Some(s) if s.mine != nbeep_store::MineState::Invited => s
                .roster
                .members
                .iter()
                .copied()
                .filter(|m| *m != me && !self.conversations.contains_key(m))
                .collect(),
            _ => return,
        };
        for m in members {
            self.start_connect(m, true);
        }
    }

    /// 공유 그룹 프레임 처리(M5-1g · ADR-0012 §4) — **명부 검증 단일 지점**.
    fn handle_sgroup(&mut self, peer: PeerId, msg: nbeep_core::SGroupMsg, el: &ActiveEventLoop) {
        use nbeep_core::SGroupMsg as G;
        let me = self.identity.peer_id();
        match msg {
            G::Invite { roster } => {
                // 세션 인증이 서명을 대체한다(ADR §2 G-1) — 발신자 = 소유자만 수용.
                if peer != roster.owner || !roster.has_member(me) {
                    return; // 위조·오배송 — fail-closed(조용히 버림)
                }
                // 재동기 재초대(G4) — 이미 있는 방(수락 여부 확정 후)이면 초대 카드를
                // 다시 열지 않고 **명부 갱신으로만** 취급한다(G::Roster와 같은 처리 —
                // 소유자는 상대의 수락 여부를 몰라 세션 성립마다 Invite를 재송한다).
                if let Some(s) = self.groups.shared_by_uid(roster.uid) {
                    if s.mine != nbeep_store::MineState::Invited {
                        let gid = s.local_id;
                        let ver = roster.version;
                        let uid = roster.uid;
                        if self
                            .groups
                            .upsert_shared(roster, nbeep_store::MineState::Joined)
                            .is_some()
                        {
                            let mut inv = Invalidations::default();
                            self.push_group_note(
                                gid,
                                &format!("구성원 명부 갱신(v{ver})"),
                                &mut inv,
                            );
                            self.connect_group_members(uid);
                            self.refresh_and_redraw();
                        }
                        // ★ 수락 재통지(08-19 자가 치유) — 이미 수락한 방의 재초대(resync)에
                        //   Accept를 되보낸다. 소유자가 재시작으로 수락 기록을 잃어도
                        //   구성원 목록에서 나를 "초대 대기"로 오인하지 않게 한다(멱등).
                        let ack = nbeep_core::SGroupMsg::Accept { uid }.encode();
                        self.send_group_frames(peer, vec![ack]);
                        return; // 같은 버전 재송 = 조용히 무시(멱등)
                    }
                }
                let name = roster.name.as_str().to_string();
                let uid = roster.uid;
                let owner = roster.owner;
                let n = roster.members.len();
                if self
                    .groups
                    .upsert_shared(roster, nbeep_store::MineState::Invited)
                    .is_none()
                {
                    return; // 구버전 재생 — 거부
                }
                let from = self.peer_title(peer);
                self.open_choice(
                    el,
                    "그룹 대화 초대",
                    &format!(
                        "{from} 님이 '{name}' 그룹(구성원 {n}명)에 초대했습니다.\n수락하면 방이 목록에 생기고 구성원과 함께 대화합니다."
                    ),
                    "수락",
                    "거절",
                    AlertCtx::GroupInvite { uid, owner },
                );
            }
            G::Accept { uid } => {
                if self
                    .groups
                    .shared_by_uid(uid)
                    .is_some_and(|s| s.roster.owner == me && s.roster.has_member(peer))
                {
                    // 수락 기록 — 구성원 목록의 "초대 대기"를 지운다(08-19).
                    self.group_accepts.entry(uid).or_default().insert(peer);
                    self.status =
                        format!("{} 님이 그룹 초대를 수락했습니다", self.peer_title(peer));
                    self.refresh_and_redraw();
                }
            }
            G::Decline { uid } => {
                // 소유자 — 거절자는 명부에서 빼고 재배포(남은 구성원이 정확한 명부를 갖게).
                let Some(s) = self.groups.shared_by_uid(uid) else {
                    return;
                };
                if s.roster.owner != me || !s.roster.has_member(peer) {
                    return;
                }
                let mut roster = s.roster.clone();
                roster.members.retain(|m| *m != peer);
                roster.version += 1;
                self.broadcast_roster(roster);
                self.set_status(nbeep_core::tf(
                    nbeep_core::Msg::StfInviteDeclinedBy,
                    &[&self.peer_title(peer)],
                ));
                self.refresh_and_redraw();
            }
            G::Roster { roster } => {
                // 기존 방의 소유자 세션에서 온 갱신만(신규 uid는 초대 경유만 — G-4).
                let Some(s) = self.groups.shared_by_uid(roster.uid) else {
                    return;
                };
                if peer != s.roster.owner {
                    return;
                }
                let gid = s.local_id;
                if !roster.has_member(me) {
                    // 제외·해산 — 방을 닫는다(마지막 통지 — ADR §4).
                    let name = s.roster.name.as_str().to_string();
                    let _ = self.groups.remove_shared(roster.uid);
                    self.close_group_views(gid);
                    self.set_status(nbeep_core::tf(
                        nbeep_core::Msg::StfRemovedFromGroup,
                        &[&name],
                    ));
                    self.refresh_and_redraw();
                    return;
                }
                let ver = roster.version;
                let uid = roster.uid;
                if self
                    .groups
                    .upsert_shared(roster, nbeep_store::MineState::Joined)
                    .is_some()
                {
                    let mut inv = Invalidations::default();
                    self.push_group_note(gid, &format!("구성원 명부 갱신(v{ver})"), &mut inv);
                    // 새 구성원과도 연결(아바타·프로필은 세션 경유 · 08-14).
                    self.connect_group_members(uid);
                    self.refresh_and_redraw();
                }
            }
            G::Msg { uid, seq, text } => {
                let Some(s) = self.groups.shared_by_uid(uid) else {
                    return; // 모르는 방(미수락·해산) — 버림(fail-closed)
                };
                if s.mine == nbeep_store::MineState::Invited || !s.roster.has_member(peer) {
                    return; // 수락 전이거나 명부 밖 발신자
                }
                if !self.dedup.accept(peer, seq) {
                    return; // 재전송 중복
                }
                let gid = s.local_id;
                let room = s.roster.name.as_str().to_string(); // 차용 종료(알림에서 &mut self)
                self.ledger.note_recv(peer);
                let (at_ms, wall) = now_stamp();
                // 발신자 라벨 = 카카오톡류 수신 풍선 위 이름(기존 with_from 재사용).
                let line = ChatLine::text(false, nbeep_core::sanitize_message(&text), at_ms, wall)
                    .with_from(self.peer_title(peer));
                self.group_threads
                    .entry(gid)
                    .or_default()
                    .push(line.clone());
                self.record_group_history(gid); // 그룹 기록 영속(08-19)
                let mut inv = Invalidations::default();
                if let Some(chat) = self.gchats.get_mut(&gid) {
                    chat.push_line(line, &mut inv);
                }
                // OS 알림(M3-8) — 방 이름 제목 · 발신자 미검증 = 무음(DR-25).
                {
                    use nbeep_core::TrustStore as _;
                    let silent = self.trust.level(peer) == nbeep_core::TrustLevel::Unverified;
                    let body = self.notify_body(&text);
                    self.notify_user(
                        &format!("g:{gid:?}"),
                        &room,
                        &body,
                        silent,
                        false,
                        NotifyTarget::Group(gid),
                    );
                }
                if self.group_visible(gid) {
                    // 보고 있는 방 — 재도만.
                } else {
                    let e = self.gunread.entry(gid).or_insert(0);
                    *e = e.saturating_add(1);
                    self.set_status(nbeep_core::tf(
                        nbeep_core::Msg::StfGroupNewMsg,
                        &[&room, &self.peer_title(peer)],
                    ));
                    self.update_main_title();
                }
                self.refresh_and_redraw();
                // 이 방이 보이는 창 재도.
                let wins: Vec<WindowId> = self
                    .windows
                    .iter()
                    .filter(|(_, e)| e.role == Role::GroupChat(gid))
                    .map(|(i, _)| *i)
                    .collect();
                for w in wins {
                    self.request_redraw(w);
                }
            }
            G::Leave { uid } => {
                let Some(s) = self.groups.shared_by_uid(uid) else {
                    return;
                };
                if s.roster.owner != me || !s.roster.has_member(peer) {
                    return;
                }
                let mut roster = s.roster.clone();
                roster.members.retain(|m| *m != peer);
                roster.version += 1;
                let gid = s.local_id;
                let who = self.peer_title(peer);
                self.broadcast_roster(roster);
                let mut inv = Invalidations::default();
                self.push_group_note(gid, &format!("{who} 님이 나갔습니다"), &mut inv);
                self.refresh_and_redraw();
            }
            G::Suggest { uid, members } => {
                // 구성원의 초대 요청(08-13) — 소유자가 정책 확인 후 명부에 반영.
                // 명부 단일 진실 유지: 반영·배포는 언제나 여기(소유자)서만.
                let Some(s) = self.groups.shared_by_uid(uid) else {
                    return;
                };
                if s.roster.owner != me || !s.roster.has_member(peer) {
                    return; // 소유자 아님·명부 밖 요청자 — fail-closed
                }
                let gid = s.local_id;
                let who = self.peer_title(peer);
                if !s.roster.member_invite {
                    // 정책 변경 전 시차 요청 — 조용히 버리지 않고 스레드에 남긴다.
                    let mut inv = Invalidations::default();
                    self.push_group_note(
                        gid,
                        &format!("{who} 님의 초대 요청 — 이 방은 소유자만 초대(거절)"),
                        &mut inv,
                    );
                    return;
                }
                let mut roster = s.roster.clone();
                let new: Vec<PeerId> = members
                    .into_iter()
                    .filter(|p| !roster.members.contains(p))
                    .collect();
                if new.is_empty() {
                    return; // 이미 전원 구성원
                }
                roster.members.extend(new.iter().copied());
                roster.members.sort();
                roster.version += 1;
                let invite = nbeep_core::SGroupMsg::Invite {
                    roster: roster.clone(),
                }
                .encode();
                self.broadcast_roster(roster);
                for p in &new {
                    self.send_group_frames(*p, vec![invite.clone()]);
                }
                let mut inv = Invalidations::default();
                self.push_group_note(
                    gid,
                    &format!("{who} 님의 초대로 {}명에게 초대 발송", new.len()),
                    &mut inv,
                );
                self.refresh_and_redraw();
            }
        }
    }

    /// 대화 활성화 — 모드에 따라 주 창 전환 또는 별도 창 생성/포커스(14 §11).
    ///
    /// 세션이 없으면 **워커로 연결을 시작하고 즉시 돌아온다**(M2-8 — UI 무정지).
    /// 성립하면 `AppEvent::Outbound`가 이 함수를 다시 부른다.
    fn activate(&mut self, peer: PeerId, el: &ActiveEventLoop) {
        if !self.conversations.contains_key(&peer) {
            self.reconnect.remove(&peer); // 수동 클릭 = 백오프 처음부터(ⓑ)
            self.start_connect(peer, false);
            if let Some(mid) = self.main_id {
                self.request_redraw(mid);
            }
            return;
        }
        // 프로필 연락처가 있으면 대화 열 때 함께 보여준다(M3-17 — 상세 UI 전 최소 노출).
        self.set_status(match self.peer_profiles.get(&peer) {
            Some(p) if p.email.is_some() || p.phone.is_some() || p.image_file.is_some() => {
                let mut parts: Vec<String> = Vec::new();
                if let Some(e) = &p.email {
                    parts.push(e.clone());
                }
                if let Some(ph) = &p.phone {
                    parts.push(ph.clone());
                }
                if p.image_file.is_some() {
                    parts.push(nbeep_core::t(nbeep_core::Msg::StItemImagePresent).into());
                }
                nbeep_core::tf(nbeep_core::Msg::ChatOpenedProfile, &[&parts.join(" · ")])
            }
            _ => nbeep_core::t(nbeep_core::Msg::ChatOpenedSession).into(),
        });
        self.send_read_ack(peer); // 대화창 열기 = 받은 것 읽음(N-2 · 사용자 요청)
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
        self.suggest_verify(peer); // 결정적 순간 유도(M3-6)
    }

    /// **지문 대조 추천**(08-15 사용자 요청) — 대화를 여는 순간이 대조를 권하기 좋은
    /// 자리다(M3-6의 "결정적 순간 유도"). 조건을 좁게 잡는다:
    ///
    /// - `Pinned`일 때만(미검증은 아직 핸드셰이크 전 · 검증 완료는 권할 것이 없다).
    /// - **대화당 1회**(`verify_hinted`) — 열 때마다 뜨면 그냥 소음이 되고, 소음이 되면
    ///   사람은 읽지 않는다. 안 읽히는 경고는 없는 것과 같다.
    ///
    /// ⚠️ 추천은 **로컬 줄**이고 상대에게 가지 않는다. 승격은 여전히 사람이 다른 채널로
    /// 숫자를 맞춘 뒤 버튼을 눌러야 한다(이 통로 안의 문답으로 승격하면 중간자가 그
    /// 문답을 대신할 수 있다 — SAS가 막으려는 바로 그것).
    fn suggest_verify(&mut self, peer: PeerId) {
        use nbeep_core::TrustStore as _;
        if self.trust.level(peer) != nbeep_core::TrustLevel::Pinned {
            return;
        }
        if !self.verify_hinted.insert(peer) {
            return; // 이 대화에서 이미 권했다
        }
        self.push_chat_notice(Some(peer), nbeep_core::t(nbeep_core::Msg::SuggestVerify));
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
            Role::ImageView => {} // 뷰어는 페인트가 창 크기로 직접 맞춘다(위젯 없음)
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
                    // 정렬 드롭다운(08-15) — 좌측 아이콘들 끝에 이어 붙는 한 칸.
                    let slot = self.toolbar.slot_px();
                    let gap = (4.0 * scale).round() as i32;
                    let ty = menu_h + ((chrome - menu_h) - slot) / 2;
                    self.sort_drop.set_scale(scale);
                    self.sort_drop.set_bounds(
                        Rect::new(self.toolbar.left_items_end() + gap, ty, slot, slot),
                        &mut inv,
                    );
                }
                self.list.set_scale(scale, &mut inv);
                self.list
                    .set_bounds(Rect::new(0, chrome, w, (body - chrome).max(0)), &mut inv);
                if let Some(chat) = self.single_open.and_then(|p| self.chats.get_mut(&p)) {
                    chat.set_scale(scale, &mut inv);
                    chat.set_bounds(Rect::new(0, 0, w, body), &mut inv);
                }
                if let Some(chat) = self.single_open_group.and_then(|g| self.gchats.get_mut(&g)) {
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
            Role::GroupChat(gid) => {
                if let Some(chat) = self.gchats.get_mut(&gid) {
                    chat.set_scale(scale, &mut inv);
                    chat.set_bounds(Rect::new(0, 0, w, h), &mut inv);
                }
            }
            Role::NamePrompt => {
                if let Some(p) = &mut self.name_prompt {
                    p.set_scale(scale, &mut inv);
                    p.set_bounds(Rect::new(0, 0, w, h), &mut inv);
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
            Role::Convbox => {
                if let Some(cv) = &mut self.convbox_view {
                    cv.set_scale(scale, &mut inv);
                    cv.set_bounds(Rect::new(0, 0, w, h), &mut inv);
                }
            }
            Role::Approve(peer) => {
                if let Some(pv) = self.approve_view.get_mut(&peer) {
                    pv.set_scale(scale, &mut inv);
                    pv.set_bounds(Rect::new(0, 0, w, h), &mut inv);
                }
            }
        }
    }

    /// 진행 중 전송 취소(08-16 — 배너 "취소" 버튼) — 방향 불문 그 상대의 활성
    /// 전송 하나를 끊는다. 발신 배치가 걸려 있으면 대기 큐도 함께 비운다(취소는
    /// "이 작업 그만"이지 "이 파일만 건너뛰기"가 아니다).
    fn cancel_active_xfer(&mut self, peer: PeerId) {
        // 준비(해시) 중 취소(08-18) — 워커는 계속 돌지만 결과는 stale 가드가
        // 폐기한다(파일 핸들만 낭비 · 스레드 강제 종료보다 안전).
        if self.preparing_send.remove(&peer) {
            self.current_send.remove(&peer);
            self.send_queue.remove(&peer);
            self.send_batch.remove(&peer);
            self.send_excluded.remove(&peer); // 배치 종료 = 제외 목록도 마감(M4-2e)
            self.set_xfer_line(
                peer,
                true,
                nbeep_ui::XferLineState::Failed {
                    why: nbeep_core::t(nbeep_core::Msg::XferCanceled).into(),
                },
            );
            self.clear_xfer(peer);
            self.set_status(nbeep_core::t(nbeep_core::Msg::XferCanceled).to_string());
            self.redraw_conversation(peer);
            return;
        }
        let (xid, mine) = match self.active_send.remove(&peer) {
            Some(x) => (Some(x), true),
            None => (self.active_recv.remove(&peer), false),
        };
        let Some(xid) = xid else {
            // ★ 활성이 없어도 **일시중지·큐가 남아 있으면 전체 취소는 성립**한다
            //   (08-19 Windows 실기 ⑤ — Paused만 남은 상태에서 "No active"로 거부).
            let has_send_state = self.paused_sends.contains_key(&peer)
                || self.send_batch.contains_key(&peer)
                || self.send_queue.get(&peer).is_some_and(|q| !q.is_empty());
            if has_send_state {
                self.cancel_send(peer, false); // 배치 전체 취소(정지분 CancelXfer 포함)
                return;
            }
            // ★ 수신측 전체취소(08-19 실기 — "No active transfer" 거부): 활성이
            //   없어도 정지·자동수락 잔여가 있으면 수신 배치를 마감한다.
            let has_recv_state = self.recv_paused.contains_key(&peer)
                || self.batch_approved.contains_key(&peer)
                || self.recv_xids.get(&peer).is_some_and(|l| !l.is_empty());
            if has_recv_state {
                self.send_cancel_all_notice(peer); // 상대도 자기 쪽 루틴(M4-2e)
                if let Some(list) = self.recv_xids.remove(&peer) {
                    if let Some(c) = self.conversations.get(&peer) {
                        for (_, rxid) in list {
                            let _ = c.out_tx.send(SessionCmd::CancelXfer(rxid));
                        }
                    }
                }
                self.recv_paused.remove(&peer);
                self.clear_batch_approval(peer);
                self.fail_pending_xfer_lines(
                    peer,
                    false,
                    nbeep_core::t(nbeep_core::Msg::XferCanceled).to_string(),
                );
                self.clear_xfer(peer);
                self.set_status(nbeep_core::t(nbeep_core::Msg::XferCanceled).to_string());
                return;
            }
            // 실패도 말한다(08-10 교훈 — 조용한 무반응은 디버깅 불가). 배너 잔존
            // 등으로 활성 xid가 없으면 왜 안 되는지 상태바로 알린다.
            self.set_status(nbeep_core::t(nbeep_core::Msg::StNoActiveXfer));
            if let Some(mid) = self.main_id {
                self.request_redraw(mid);
            }
            return;
        };
        let sent = self
            .conversations
            .get(&peer)
            .is_some_and(|c| c.out_tx.send(SessionCmd::CancelXfer(xid)).is_ok());
        self.send_cancel_all_notice(peer); // 상대도 자기 쪽 전체취소 루틴(M4-2e)
        if mine {
            // ★ 전체취소(08-19 사용자 확정 — 배너 버튼): 활성뿐 아니라 **보관
            //   정지분·정지 표식·남은 라인**까지 배치 전체를 마감한다(종전엔
            //   paused_sends가 남아 개별 취소처럼 보였다).
            let paused_names: Vec<(String, u64)> = self
                .paused_sends
                .get(&peer)
                .map(|l| l.iter().map(|(n, sz, _, _)| (n.clone(), *sz)).collect())
                .unwrap_or_default();
            if let Some(list) = self.paused_sends.remove(&peer) {
                if let Some(c) = self.conversations.get(&peer) {
                    for (_, _, _, pxid) in list {
                        let _ = c.out_tx.send(SessionCmd::CancelXfer(pxid));
                    }
                }
            }
            self.send_paused.remove(&peer);
            self.send_queue.remove(&peer);
            self.send_batch.remove(&peer);
            self.send_excluded.remove(&peer); // 배치 종료 = 제외 목록도 마감(M4-2e)
            self.current_send.remove(&peer); // 취소 = 의사 표시(M4-10c)
            self.preparing_send.remove(&peer);
            self.resend_offers.remove(&peer);
            // 남은 미종결 발신 라인 일괄 종결(대기·정지 라인 영구 잔존 방지).
            let why = nbeep_core::t(nbeep_core::Msg::XferCanceled).to_string();
            while self.conversations.get_mut(&peer).is_some_and(|conv| {
                nbeep_ui::update_xfer_in(
                    &mut conv.lines,
                    true,
                    nbeep_ui::XferLineState::Failed { why: why.clone() },
                )
            }) {}
            // Paused 라인 잔여 종결(FIFO는 Paused를 건너뛴다).
            if let Some(conv) = self.conversations.get_mut(&peer) {
                for line in conv.lines.iter_mut() {
                    if line.mine {
                        if let nbeep_ui::ChatBody::Xfer(x) = &mut line.body {
                            if matches!(
                                x.state,
                                nbeep_ui::XferLineState::Paused { .. }
                                    | nbeep_ui::XferLineState::Waiting
                                    | nbeep_ui::XferLineState::Active { .. }
                            ) {
                                x.state = nbeep_ui::XferLineState::Failed { why: why.clone() };
                            }
                        }
                    }
                }
            }
            {
                let mut inv = Invalidations::default();
                if let Some(chat) = self.chats.get_mut(&peer) {
                    while chat.update_xfer_line(
                        true,
                        nbeep_ui::XferLineState::Failed { why: why.clone() },
                        &mut inv,
                    ) {}
                    // 뷰의 Paused 라인(FIFO가 건너뜀) — 이름 대상으로 종결.
                    for (n, sz) in &paused_names {
                        chat.set_xfer_named(
                            true,
                            n,
                            *sz,
                            nbeep_ui::XferLineState::Failed { why: why.clone() },
                            &mut inv,
                        );
                    }
                }
            }
            self.record_history(peer);
        } else {
            // ★ 수신 전체취소(M4-2e — 송신과 대칭): 활성 외 장부 잔여 xid도 Cancel
            //   통지하고 자동수락 잔여·정지 표식·미종결 라인을 일괄 마감한다.
            if let Some(list) = self.recv_xids.remove(&peer) {
                if let Some(c) = self.conversations.get(&peer) {
                    for (_, rxid) in list {
                        if rxid != xid {
                            let _ = c.out_tx.send(SessionCmd::CancelXfer(rxid));
                        }
                    }
                }
            }
            self.recv_paused.remove(&peer);
            self.clear_batch_approval(peer);
            self.fail_pending_xfer_lines(
                peer,
                false,
                nbeep_core::t(nbeep_core::Msg::XferCanceled).to_string(),
            );
            if let Some(sha) = self.resumed_recv.remove(&peer) {
                // 수동 취소 = 의사 표시 — 보존 부분물도 함께 폐기(재개 제안 금지 · M4-10).
                crate::part::remove_partial(crate::gate::CH_GUI, &sha);
            }
        }
        self.set_xfer_line(
            peer,
            mine,
            nbeep_ui::XferLineState::Failed {
                why: nbeep_core::t(nbeep_core::Msg::XferCanceled).into(),
            },
        );
        self.clear_xfer(peer);
        self.set_status(if sent {
            format!("전송 취소 — {}", self.peer_title(peer))
        } else {
            "취소 — 세션이 이미 끊겨 있음(로컬 정리만)".into()
        });
        self.redraw_conversation(peer);
    }

    /// 라인별 트랜스포트 제어 실행(M4-2e) — 채팅창 안 항목 옆 아이콘을 큐·액터·
    /// 와이어로 옮긴다. 발신 = 로컬 큐/액터 게이트 · 수신 = 와이어 PauseReq/
    /// ResumeReq(구버전 발신자는 무시 = 자연 강등).
    fn handle_xfer_ctl(&mut self, peer: PeerId, ctl: nbeep_ui::XferCtl) {
        use nbeep_ui::{XferCtlAct as A, XferLineState as St};
        let done_now = |app: &Self, sending: bool| -> u64 {
            app.xfer_progress
                .get(&peer)
                .filter(|p| p.sending == sending)
                .map_or(0, |p| p.done_bytes)
        };
        match (ctl.mine, ctl.act) {
            (true, A::Pause) => {
                let is_current = self
                    .current_send
                    .get(&peer)
                    .and_then(|p| p.file_name())
                    .is_some_and(|n| n.to_string_lossy() == ctl.name);
                if is_current && self.active_send.contains_key(&peer) {
                    // 액터에 정지 요청만 — 라인·큐 동기화는 **XferPaused 이벤트**가
                    // 단일 지점에서 수행한다(수락 경합으로 상태가 어긋나던 실기).
                    self.pause_active_send(peer, true);
                } else if let Some(path) = self.batch_paths(peer).into_iter().find(|p| {
                    p.file_name()
                        .is_some_and(|n| n.to_string_lossy() == ctl.name)
                }) {
                    self.send_paused.entry(peer).or_default().insert(path);
                    self.send_qpause_notice(peer, &ctl.name, true); // ①ⓐ 수신 동기화
                    self.set_xfer_line_named(
                        peer,
                        true,
                        &ctl.name,
                        ctl.size,
                        St::Paused { done: 0 },
                    );
                    self.note_xfer_state_change(peer); // ⓓ 타이머
                }
            }
            (true, A::Resume) => {
                // 보관 정지분(ⓐ) — 액터에 재개 요청(활성 중이면 끝난 뒤 이어감 ·
                // 상태 전환은 XferResumed 도착 시).
                if let Some(xid) = self.paused_sends.get(&peer).and_then(|l| {
                    l.iter()
                        .find(|(n, s, _, _)| n == &ctl.name && *s == ctl.size)
                        .map(|(_, _, _, x)| *x)
                }) {
                    if let Some(c) = self.conversations.get(&peer) {
                        let _ = c.out_tx.send(SessionCmd::ResumeXfer(xid));
                    }
                    self.refresh_send_batch(peer);
                    self.redraw_conversation(peer);
                    return;
                }
                let is_current = self
                    .current_send
                    .get(&peer)
                    .and_then(|p| p.file_name())
                    .is_some_and(|n| n.to_string_lossy() == ctl.name);
                if let Some(set) = self.send_paused.get_mut(&peer) {
                    set.retain(|p| {
                        !p.file_name()
                            .is_some_and(|n| n.to_string_lossy() == ctl.name)
                    });
                }
                if is_current {
                    self.pause_active_send(peer, false);
                    let done = done_now(self, true);
                    self.set_xfer_line_named(peer, true, &ctl.name, ctl.size, St::Active { done });
                } else {
                    self.send_qpause_notice(peer, &ctl.name, false); // ①ⓐ 수신 동기화
                    self.set_xfer_line_named(peer, true, &ctl.name, ctl.size, St::Waiting);
                    self.pump_send_queue(peer);
                    self.note_xfer_state_change(peer); // ⓓ 타이머
                }
            }
            (true, A::Cancel) => {
                if let Some(pos) = self.paused_sends.get(&peer).and_then(|l| {
                    l.iter()
                        .position(|(n, s, _, _)| n == &ctl.name && *s == ctl.size)
                }) {
                    // 보관 정지분 취소(ⓐ) — 액터 parked에서도 지운다.
                    if let Some(list) = self.paused_sends.get_mut(&peer) {
                        let (_, _, _, xid) = list.remove(pos);
                        if list.is_empty() {
                            self.paused_sends.remove(&peer);
                        }
                        if let Some(c) = self.conversations.get(&peer) {
                            let _ = c.out_tx.send(SessionCmd::CancelXfer(xid));
                        }
                    }
                    self.set_xfer_line_named(
                        peer,
                        true,
                        &ctl.name,
                        ctl.size,
                        St::Failed {
                            why: nbeep_core::t(nbeep_core::Msg::XferCanceled).into(),
                        },
                    );
                    self.refresh_send_batch(peer);
                    self.redraw_conversation(peer);
                    return;
                }
                if let Some(i) = self.batch_paths(peer).iter().position(|p| {
                    p.file_name()
                        .is_some_and(|n| n.to_string_lossy() == ctl.name)
                }) {
                    self.cancel_one_send(peer, i);
                    self.set_xfer_line_named(
                        peer,
                        true,
                        &ctl.name,
                        ctl.size,
                        St::Failed {
                            why: nbeep_core::t(nbeep_core::Msg::XferCanceled).into(),
                        },
                    );
                }
            }
            (false, A::Pause) => {
                let xid_by_name = self
                    .recv_xids
                    .get(&peer)
                    .and_then(|l| l.iter().find(|(n, _)| n == &ctl.name).map(|(_, x)| *x));
                if let Some(xid) = xid_by_name.or_else(|| self.active_recv.get(&peer).copied()) {
                    if let Some(c) = self.conversations.get(&peer) {
                        let _ = c.out_tx.send(SessionCmd::Control(vec![
                            nbeep_core::xfer::encode_pause_req(xid, true),
                        ]));
                    }
                    let done = done_now(self, false);
                    self.set_xfer_line_named(peer, false, &ctl.name, ctl.size, St::Paused { done });
                }
            }
            (false, A::Resume) => {
                let xid_by_name = self
                    .recv_xids
                    .get(&peer)
                    .and_then(|l| l.iter().find(|(n, _)| n == &ctl.name).map(|(_, x)| *x));
                if let Some(xid) = xid_by_name.or_else(|| self.active_recv.get(&peer).copied()) {
                    if let Some(c) = self.conversations.get(&peer) {
                        let _ = c.out_tx.send(SessionCmd::Control(vec![
                            nbeep_core::xfer::encode_pause_req(xid, false),
                        ]));
                    }
                    let done = done_now(self, false);
                    self.set_xfer_line_named(peer, false, &ctl.name, ctl.size, St::Active { done });
                }
            }
            (false, A::Cancel) => self.cancel_active_xfer(peer),
        }
        self.refresh_send_batch(peer);
        self.redraw_conversation(peer);
    }

    /// 이름 대상 우선·실패 시 FIFO(M4-2e — 수신 완료처럼 크기를 모르는 호출부).
    fn set_xfer_line_named_or_fifo(
        &mut self,
        peer: PeerId,
        mine: bool,
        name: &str,
        state: nbeep_ui::XferLineState,
    ) {
        let hit = {
            let mut any = false;
            if let Some(conv) = self.conversations.get_mut(&peer) {
                any |= nbeep_ui::update_xfer_named(&mut conv.lines, mine, name, 0, state.clone());
            }
            let mut inv = Invalidations::default();
            if let Some(chat) = self.chats.get_mut(&peer) {
                any |= chat.set_xfer_named(mine, name, 0, state.clone(), &mut inv);
            }
            any
        };
        if !hit {
            self.set_xfer_line(peer, mine, state);
        }
    }

    /// 이름 대상 라인 상태 갱신(M4-2e) — conv 저장분과 뷰 양쪽.
    fn set_xfer_line_named(
        &mut self,
        peer: PeerId,
        mine: bool,
        name: &str,
        size: u64,
        state: nbeep_ui::XferLineState,
    ) {
        if let Some(conv) = self.conversations.get_mut(&peer) {
            nbeep_ui::update_xfer_named(&mut conv.lines, mine, name, size, state.clone());
        }
        let mut inv = Invalidations::default();
        if let Some(chat) = self.chats.get_mut(&peer) {
            chat.set_xfer_named(mine, name, size, state, &mut inv);
        }
    }

    /// 대화 뷰에서 나온 발신·복귀를 처리한다. `peer` = 그 뷰의 상대.
    fn drain_chat_effects(&mut self, el: &ActiveEventLoop, peer: PeerId, id: WindowId) {
        let mut inv = Invalidations::default();
        // 진행 배너 "취소"(08-16) — 위젯은 세션을 모른다 · 라우팅은 호스트.
        if self
            .chats
            .get_mut(&peer)
            .is_some_and(ChatViewWidget::take_xfer_cancel)
        {
            self.cancel_active_xfer(peer);
        }
        // 라인별 트랜스포트 제어(M4-2e) — 항목 옆 ⏸▶✕ 아이콘.
        if let Some(ctl) = self
            .chats
            .get_mut(&peer)
            .and_then(ChatViewWidget::take_xfer_ctl)
        {
            self.handle_xfer_ctl(peer, ctl);
        }
        // 인라인 썸네일 클릭 = 확대 미리보기(08-16 · M4-5 잔여) — 파일명은 그
        // 항목의 무해화 통과분을 쓴다(경로 basename은 격리 해시 이름이라 무의미).
        if let Some(qp) = self
            .chats
            .get_mut(&peer)
            .and_then(ChatViewWidget::take_open_image)
        {
            let title = std::path::Path::new(&qp)
                .file_stem()
                .map(|s| s.to_string_lossy().into_owned())
                .unwrap_or_else(|| "이미지".into());
            self.open_image_view(el, id, qp, title);
        }
        // 풍선 우클릭 복사(08-10) — 위젯은 OS를 모르므로 여기서 클립보드에 쓴다.
        if let Some(t) = self
            .chats
            .get_mut(&peer)
            .and_then(ChatViewWidget::take_copy_text)
        {
            // 실패도 말한다(조용한 무반응은 디버깅 불가 — 08-10 실기에서 배운 것).
            self.set_status(if nbeep_plat::clipboard::set_text(&t) {
                "메시지 복사됨".to_string()
            } else {
                "복사 실패 — 클립보드를 열 수 없습니다".to_string()
            });
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
            } else if !self.try_clipboard_image_paste(id) {
                // 텍스트도 이미지도 없다(③ 08-20 — 이미지는 파일 전송으로 폴백).
                self.set_status(nbeep_core::t(nbeep_core::Msg::StPasteFail));
                if let Some(mid) = self.main_id {
                    self.request_redraw(mid);
                }
            }
            self.request_redraw(id);
        }
        // 등급 배지가 긴급으로 순환됨(④) — 마찰 1단계 = 상태바 경고.
        if self
            .chats
            .get_mut(&peer)
            .is_some_and(ChatViewWidget::take_grade_notice)
        {
            self.set_status(nbeep_core::t(nbeep_core::Msg::StUrgentArmed));
            if let Some(mid) = self.main_id {
                self.request_redraw(mid);
            }
        }
        let outgoing = self
            .chats
            .get_mut(&peer)
            .and_then(ChatViewWidget::take_outgoing);
        // ★ 명령 가름(08-15) — 입력이 `/…`면 **보내지 않고** 로컬에서 실행한다.
        //   판정은 `core::command` 한 곳(1:1·그룹·CLI 공용 문법).
        let (outgoing, cmd_grade) = match self.run_chat_command(outgoing.as_ref(), Some(peer)) {
            CmdOutcome::Send(t) => (t, None),
            CmdOutcome::SendGraded(t, g) => (Some(t), Some(g)),
            CmdOutcome::Handled => {
                self.request_redraw(id);
                return;
            }
        };
        if let Some(text) = outgoing {
            // 등급(④ 08-20) — 명령(/notice·/urgent)이 우선, 아니면 입력줄 배지
            // 선택(1회 적용 후 일반 복귀 — Urgent 마찰 원칙 · docs/24 §3-1).
            let grade = cmd_grade.unwrap_or_else(|| {
                self.chats
                    .get_mut(&peer)
                    .map_or(0, ChatViewWidget::take_grade)
            });
            let importance = match grade {
                2 => nbeep_core::Importance::Urgent,
                1 => nbeep_core::Importance::Notice,
                _ => nbeep_core::Importance::Normal,
            };
            let (at_ms, wall) = now_stamp();
            self.trust.note_chat(peer, unix_now_ms()); // 최근 대화(08-15 — 발신도)
            if self.conversations.contains_key(&peer) {
                let msg = nbeep_core::ChatMessage {
                    sender_device: self.identity.peer_id(),
                    seq: self.seq.issue(),
                    body: nbeep_core::MessageBody::Text(text.as_str().to_string()),
                    importance,
                    broadcast: false,
                };
                if let Some(chat) = self.chats.get_mut(&peer) {
                    chat.push_line(
                        ChatLine::text(true, text.clone(), at_ms, wall)
                            .with_seq(msg.seq)
                            .with_importance(grade),
                        &mut inv,
                    );
                }
                // 왕래 장부 — 파일 전송 자격(상호 확인)의 근거(사용자 확정 08-09).
                self.ledger.note_sent(peer);
                if let Some(conv) = self.conversations.get_mut(&peer) {
                    conv.lines.push(
                        ChatLine::text(true, text, at_ms, wall)
                            .with_seq(msg.seq)
                            .with_importance(grade),
                    );
                    // 액터에 발신 요청 — 수신은 비동기로 AppEvent::Recv로 돌아온다(M2-7).
                    if conv.out_tx.send(SessionCmd::Chat(msg.encode())).is_err() {
                        self.set_status(nbeep_core::t(nbeep_core::Msg::StSessionEnded));
                    } else {
                        self.status =
                            nbeep_core::tf(nbeep_core::Msg::StfSentSeq, &[&msg.seq.to_string()]);
                    }
                }
                self.record_history(peer); // 대화 기록 영속(M2-5b · 빌림 밖)
            } else {
                // ★ 세션 없음 = **오프라인 대기**(M4-6 · 08-20 사용자 확정 — 재시작
                //   유지). 종전엔 풍선만 남고 전송·기록 모두 **조용히 유실**됐다.
                //   보관 후 상대가 나타나면 자동 전달(한계 = 내 PC가 켜져 있어야 —
                //   Q-25-2 · 상태바 문구로 명시). 발신 의사 = 즉시 연결 시도.
                let q = self.pending_direct.entry(peer).or_default();
                q.push(PendingDirect {
                    text: text.as_str().to_string(),
                    at_ms,
                    importance: grade,
                });
                if q.len() > PENDING_DIRECT_MAX {
                    let drop_n = q.len() - PENDING_DIRECT_MAX;
                    q.drain(..drop_n);
                }
                let total = q.len();
                if let Some(chat) = self.chats.get_mut(&peer) {
                    chat.push_line(
                        ChatLine::text(true, text, at_ms, wall)
                            .with_queued(true)
                            .with_importance(grade),
                        &mut inv,
                    );
                }
                self.save_pending(peer);
                self.set_status(nbeep_core::tf(
                    nbeep_core::Msg::StfQueuedSaved,
                    &[&total.to_string()],
                ));
                self.reconnect.remove(&peer); // 발신 의사 = 백오프 처음부터(그룹 규약)
                self.start_connect(peer, true);
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
                    self.set_status(nbeep_core::t(nbeep_core::Msg::ListNavHint));
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
        // 앱 모달(08-14 표준 재정리 — About에만 있던 문법을 일반화): 모달이 열려
        // 있으면 **다른 앱 창의 입력은 삼키고** 클릭/키는 모달로 포커스를 되돌린다.
        // 다른 프로그램 위로는 뜨지 않는다(AlwaysOnTop 제거 — OS 창 전환 관례).
        if let Some(mid) = self.modal_id() {
            if mid != id {
                if matches!(
                    ev,
                    InputEvent::MouseDown { .. }
                        | InputEvent::Key { .. }
                        | InputEvent::RightDown { .. }
                ) {
                    if let Some(e) = self.windows.get(&mid) {
                        e.window.focus_window();
                    }
                }
                return;
            }
        }
        let mut inv = Invalidations::default();
        match role {
            Role::ImageView => {
                // 확대 미리보기(08-16) — Esc = 닫기(카드류 관례). 그 외 입력 없음.
                if matches!(
                    ev,
                    nbeep_ui::InputEvent::Key {
                        key: nbeep_ui::Key::Escape,
                        ..
                    }
                ) {
                    self.close_image_view();
                }
            }
            Role::Main => {
                if let Some(gid) = self.single_open_group {
                    // 그룹 스레드(단일 모드 · M5-1) — 1:1 전환과 같은 문법.
                    if let Some(chat) = self.gchats.get_mut(&gid) {
                        chat.on_event(&ev, &mut inv);
                    }
                    self.drain_group_effects(gid, id);
                } else if let Some(peer) = self.single_open {
                    if let Some(chat) = self.chats.get_mut(&peer) {
                        chat.on_event(&ev, &mut inv);
                    }
                    self.drain_chat_effects(el, peer, id);
                } else {
                    // 메뉴가 열려 있으면 모달 캡처(목록으로 전파 금지).
                    if self.menu.is_open() {
                        self.menu.on_event(&ev, &mut inv);
                        inv.push(self.list.bounds()); // 팝업 영역 재도색
                    } else if self.sort_drop.is_open() {
                        // 정렬 팝업 캡처(08-15) — 콤보·메뉴와 같은 모달 규약.
                        self.sort_drop.on_event(&ev, &mut inv);
                        inv.push(self.list.bounds());
                    } else {
                        self.menu.on_event(&ev, &mut inv);
                        self.toolbar.on_event(&ev, &mut inv);
                        self.sort_drop.on_event(&ev, &mut inv);
                        // ★ 스트레이 Enter 가드(08-21 — Focused(true) 참조): 모달
                        //   제출 Enter의 잔향이 목록 캐럿 행을 활성화하지 않게.
                        let stray_enter = matches!(
                            ev,
                            nbeep_ui::InputEvent::Key {
                                key: nbeep_ui::Key::Enter,
                                ..
                            }
                        ) && self.now_ms() < self.enter_guard_until_ms;
                        if !self.menu.is_open() && !self.sort_drop.is_open() && !stray_enter {
                            self.list.on_event(&ev, &mut inv);
                        }
                    }
                    // 정렬 선택(08-15) — 정식 깔때기(영속 + hot-swap 재조립).
                    if let Some(v) = self.sort_drop.take_changed() {
                        self.apply_settings(vec![("ui.list_sort", v.to_string())]);
                    }
                    // 액션 드레인 — 메뉴/툴바.
                    if let Some(a) = self.menu.take_picked() {
                        match a.as_str() {
                            "settings" => self.open_settings(el),
                            "quarantine" => self.open_quarantine(el),
                            "convbox" => self.open_convbox(el),
                            // 공지(④) — 본문 입력 프롬프트(발견된 전체 · Notice).
                            "broadcast" => self.open_name_prompt(
                                el,
                                NamePurpose::Broadcast,
                                nbeep_core::t(nbeep_core::Msg::BroadcastTitle),
                                "",
                            ),
                            "gallery" => self.open_gallery(el),
                            "about" => self.open_about(el),
                            // 명시적 종료(사용자 요청 08-15 — 메뉴에 종료가 없어
                            // close_to_tray on이면 앱을 끝낼 길이 트레이뿐이었다).
                            // 트레이 '종료'와 같은 확정적 경로 = 즉시 exit(Drop 체인이
                            // GOODBYE·정리 — X의 전송 가드는 습관적 닫기 방어라 별개).
                            "quit" => el.exit(),
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
                            "convbox" => self.open_convbox(el),
                            "profile" => self.open_profile(el),
                            "gallery" => self.open_gallery(el),
                            _ => {}
                        }
                    }
                    // ★ 잔향 활성화 폐기(08-21 2차) — 프롬프트 모달이 열려 있는 동안
                    //   목록 활성화는 정의상 존재할 수 없고(모달 캡처), 닫힌 직후
                    //   가드 창 안의 활성화는 제출 Enter의 잔향이다. 이벤트 차단
                    //   (stray_enter)이 순서 경합으로 놓친 것까지 여기서 최종 차단.
                    //   단 **키보드(Enter) 유래만**(08-21 3차 실기 — 잔향의 정체가
                    //   키 반복이므로 마우스 더블클릭은 잔향일 수 없다. 종전 무차별
                    //   드레인은 비활성 창 더블클릭까지 삼켰다: 첫 클릭 = 활성화 =
                    //   Focused 가드 무장 → 둘째 클릭의 활성화가 300ms 안 = 폐기).
                    let stray_act = self.name_prompt.is_some()
                        || (self.list.activation_by_key()
                            && self.now_ms() < self.enter_guard_until_ms);
                    match self.list.take_activated() {
                        Some(_) if stray_act => {}
                        Some(nbeep_ui::Activated::Peer(peer)) => self.activate(peer, el),
                        Some(nbeep_ui::Activated::Group(gid)) => self.open_group_thread(gid, el),
                        None => {}
                    }
                    // 우클릭 ▸ 프로필 보기(M3-17).
                    if let Some(peer) = self.list.take_profile_request() {
                        self.open_peer_info(peer, el);
                    }
                    // 목록 고정 토글(08-15) — 신뢰 저장소(암호화 · trust.seg v2)에 영속.
                    if let Some((peer, fav)) = self.list.take_fav_toggle() {
                        self.trust.set_fav(peer, fav);
                        self.refresh_and_redraw();
                        self.set_status(if fav {
                            format!("{} — 목록 상단에 고정", self.peer_title(peer))
                        } else {
                            format!("{} — 목록 고정 해제", self.peer_title(peer))
                        });
                    }
                    // 우클릭 ▸ 목록에서 삭제(08-17) — 핀·캐시 파일까지 지운다.
                    if let Some(peer) = self.list.take_forget_request() {
                        self.forget_peer(peer);
                    }
                    // 그룹 행동(M5-1) — 저장소 반영·이름 모달은 여기(호스트) 몫.
                    if let Some(action) = self.list.take_group_action() {
                        self.handle_group_action(action, el);
                    }
                }
            }
            Role::Chat(peer) => {
                if let Some(chat) = self.chats.get_mut(&peer) {
                    chat.on_event(&ev, &mut inv);
                }
                self.drain_chat_effects(el, peer, id);
            }
            Role::GroupChat(gid) => {
                if let Some(chat) = self.gchats.get_mut(&gid) {
                    chat.on_event(&ev, &mut inv);
                }
                self.drain_group_effects(gid, id);
            }
            Role::NamePrompt => {
                if let Some(p) = &mut self.name_prompt {
                    p.on_event(&ev, &mut inv);
                    let submit = p.take_submit();
                    let cancel = p.take_cancel();
                    if submit.is_some() || cancel {
                        // ★ 스트레이 Enter 가드를 **닫는 시점에도** 세운다(08-21 2차 —
                        //   Focused(true)만으로는 잔향 Enter가 포커스 이벤트보다 먼저
                        //   메인에 도달하는 순서에서 가드가 늦는다). 두 지점 중 먼저
                        //   오는 쪽이 창을 연다.
                        self.enter_guard_until_ms = self.now_ms() + ENTER_GUARD_MS;
                    }
                    if let Some(name) = submit {
                        self.apply_name_prompt(&name);
                        self.name_prompt = None;
                        self.windows.remove(&id);
                    } else if cancel {
                        self.name_prompt = None;
                        self.name_prompt_for = None;
                        self.windows.remove(&id);
                    }
                }
            }
            Role::Settings => {
                if let Some(sv) = &mut self.settings_view {
                    sv.on_event(&ev, &mut inv);
                    let changes = sv.take_changes();
                    let warnings = sv.take_warnings();
                    let close = sv.take_back();
                    if !changes.is_empty() {
                        self.apply_settings(changes);
                    }
                    // 검증 실패(08-20) — 확정 즉시 경고 모달(원복은 위젯이 끝냈다).
                    for w in warnings {
                        self.pending_alert = Some((
                            nbeep_core::t(nbeep_core::Msg::ValOutOfRangeTitle).to_string(),
                            nbeep_core::t(w).to_string(),
                            Some(id),
                        ));
                    }
                    if close {
                        self.settings_view = None;
                        self.windows.remove(&id);
                    }
                }
            }
            Role::Gallery => {
                // Esc = 닫기. 단 텍스트박스 우클릭 메뉴가 열려 있으면 메뉴 몫(메뉴만 닫힘).
                if matches!(
                    ev,
                    InputEvent::Key {
                        key: Key::Escape,
                        ..
                    }
                ) && !self.gallery_view.as_ref().is_some_and(|gv| gv.popup_open())
                {
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
                                        let m = match ctx.purpose {
                                            PickerPurpose::SettingsBackupDir => {
                                                self.do_backup_settings(&dir)
                                            }
                                            PickerPurpose::HistoryBackupDir => {
                                                self.do_backup_history(&dir)
                                            }
                                            PickerPurpose::HistoryRestoreDir => {
                                                self.do_restore_history_dir(&dir)
                                            }
                                            _ => self.do_backup_identity(&dir),
                                        };
                                        self.set_status(m);
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
                                                // 프로필 이미지 경로 반영(M3-17) — 위젯에
                                                // 넣고 **위젯이 보고한 변경 전부**를 정식
                                                // 깔때기(apply_settings)로 저장한다.
                                                // ★ 08-14 실기: 여기서 take_changes를
                                                // 버려서 최근 목록(image_recent)이 영속되지
                                                // 않았다(재시작 = 전부 증발). 디코드도
                                                // apply_settings의 image_path 팔이 한다
                                                // (수동 spawn과 이중이었다).
                                                // 관리 복사(08-16) — 사본 경로가 정본.
                                                let path =
                                                    self.manage_profile_image(&p.to_string_lossy());
                                                let changes =
                                                    if let Some(pv) = &mut self.profile_view {
                                                        let mut pinv = Invalidations::default();
                                                        pv.set_image_path(&path, &mut pinv);
                                                        pv.take_changes()
                                                    } else {
                                                        vec![("profile.image_path", path.clone())]
                                                    };
                                                self.apply_settings(changes);
                                                // M3-18 — 선택은 보류 편입: 적용을
                                                // 눌러야 저장·전파된다(원자적 저장).
                                                self.set_status(format!(
                                                    "프로필 이미지 = {path} — 적용 시 반영"
                                                ));
                                                if let Some((pid, _)) = self
                                                    .windows
                                                    .iter()
                                                    .find(|(_, e)| e.role == Role::Profile)
                                                {
                                                    let pid = *pid;
                                                    self.request_redraw(pid);
                                                }
                                            }
                                            PickerPurpose::SettingsRestoreFile => {
                                                let m = self.do_restore_settings(&p);
                                                self.set_status(m);
                                            }
                                            PickerPurpose::HistoryRestoreDir => {
                                                let m = self.do_restore_history_files(&[p]);
                                                self.set_status(m);
                                            }
                                            _ => {
                                                let m = self.do_restore_identity(&p);
                                                self.set_status(m);
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
                let mut row = None;
                let mut closed_choice = None;
                if let Some(av) = &mut self.alert_view {
                    av.on_event(&ev, &mut inv);
                    row = av.take_row_clicked();
                    let choice = av.take_choice();
                    if av.take_closed() {
                        self.alert_view = None;
                        self.windows.remove(&id);
                        self.members_ctx = None; // 모달 닫힘 = 문맥 소거(G4)
                        closed_choice = Some(choice);
                    }
                }
                // G4(08-15) — 구성원 행 클릭(소유자) = 제외 확인 모달로 진입.
                if let Some(i) = row {
                    if let Some((gid, order, owned)) = self.members_ctx.clone() {
                        let me = self.identity.peer_id();
                        if owned {
                            if let Some(&peer) = order.get(i) {
                                if peer != me {
                                    let name = self.peer_title(peer);
                                    self.open_choice(
                                        el,
                                        "구성원 제외",
                                        &format!(
                                            "'{name}' 님을 이 그룹에서 제외할까요?\n(제외자에게 마지막 명부가 1회 배포됩니다)"
                                        ),
                                        "제외",
                                        "취소",
                                        AlertCtx::GroupKick { gid, peer },
                                    );
                                }
                            }
                        }
                    }
                }
                // 선택 모달(초대·제외 등) — 결과를 문맥으로 라우팅(M5-1g).
                if let Some(choice) = closed_choice {
                    if let (Some(yes), Some(ctx)) = (choice, self.alert_ctx.take()) {
                        self.apply_alert_choice(yes, ctx, el);
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
            Role::PeerInfo(peer) => {
                let (mut closed, mut verify, mut unverify) = (false, false, false);
                if let Some(pv) = &mut self.peer_info_view {
                    pv.on_event(&ev, &mut inv);
                    closed = pv.take_closed();
                    verify = pv.take_verify();
                    unverify = pv.take_unverify();
                }
                if unverify {
                    // 인증 취소(08-17) — SAS 승격을 되돌려 배지를 강등한다(`/unverify`와
                    // 같은 동작). 카드는 열어 둔 채 버튼만 다시 '대조 완료'로 돌아간다.
                    self.unverify_peer(peer);
                    self.refresh_peer_info_card(peer);
                    if let Some(mid) = self.main_id {
                        self.request_redraw(mid);
                    }
                    self.request_redraw(id);
                }
                if verify {
                    // SAS 대조 완료(M3-6) — 사람이 확인했다는 선언을 신뢰 저장소에
                    // 승격(FingerprintVerified · write-through 영속). 배지는 파란
                    // 실(verified)로 — 목록·카드가 즉시 따라간다.
                    use nbeep_core::TrustStore as _;
                    self.trust.verify(peer);
                    self.status =
                        nbeep_core::tf(nbeep_core::Msg::StfVerified, &[&self.peer_title(peer)]);
                    let mut rinv = Invalidations::default();
                    self.refresh_rows(&mut rinv);
                    self.refresh_peer_info_card(peer); // 버튼 → ✓ 완료 표시로
                    if let Some(mid) = self.main_id {
                        self.request_redraw(mid);
                    }
                    self.request_redraw(id);
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
                    // 관리 복사 관문(08-16) — 위젯발 image_path(캐러셀의 옛 원본
                    // 경로 항목 포함)도 사본으로 치환해 한 관문으로 모은다.
                    let changes: Vec<_> = changes
                        .into_iter()
                        .map(|(k, v)| {
                            if k == "profile.image_path" && !v.is_empty() {
                                (k, self.manage_profile_image(&v))
                            } else {
                                (k, v)
                            }
                        })
                        .collect();
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
            Role::Quarantine => {
                let (mut act, mut preview) = (None, None);
                if let Some(qv) = &mut self.quarantine_view {
                    qv.on_event(&ev, &mut inv);
                    act = qv.take_action();
                    preview = qv.take_preview();
                    if qv.take_back() {
                        self.quarantine_view = None;
                        self.windows.remove(&id);
                    }
                }
                if let Some(qp) = preview {
                    let secret = self.identity.wrap_secret();
                    let meta = crate::gate::read_qmeta(std::path::Path::new(&qp), &secret);
                    if meta.as_ref().map(|m| m.risk) == Some(nbeep_core::RiskLevel::Archive) {
                        // 아카이브 재클릭 = **내용 목록**(M4-4 ⓐ — 해제 없는 중앙
                        // 디렉터리 목록 · 이미지의 확대 미리보기와 같은 진입점).
                        // 개봉(수백 MB 가능)은 워커 — 완료가 ArchiveList로 돌아온다.
                        let title = meta.map_or_else(|| "archive".into(), |m| m.name);
                        let proxy = self.proxy.clone();
                        std::thread::spawn(move || {
                            let body =
                                crate::gate::read_beepq_bytes(std::path::Path::new(&qp), &secret)
                                    .map_or_else(
                                        || {
                                            nbeep_core::t(nbeep_core::Msg::ArchiveUnreadable)
                                                .to_string()
                                        },
                                        |b| archive_listing_body(&b),
                                    );
                            let _ = proxy.send_event(AppEvent::ArchiveList {
                                title,
                                body,
                                anchor: id,
                            });
                        });
                    } else {
                        // 선택 행 재클릭 = 확대 미리보기(08-16 — 진입점 ② · 격리
                        // 상태 그대로 · 스레드 썸네일과 같은 뷰어).
                        let title = std::path::Path::new(&qp)
                            .file_stem()
                            .map(|t| t.to_string_lossy().into_owned())
                            .unwrap_or_else(|| "격리물".into());
                        self.open_image_view(el, id, qp, title);
                    }
                }
                if let Some(a) = act {
                    self.run_quarantine_action(a, id);
                }
            }
            Role::Convbox => {
                let mut act = None;
                if let Some(cv) = &mut self.convbox_view {
                    cv.on_event(&ev, &mut inv);
                    act = cv.take_action();
                    if cv.take_back() {
                        self.convbox_view = None;
                        self.windows.remove(&id);
                    }
                }
                if let Some(a) = act {
                    self.run_convbox_action(a, id, el);
                }
            }
        }
        if !inv.is_empty() {
            self.request_redraw(id);
        }
        // 텍스트 필드 우클릭 메뉴의 클립보드 행동(08-13 전수 검사) — ⌘C/X/V와
        // 같은 경로로 잇는다(위젯은 요청만 남기고 OS 클립보드는 여기서).
        self.drain_edit_ctx(id);
    }

    /// 텍스트 필드(프로필·이름/주소 프롬프트·설정)의 우클릭 편집 메뉴 행동 처리.
    fn drain_edit_ctx(&mut self, id: WindowId) {
        let action = match self.windows.get(&id).map(|e| e.role) {
            Some(Role::Profile) => self.profile_view.as_mut().and_then(|v| v.take_edit_ctx()),
            Some(Role::NamePrompt) => self.name_prompt.as_mut().and_then(|v| v.take_edit_ctx()),
            Some(Role::Convbox) => self.convbox_view.as_mut().and_then(|v| v.take_edit_ctx()),
            Some(Role::AddEndpoint) => self.addr_view.as_mut().and_then(|v| v.take_edit_ctx()),
            Some(Role::Settings) => self.settings_view.as_mut().and_then(|v| v.take_edit_ctx()),
            Some(Role::Gallery) => self.gallery_view.as_mut().and_then(|v| v.take_edit_ctx()),
            _ => None,
        };
        let Some(a) = action else { return };
        match a {
            nbeep_ui::controls::EditCtxAction::Copy => {
                if let Some(t) = self.clipboard_copy_for(id) {
                    if nbeep_plat::clipboard::set_text(&t) {
                        self.set_status(nbeep_core::t(nbeep_core::Msg::StCopied));
                    }
                }
            }
            nbeep_ui::controls::EditCtxAction::Cut => {
                if let Some(t) = self.clipboard_cut_for(id) {
                    nbeep_plat::clipboard::set_text(&t);
                }
            }
            nbeep_ui::controls::EditCtxAction::Paste => {
                if let Some(t) = nbeep_plat::clipboard::get_text() {
                    self.clipboard_paste_for(id, &t);
                }
            }
        }
        self.request_redraw(id);
    }

    fn redraw(&mut self, id: WindowId) {
        let theme = self.theme;
        let prefs = self.fonts;
        // 캐럿 깜빡임 위상(08-13 사용자 요청) — **OS 포커스 창에서만** 점멸(비포커스
        // 창은 소등 · 네이티브 관례 DR-16). 입력이 기준점을 리셋해 타이핑 중엔 항상 밝다.
        let caret_on = self.os_focused == Some(id)
            && (self.now_ms().saturating_sub(self.blink_anchor_ms) / CARET_BLINK_MS) % 2 == 0;
        // 슬롯 얼굴을 **필드에서 직접** 빌린다 — 헬퍼 메서드로 감싸면 self 전체를 빌려
        // 아래 windows 가변 차용과 충돌한다(필드 단위 분할 차용을 쓰기 위한 형태).
        let fonts = nbeep_ui::FontSet {
            base: self.face_base.as_ref().unwrap_or(&self.font),
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
            .with_scale(entry.scale)
            .with_caret_on(caret_on);
        match entry.role {
            Role::ImageView => {
                // 확대 미리보기(08-16) — contain 맞춤 중앙 배치 · 상태 3종.
                let (fw, fh) = (size.width as i32, size.height as i32);
                let full = Rect::new(0, 0, fw, fh);
                ctx.fill_rect(full, theme.panel_bg);
                match self.image_view.as_ref().map(|v| &v.img) {
                    Some(ImgLoad::Ready(img)) => {
                        let (iw, ih) = (img.w as i32, img.h as i32);
                        if iw > 0 && ih > 0 {
                            let pad = (8.0 * entry.scale) as i32;
                            let (aw, ah) = ((fw - pad * 2).max(1), (fh - pad * 2).max(1));
                            // contain — 비율 유지 최대(확대는 원본 크기까지만 —
                            // 96px 썸네일 뻥튀기의 흐림을 피한다).
                            let sc = (f64::from(aw) / f64::from(iw))
                                .min(f64::from(ah) / f64::from(ih))
                                .min(f64::from(entry.scale)); // 논리 1x 이상 확대 금지
                            #[allow(clippy::cast_possible_truncation)]
                            let (dw, dh) = (
                                ((f64::from(iw) * sc).round() as i32).max(1),
                                ((f64::from(ih) * sc).round() as i32).max(1),
                            );
                            let dst = Rect::new((fw - dw) / 2, (fh - dh) / 2, dw, dh);
                            ctx.image_scaled(dst, img, full);
                        }
                    }
                    Some(ImgLoad::Loading) => {
                        ctx.select_font(nbeep_ui::FontSlot::Status, false);
                        let msg = "불러오는 중…";
                        let tw = ctx.text_width(msg);
                        ctx.text((fw - tw) / 2, fh / 2, full, msg, theme.text_dim);
                    }
                    _ => {
                        ctx.select_font(nbeep_ui::FontSlot::Status, false);
                        let msg = "미리보기를 만들 수 없습니다(16MiB 초과·손상·imgdec 부재)";
                        let tw = ctx.text_width(msg);
                        ctx.text((fw - tw) / 2, fh / 2, full, msg, theme.text_dim);
                    }
                }
            }
            Role::Main => {
                if let Some(chat) = self.single_open_group.and_then(|g| self.gchats.get(&g)) {
                    chat.paint(&mut ctx, &theme); // 그룹 스레드(M5-1 — 1:1 전환과 같은 문법)
                } else if let Some(chat) = self.single_open.and_then(|p| self.chats.get(&p)) {
                    chat.paint(&mut ctx, &theme);
                } else {
                    self.list.paint(&mut ctx, &theme);
                    // 상단 크롬(툴바 → 메뉴 순 — 메뉴 팝업이 최상위).
                    self.toolbar.paint(&mut ctx, &theme);
                    self.sort_drop.paint(&mut ctx, &theme); // 정렬 드롭다운 버튼(08-15)
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
                    let label = format!("{}{p}", nbeep_core::t(nbeep_core::Msg::PortLabel));
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
                // 팝업(우클릭 메뉴) 재도색 — 상태 바가 메뉴를 덮지 않게 **맨 마지막**
                // (08-13 실기: 메뉴가 하단 정보 텍스트 아래 깔렸다).
                if let Some(chat) = self.single_open_group.and_then(|g| self.gchats.get(&g)) {
                    chat.paint_popup(&mut ctx, &theme);
                } else if let Some(chat) = self.single_open.and_then(|p| self.chats.get(&p)) {
                    chat.paint_popup(&mut ctx, &theme);
                } else {
                    self.list.paint_popup(&mut ctx, &theme);
                    self.sort_drop.paint_popup(&mut ctx, &theme); // 정렬 팝업(맨 위)
                }
            }
            Role::Chat(peer) => {
                if let Some(chat) = self.chats.get(&peer) {
                    chat.paint(&mut ctx, &theme);
                }
            }
            Role::GroupChat(gid) => {
                if let Some(chat) = self.gchats.get(&gid) {
                    chat.paint(&mut ctx, &theme);
                }
            }
            Role::NamePrompt => {
                if let Some(p) = &self.name_prompt {
                    p.paint(&mut ctx, &theme);
                    // 제목 word-wrap 실측 높이에 창을 맞춘다(08-21 — 긴 제목 잘림).
                    // paint가 실측을 남기므로 첫 프레임 뒤 1회 커진다(±2px 무시 ·
                    // Resized 이벤트가 재레이아웃을 몰고 온다). entry.window는
                    // ctx가 빌린 surface와 다른 필드라 동시 접근 가능.
                    let want = p.desired_height();
                    #[allow(clippy::cast_possible_wrap, clippy::cast_sign_loss)]
                    if want > 0 && (want - size.height as i32).abs() > 2 {
                        let _ = entry
                            .window
                            .request_inner_size(winit::dpi::PhysicalSize::new(
                                size.width,
                                want.max(1) as u32,
                            ));
                    }
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
            Role::Convbox => {
                if let Some(cv) = &self.convbox_view {
                    cv.paint(&mut ctx, &theme);
                }
            }
            Role::Approve(peer) => {
                if let Some(pv) = self.approve_view.get(&peer) {
                    pv.paint(&mut ctx, &theme);
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
        // 마지막 종료 위치·크기 복원(08-14 사용자 확정 — 다중 인스턴스 실기에서
        // 매번 재배치하던 것. 신원(data/)별로 저장되니 인스턴스마다 자기 자리를
        // 기억한다). 값이 없거나 무효면 기본값 — ADR-0011 관용 파싱 원칙 그대로.
        let geo = |k: &str| self.settings.get(k).parse::<i32>().ok();
        let (ww, wh) = match (geo("ui.win_w"), geo("ui.win_h")) {
            (Some(w), Some(h)) if (200..=8000).contains(&w) && (150..=8000).contains(&h) => {
                (f64::from(w), f64::from(h))
            }
            _ => (460.0, 640.0), // 기본 크기(사용자 확정 08-09)
        };
        let mut attrs = Window::default_attributes()
            .with_title("Nexa Beep")
            .with_inner_size(winit::dpi::LogicalSize::new(ww, wh))
            .with_window_icon(self.icon.clone());
        if let (Some(x), Some(y)) = (geo("ui.win_x"), geo("ui.win_y")) {
            // 화면 밖 좌표 방어는 OS 몫이 크지만, 음수 심연(-32000 최소화 잔재)은 거른다.
            if (-4000..=16000).contains(&x) && (-4000..=16000).contains(&y) {
                attrs = attrs
                    .with_position(winit::dpi::LogicalPosition::new(f64::from(x), f64::from(y)));
            }
        }
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
        // 첫 그리기 전에 목록을 한 번 조립한다 — 부팅 복원(캐시 프로필·핀 행)이
        // 발견 이벤트 없이도 바로 보이게(08-14 · 그전엔 첫 ANNOUNCE까지 빈 목록).
        let mut inv = Invalidations::default();
        self.refresh_rows(&mut inv);
        // keytap 설치(G1 · H-26 — mac 한정): winit이 삼키는 "조합 직후 첫 1byte"를
        // 로컬 모니터로 관측해 보충한다. 단일 설치(resumed는 main_id 가드로 1회).
        #[cfg(target_os = "macos")]
        {
            let proxy = self.proxy.clone();
            nbeep_plat::keytap::install_keydown_tap(Box::new(move |c| {
                let _ = proxy.send_event(AppEvent::RawKey(c));
            }));
        }
    }

    fn user_event(&mut self, el: &ActiveEventLoop, event: AppEvent) {
        // 세션 액터 → GUI(M2-7). 수신 메시지를 해당 대화 스레드에 실시간 반영한다.
        match event {
            #[cfg(target_os = "macos")]
            AppEvent::RawKey(c) => {
                // keytap 관측(G1) — 정산은 게이트가(다음 keydown 대조 · 틱 폴백).
                let _ = el;
                let now = self.now_ms();
                if self.ime_trace {
                    eprintln!("[ime] raw={c:?}");
                }
                self.ime.observe_raw(c, now);
            }
            // 클립보드 이미지 준비 완료(③ 08-20) — data/clipboard/에 PNG로 저장 후
            // 기존 파일 전송 경로(offer_file — 1:1·그룹·상한·승인 전부 공용)에 태운다.
            AppEvent::ClipImage { win, png } => {
                let Some(png) = png else {
                    self.set_status(nbeep_core::t(nbeep_core::Msg::StClipImageNone));
                    if let Some(mid) = self.main_id {
                        self.request_redraw(mid);
                    }
                    return;
                };
                let dir = self.data_dir.join("clipboard");
                let _ = std::fs::create_dir_all(&dir);
                let (at_ms, _) = now_stamp();
                let path = dir.join(format!("clip-{at_ms}.png"));
                if std::fs::write(&path, &png).is_ok() {
                    self.offer_file(win, &path);
                } else {
                    self.set_status(nbeep_core::t(nbeep_core::Msg::StClipImageNone));
                }
                self.request_redraw(win);
                if let Some(mid) = self.main_id {
                    self.request_redraw(mid);
                }
            }
            AppEvent::Recv {
                peer,
                text,
                seq,
                sender,
                importance,
                broadcast,
            } => {
                if !self.dedup.accept(sender, seq) {
                    return; // 중복(다중 경로 — FR-M-9)
                }
                // ★ 공지 받지 않기(08-21 사용자 확정 — 옵트아웃): 공지 표식이 선
                //   메시지는 표시·알림·기록·전달 ack 전부 없이 버린다 — 발신자
                //   쪽에서는 오프라인·미검증 수신자와 구분되지 않는다(그림자 무시 ·
                //   설정 hot-swap · 무시 설정 전에 도착한 공지는 그대로 남는다).
                if broadcast && self.settings.get("notify.broadcast_mute") == "on" {
                    return;
                }
                let cur = self.last_recv_seq.entry(peer).or_insert(0);
                *cur = (*cur).max(seq); // 읽음 ack 대상(N-2 up-to)
                self.ledger.note_recv(peer); // 왕래 장부(상호 확인)
                self.trust.note_chat(peer, unix_now_ms()); // 최근 대화(08-15)
                let notify_body = self.notify_body(text.as_str()); // move 전에 뜬다
                let (at_ms, wall) = now_stamp();
                // 발신자 표시 이름(08-19 사용자 요청 — 1:1도 수신 풍선 위에 이름).
                let from = self.peer_title(peer);
                let line = ChatLine::text(false, text, at_ms, wall)
                    .with_from(from)
                    .with_importance(importance); // ④ 등급 링(발신자의 요청 표시)
                if let Some(conv) = self.conversations.get_mut(&peer) {
                    conv.lines.push(line.clone());
                }
                self.record_history(peer); // 대화 기록 영속(M2-5b · 빌림 밖)
                                           // ★ 수신 확인(N-2 · 사용자 요청 08-17) — **수신자가 설정으로 제어**.
                                           //   `chat.send_delivered`가 켜져 있고, **검증된 상대**일 때만 자동
                                           //   Delivered를 되쏜다(프라이버시: 미검증에게 "받았다/온라인"을 안
                                           //   흘린다 — 알림 신뢰 게이트와 같은 결). 사람 확인(Acknowledged)은
                                           //   수동 버튼(M3-9). 액터가 아니라 여기서 — 설정이 단일 원천(hot-swap).
                {
                    use nbeep_core::TrustStore as _;
                    let on = self.settings.get("chat.send_delivered") == "on";
                    let verified = self.trust.level(peer) != nbeep_core::TrustLevel::Unverified;
                    if on && verified {
                        if let Some(conv) = self.conversations.get(&peer) {
                            let ack = nbeep_core::ChatAck {
                                target_seq: seq,
                                kind: nbeep_core::AckKind::Delivered,
                            };
                            let _ = conv.out_tx.send(SessionCmd::Control(vec![ack.encode()]));
                        }
                    }
                }
                let mut inv = Invalidations::default();
                if let Some(chat) = self.chats.get_mut(&peer) {
                    chat.push_line(line, &mut inv);
                }
                // 읽음/안읽음 계상(③) — 뷰가 닫혀 있으면 배지·제목으로 알린다.
                self.note_incoming(peer);
                // OS 알림(M3-8) — 앱이 뒤에 있을 때만 · 미검증 = 무음(DR-25).
                // ④ 등급 강도(docs/24 §3-3 근사): **미검증 = 자동 강등(종전 무음
                // 게이트가 이긴다)** · 검증·핀 상대의 Urgent = **앱이 앞에 있어도**
                // 알림(force — "지금 당장"의 요청). Notice는 종전 배경 알림 그대로.
                use nbeep_core::TrustStore as _;
                let silent = self.trust.level(peer) == nbeep_core::TrustLevel::Unverified;
                let title = self.peer_title(peer);
                let force = importance >= 2 && !silent;
                self.notify_user(
                    &format!("p:{}", peer.short()),
                    &title,
                    &notify_body,
                    silent,
                    force,
                    NotifyTarget::Peer(peer),
                );
                // 이 대화가 보이는 창을 다시 그린다.
                self.redraw_conversation(peer);
            }
            AppEvent::ChatAck {
                peer,
                target_seq,
                kind,
            } => {
                // 수신 확인 도착(N-2) — 내가 보낸 seq의 전달/읽음을 **독립 갱신**.
                // '읽음 up-to' 규약: Read는 그 seq 이하 내 메시지 전부 읽음(대화창
                // 열면 보이는 것 다 읽힌다) · Delivered는 그 seq 하나(전달과 독립).
                use nbeep_core::AckKind;
                let (deliv, read) = match kind {
                    AckKind::Delivered => (true, false),
                    AckKind::Read => (false, true),
                };
                let mut inv = Invalidations::default();
                if let Some(chat) = self.chats.get_mut(&peer) {
                    if read {
                        chat.mark_read_upto(target_seq, &mut inv);
                    } else {
                        chat.mark_ack(target_seq, deliv, false, &mut inv);
                    }
                }
                if let Some(conv) = self.conversations.get_mut(&peer) {
                    for l in &mut conv.lines {
                        if l.mine && l.seq != 0 {
                            if read && l.seq <= target_seq {
                                l.read = true;
                            } else if deliv && l.seq == target_seq {
                                l.delivered = true;
                            }
                        }
                    }
                }
                self.redraw_conversation(peer);
            }
            AppEvent::XferOffer {
                peer,
                id,
                name,
                size,
                sha256,
            } => {
                let _ = sha256; // 지연 해시(08-18) — Offer 선언은 0, 검증은 Done에서
                use nbeep_core::{
                    judge_offer, DenyReason, OfferVerdict, RejectWhy, TrustStore as _,
                };
                // 수신 xid 장부(M4-2e ⑥) — 이름으로 ⏸▶✕ 대상 xid를 찾는다
                // (active_recv 1슬롯은 배치에서 최신 파일로 덮인다).
                {
                    let list = self.recv_xids.entry(peer).or_default();
                    list.retain(|(n, _)| n != &name);
                    list.push((name.clone(), id));
                    if list.len() > 32 {
                        list.remove(0);
                    }
                }
                // ★ 원격 × 미대조 = 수신도 차단(M5-3b — 발신 게이트의 대칭 · 같은
                //   판정 함수). 승인 창까지 안 가고 즉시 거절 — 원격 미대조 상대의
                //   파일 제안은 사용자에게 물을 일 자체가 아니다(§5-1-3 fail-closed).
                if self.remote_file_blocked(peer) {
                    self.send_xfer_decision(peer, id, false, RejectWhy::Declined, None);
                    self.set_status(nbeep_core::t(nbeep_core::Msg::StRemoteFileBlocked));
                    if let Some(mid) = self.main_id {
                        self.request_redraw(mid);
                    }
                    return;
                }
                // ★ 판정은 **여기 한 곳**에서만 — 신뢰·왕래 장부·설정이 전부 여기 있다.
                // 액터는 중계만 하므로 정책이 두 벌로 갈라지지 않는다.
                self.tick_approval();
                let verdict = judge_offer(
                    self.trust.level(peer),
                    self.ledger.get(peer),
                    self.approval,
                    self.now_ms(),
                );
                // ★ 이전 승인 연장(M4-10c · 08-18 사용자 요청) — `.part`는 **수락된**
                //   수신에서만 남는다(take_partials가 accepted만 회수). 즉 매치 =
                //   "이 파일은 이미 승인했었다"의 증거 → 승인 창을 다시 묻지 않고
                //   이어받는다. 차단·미검증(Deny)은 그대로 거절(우회 아님).
                let rkey = crate::part::resume_key(&name, size, peer); // 지연 해시 — 키 기반
                if !matches!(verdict, OfferVerdict::Deny(_)) {
                    if let Some(prefix) = self.load_resume(peer, &rkey, size) {
                        let pct = prefix.len() as u64 * 100 / size.max(1);
                        self.send_xfer_decision(peer, id, true, RejectWhy::Declined, Some(prefix));
                        self.set_status(nbeep_core::tf(
                            nbeep_core::Msg::XferResumeFrom,
                            &[&name, &pct.to_string()],
                        ));
                        self.push_xfer_line(peer, false, &name, size); // 재활성화 우선
                        self.redraw_conversation(peer);
                        if let Some(mid) = self.main_id {
                            self.request_redraw(mid);
                        }
                        return;
                    }
                }
                match verdict {
                    OfferVerdict::Accept => {
                        let resume = self.load_resume(peer, &rkey, size);
                        self.send_xfer_decision(peer, id, true, RejectWhy::Declined, resume);
                        self.set_status(nbeep_core::tf(
                            nbeep_core::Msg::StfAutoAcceptRecv,
                            &[&name, &human_size(size)],
                        ));
                        self.push_xfer_line(peer, false, &name, size);
                    }
                    OfferVerdict::Ask => {
                        // ★ 요청 단위 **거절** 잔여(M4-2e · 08-20 — 승인의 대칭):
                        //   프롬프트에서 이미 거절한 배치의 파일이면 재확인 없이
                        //   자동 거절 + 대기 라인 종결(2파일 실기: 파일마다 재질문 구멍).
                        let declined_hit = self
                            .batch_declined
                            .get_mut(&peer)
                            .and_then(|rem| batch_take(rem, &name, size));
                        if let Some(emptied) = declined_hit {
                            if emptied {
                                self.batch_declined.remove(&peer);
                            }
                            self.send_xfer_decision(peer, id, false, RejectWhy::Declined, None);
                            self.set_xfer_line(
                                peer,
                                false,
                                nbeep_ui::XferLineState::Failed {
                                    why: nbeep_core::t(nbeep_core::Msg::XferDeclined).into(),
                                },
                            );
                            self.redraw_conversation(peer);
                            if let Some(mid) = self.main_id {
                                self.request_redraw(mid);
                            }
                            return;
                        }
                        // ★ 요청 단위 승인(M4-2e · 사용자 확정) — 앞서 승인한 배치의
                        //   잔여 파일이면 재확인 없이 자동 수락. **manifest의 이름+
                        //   크기와 일치할 때만**(불일치 = 아래 수동 폴백 · fail-closed).
                        let batch_hit = self
                            .batch_approved
                            .get_mut(&peer)
                            .and_then(|rem| batch_take(rem, &name, size));
                        if let Some(emptied) = batch_hit {
                            if emptied {
                                self.batch_approved.remove(&peer);
                            }
                            let resume = self.load_resume(peer, &rkey, size);
                            self.send_xfer_decision(peer, id, true, RejectWhy::Declined, resume);
                            self.push_xfer_line(peer, false, &name, size);
                            self.redraw_conversation(peer);
                            if let Some(mid) = self.main_id {
                                self.request_redraw(mid);
                            }
                            return;
                        }
                        // OS 알림(M3-8) — **파일명은 싣지 않는다**(FR-S-41 금지 목록).
                        {
                            use nbeep_core::TrustStore as _;
                            let silent =
                                self.trust.level(peer) == nbeep_core::TrustLevel::Unverified;
                            let title = self.peer_title(peer);
                            let body = nbeep_core::t(nbeep_core::Msg::NotifyFileOffer).to_string();
                            self.notify_user(
                                &format!("x:{}", peer.short()),
                                &title,
                                &body,
                                silent,
                                false,
                                NotifyTarget::Peer(peer),
                            );
                        }
                        // 스레드에 수신 항목(승인 대기) — 거절하면 이 항목이 실패로 남는다.
                        self.push_xfer_line(peer, false, &name, size);
                        let q = self.pending_offers.entry(peer).or_default();
                        q.push_back((id, name.clone(), size, rkey));
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
                        self.set_status(format!(
                            "파일 수신 요청: {name} ({}) — ⌘/Ctrl+Y 수락 · ⌘/Ctrl+N 거절{more}",
                            human_size(size)
                        ));
                    }
                    OfferVerdict::Deny(reason) => {
                        let why = match reason {
                            DenyReason::Blocked => RejectWhy::Blocked,
                            DenyReason::NotPinned | DenyReason::NoMutualConversation => {
                                RejectWhy::Unverified
                            }
                        };
                        self.send_xfer_decision(peer, id, false, why, None);
                        self.set_status(nbeep_core::tf(
                            nbeep_core::Msg::StfFileRejected,
                            &[&name, reason.message()],
                        ));
                    }
                }
                self.redraw_conversation(peer);
            }
            AppEvent::XferProgress {
                peer,
                got,
                total,
                sending,
                name,
            } => {
                // ★ 취소 경합 가드(08-18 실기) — 취소가 배너를 지운 **뒤에** 액터가
                //   이미 큐에 실어 둔 지각 진행 이벤트가 도착해 배너를 되살렸다
                //   (이후 이벤트가 없어 "수신 25%"에 영구 고착). 활성 전송이
                //   등록돼 있을 때만 반영한다(등록 = 합류점 send_xfer_decision /
                //   XferAccepted · 해제 = 취소·완료·실패·Closed — 축이 하나다).
                let active = if sending {
                    self.active_send.contains_key(&peer)
                } else {
                    self.active_recv.contains_key(&peer)
                };
                // ★ 이름 있는 이벤트는 **라인 갱신만** 가드를 우회한다(M4-2e —
                //   1슬롯 가드가 재개된 파일의 진행 표시를 얼렸던 실기). 단
                //   **배너(xfer_progress)는 활성일 때만** 갱신 — 지각 이벤트가
                //   전체취소 뒤 배너를 되살리던 회귀(실기 08-19 2차) 차단.
                if !active {
                    if !name.is_empty() {
                        self.set_xfer_line_named(
                            peer,
                            sending,
                            &name,
                            total,
                            nbeep_ui::XferLineState::Active { done: got },
                        );
                        self.redraw_conversation(peer);
                    }
                    return;
                }
                let prev = self.xfer_progress.get(&peer).copied();
                // ★ 배너 = **배치 합산**(사용자 확정 08-19 2차): 완료 누적 + 현재
                //   진행 / 배치 총량 · (현재 순서/전체 개수). 발신 = send_batch ·
                //   수신 = recv_batch(manifest 합산). 배치 없으면 현재 파일 값.
                let batch = if sending {
                    self.send_batch.get(&peer).copied()
                } else {
                    self.recv_batch.get(&peer).copied()
                };
                let xp = if let Some((done_f, total_f, done_b, total_b)) = batch {
                    nbeep_ui::XferProgress {
                        done_bytes: done_b.saturating_add(got),
                        total_bytes: total_b.max(done_b.saturating_add(total)),
                        done_files: (done_f + 1).min(total_f.max(1)),
                        total_files: total_f.max(1),
                        sending,
                        auto_cancel_ms: None, // 진행 중 = 카운트다운 아님(ⓓ 틱이 채움)
                    }
                } else {
                    nbeep_ui::XferProgress {
                        done_bytes: got,
                        total_bytes: total,
                        done_files: prev.map_or(1, |p| p.done_files.max(1)),
                        total_files: prev.map_or(1, |p| p.total_files.max(1)),
                        sending,
                        auto_cancel_ms: None,
                    }
                };
                self.xfer_progress.insert(peer, xp);
                self.note_xfer_state_change(peer);
                // 스레드 항목 진행률 — **이름 대상**(M4-2e: 지연 이벤트가 다음
                // 라인을 오염시키지 않게) · 이름 없으면 종전 FIFO(구 경로).
                if name.is_empty() {
                    self.set_xfer_line(
                        peer,
                        sending,
                        nbeep_ui::XferLineState::Active { done: got },
                    );
                } else {
                    self.set_xfer_line_named(
                        peer,
                        sending,
                        &name,
                        total,
                        nbeep_ui::XferLineState::Active { done: got },
                    );
                }
                if sending {
                    self.refresh_send_batch(peer); // 배치 패널 활성 행 진행률(M4-2d)
                }
                self.apply_xfer_view_throttled(peer); // 목록 재조립은 주기(RL-6)
                self.redraw_conversation(peer);
                if let Some(mid) = self.main_id {
                    self.request_redraw(mid);
                }
            }
            AppEvent::XferManifest { peer, entries } => {
                // 새 요청 도착(M4-2e · 08-20) — 이전 요청의 거절 장부는 낡았다
                // (같은 파일을 다시 보내는 건 새 의사 — 다시 묻는 게 맞다).
                self.batch_declined.remove(&peer);
                // 배치 목록 도착(M4-2e) — 스레드에 전 파일 라인을 세운다: 제외는
                // 종결 라인, 나머지는 대기 라인(오퍼 도착이 재활성). ★ 발신자가
                // 드롭마다 재송하므로 **이전 manifest와의 차분만** 라인으로 추가
                // (제외 라인은 종결이라 재활성 가드가 없다 — 그대로 밀면 중복).
                let old = self.recv_manifest.get(&peer).cloned().unwrap_or_default();
                let mut consumed = vec![false; old.len()];
                for e in &entries {
                    let dup = old.iter().enumerate().any(|(i, o)| {
                        if !consumed[i] && o == e {
                            consumed[i] = true;
                            true
                        } else {
                            false
                        }
                    });
                    if dup {
                        continue;
                    }
                    let (name, size, excluded) = e;
                    if *excluded {
                        self.push_excluded_line(peer, false, name, *size);
                    } else {
                        self.push_xfer_line(peer, false, name, *size);
                    }
                }
                // 수신 배치 집계(M4-2e — 배너 합산): 총 수·총 바이트는 manifest
                // 비제외분 재계산(드롭 추가 재송 = 총량 성장) · 완료분은 보존.
                {
                    let sizes: Vec<(String, u64)> = entries
                        .iter()
                        .filter(|(_, _, ex)| !ex)
                        .map(|(n, s2, _)| (n.clone(), *s2))
                        .collect();
                    let (count, sum) = (
                        u32::try_from(sizes.len()).unwrap_or(u32::MAX),
                        sizes.iter().map(|(_, s2)| *s2).sum::<u64>(),
                    );
                    let e = self.recv_batch.entry(peer).or_insert((0, 0, 0, 0));
                    e.1 = count.max(e.0);
                    e.3 = sum.max(e.2);
                    self.recv_batch_sizes.insert(peer, sizes);
                }
                self.recv_manifest.insert(peer, entries);
                self.redraw_conversation(peer);
            }
            AppEvent::XferDone {
                peer,
                name,
                risk,
                mismatch,
                qpath,
                avg_bps,
                scan,
            } => {
                self.active_recv.remove(&peer); // 수신 완결 — 취소 대상 아님(08-16)
                                                // 수신은 액터에 계측기가 없다 — 평균이 유일한 실측(보수적 하한).
                                                // 이어받기였다면 `.part`는 승격 완료로 소임을 다했다(M4-10a).
                if let Some(sha) = self.resumed_recv.remove(&peer) {
                    crate::part::remove_partial(crate::gate::CH_GUI, &sha);
                }
                self.recv_meter.note_peak(avg_bps);
                self.refresh_approval_ui();
                // ★ 격리함이 열려 있으면 새 격리물을 즉시 반영(08-18 — 열어둔 채
                //   수신하면 재열기 전까지 안 뜨던 것. 스캔은 사이드카라 값싸다).
                if self.windows.values().any(|e| e.role == Role::Quarantine) {
                    self.spawn_quarantine_scan();
                }
                // 격리 보관까지 끝난 상태 — **실체화는 승인 후 별도**(FR-S-9).
                // ★ 검사 탐지(FR-S-15 · 08-21) = 즉시 상태바 경고(격리함을 안 열어도
                //   안다). 표기는 사실뿐 — "안전" 단정 금지(NFR-S-5).
                if scan == nbeep_core::ScanOutcome::Detected {
                    self.set_status(nbeep_core::tf(nbeep_core::Msg::StfScanDetected, &[&name]));
                } else {
                    self.set_status(format!(
                        "파일 격리 완료: {name} · 위험 {risk:?}{} — 승인 전까지 실행 불가",
                        if mismatch {
                            " · ⚠️ 형식 불일치"
                        } else {
                            ""
                        }
                    ));
                }
                self.set_xfer_line_named_or_fifo(
                    peer,
                    false,
                    &name,
                    nbeep_ui::XferLineState::Done {
                        note: format!(
                            "{} · {} {risk:?}{} · {} {}",
                            nbeep_core::t(nbeep_core::Msg::XferQuarantined),
                            nbeep_core::t(nbeep_core::Msg::XferRisk),
                            if mismatch {
                                format!(" · {}", nbeep_core::t(nbeep_core::Msg::XferMismatch))
                            } else {
                                String::new()
                            },
                            nbeep_core::t(nbeep_core::Msg::XferAvg),
                            speed_label(avg_bps)
                        ),
                    },
                );
                // 이미지면 소형 미리보기 부착(M4-5ⓑ) — 디코드는 워커로(08-13 —
                // 수신 완료 순간 메인이 멈추지 않게). 도착 = `Decoded(XferThumb)`가
                // 마지막 수신 항목에 부착(연속 수신과의 경합 창은 짧다 — 실해 없음).
                // 이미지가 아니거나 실패면 조용히 없음(스레드는 텍스트 그대로).
                {
                    let qp = qpath.clone();
                    let secret = self.identity.wrap_secret();
                    spawn_decode(
                        self.proxy.clone(),
                        DecodeTarget::XferThumb(peer, qpath),
                        move || {
                            crate::imgdec::thumb_raw_from_beepq(
                                std::path::Path::new(&qp),
                                96,
                                &secret,
                            )
                        },
                    );
                }
                // 수신 장부 정리(M4-2e) — 이 파일은 끝났다.
                if let Some(l) = self.recv_xids.get_mut(&peer) {
                    l.retain(|(n, _)| n != &name);
                }
                if let Some(setp) = self.recv_paused.get_mut(&peer) {
                    setp.remove(&name);
                    if setp.is_empty() {
                        self.recv_paused.remove(&peer);
                    }
                }
                // 수신 배치 완료 가산(M4-2e 배너 합산) — 크기는 manifest 장부에서.
                if let Some(sizes) = self.recv_batch_sizes.get_mut(&peer) {
                    if let Some(pos) = sizes.iter().position(|(n, _)| n == &name) {
                        let (_, fsize) = sizes.remove(pos);
                        if let Some(e) = self.recv_batch.get_mut(&peer) {
                            e.0 += 1;
                            e.2 = e.2.saturating_add(fsize);
                        }
                    }
                }
                // ★ 수신 배너 유지(사용자 확정 08-19 — 송신측과 대칭): 자동수락
                //   잔여(batch_approved)나 정지 파일이 남아 있으면 진행상태·
                //   전체취소를 지우지 않는다. 전부 끝났을 때만 마감.
                let recv_left = self.batch_approved.contains_key(&peer)
                    || self.recv_paused.contains_key(&peer)
                    || self.active_recv.contains_key(&peer);
                if !recv_left {
                    self.clear_xfer(peer);
                    self.recv_batch.remove(&peer);
                    self.recv_batch_sizes.remove(&peer);
                }
                self.redraw_conversation(peer);
            }
            AppEvent::XferCancelAllNotice { peer } => {
                // 상대의 전체 취소 — **내 쪽 전체취소 루틴을 그대로 실행**(사용자
                // 확정 08-19: 상태 공유 후 각자 함수 호출). 와이어 재전파는 안
                // 한다(에코 루프 방지 — 발신자가 이미 자기 쪽을 정리했다).
                self.apply_local_cancel_all(peer);
                self.set_status(nbeep_core::t(nbeep_core::Msg::XferPeerCanceledAll).to_string());
            }
            AppEvent::XferPeerPauseNotice { peer, name, paused } => {
                {
                    let set = self.recv_paused.entry(peer).or_default();
                    if paused {
                        set.insert(name.clone());
                    } else {
                        set.remove(&name);
                    }
                    if set.is_empty() {
                        self.recv_paused.remove(&peer);
                    }
                }
                // 발신측이 멈췄다/이었다 — 내 수신 라인을 같은 상태로(done 승계 =
                // u64::MAX 센티널 · ▶는 ResumeReq, ⏸는 PauseReq로 원격 제어).
                let st = if paused {
                    nbeep_ui::XferLineState::Paused { done: u64::MAX }
                } else {
                    nbeep_ui::XferLineState::Active { done: u64::MAX }
                };
                self.set_xfer_line_named(peer, false, &name, 0, st);
                self.note_xfer_state_change(peer);
                self.redraw_conversation(peer);
            }
            AppEvent::XferPaused {
                peer,
                id,
                name,
                size,
                done,
            } => {
                // 정지 확정(액터 단일 진실) — 라인·큐·슬롯을 여기서만 동기화한다
                // (수락 직전 정지의 경합으로 라인이 Active로 남아 ▶가 안 뜨던 실기).
                if let Some(path) = self.current_send.get(&peer).cloned() {
                    let matches = path
                        .file_name()
                        .is_some_and(|n| n.to_string_lossy() == name);
                    if matches {
                        self.current_send.remove(&peer);
                        self.active_send.remove(&peer);
                        if let Some(set) = self.send_paused.get_mut(&peer) {
                            set.remove(&path); // 큐 정지 표식은 보관 이동으로 승격
                        }
                        self.paused_sends.entry(peer).or_default().push((
                            name.clone(),
                            size,
                            path,
                            id,
                        ));
                    }
                }
                self.set_xfer_line_named(
                    peer,
                    true,
                    &name,
                    size,
                    nbeep_ui::XferLineState::Paused { done },
                );
                self.pump_send_queue(peer); // 다음 파일 진행(ⓐ)
                self.refresh_send_batch(peer);
                self.note_xfer_state_change(peer);
                self.redraw_conversation(peer);
            }
            AppEvent::XferResumed {
                peer,
                id,
                name,
                size,
                done,
            } => {
                // 보관 정지분이 활성으로 복귀(M4-2e ⓐ) — 앱 맵·라인 동기화.
                self.active_send.insert(peer, id);
                if let Some(list) = self.paused_sends.get_mut(&peer) {
                    if let Some(pos) = list.iter().position(|(n, _, _, _)| n == &name) {
                        let (_, _, path, _) = list.remove(pos);
                        self.current_send.insert(peer, path);
                    }
                    if list.is_empty() {
                        self.paused_sends.remove(&peer);
                    }
                }
                self.set_xfer_line_named(
                    peer,
                    true,
                    &name,
                    size,
                    nbeep_ui::XferLineState::Active { done },
                );
                self.note_xfer_state_change(peer);
                self.redraw_conversation(peer);
            }
            AppEvent::XferAccepted { peer } => {
                // 수락 후 취소 UX(08-16) — 대기 xid를 진행 중으로 이동(취소 경로 유지).
                if let Some(xid) = self.awaiting_accept.get(&peer).copied() {
                    self.active_send.insert(peer, xid);
                }
                // 협상 성립(M4-2d) — 창은 닫지 않는다. 승인 타임아웃만 끄고(더는
                // 승인 대기 아님) 배치 패널을 갱신한다(이 파일 = 전송 중). 창은
                // 배치가 끝날 때만 닫힌다(refresh_send_batch).
                self.awaiting_accept.remove(&peer);
                self.send_wait.remove(&peer); // 승인 타임아웃 해제
                                              // ★ 승인 대기 중 일시정지한 파일(08-19) — 오퍼는 이미 나가 있어
                                              //   수락되면 액터 펌프가 돌기 시작한다. 정지 의사를 액터에 지금
                                              //   전달해 0%에서 멈춘 채 두고, 재개 아이콘으로 잇게 한다(안
                                              //   보내면 패널은 "일시정지"인데 실제로는 전송되는 모순).
                let paused_now = self
                    .current_send
                    .get(&peer)
                    .is_some_and(|p| self.send_paused.get(&peer).is_some_and(|s| s.contains(p)));
                if paused_now {
                    self.pause_active_send(peer, true);
                }
                self.refresh_send_batch(peer);
                self.set_xfer_line(peer, true, nbeep_ui::XferLineState::Active { done: 0 });
                self.set_status(nbeep_core::t(nbeep_core::Msg::StPeerAccepted));
                self.redraw_conversation(peer);
            }
            AppEvent::QuarantineScanned { gen, rows } => {
                if gen != self.qscan_gen {
                    return; // 낡은 스캔 — 그 사이 재요청됨
                }
                self.qrows_raw = rows;
                let rows = self.quarantine_rows();
                let mut inv = Invalidations::default();
                if let Some(qv) = &mut self.quarantine_view {
                    qv.set_rows(rows, &mut inv);
                }
                if let Some((qid, _)) = self
                    .windows
                    .iter()
                    .find(|(_, e)| e.role == Role::Quarantine)
                {
                    let qid = *qid;
                    self.layout_window(qid);
                    self.request_redraw(qid);
                }
                // ★ 무결성 검증 백그라운드(08-18) — 목록은 이미 떴다. 행마다 전체
                //   개봉·해시 확인을 워커로 → `QVerified`가 그 행 Approve를 활성화.
                self.spawn_quarantine_verify(gen);
            }
            AppEvent::QVerified { gen, path, ok } => {
                if gen != self.qscan_gen {
                    return; // 낡은 스캔 — 다음 스캔이 다시 검증한다
                }
                if let Some(r) = self.qrows_raw.iter_mut().find(|r| r.path == path) {
                    r.verified = ok; // 손상(ok=false)은 미검증 유지 → 승인 차단
                }
                let rows = self.quarantine_rows();
                let mut inv = Invalidations::default();
                if let Some(qv) = &mut self.quarantine_view {
                    qv.set_rows(rows, &mut inv);
                }
                if let Some((qid, _)) = self
                    .windows
                    .iter()
                    .find(|(_, e)| e.role == Role::Quarantine)
                {
                    let qid = *qid;
                    self.request_redraw(qid);
                }
            }
            AppEvent::PeerRecvCap { peer, cap } => {
                // 상한 공지·질의 응답(08-18) — 캐시 갱신 후, 질의를 기다리던
                // 펌프가 있으면 즉시 재개.
                self.peer_recv_cap.insert(peer, (cap, self.now_ms()));
                if self.cap_req_deadline.remove(&peer).is_some() {
                    self.pump_send_queue(peer);
                }
            }
            AppEvent::HashProgress { peer, name, pct } => {
                if self.preparing_send.contains(&peer) {
                    self.set_status(nbeep_core::tf(
                        nbeep_core::Msg::XferHashingPct,
                        &[&name, &pct.to_string()],
                    ));
                    if let Some(mid) = self.main_id {
                        self.request_redraw(mid);
                    }
                    self.redraw_conversation(peer);
                }
            }
            AppEvent::SendHashed {
                peer,
                path,
                size,
                sha,
            } => {
                if !self.preparing_send.remove(&peer) {
                    return; // 준비 중 취소됨(08-18) — 낡은 해시 결과 폐기
                }
                let Some(sha) = sha else {
                    self.set_status(nbeep_core::tf(
                        nbeep_core::Msg::StfFileReadFail,
                        &[&path.display().to_string()],
                    ));
                    self.current_send.remove(&peer);
                    self.pump_send_queue(peer);
                    return;
                };
                if !self.conversations.contains_key(&peer) {
                    // 해시 도는 사이 세션이 끊겼다 — Closed가 current_send를
                    // resend_offers로 옮겼으니 재성립 때 다시 온다. 조용히 종료.
                    return;
                }
                let name = path
                    .file_name()
                    .map_or_else(|| "file".to_string(), |n| n.to_string_lossy().into_owned());
                // 전송 id — 새 키의 앞 16B(세션 내 유일하면 충분한 간이 난수).
                let mut xid = [0u8; 16];
                xid.copy_from_slice(&nbeep_crypto::Identity::generate().peer_id().as_bytes()[..16]);
                let sent = self.conversations.get(&peer).is_some_and(|c| {
                    c.out_tx
                        .send(SessionCmd::OfferFile {
                            id: xid,
                            name: name.clone(),
                            sha,
                            path,
                            size,
                        })
                        .is_ok()
                });
                if !sent {
                    self.set_status(nbeep_core::t(nbeep_core::Msg::StSessionDropSend));
                    self.send_queue.remove(&peer);
                    self.send_batch.remove(&peer);
                    self.send_excluded.remove(&peer); // 배치 종료 = 제외 목록도 마감(M4-2e)
                    return;
                }
                self.awaiting_accept.insert(peer, xid);
                self.set_status(nbeep_core::tf(
                    nbeep_core::Msg::StfFileOffer,
                    &[&name, &human_size(size)],
                ));
                // 스레드에 송신 항목 추가(승인 대기 → 진행 → 완료가 이 항목 위에서 갱신).
                self.push_xfer_line(peer, true, &name, size);
                self.open_send_wait(peer, &name);
            }
            AppEvent::XferSendDone {
                peer,
                avg_bps,
                peak_bps,
                name,
                size,
            } => {
                self.active_send.remove(&peer); // 다 보냈다 — 취소 대상 아님(08-16)
                                                // Auto 설정 행 표시용 집계(08-16) — 세션 peak가 0일 수 있다
                                                // (프로브 크기 미만의 작은 파일 — 창이 한 번도 안 닫힘). 평균이 하한.
                self.current_send.remove(&peer); // 완주 — 재-Offer 원료 아님(M4-10c)
                self.send_meter.note_peak(peak_bps.max(avg_bps));
                self.refresh_approval_ui();
                self.send_avg.insert(peer, avg_bps); // ack 도착 시 완료 문구에(08-16)
                                                     // ★ 전송 끝 ≠ 완료(M4-9) — 청크·Done을 다 보냈을 뿐, 상대 격리 확인(ack)
                                                     //   전까지는 **확인 대기**다. 미확인 카운트를 올려 종료 가드가 본다.
                if name.is_empty() {
                    self.set_xfer_line(peer, true, nbeep_ui::XferLineState::AwaitingAck);
                } else {
                    // 파일 단위 즉시 종료 처리(M4-2e — 사용자 확정: 전부 끝나야
                    // 닫히던 것을 파일마다 그 라인에서 닫는다).
                    self.set_xfer_line_named(
                        peer,
                        true,
                        &name,
                        size,
                        nbeep_ui::XferLineState::AwaitingAck,
                    );
                }
                *self.awaiting_ack.entry(peer).or_insert(0) += 1;
                // 배치 집계 갱신 후 다음 파일로.
                if let Some(b) = self.send_batch.get_mut(&peer) {
                    b.0 += 1;
                    // 완료 누적 = **그 파일 크기**(이벤트 동봉 — 종전 xp.total은
                    // 배치 합산 표기 도입 후 배치 총량이라 이중 가산됐다).
                    b.2 = b.2.saturating_add(size);
                    let (done_f, total_f, done_b, total_b) = *b;
                    self.xfer_progress.insert(
                        peer,
                        nbeep_ui::XferProgress {
                            done_bytes: done_b,
                            total_bytes: total_b.max(done_b),
                            done_files: done_f,
                            total_files: total_f,
                            sending: true,
                            auto_cancel_ms: None,
                        },
                    );
                    self.apply_xfer_view(peer);
                    self.set_status(nbeep_core::tf(
                        nbeep_core::Msg::StfDeliveredWait,
                        &[&done_f.to_string(), &total_f.to_string()],
                    ));
                }
                let more = self.send_queue.get(&peer).is_some_and(|q| !q.is_empty());
                let paused_left = self.paused_sends.contains_key(&peer);
                if more {
                    self.pump_send_queue(peer);
                } else if paused_left {
                    // ★ 일시중지 잔여(사용자 확정 08-19) — 미완 항목이 남아 있으면
                    //   상태 배너(진행·전체취소)와 배치를 **유지**한다(ⓑ 대기 상태).
                    //   진행률 이벤트는 멈추므로 마지막 값에서 정지 표시로 남는다.
                } else {
                    self.send_batch.remove(&peer);
                    self.send_excluded.remove(&peer); // 배치 종료 = 제외 목록도 마감(M4-2e)
                    self.clear_xfer(peer);
                }
                self.refresh_send_batch(peer); // 배치 패널 갱신·종료(M4-2d)
                self.redraw_conversation(peer);
            }
            AppEvent::XferAcked {
                peer,
                ok,
                name,
                size,
            } => {
                // 수신 종단 확인 도착 — 확인 대기 항목을 완료/실패로 닫는다(M4-9).
                let terminal = if ok {
                    nbeep_ui::XferLineState::Done {
                        // 평균 송신 속도(08-16) — 청크 첫 발신~Done 기준.
                        note: self
                            .send_avg
                            .remove(&peer)
                            .map(|b| {
                                format!(
                                    "{} {}",
                                    nbeep_core::t(nbeep_core::Msg::XferAvg),
                                    speed_label(b)
                                )
                            })
                            .unwrap_or_default(),
                    }
                } else {
                    nbeep_ui::XferLineState::Failed {
                        why: nbeep_core::t(nbeep_core::Msg::XferPeerFailed).into(),
                    }
                };
                if name.is_empty() {
                    self.ack_xfer_line(peer, terminal); // 구 경로 FIFO
                } else {
                    // 파일 단위 즉시 종결(M4-2e — 사용자 확정: 파일마다 그 라인에서).
                    self.set_xfer_line_named(peer, true, &name, size, terminal);
                    self.record_history(peer);
                }
                if let Some(n) = self.awaiting_ack.get_mut(&peer) {
                    *n = n.saturating_sub(1);
                    if *n == 0 {
                        self.awaiting_ack.remove(&peer);
                    }
                }
                if self.awaiting_ack.is_empty() {
                    self.close_armed = false; // 확인이 다 끝났으면 종료 가드 해제
                }
                self.set_status(if ok {
                    "상대 수신 확인 — 완료".to_string()
                } else {
                    "상대가 받지 못함 — 실패".to_string()
                });
                self.redraw_conversation(peer);
                if let Some(mid) = self.main_id {
                    self.request_redraw(mid);
                }
            }
            AppEvent::XferFailed { peer, why } => {
                self.active_send.remove(&peer);
                self.active_recv.remove(&peer);
                self.resumed_recv.remove(&peer); // .part는 남긴다 — 실패가 곧 재개 사유
                self.current_send.remove(&peer); // 거절·취소·오류 = 의사/종결 — 재-Offer 없음
                                                 // 방향 판정 — 발신이 걸려 있으면 발신 실패, 아니면 수신 실패.
                let mine =
                    self.awaiting_accept.contains_key(&peer) || self.send_batch.contains_key(&peer);
                self.set_xfer_line(
                    peer,
                    mine,
                    nbeep_ui::XferLineState::Failed { why: why.clone() },
                );
                self.set_status(nbeep_core::tf(nbeep_core::Msg::StfFileWhy, &[&why]));
                self.awaiting_accept.remove(&peer);
                self.close_send_wait(peer);
                self.send_queue.remove(&peer);
                self.send_batch.remove(&peer);
                self.send_excluded.remove(&peer); // 배치 종료 = 제외 목록도 마감(M4-2e)
                if mine {
                    // ★ 요청 단위 종결(08-20 실기 — "둘 다 Declined"): 큐를 비우면서
                    //   잔여 대기·정지 **라인**을 같은 사유로 닫는다. 종전엔 큐만
                    //   버려 남은 파일 라인이 "승인 대기"로 영구 잔존했다(3.hsh).
                    self.fail_pending_xfer_lines(peer, true, why);
                }
                self.clear_xfer(peer);
                self.redraw_conversation(peer);
            }
            AppEvent::Closed { peer } => {
                self.active_send.remove(&peer);
                self.active_recv.remove(&peer);
                self.clear_batch_approval(peer); // 세션 종료 = 요청 승인 잔여 마감(M4-2e)
                                                 // ack 대기 정리(RL-14ⓐ · 08-18) — 상대가 떠났으면 수신 확인은
                                                 // 영영 안 온다. 남기면 종료 가드가 영구 "확인 대기 N건"이 된다.
                self.awaiting_ack.remove(&peer);
                self.preparing_send.remove(&peer); // 해시 워커 결과는 도착 시 무시된다
                self.cap_req_deadline.remove(&peer);
                self.peer_recv_cap.remove(&peer); // 재성립 시 새 공지가 온다
                                                  // 중단 발신 기억(M4-10c) — 진행 중이던 파일 + 남은 큐. 세션이
                                                  // 되살아나면 능동 재-Offer 1회(수신측 .part 매치 = 이어받기).
                {
                    let mut interrupted: VecDeque<std::path::PathBuf> = VecDeque::new();
                    // 보관 정지분(ⓐ) — 세션이 죽으면 액터 parked도 죽는다. 경로를
                    // 재-Offer 원료로 접어 재성립 시 이어받기 후보로 살린다.
                    if let Some(list) = self.paused_sends.remove(&peer) {
                        for (_, _, path, _) in list {
                            interrupted.push_back(path);
                        }
                    }
                    if let Some(p) = self.current_send.remove(&peer) {
                        interrupted.push_back(p);
                    }
                    if let Some(q) = self.send_queue.remove(&peer) {
                        interrupted.extend(q);
                    }
                    if !interrupted.is_empty() {
                        self.resend_offers.insert(peer, interrupted);
                    }
                }
                // 세션 채널은 지우되 **스레드는 대피**(DR-26 — 08-13 실기: 끊김·재연결마다
                // 받았던 메시지가 통째로 사라졌다). 재수립 시 install이 되찾아 간다.
                if let Some(c) = self.conversations.remove(&peer) {
                    if !c.lines.is_empty() {
                        self.parked_lines.insert(peer, c.lines);
                    }
                }
                self.closed_peers.insert(peer); // 목록 상태 점 = 끊김(빨강)
                                                // 인바운드 전용 비발견 상대(수동 주소도, 발견 경로도 없음)는 세션이
                                                // 끝나면 **다시 닿을 수단이 없다** — 목록에 유령으로 남기지 않는다(08-13).
                                                // 수동 주소가 있으면 남긴다(빨강 · 클릭 = 그 주소로 재연결).
                if !self.manual_addrs.contains_key(&peer) && self.table.get(peer).is_none() {
                    self.extra_peers.remove(&peer);
                    self.unread.remove(&peer);
                    self.update_main_title();
                } else {
                    // 재연결 수단이 있는 상대는 자동 재연결 시작(ⓑ) — 첫 시도 5초 후.
                    let due = self.now_ms() + RECONNECT_BACKOFF_MS[0];
                    self.reconnect.insert(peer, (0, due));
                }
                self.set_status(nbeep_core::t(nbeep_core::Msg::StSessionEndedPeer));
                self.refresh_chat_link(peer); // 헤더 아이콘 = 끊김(M3-20 — 제거 뒤라야 Lost)
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
                auto,
            } => {
                // 워커가 만든 아웃바운드 세션(M2-8) — TOFU 판정은 여기(메인 · TrustStore 소유).
                use nbeep_core::Session as _;
                let InboundSession { session, path } = *session;
                let peer = session.peer();
                self.connecting.finish(intent, Some(peer));
                // ★ P-3(M5-3c · R-20) — 클릭한 상대와 **다른 키**가 성립했다면 그 수동
                //   주소는 이제 그 사람의 경로가 아니다. 즉시 무효화(다음 클릭이 낡은
                //   주소로 남에게 붙는 것을 막는다 — "경로 등급은 성립한 세션이 정한다").
                if let Some(want) = intent {
                    if want != peer && self.manual_addrs.remove(&want).is_some() {
                        self.set_status(nbeep_core::tf(
                            nbeep_core::Msg::StfPathInvalidated,
                            &[&self.peer_title(want)],
                        ));
                    }
                }
                self.reconnect.remove(&peer); // 성립 = 자동 재연결 스케줄 해제(ⓑ)
                                              // 대기 중이던 그룹 본문이 있으면 성립 직후 이어 보낸다(M5-1 — 자동 연결 후 전송).
                                              // install보다 뒤여야 하지만 install은 아래 분기에서 일어난다 — flush는 그 뒤.
                if let Some(addr) = via_addr {
                    // 수동 등록 성공(DR-19) — 세션이 끊겨도 이 주소로 재연결한다(④).
                    self.manual_addrs.insert(peer, addr);
                }
                if !self.conversations.contains_key(&peer) {
                    let est = match nbeep_core::TrustedSession::wrap(session, &mut self.trust) {
                        Ok(est) => est,
                        Err(e) => {
                            self.set_status(nbeep_core::tf(
                                nbeep_core::Msg::StfTrustReject,
                                &[&e.to_string()],
                            ));
                            if let Some(mid) = self.main_id {
                                self.request_redraw(mid);
                            }
                            return;
                        }
                    };
                    let decision = est.decision;
                    self.install_conversation(nbeep_core::MuxSession::new(est.session), path);
                    if auto {
                        // **조용한 연결**(자동 재연결 ⓑ · 프로필 pull 08-14) — 창을
                        // 열지 않는다(② 자동 열림 금지와 같은 규칙 · 카드와 대화 분리).
                        self.set_status(nbeep_core::tf(
                            nbeep_core::Msg::StfConnectedOpen,
                            &[&self.peer_title(peer)],
                        ));
                        let mut inv = Invalidations::default();
                        self.refresh_rows(&mut inv);
                    } else {
                        self.activate(peer, el); // 사용자가 시작한 연결 — 뷰를 연다
                        self.set_status(match decision {
                            nbeep_core::TrustDecision::FirstContact => {
                                "Noise 세션 수립 — 첫 접촉(TOFU 핀 고정)".into()
                            }
                            d => format!("Noise 세션 수립 — {d:?}"),
                        });
                    }
                } else if !auto {
                    // 그 사이 인바운드가 먼저 성립 — 이 세션은 버리고 그 대화를 연다.
                    self.activate(peer, el);
                }
                // 대기 중이던 그룹 본문 이어 보내기(M5-1 — "자동 연결 시도 후 전송").
                self.flush_group_sends(peer);
                self.flush_direct_sends(peer); // 1:1 오프라인 대기(M4-6)도 같은 합류점
                self.refresh_chat_link(peer); // 헤더 아이콘 = 연결됨(M3-20)
                if let Some(mid) = self.main_id {
                    self.request_redraw(mid);
                }
            }
            AppEvent::ConnectFailed { peer, why } => {
                self.connecting.finish(Some(peer), None);
                self.refresh_chat_link(peer); // 헤더 아이콘 = 끊김/유휴(M3-20)
                                              // 닿지 않은 상대 = 목록 점 빨강(08-13 실기 — 실패 후 회색으로 남으면
                                              // "종료된 상대"라는 사실이 표시되지 않았다).
                self.closed_peers.insert(peer);
                // 자동 재연결 백오프 진행(ⓑ) — 다음 단계 등록, 다 썼으면 중단 고지.
                // 수단(발견 중이거나 수동 주소)이 없는 상대는 등록하지 않는다 —
                // 어차피 같은 이유로 실패만 반복한다(무의미 트래픽 금지).
                // 서버 사다리(X-2b ③)도 수단이다 — 발견·수동 주소가 없어도 Managed
                // 서버가 붙어 있으면 재시도할 근거가 있다(NotFound는 백오프가 흡수).
                let reachable = self.manual_addrs.contains_key(&peer)
                    || self.table.get(peer).is_some()
                    || self.relay.is_some();
                let stage = self.reconnect.get(&peer).map_or(0, |(s, _)| s + 1);
                match reconnect_delay(stage).filter(|_| reachable) {
                    Some(delay) => {
                        let due = self.now_ms() + delay;
                        self.reconnect.insert(peer, (stage, due));
                        self.set_status(format!(
                            "연결 실패({}): {why} — {}초 후 재시도",
                            self.peer_title(peer),
                            delay / 1000
                        ));
                    }
                    None => {
                        self.reconnect.remove(&peer);
                        // 그룹 대기 본문도 실패로 종결(FR-G-4 — 조용히 버리지 않는다).
                        self.fail_group_sends(peer);
                        self.set_status(format!(
                            "연결 실패({}): {why} — 자동 재시도 중단(클릭 = 재개)",
                            self.peer_title(peer)
                        ));
                    }
                }
                let mut inv = Invalidations::default();
                self.refresh_rows(&mut inv);
                if let Some(mid) = self.main_id {
                    self.request_redraw(mid);
                }
            }
            AppEvent::ServerAttach { gen, outcome } => {
                self.relay_connecting = false;
                if gen != self.relay_gen {
                    return; // 낡은 결과(그 사이 설정 변경) — 틱이 새 목표로 다시 시도
                }
                match outcome {
                    Ok(at) => {
                        if at.pin_write_failed {
                            self.set_status(nbeep_core::t(nbeep_core::Msg::StServerPinWriteFail));
                        } else if at.first_pin {
                            self.set_status(nbeep_core::tf(
                                nbeep_core::Msg::StfServerFirstPin,
                                &[&at.addr],
                            ));
                        } else {
                            self.set_status(nbeep_core::tf(
                                nbeep_core::Msg::StfServerAttached,
                                &[&at.addr],
                            ));
                        }
                        self.relay_backoff = (0, 0);
                        let client = std::sync::Arc::new(at.client);
                        // 인바운드 수락 루프(X-2b ②) — 수명은 이 Arc에 묶인다.
                        spawn_relay_accept(
                            &client,
                            std::sync::Arc::clone(&self.identity),
                            self.proxy.clone(),
                        );
                        self.relay = Some(client);
                    }
                    Err(ServerAttachFail::PinMismatch) => {
                        // ★ 신원이 바뀌면 시끄럽게(DR-28) — 모달 + 자동 재시도 정지.
                        //   재개 = 사람이 핀 줄을 지우고 서버 설정을 다시 저장
                        //   (server_settings_changed가 backoff를 푼다).
                        self.relay_backoff = (0, u64::MAX);
                        let pin = self.data_dir.join("server.pin");
                        let body = nbeep_core::tf(
                            nbeep_core::Msg::StfServerPinMismatch,
                            &[
                                self.settings.get("net.server.address"),
                                &pin.display().to_string(),
                            ],
                        );
                        self.open_alert(
                            el,
                            nbeep_core::t(nbeep_core::Msg::AlertServerPinTitle),
                            &body,
                            None,
                        );
                    }
                    Err(fail) => {
                        let stage = self.relay_backoff.0;
                        let delay = server_retry_delay(stage);
                        self.relay_backoff = (stage.saturating_add(1), self.now_ms() + delay);
                        let why = match fail {
                            ServerAttachFail::Resolve => {
                                nbeep_core::t(nbeep_core::Msg::StServerResolveFail).to_string()
                            }
                            ServerAttachFail::Other(w) => w,
                            ServerAttachFail::PinMismatch => String::new(), // 위 갈래가 소진
                        };
                        self.set_status(nbeep_core::tf(
                            nbeep_core::Msg::StfServerRetry,
                            &[&why, &(delay / 1000).to_string()],
                        ));
                    }
                }
                if let Some(mid) = self.main_id {
                    self.request_redraw(mid);
                }
            }
            AppEvent::SGroup { peer, msg } => {
                self.handle_sgroup(peer, msg, el);
            }
            AppEvent::AddFailed { addr, why } => {
                // 수동 주소 연결 실패(워커에서 복귀 · M2-8 잔여) — 주소로 알린다(피어 미확정).
                self.set_status(nbeep_core::tf(
                    nbeep_core::Msg::StfManualConnFail,
                    &[&addr, &why],
                ));
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
                avatar: avatar_key,
                border,
                image_keep,
                bio,
            } => {
                // ★ RL-1 멱등 가드(08-18) — 같은 내용 재수신은 여기서 끝낸다.
                //   상대의 재접속·생사 프로브 응답(Full)이 올 때마다 사진 재봉인 +
                //   imgdec 자식 프로세스 + trust.seg 전체 재암호화 + 목록 재조립이
                //   전부 다시 돌았다(원조 08-16 증상의 비용 측). 지문은 수신 원문
                //   기준(적용 결과가 아니라) — 검증·정규화 이전이라 판정이 값싸다.
                let fp = {
                    use std::hash::{Hash as _, Hasher as _};
                    let mut h = std::collections::hash_map::DefaultHasher::new();
                    (
                        &name,
                        &email,
                        &phone,
                        &image,
                        &avatar_key,
                        &border,
                        image_keep,
                        &bio,
                    )
                        .hash(&mut h);
                    h.finish()
                };
                if self.peer_profile_fp.get(&peer) == Some(&fp) {
                    // 수신 사실만 기록(카드의 "N시간 전 수신" — 메모리만·디스크 0).
                    if let Some(p) = self.peer_profiles.get_mut(&peer) {
                        p.received_ms = unix_now_ms();
                    }
                    return;
                }
                self.peer_profile_fp.insert(peer, fp);
                // 이름은 무해화(DisplayName) 통과분만 채택 · 이력은 신뢰 저장소에도 남긴다.
                let display = name.and_then(|n| nbeep_core::DisplayName::parse(&n).ok());
                if let Some(n) = &display {
                    self.trust.record_name(peer, n.clone());
                }
                // 내장 아바타 키(08-14) — **내 자산의 12간지 키로 검증**하고 즉시 반영
                // (바이트가 아니라 키라 디코드 자체가 없다 — imgdec 무관·R-5 무관).
                // 미지 키(신버전 값)는 조용히 버린다(전방 호환).
                let builtin_key =
                    avatar_key.filter(|k| nbeep_core::avatar::ZODIAC.contains(&k.as_str()));
                let builtin = builtin_key
                    .as_ref()
                    .and_then(|k| self.builtin_avatars.get(k).cloned());
                // 보더 색 — 검증 통과분만(무효는 조용히 폐기 · fail-closed).
                let border = border.as_deref().and_then(nbeep_core::avatar::parse_border);
                // 캐시 파일 경로(부팅 복원의 짝 — restore_cached_profiles가 읽는다).
                let dir = self.data_dir.join("profiles");
                let img_path = dir.join(format!("{}.img", peer.short()));
                let meta_path = dir.join(format!("{}.meta", peer.short()));
                // 경량 갱신(M3-21) — image_keep = "공유 사진은 그대로": 이전 사진
                // 경로·아바타를 승계한다(텍스트 변경마다 256KiB 재전송을 피하는 마커).
                let kept = image_keep
                    .then(|| self.peer_profiles.get(&peer))
                    .flatten()
                    .map(|p| (p.image_file.clone(), p.avatar.clone()));
                // 이미지 바이트 캐시 + **격리 디코드는 워커로**(M4-5 — 자식 프로세스
                // 왕복이 메인을 1~2초 멈췄다 · 08-13 실기). 도착 전엔 기존 아바타를
                // 유지하고(깜빡임 방지) `Decoded(PeerAvatar)`가 교체한다. 실패 = 이니셜.
                let mut avatar = builtin.clone();
                let has_image = image.is_some();
                let image_file = image
                    .and_then(|bytes| {
                        std::fs::create_dir_all(&dir).ok()?;
                        // 디스크 봉인(08-17 평문 3면 ③) — 상대의 사진은 상대가 나에게만
                        // 공개한 PII다. 봉인 실패 = 캐시 포기(평문 폴백 없음 · 표시용
                        // 디코드는 메모리의 bytes로 그대로 진행되니 화면은 안 죽는다).
                        let env = nbeep_store::sealed::seal(
                            crate::gate::SEAL_PROFILE_CACHE,
                            &self.identity.wrap_secret(),
                            &bytes,
                        )
                        .ok()?;
                        std::fs::write(&img_path, env).ok()?;
                        avatar = self.peer_profiles.get(&peer).and_then(|p| p.avatar.clone());
                        spawn_decode(
                            self.proxy.clone(),
                            DecodeTarget::PeerAvatar(peer),
                            move || crate::imgdec::avatar_raw_from_bytes(&bytes, 256),
                        );
                        Some(img_path.clone())
                    })
                    .or_else(|| kept.as_ref().and_then(|(f, _)| f.clone()));
                if !has_image && !image_keep {
                    // 이미지 철회(응답에 미포함·유지 마커도 없음) — 캐시 파일도 지운다.
                    // 안 지우면 재시작 복원이 옛 사진을 되살린다(철회 대칭).
                    let _ = std::fs::remove_file(&img_path);
                }
                if let Some((kept_file, prev)) = &kept {
                    if kept_file.is_some() && prev.is_some() {
                        // ★ 사진 우선(08-16 확정 — 두 필드 동시 수신): 유지 마커가
                        // 왔고 공유 사진이 살아 있으면 그 그림을 승계한다. 함께 실려
                        // 온 내장 키는 폴백일 뿐 — 사진을 덮으면 "본인은 사진,
                        // 상대는 내장 그림" 불일치가 재발한다.
                        avatar = prev.clone();
                    } else if avatar.is_none() {
                        // 종전 승계(M3-21) — 새 아바타 재료가 없으면 이전 그림.
                        avatar = prev.clone();
                    }
                }
                let bio = bio.filter(|s| !s.is_empty());
                let has_any = display.is_some()
                    || email.is_some()
                    || phone.is_some()
                    || image_file.is_some()
                    || builtin.is_some()
                    || bio.is_some();
                if has_any {
                    // 메타 캐시(내장 키·보더 — 08-14 부팅 복원): 검증 통과분만 쓴다.
                    // 둘 다 없으면 파일 제거(철회 반영 — 이미지와 같은 규칙).
                    let meta = encode_profile_meta(builtin_key.as_deref(), border);
                    if meta.is_empty() {
                        let _ = std::fs::remove_file(&meta_path);
                    } else if std::fs::create_dir_all(&dir).is_ok() {
                        let _ = std::fs::write(&meta_path, meta);
                    }
                    // 받은 항목을 상태바에 요약(상세는 프로필 카드가 그린다).
                    // 받은 항목 요약(08-17 i18n) — 상세는 카드가 그린다.
                    let mut got: Vec<&str> = Vec::new();
                    if display.is_some() {
                        got.push(nbeep_core::t(nbeep_core::Msg::ItemName));
                    }
                    if email.is_some() {
                        got.push(nbeep_core::t(nbeep_core::Msg::ItemEmail));
                    }
                    if phone.is_some() {
                        got.push(nbeep_core::t(nbeep_core::Msg::ItemPhone));
                    }
                    if bio.is_some() {
                        got.push(nbeep_core::t(nbeep_core::Msg::ItemBio));
                    }
                    if image_file.is_some() {
                        got.push(nbeep_core::t(nbeep_core::Msg::ItemImage));
                    }
                    self.peer_profiles.insert(
                        peer,
                        PeerProfile {
                            name: display,
                            email,
                            phone,
                            bio,
                            image_file,
                            avatar,
                            border,
                            received_ms: unix_now_ms(), // 경량(Info)도 수신이다
                        },
                    );
                    self.set_status(nbeep_core::tf(
                        nbeep_core::Msg::ProfileReceived,
                        &[&self.peer_title(peer), &got.join("·")],
                    ));
                } else {
                    // 전부 비공개(빈 응답) — 이전 프로필이 있었다면 걷어낸다(철회 반영).
                    // 캐시 파일도 함께 — 재시작 복원이 철회를 되돌리면 안 된다.
                    self.peer_profiles.remove(&peer);
                    let _ = std::fs::remove_file(&meta_path);
                }
                let mut inv = Invalidations::default();
                self.refresh_rows(&mut inv);
                // 열려 있는 카드 즉시 갱신(M3-21 pull/push 도착 반영).
                self.refresh_peer_info_card(peer);
                if let Some(mid) = self.main_id {
                    self.request_redraw(mid);
                }
            }
            AppEvent::LinkChanged => {
                // L1 재발견(M1-2) — 전송이 그룹 재조인 + 즉시 HELLO + S4로 반응.
                // 목록은 상대 재공지(발견 이벤트)로 다시 찬다 — 여기서 지우지 않는다.
                self.transport.link_changed();
                // ★ M1-2b(부분 · 사용자 요청 "전환에도 상태 유지"): 재연결 **가속** —
                // 유선↔무선 전환 뒤 백오프의 긴 대기를 기다리지 않는다.
                // ⓐ 걸려 있는 재연결 스케줄 = 전부 0단·지금으로 리셋(다음 펌프서 즉시).
                let now = self.now_ms();
                for e in self.reconnect.values_mut() {
                    *e = (0, now);
                }
                // ⓑ 직전에 대화하다 끊긴 상대(스케줄 소진 포함) = 즉시 조용한 연결 1회
                //    — 수단(발견·수동 주소)이 있는 상대만(무의미 트래픽 금지).
                let targets: Vec<PeerId> = self
                    .closed_peers
                    .iter()
                    .copied()
                    .filter(|p| !self.conversations.contains_key(p))
                    .filter(|p| self.manual_addrs.contains_key(p) || self.table.get(*p).is_some())
                    .collect();
                for p in targets {
                    self.start_connect(p, true); // 중복 클릭 가드는 connecting이 맡는다
                }
                let probe_ok = now.saturating_sub(self.last_link_probe_ms) >= 10_000;
                // ⓒ 활성 세션 능동 생사 촉진(M1-2b · 08-16) — 링크가 바뀐 직후의
                //    "성립돼 있다고 믿는" 세션이 실제로는 죽었을 수 있다(경로 소멸).
                //    새 와이어 프레임 없이 **기존 ProfileMsg::Request 1발**을 쏜다:
                //    죽은 소켓은 write가 RST를 만나 즉시 Closed로 떨어지고(22초
                //    keepalive 감지를 단축), 산 세션은 프로필 캐시가 신선해지는
                //    부수 효과만 남는다. LinkChanged당 1회 — 반복 타이머 아님
                //    (13 §12-1 · 진짜 주기 하트비트는 M2-4b 몫).
                if probe_ok {
                    self.last_link_probe_ms = now;
                    for conv in self.conversations.values() {
                        let _ = conv.out_tx.send(SessionCmd::Control(vec![
                            nbeep_core::ProfileMsg::Request.encode(),
                        ]));
                    }
                }
                self.set_status(nbeep_core::t(nbeep_core::Msg::StNetChanged));
                if let Some(mid) = self.main_id {
                    self.request_redraw(mid);
                }
            }
            AppEvent::ArchiveList {
                title,
                body,
                anchor,
            } => {
                // 격리 아카이브 내용 목록(M4-4 ⓐ) — 워커 완료분을 경고 모달로.
                // 격리함 창이 그 사이 닫혔으면 open_alert가 메인 소유로 폴백한다.
                self.open_alert(el, &title, &body, Some(anchor));
            }
            AppEvent::Inbound { session } => {
                use nbeep_core::Session as _;
                let InboundSession { session, path } = *session;
                let peer = session.peer();
                if self.conversations.contains_key(&peer) {
                    return; // 이미 이 상대와 대화 중(아웃바운드 세션 존재) — 중복 인바운드 무시
                }
                // ★ 원격 인바운드 × 미등록 = 요청 대기(M5-3b · ADR-0006 §6 — FR-S-25).
                //   LAN 밖에서 걸어온 모르는 키를 TOFU로 자동 등록하면 공인망 스캐너가
                //   목록에 스스로를 심는다 — 등록은 사람이 결정한다. 아는 상대(핀·대조)의
                //   원격 인바운드는 즉시 통과.
                if path == nbeep_core::PathClass::Remote {
                    use nbeep_core::TrustStore as _;
                    if self.trust.level(peer) == nbeep_core::TrustLevel::Unverified {
                        // 대기 슬롯 1개(모달 1개 규칙) — 점유 중 추가 원격 인바운드는
                        // 드롭(fail-closed · 정보 최소 — 침묵 폐기, 상대는 Closed만 관측).
                        if self.pending_remote.is_some() {
                            self.set_status(nbeep_core::t(nbeep_core::Msg::StRemoteInboundDropped));
                            if let Some(mid) = self.main_id {
                                self.request_redraw(mid);
                            }
                            return;
                        }
                        let title = self.peer_title(peer);
                        self.pending_remote = Some((peer, session, path));
                        self.open_choice(
                            el,
                            nbeep_core::t(nbeep_core::Msg::AlertRemoteReqTitle),
                            &nbeep_core::tf(nbeep_core::Msg::StfRemoteReqBody, &[&title]),
                            nbeep_core::t(nbeep_core::Msg::BtnAccept),
                            nbeep_core::t(nbeep_core::Msg::BtnDecline),
                            AlertCtx::RemoteInbound { peer },
                        );
                        return;
                    }
                }
                self.accept_inbound(session, path);
            }
            AppEvent::Tray(ev) => match ev {
                // 트레이(M3-2a) — 좌클릭/"열기" = 메인 복원 · "종료" = 명시적 종료.
                nbeep_plat::tray::TrayEvent::Open => {
                    if let Some(e) = self.main_id.and_then(|m| self.windows.get(&m)) {
                        e.window.set_visible(true);
                        e.window.focus_window();
                    }
                }
                // 알림 클릭(08-15 사용자 요청) — 메인 표시 + **해당 대화까지**:
                // 단일 모드 = 메인 안에서 그 대화로 전환 · 분리 모드 = 그 대화 창을
                // 열어 마지막 스레드 표시 — 둘 다 기존 "대화 열기" 경로(activate/
                // open_group_thread)가 모드 분기까지 그대로 맡는다(새 규칙 0).
                nbeep_plat::tray::TrayEvent::OpenTarget(tok) => {
                    if let Some(e) = self.main_id.and_then(|m| self.windows.get(&m)) {
                        e.window.set_visible(true);
                        e.window.focus_window();
                    }
                    match self.notify_targets.get(&tok).copied() {
                        Some(NotifyTarget::Peer(p)) => self.activate(p, el),
                        Some(NotifyTarget::Group(g)) => self.open_group_thread(g, el),
                        None => {} // 재시작 등으로 맵이 비었다 — 메인 표시로 충분
                    }
                }
                nbeep_plat::tray::TrayEvent::Quit => el.exit(),
            },
            AppEvent::WireAvatar { gen, png } => {
                if gen != self.wire_gen {
                    // 낡은 세대 — 그 사이 사진이 또 바뀌었다. 새 세대의 워커(또는
                    // 무 워커 상태)가 진실이므로 조용히 폐기(pending도 안 만진다 —
                    // 세대를 올린 쪽이 이미 재설정했다).
                    return;
                }
                self.wire_pending = false;
                let ok = png.is_some_and(|bytes| {
                    let wire = self.wire_avatar_path();
                    wire.parent()
                        .is_some_and(|d| std::fs::create_dir_all(d).is_ok())
                        && std::fs::write(&wire, bytes).is_ok()
                });
                if ok {
                    // 보류했던 push가 여기서 **한 번에** 나간다(실사진 동봉 —
                    // 08-16 실기: 종전엔 키만 실린 프레임이 먼저 나가 2단계였다).
                    self.push_profile(ProfileScope::Full);
                    self.set_status(nbeep_core::t(nbeep_core::Msg::StWireAvatarReady));
                } else {
                    // ★ 조용한 생략 금지(08-16 사용자 확정) — 초과 사진이 안 나가면
                    // 소리 내어 알린다. 보류했던 push는 그래도 내보낸다(사진만 빠질
                    // 뿐 텍스트·내장 키 폴백은 상대에게 닿아야 한다).
                    self.push_profile(ProfileScope::Full);
                    self.status =
                        "⚠ 사진이 256KiB 초과인데 축소 생성에 실패해 상대에게 전송되지 않습니다"
                            .into();
                }
                if let Some(id) = self.main_id {
                    self.request_redraw(id);
                }
            }
            AppEvent::Decoded { target, image } => {
                // 워커 격리 디코드 복귀(M4-5 · 08-13) — 메인은 감싸고, 꽂고, 다시 그린다.
                let icon = image.map(|(w, h, rgba)| {
                    std::rc::Rc::new(nbeep_ui::IconImage::from_rgba(w, h, rgba))
                });
                match target {
                    DecodeTarget::PeerAvatar(peer) => {
                        if let Some(p) = self.peer_profiles.get_mut(&peer) {
                            p.avatar = icon;
                        }
                        let mut inv = Invalidations::default();
                        self.refresh_rows(&mut inv);
                        // 카드가 열려 있으면 새 얼굴로(M3-21 — pull 응답의 늦은 디코드).
                        self.refresh_peer_info_card(peer);
                        if let Some(mid) = self.main_id {
                            self.request_redraw(mid);
                        }
                    }
                    DecodeTarget::MyAvatar => {
                        // 앱 보관(08-14) — 툴바 프로필 버튼이 상시 쓴다(프로필 창 없어도).
                        self.my_avatar = icon.clone();
                        self.refresh_toolbar_avatar();
                        if let Some(pv) = &mut self.profile_view {
                            let mut pinv = Invalidations::default();
                            if icon.is_none() {
                                self.status =
                                    "프로필 이미지 미리보기 불가 — PNG/JPEG 아님/imgdec 부재"
                                        .into();
                            }
                            // 현재 경로의 최근 캐러셀 썸네일도 이 디코드로 채운다
                            // (같은 원본 — 별도 64 디코드가 필요 없다 · 08-14).
                            let cur = self.settings.get("profile.image_path").to_string();
                            if !cur.is_empty() {
                                pv.set_recent_thumb(&cur, icon.clone(), &mut pinv);
                            }
                            pv.set_avatar(icon, &mut pinv);
                            // (take_changes를 여기서 버리지 않는다 — 08-14: 그 버림이
                            // 최근 목록 영속을 막았다. set_avatar는 변경을 안 만든다.)
                        }
                        if let Some((pid, _)) =
                            self.windows.iter().find(|(_, e)| e.role == Role::Profile)
                        {
                            let pid = *pid;
                            self.request_redraw(pid);
                        }
                    }
                    DecodeTarget::XferThumb(peer, qpath) => {
                        if let Some(t) = icon {
                            if let Some(conv) = self.conversations.get_mut(&peer) {
                                nbeep_ui::chat_view::attach_xfer_thumb(
                                    &mut conv.lines,
                                    false,
                                    t.clone(),
                                    Some(qpath.clone()),
                                );
                            }
                            if let Some(chat) = self.chats.get_mut(&peer) {
                                let mut inv = Invalidations::default();
                                chat.attach_xfer_thumb(false, t, Some(qpath), &mut inv);
                            }
                            self.redraw_conversation(peer);
                        }
                    }
                    DecodeTarget::FullImage(qpath) => {
                        // 확대 미리보기 도착(08-16) — 열린 뷰어의 경로와 일치할 때만.
                        if let Some(v) = self.image_view.as_mut() {
                            if v.qpath == qpath {
                                v.img = icon.map_or(ImgLoad::Failed, ImgLoad::Ready);
                                if let Some((wid, _)) =
                                    self.windows.iter().find(|(_, e)| e.role == Role::ImageView)
                                {
                                    let wid = *wid;
                                    self.request_redraw(wid);
                                }
                            }
                        }
                    }
                    DecodeTarget::RecentThumb(path) => {
                        // 프로필 창이 열려 있을 때만 의미(닫혔으면 버림 — 재열림에 재디코드).
                        if let Some(pv) = &mut self.profile_view {
                            let mut pinv = Invalidations::default();
                            pv.set_recent_thumb(&path, icon, &mut pinv);
                        }
                        if let Some((pid, _)) =
                            self.windows.iter().find(|(_, e)| e.role == Role::Profile)
                        {
                            let pid = *pid;
                            self.request_redraw(pid);
                        }
                    }
                    DecodeTarget::QThumb(path) => {
                        self.qthumbs.insert(path, icon);
                        if self.quarantine_view.is_some() {
                            let rows = self.quarantine_rows();
                            if let Some(v) = &mut self.quarantine_view {
                                let mut inv = Invalidations::default();
                                v.set_rows(rows, &mut inv);
                            }
                            if let Some((qid, _)) = self
                                .windows
                                .iter()
                                .find(|(_, e)| e.role == Role::Quarantine)
                            {
                                let qid = *qid;
                                self.request_redraw(qid);
                            }
                        }
                    }
                }
            }
        }
    }

    fn exiting(&mut self, _el: &ActiveEventLoop) {
        // 부분 수신물 보존(M4-10a — 수신자 정상 종료 경로): 대화 채널을 끊으면
        // 액터가 Disconnected를 보고 부분물을 봉인 보존한 뒤 끝난다 — 그 완주를
        // 기다린다(액터 감지 지연 ≤ 수신 폴 100ms · 죽은 핸들 join은 즉시).
        // 강제 종료(kill)는 어쩔 수 없다 — 그 경우 발신자 재-Offer는 0부터.
        self.conversations.clear();
        for j in self.actor_joins.drain(..) {
            let _ = j.join();
        }
        // 상태 로그 flush(M3-22 — 종료 훅: 마지막 줄까지 디스크에).
        if let Some(l) = self.statuslog.take() {
            l.stop();
        }
        if let Some(l) = self.netmon_log.take() {
            l.stop(); // 네트워크 점검 로그도 같은 규약(08-21)
        }
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
        self.server_tick(); // Managed 서버 접속 수렴(X-2b — 2s 페이스 내부 가드)
                            // 설정 영속 tick(FR-P-9) — 조용 1s OR 상한 10s 충족 시 스냅샷 1회 저장.
        if self.conf.sched.tick(Instant::now()) {
            self.conf_save(false);
        }
        // 정지 방치 자동 취소(M4-2e ⓓ · 사용자 확정 08-19 — 한도는 설정
        // `xfer.auto_cancel_min` · 기본 2분) — 최종 상태 변화 이후 한도 넘게
        // 정지가 그대로면 **양쪽 모두 전체 취소**(cancel 경로가 CANCEL_ALL을
        // 전파 → 상대도 자기 로컬 루틴 실행). 받은 파일은 유지.
        {
            let now = self.now_ms();
            let limit = self.auto_cancel_limit_ms;
            let stale: Vec<PeerId> = self
                .xfer_pause_since
                .iter()
                .filter(|(_, &t)| now.saturating_sub(t) > limit)
                .map(|(p, _)| *p)
                .collect();
            for peer in stale {
                self.xfer_pause_since.remove(&peer);
                self.set_status(nbeep_core::t(nbeep_core::Msg::XferStaleAutoCancel).to_string());
                self.cancel_active_xfer(peer);
            }
            // 배너 카운트다운(사용자 확정 08-20 — 배너 우측 텍스트): 정지만 남아
            // 카운트다운이 실제 진행 중일 때만 남은 시간을 싣고, 초가 바뀔 때만
            // 다시 그린다(1초 틱 — 활성 전송 중엔 진행 이벤트가 타이머를 계속
            // 리셋하므로 표시하지 않는다).
            let peers: Vec<PeerId> = self.xfer_progress.keys().copied().collect();
            for peer in peers {
                let counting =
                    !self.active_send.contains_key(&peer) && !self.active_recv.contains_key(&peer);
                let desired = self
                    .xfer_pause_since
                    .get(&peer)
                    .filter(|_| counting)
                    .map(|&t| limit.saturating_sub(now.saturating_sub(t)));
                let Some(xp) = self.xfer_progress.get_mut(&peer) else {
                    continue;
                };
                let changed = match (xp.auto_cancel_ms, desired) {
                    (None, None) => false,
                    (Some(a), Some(b)) => a / 1000 != b / 1000,
                    _ => true,
                };
                if changed {
                    xp.auto_cancel_ms = desired;
                    self.apply_xfer_view(peer);
                    self.redraw_conversation(peer);
                }
            }
        }
        // 타입어헤드 유효시간 경과 → 버퍼 초기화·HUD 자동 숨김(마지막 입력 후 N초).
        {
            let now_ms = self.now_ms();
            let mut inv = Invalidations::default();
            // 보류 자모: IME 이벤트가 따라오지 않았으면 진짜 입력 — 다음 틱(~200ms)에 방출.
            {
                let outs = self.ime.tick(now_ms);
                self.apply_ime(outs, el);
            }
            // G1 틱 폴백 — 뒤따르는 키가 없어 대조가 못 정산한 잔여(게이트 판정).
            if let Some(fid) = self.os_focused {
                let ime_on = !self.is_list_mode(fid);
                let outs = self.ime.reconcile_stale(fid, now_ms, ime_on);
                if self.ime_trace && !outs.is_empty() {
                    eprintln!("[ime] inject(틱 폴백 — G1) {outs:?}");
                }
                self.apply_ime(outs, el);
            }
            if self.list.typeahead_tick(now_ms, &mut inv) {
                // 직접 조합 모드: TypeAhead.tick이 버퍼+조합기를 리셋 = 그게 전부(결정적).
                // 목록은 IME 자체가 꺼져 있어 세션 경합이 존재하지 않는다(보류는 게이트 몫).
                if let Some(id) = self.main_id {
                    self.request_redraw(id);
                }
            }
        }
        // 설정에서 요청된 백업·복원 피커 열기(M2-5a).
        if let Some(purpose) = self.pending_picker.take() {
            self.open_picker(el, purpose);
        }
        // DnD 묶음 처리(08-20 4차) — 상한 초과면 **전송 시도 자체를 하지 않는다**
        // (부분 전송 금지 — 사용자 확정 "2개 설정이면 2개 초과 = 진행 중지").
        if !self.pending_drops.is_empty() {
            let drops = std::mem::take(&mut self.pending_drops);
            self.offer_dropped(drops);
        }
        // 경고 모달 열기(08-13 — 이벤트 루프 참조가 없는 지점의 요청을 여기서 처리).
        if let Some((title, message, anchor)) = self.pending_alert.take() {
            self.open_alert(el, &title, &message, anchor);
        }
        // 구성원 모달(08-15) — 열리는 순간의 상태로 계산(pending_alert와 같은 문법).
        if let Some(gid) = self.pending_members.take() {
            self.open_members_alert(el, gid);
        }
        // `/verify` 로 연 상대 카드(08-15) — 같은 이유로 여기서 연다.
        if let Some(p) = self.pending_peer_info.take() {
            self.open_peer_info(p, el);
        }
        // 설정 초기화 확인(08-15 · 고급) — 파괴적 행위라 확인 모달을 먼저.
        if std::mem::take(&mut self.pending_reset) {
            self.open_choice(
                el,
                "설정 초기화",
                "표시되는 설정 전부를 기본값으로 되돌립니다. 계속할까요?\n(신원·핀·그룹·창 위치 등 숨김 상태는 유지됩니다)",
                "초기화",
                "취소",
                AlertCtx::SettingsReset,
            );
        }
        // 자동 재연결 tick(ⓑ) — 시점이 된 상대를 워커로 재시도(성패는 이벤트로 복귀).
        // 유휴 폴(~5Hz · M1-8x)이 이 지점을 주기적으로 지나간다.
        {
            let now = self.now_ms();
            let due: Vec<PeerId> = self
                .reconnect
                .iter()
                .filter(|(p, (_, at))| now >= *at && !self.conversations.contains_key(p))
                .map(|(p, _)| *p)
                .collect();
            for peer in due {
                // 다음 실패가 단계를 올리도록 due를 미래로 밀어 둔다(중복 발사 방지).
                if let Some(e) = self.reconnect.get_mut(&peer) {
                    e.1 = u64::MAX;
                }
                self.start_connect(peer, true);
            }
        }
        // 수신 승인 창 생성.
        if let Some(peer) = self.pending_approve_window.take() {
            if let Some(info) = self.front_offer_info(peer) {
                let mut pv = nbeep_ui::OfferPromptWidget::new(info, self.wait_timeout_sec);
                pv.start(self.now_ms());
                self.approve_view.insert(peer, pv);
                let attrs = Window::default_attributes()
                    .with_title(format!(
                        "Nexa Beep — {}",
                        nbeep_core::t(nbeep_core::Msg::WinFileRequest)
                    ))
                    .with_inner_size(winit::dpi::LogicalSize::new(440.0, 400.0))
                    .with_resizable(false)
                    // ★ 최상위(사용자 확정 08-19) — 수신 승인은 타임아웃이 걸린
                    //   **행동 요구** 창이라 다른 창에 묻히면 자동 거절로 흘러간다.
                    //   AlwaysOnTop 금지 표준(08-14)의 명시 예외(경고 모달과 같은 축).
                    .with_window_level(winit::window::WindowLevel::AlwaysOnTop)
                    .with_window_icon(self.icon.clone());
                let attrs = self.modal_attrs(attrs, false); // 메인 소유(창 묶음 부상)
                                                            // ★ 대화창 기준 중앙(08-20 사용자 확정) — 그 대화가 보이는 창
                                                            //   (분리 모드 = 해당 Chat 창 · 단일 = 메인)의 한가운데에 띄운다.
                let attrs = {
                    let anchor = self
                        .windows
                        .iter()
                        .find(|(_, e)| e.role == Role::Chat(peer))
                        .map(|(_, e)| &e.window)
                        .or_else(|| {
                            self.main_id
                                .and_then(|m| self.windows.get(&m))
                                .map(|e| &e.window)
                        });
                    if let Some(w) = anchor {
                        if let (Ok(pos), size) = (w.outer_position(), w.inner_size()) {
                            let sf = w.scale_factor();
                            let (aw, ah) =
                                ((440.0 * sf).round() as i32, (400.0 * sf).round() as i32);
                            let cx = pos.x + (i32::try_from(size.width).unwrap_or(0) - aw) / 2;
                            let cy = pos.y + (i32::try_from(size.height).unwrap_or(0) - ah) / 2;
                            attrs.with_position(winit::dpi::PhysicalPosition::new(cx, cy))
                        } else {
                            attrs
                        }
                    } else {
                        attrs
                    }
                };
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
            let _ = redraw; // 별도 발신 창 제거(M4-2e) — 진행 표시는 인챗 라인.
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
        // 네트워크 점검 주기 기록(netmon · 08-21 — 켜져 있을 때만 한 줄).
        self.tick_netmon();
        // 스크롤바 자동 숨김 틱 — **시각을 넘긴다**(호출 횟수가 아니라 벽시계로 재야
        // 이벤트가 몰리는 드래그 중에도 설정한 시간만큼 보인다 · 08-10).
        let bar_now = self.now_ms();
        // 상한 질의 타임아웃(08-18) — 구버전 상대(공지 미지원)는 2초 후 미상으로
        // 진행한다(종전 동작 = 해시 후 상대 거절로 판명).
        {
            let expired: Vec<PeerId> = self
                .cap_req_deadline
                .iter()
                .filter(|(_, d)| bar_now >= **d)
                .map(|(p, _)| *p)
                .collect();
            for p in expired {
                self.cap_req_deadline.remove(&p);
                // 미상 표시(캐시에 "무제한·지금" — 3초 신선 창 동안 질의 재발 방지).
                self.peer_recv_cap.entry(p).or_insert((u64::MAX, bar_now));
                if let Some((_, at)) = self.peer_recv_cap.get_mut(&p) {
                    *at = bar_now;
                }
                self.pump_send_queue(p);
            }
        }
        // wire_pending watchdog(RL-11 · 08-18) — 해제가 워커 완료 이벤트 **단일
        // 경로**라, 유실(워커 패닉·프록시 사망 — 둘 다 무음)이면 이후 모든
        // push_profile이 조용히 return = 프로필 전파 영구 침묵(M3-21 증상의
        // 재발 자리). imgdec는 3s kill이라 15s면 정상 경로가 절대 안 걸린다.
        if self.wire_pending && bar_now.saturating_sub(self.wire_pending_ms) > 15_000 {
            self.wire_pending = false;
            // 사진 없이라도 보류분을 내보낸다(텍스트·내장 키는 닿아야 한다 —
            // WireAvatar 실패 가지와 같은 의미론 · 조용한 생략 금지).
            self.push_profile(ProfileScope::Full);
            self.set_status("⚠ 사진 축소 응답 없음(15초) — 사진 없이 프로필을 전파했습니다");
            if let Some(id) = self.main_id {
                self.request_redraw(id);
            }
        }
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
        // 그룹 대화 스크롤바 페이드 틱(RL-14ⓑ · 08-18) — 1:1(chats)만 틱을 받아
        // 그룹 방은 자동 숨김이 발화하지 않았다(과다가 아니라 **미실행** 결함).
        {
            let gids: Vec<nbeep_core::group::GroupId> = self
                .gchats
                .iter_mut()
                .filter_map(|(g, c)| c.tick(bar_now).then_some(*g))
                .collect();
            for g in gids {
                if let Some((wid, _)) = self
                    .windows
                    .iter()
                    .find(|(_, e)| e.role == Role::GroupChat(g))
                {
                    let wid = *wid;
                    self.request_redraw(wid);
                } else if self.single_open_group == Some(g) {
                    if let Some(mid) = self.main_id {
                        self.request_redraw(mid);
                    }
                }
            }
        }
        // 프로필 최근 이미지 툴팁 틱(08-14 — 3초 호버 = 파일명 표시).
        if let Some(pv) = &mut self.profile_view {
            if pv.tick(bar_now) {
                if let Some((pid, _)) = self.windows.iter().find(|(_, e)| e.role == Role::Profile) {
                    let pid = *pid;
                    self.request_redraw(pid);
                }
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
        // 캐럿 깜빡임(08-13) — 위상이 **바뀌는 틱에만** 포커스 창을 다시 그린다
        // (매 틱 전체 재도색은 유휴 CPU 낭비 — 위상 불변이면 화면도 불변이다).
        {
            let phase = (bar_now.saturating_sub(self.blink_anchor_ms) / CARET_BLINK_MS) % 2 == 0;
            if phase != self.blink_phase_seen {
                self.blink_phase_seen = phase;
                if let Some(fid) = self.os_focused {
                    self.request_redraw(fid);
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
                // G2 — 창 닫기도 저장 트리거: 조합 중 음절을 먼저 확정 합류.
                if self.ime.composing() {
                    let now_ms = self.now_ms();
                    let outs = self.ime.commit_now(id, now_ms);
                    self.apply_ime(outs, el);
                }
                if Some(id) == self.main_id {
                    // 트레이 상주(M3-2a · 사용자 확정 08-15) — 스위치 on이면 종료 대신
                    // 숨김(전송·세션은 계속 돈다 · 복귀 = 트레이 좌클릭/열기).
                    if self.settings.get("ui.close_to_tray") == "on" && self.tray.is_some() {
                        if let Some(e) = self.windows.get(&id) {
                            e.window.set_visible(false);
                        }
                        return;
                    }
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
                        Role::GroupChat(gid) => {
                            self.gchats.remove(&gid); // 뷰만 닫힘(스레드 유지 — DR-26 동형)
                        }
                        Role::NamePrompt => {
                            self.name_prompt = None;
                            self.name_prompt_for = None;
                        }
                        Role::Settings => self.settings_view = None,
                        Role::Gallery => self.gallery_view = None,
                        Role::Picker => self.picker_view = None,
                        Role::About => self.about_view = None,
                        Role::Alert => {
                            self.alert_view = None;
                            // 선택 없이 닫음(X) — 원격 요청 대기(§6)면 거절과 동치:
                            // 세션을 드롭해 슬롯을 비운다(응답 없는 대기를 남기지 않는다).
                            if matches!(self.alert_ctx, Some(AlertCtx::RemoteInbound { .. })) {
                                self.alert_ctx = None;
                                if self.pending_remote.take().is_some() {
                                    self.set_status(nbeep_core::t(
                                        nbeep_core::Msg::StRemoteInboundDropped,
                                    ));
                                }
                            }
                        }
                        Role::AddEndpoint => self.addr_view = None,
                        Role::Profile => self.profile_view = None,
                        Role::PeerInfo(_) => self.peer_info_view = None,
                        Role::ImageView => self.image_view = None,
                        Role::Quarantine => self.quarantine_view = None,
                        Role::Convbox => self.convbox_view = None,
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
            WindowEvent::Resized(size) => {
                // 주 창 크기 기억(08-14 — 마지막 종료 위치·크기 복원). 저장은
                // SaveScheduler가 디바운스한다(DR-27 — 리사이즈 연타에도 안전).
                if Some(id) == self.main_id {
                    if let Some(e) = self.windows.get(&id) {
                        let s = f64::from(e.scale.max(0.5));
                        let lw = (f64::from(size.width) / s).round() as i64;
                        let lh = (f64::from(size.height) / s).round() as i64;
                        if lw >= 200 && lh >= 150 {
                            self.settings.set("ui.win_w", lw.to_string());
                            self.settings.set("ui.win_h", lh.to_string());
                            self.conf_mark();
                        }
                    }
                }
                self.layout_window(id);
                self.request_redraw(id);
            }
            WindowEvent::Moved(pos) => {
                // 주 창 위치 기억(08-14) — 최소화 시 OS가 주는 심연 좌표는 거른다.
                if Some(id) == self.main_id {
                    if let Some(e) = self.windows.get(&id) {
                        let s = f64::from(e.scale.max(0.5));
                        let lx = (f64::from(pos.x) / s).round() as i64;
                        let ly = (f64::from(pos.y) / s).round() as i64;
                        if (-4000..=16000).contains(&lx) && (-4000..=16000).contains(&ly) {
                            self.settings.set("ui.win_x", lx.to_string());
                            self.settings.set("ui.win_y", ly.to_string());
                            self.conf_mark();
                        }
                    }
                }
            }
            WindowEvent::ScaleFactorChanged { scale_factor, .. } => {
                if let Some(e) = self.windows.get_mut(&id) {
                    e.scale = scale_factor as f32;
                }
                self.layout_window(id);
                self.request_redraw(id);
            }
            // ⚠️ `Focused(true)`는 **한 팔로 유지한다.** 08-14에 캐럿 깜빡임과 모달 재부상이
            //    각자 팔을 만들어, 뒤엣것(재부상)이 `unreachable_pattern`으로 죽어 있었다
            //    — "메인을 클릭해도 모달이 위"가 조용히 동작하지 않았다.
            WindowEvent::Focused(true) => {
                // 캐럿 깜빡임 — 포커스 창 추적 + 기준점 리셋(받자마자 밝게 시작).
                self.os_focused = Some(id);
                self.blink_anchor_ms = self.now_ms();
                // ★ 스트레이 Enter 가드(08-21 실기 — "공지 후 대화창 진입"): 모달
                //   (공지 프롬프트 등)을 Enter로 제출하면 창 파괴 직후 메인이 포커스를
                //   돌려받는데, 같은 keypress의 잔향·키 반복 Enter가 메인 목록에
                //   도달해 **캐럿 행이 활성화**됐다(기준을 모르겠던 "1명" = 캐럿).
                //   포커스 복귀 직후 짧은 창 동안 목록행 Enter만 무시한다.
                if self.main_id == Some(id) {
                    self.enter_guard_until_ms = self.now_ms() + ENTER_GUARD_MS;
                }
                self.request_redraw(id);
                // 앱 모달이 열려 있는데 **우리 앱의 다른 창**이 활성화되면 모달을 다시
                // 앞으로(표준 모달). 다른 프로그램의 활성화에는 관여하지 않는다
                // (08-14 사용자 확정).
                if let Some(mid) = self.modal_id() {
                    if mid != id {
                        if let Some(e) = self.windows.get(&mid) {
                            e.window.focus_window();
                        }
                    }
                }
            }
            WindowEvent::Focused(false) => {
                // 캐럿 소등 — 비포커스 창은 캐럿을 그리지 않는다(네이티브 관례).
                if self.os_focused == Some(id) {
                    self.os_focused = None;
                }
                self.request_redraw(id);
                if self.ime_trace {
                    eprintln!("[ime] focus-out");
                }
                // 조합 보존·유출 확정·selfcommit 기록 — 전부 게이트 몫(H-9·H-24).
                let now_ms = self.now_ms();
                let outs = self.ime.focus_out(id, now_ms);
                self.apply_ime(outs, el);
                self.request_redraw(id);
            }
            WindowEvent::Ime(winit::event::Ime::Preedit(text, _)) => {
                self.blink_anchor_ms = self.now_ms(); // 조합 중 = 캐럿 계열 항상 밝다
                if self.ime_trace {
                    eprintln!("[ime] preedit={text:?} composing={}", self.ime.composing());
                }
                if self.is_list_mode(id) {
                    // 목록(경로 B) — 타입어헤드 즉시 매칭(게이트 미경유).
                    let now_ms = self.now_ms();
                    let mut inv = Invalidations::default();
                    self.list.set_preedit(&text, now_ms, &mut inv);
                    self.request_redraw(id);
                } else {
                    // 보류 자모 판정·조합 추적·프리에딧 보존·표시 합성 — 게이트 몫
                    // (H-2·H-9·H-14·H-16 — 규칙은 ime_gate 재생 테스트가 지킨다).
                    let now_ms = self.now_ms();
                    let outs = self.ime.preedit(id, &text, now_ms);
                    self.apply_ime(outs, el);
                }
            }
            WindowEvent::Ime(winit::event::Ime::Commit(text)) => {
                self.blink_anchor_ms = self.now_ms(); // 확정 직후에도 밝게 시작
                if self.ime_trace {
                    eprintln!("[ime] commit={text:?}");
                }
                // 낱개 자모 조합 잇기(H-14)·selfcommit 잔향(H-24)·보류 판정(H-11)·
                // 잔향 기억(H-15)·이동 키 재생(H-16) — 전부 게이트 몫.
                let now_ms = self.now_ms();
                let ime_on = !self.is_list_mode(id);
                let outs = self.ime.commit(id, &text, now_ms, ime_on);
                self.apply_ime(outs, el);
            }
            WindowEvent::DroppedFile(path) => {
                // 드래그앤드롭 = 파일 전송 시작(FR-X-1). 파일당 이벤트로 오므로
                // 여기서는 모으기만 — 묶음 판정·시도는 about_to_wait(08-20 4차).
                self.pending_drops.push((id, path));
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
                // ★ G2 — 클릭 = 조합 확정(저장 트리거 전 조합 확정 · [34 §4-4]):
                // 조합 중 버튼/토글/다른 필드 클릭 시 조합 음절이 유실되지 않게,
                // 클릭 라우팅 **전에** 지금 확정 합류시킨다(네이티브 관례와 동일).
                if self.ime.composing() && !self.is_list_mode(id) {
                    let now_ms = self.now_ms();
                    let outs = self.ime.commit_now(id, now_ms);
                    self.apply_ime(outs, el);
                }
                if let Some(e) = self.windows.get(&id) {
                    let (x, y) = e.cursor;
                    self.blink_anchor_ms = self.now_ms(); // 캐럿 재배치 = 밝게 시작
                                                          // ★ 수식키를 마우스에 싣는다(08-13 실기 — false 하드코딩이라
                                                          // ⌘클릭 다중 선택이 죽어 있었다. 키 추적값을 그대로 전달).
                    self.route(
                        id,
                        InputEvent::MouseDown {
                            x,
                            y,
                            shift: self.shift_down,
                            primary: self.primary_down,
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
                    } else if let Some(gid) = self.group_chat_for(id) {
                        if let Some(c) = self.gchats.get_mut(&gid) {
                            c.set_clipboard_has_text(has_clip);
                        }
                    } else {
                        // 텍스트 필드 창(08-13 전수 검사) — 붙여넣기 항목 활성 근거.
                        match self.windows.get(&id).map(|e| e.role) {
                            Some(Role::Profile) => {
                                if let Some(v) = self.profile_view.as_mut() {
                                    v.set_clipboard_has_text(has_clip);
                                }
                            }
                            Some(Role::NamePrompt) => {
                                if let Some(v) = self.name_prompt.as_mut() {
                                    v.set_clipboard_has_text(has_clip);
                                }
                            }
                            Some(Role::Convbox) => {
                                if let Some(v) = self.convbox_view.as_mut() {
                                    v.set_clipboard_has_text(has_clip);
                                }
                            }
                            Some(Role::AddEndpoint) => {
                                if let Some(v) = self.addr_view.as_mut() {
                                    v.set_clipboard_has_text(has_clip);
                                }
                            }
                            Some(Role::Settings) => {
                                if let Some(v) = self.settings_view.as_mut() {
                                    v.set_clipboard_has_text(has_clip);
                                }
                            }
                            Some(Role::Gallery) => {
                                if let Some(v) = self.gallery_view.as_mut() {
                                    v.set_clipboard_has_text(has_clip);
                                }
                            }
                            _ => {}
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
                // 주 키 판정은 conventions 한 곳(M3-1b — 종전 cfg! 리터럴 산재).
                self.primary_down = if nbeep_plat::conventions::primary_is_super() {
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
                self.blink_anchor_ms = self.now_ms(); // 타이핑 중엔 캐럿이 항상 밝다
                if self.ime_trace {
                    eprintln!(
                        "[ime] key={:?} composing={}",
                        event.logical_key,
                        self.ime.composing()
                    );
                }
                // ImeGate(M3-1e G3) — 조합 게이트(H-1)·잔향(H-15)·이동 키 보류(H-16)·
                // 비자모 보류(H-11)·유출 조합 개입(H-10)을 한 곳에서. 규칙과 근거는
                // ime_gate 문서·재생 테스트가 지킨다.
                let key_in = match &event.logical_key {
                    WKey::Character(t) => t
                        .chars()
                        .next()
                        .map_or(crate::ime_gate::KeyIn::Other, crate::ime_gate::KeyIn::Char),
                    WKey::Named(NamedKey::Space) => crate::ime_gate::KeyIn::Char(' '),
                    WKey::Named(NamedKey::ArrowLeft) => crate::ime_gate::KeyIn::Arrow(Key::Left),
                    WKey::Named(NamedKey::ArrowRight) => crate::ime_gate::KeyIn::Arrow(Key::Right),
                    WKey::Named(NamedKey::ArrowUp) => crate::ime_gate::KeyIn::Arrow(Key::Up),
                    WKey::Named(NamedKey::ArrowDown) => crate::ime_gate::KeyIn::Arrow(Key::Down),
                    WKey::Named(NamedKey::Home) => crate::ime_gate::KeyIn::Arrow(Key::Home),
                    WKey::Named(NamedKey::End) => crate::ime_gate::KeyIn::Arrow(Key::End),
                    _ => crate::ime_gate::KeyIn::Other,
                };
                let now_ms = self.now_ms();
                // ★ G1(개정 3차 · H-27): raw 대조·선배달 상쇄·주입은 전부 게이트 판정 —
                // 같은 문자 반복(222)·프록시 지연("가나다1223")·조합 중 순서는
                // ime_gate의 replay_h27_*·replay_g1_* 재생 테스트가 지킨다.
                if let crate::ime_gate::KeyIn::Char(cur) = key_in {
                    if !self.primary_down {
                        let ime_on = !self.is_list_mode(id);
                        let outs = self.ime.reconcile_raw(id, cur, now_ms, ime_on);
                        if self.ime_trace && !outs.is_empty() {
                            eprintln!("[ime] inject(순서 보존 — G1) {outs:?}");
                        }
                        self.apply_ime(outs, el);
                    }
                }
                if self
                    .ime
                    .keydown_gate(id, key_in, now_ms, self.shift_down, self.primary_down)
                    == crate::ime_gate::GatePass::Swallowed
                {
                    return;
                }
                let outs = self.ime.flush_pending(now_ms);
                self.apply_ime(outs, el);
                // 유출 조합 개입 — Backspace/Esc는 조합기 몫(소비), 그 외는 게이트 판단.
                if matches!(&event.logical_key, WKey::Named(NamedKey::Backspace))
                    && !self.primary_down
                {
                    let (consumed, outs) = self.ime.leak_backspace();
                    self.apply_ime(outs, el);
                    if consumed {
                        return;
                    }
                }
                if matches!(&event.logical_key, WKey::Named(NamedKey::Escape)) {
                    let (consumed, outs) = self.ime.leak_cancel();
                    self.apply_ime(outs, el);
                    if consumed {
                        return;
                    }
                }
                let (_, outs) = self.ime.leak_intercept(key_in, self.primary_down);
                self.apply_ime(outs, el);
                // 한/영 키(Windows · 목록 전용 — [docs/27 §8]): 목록 창은 IME를 끊어
                // OS 전환이 무력하므로 앱이 모드를 토글한다. VK_HANGUL은 키보드 드라이버
                // 수준이라 IME 없이도 온다(winit: 논리 HangulMode · 물리 Lang1 — 둘 다 받는다).
                // IME 켠 창(대화 등)은 OS IME 몫 — 상태를 건드리지 않고 문자 취급도 안 한다.
                if cfg!(windows)
                    && (event.logical_key == WKey::Named(NamedKey::HangulMode)
                        || event.physical_key
                            == winit::keyboard::PhysicalKey::Code(winit::keyboard::KeyCode::Lang1))
                {
                    let list_mode = Some(id) == self.main_id
                        && self.single_open.is_none()
                        && self.single_open_group.is_none();
                    if list_mode {
                        self.hangul_mode = !self.hangul_mode;
                        self.set_status(if self.hangul_mode {
                            "입력: 한글 (한/영 키로 전환)".to_string()
                        } else {
                            "입력: English (한/영 키로 전환)".to_string()
                        });
                        self.request_redraw(id);
                    }
                    return;
                }
                // Cmd/Ctrl+, = 설정 · Cmd/Ctrl+K = 수동 엔드포인트 추가(DR-19).
                if self.primary_down {
                    // 단축키 판정은 **물리 키 우선**(08-13 실기: 한글 자판에선 ⌘A의
                    // logical이 "ㅁ"으로 와서 단축키 대신 자모가 입력됐다). 물리 KeyA는
                    // 입력 소스와 무관 — [docs/27 §8] Lang1과 같은 원리. 물리 매핑이
                    // 없는 배치(비QWERTY 등)는 logical로 폴백한다.
                    {
                        use winit::keyboard::KeyCode as KC;
                        let phys = match event.physical_key {
                            winit::keyboard::PhysicalKey::Code(c) => Some(c),
                            winit::keyboard::PhysicalKey::Unidentified(_) => None,
                        };
                        let sc = match phys {
                            Some(KC::KeyA) => Some("a"),
                            Some(KC::KeyC) => Some("c"),
                            Some(KC::KeyX) => Some("x"),
                            Some(KC::KeyV) => Some("v"),
                            Some(KC::KeyG) => Some("g"),
                            Some(KC::KeyK) => Some("k"),
                            Some(KC::KeyY) => Some("y"),
                            Some(KC::KeyN) => Some("n"),
                            Some(KC::Comma) => Some(","),
                            _ => None,
                        };
                        let eff: Option<String> =
                            sc.map(str::to_string).or_else(|| match &event.logical_key {
                                WKey::Character(t) => Some(t.to_string()),
                                _ => None,
                            });
                        if let Some(t) = eff {
                            // 매핑은 conventions 한 곳(M3-1b) — 여기는 **행동만**.
                            use nbeep_plat::conventions::{std_accel, StdAccel};
                            match std_accel(&t) {
                                Some(StdAccel::Settings) => {
                                    self.open_settings(el);
                                    return;
                                }
                                Some(StdAccel::Gallery) => {
                                    self.open_gallery(el);
                                    return;
                                }
                                // 텍스트 기본 단축키 — 전체 선택(사용자 지적 08-09).
                                Some(StdAccel::SelectAll) => {
                                    self.route(id, InputEvent::SelectAll, el);
                                    return;
                                }
                                // 복사/잘라내기/붙여넣기 — **모든 텍스트 컨트롤**(① 08-13 —
                                // 그전엔 대화 입력창만). ui는 OS를 모른다 — plat 어댑터가 잇는다.
                                Some(StdAccel::Copy) => {
                                    if let Some(t) = self.clipboard_copy_for(id) {
                                        if nbeep_plat::clipboard::set_text(&t) {
                                            self.status =
                                                nbeep_core::t(nbeep_core::Msg::StCopied).into();
                                        }
                                    }
                                    return;
                                }
                                Some(StdAccel::Cut) => {
                                    if let Some(t) = self.clipboard_cut_for(id) {
                                        nbeep_plat::clipboard::set_text(&t);
                                        self.request_redraw(id);
                                    }
                                    return;
                                }
                                Some(StdAccel::Paste) => {
                                    if let Some(t) = nbeep_plat::clipboard::get_text() {
                                        self.clipboard_paste_for(id, &t);
                                        self.request_redraw(id);
                                    } else {
                                        // 텍스트 없음 → 클립보드 **이미지**면 파일
                                        // 전송으로(③ 08-20 — 대화 창에서만 발화).
                                        let _ = self.try_clipboard_image_paste(id);
                                    }
                                    return;
                                }
                                Some(StdAccel::AcceptOffer) => {
                                    self.answer_offer(id, true);
                                    return;
                                }
                                Some(StdAccel::RejectOffer) => {
                                    self.answer_offer(id, false);
                                    return;
                                }
                                // 주소 직접 입력 = 별도 모달 창(M3-16 · 인라인 상태바 입력 대체).
                                Some(StdAccel::AddEndpoint) => {
                                    self.open_add_endpoint(el);
                                    return;
                                }
                                _ => {}
                            }
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
                    // 조합을 끝낸 **Esc**의 키다운 잔향(Windows IME · WIME-6 실기): 조합
                    // 취소 직후(<120ms) 도착한 Esc를 화면 닫기로 보내지 않는다(1회성).
                    // ★ Enter는 **일부러 통과** — Commit(조합 확정)이 먼저 버퍼에 들어간 뒤
                    // Enter가 전송하므로 "확정+전송이 한 번에"가 된다(사용자 확정 08-13 ·
                    // Windows 메신저 관례 · DR-16 "동작 = OS 네이티브". macOS는 IME가 키를
                    // 삼켜 확정만 되는 2단 — OS별 관례 차이 그대로 둔다).
                    let now_ms = self.now_ms();
                    if cfg!(windows)
                        && matches!(key, Key::Escape)
                        && self.ime.take_cleared_if(now_ms, 120)
                    {
                        return;
                    }
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
                    // ⌘/Ctrl+⌫ = 줄 처음까지 삭제(mac 관례 · DR-16 — 08-13 전수 검사):
                    // 줄 처음까지 선택(Home+shift) 후 지우기 — 위젯 공통 문법 재사용.
                    if self.primary_down {
                        self.route(
                            id,
                            InputEvent::Key {
                                key: Key::Home,
                                shift: true,
                                primary: false,
                            },
                            el,
                        );
                    }
                    self.route(id, InputEvent::Char { c: '\u{8}', now_ms }, el);
                    return;
                }
                // 스페이스는 Named(Space)로 와서 Character 경로에 안 잡힌다 — 문자로 라우팅
                // (목록 타입어헤드·대화 입력 공통. 이전엔 직타 스페이스가 유실됐다).
                if let WKey::Named(NamedKey::Space) = &event.logical_key {
                    // ⌘/Ctrl+Space = 목록 선택 토글(탐색기 키보드 다중 선택 · 08-15) —
                    // 문자(타입어헤드) 경로보다 먼저 가로챈다. 목록 모드에서만(대화
                    // 입력창의 ⌘Space는 OS 입력 전환 등과 충돌하지 않게 그대로 둔다).
                    if self.primary_down && self.is_list_mode(id) {
                        self.route(
                            id,
                            InputEvent::Key {
                                key: nbeep_ui::Key::Space,
                                shift: self.shift_down,
                                primary: true,
                            },
                            el,
                        );
                        return;
                    }
                    let now_ms = self.now_ms();
                    // 게이트 경유(★ G1 2차 회귀의 교훈): 직접 route하면 배달 증거가
                    // 안 남아 raw 대조가 "미배달"로 오판·재주입한다. 잔향은 keydown_gate.
                    let ime_on = !self.is_list_mode(id);
                    let outs = self.ime.route_char(id, ' ', now_ms, ime_on);
                    self.apply_ime(outs, el);
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
                        let list_mode = Some(id) == self.main_id
                            && self.single_open.is_none()
                            && self.single_open_group.is_none();
                        if is_jamo && !list_mode {
                            // IME 켜진 창: 중복/유출 판정은 게이트 몫(H-2 — 보류 등록).
                            let outs = self.ime.route_char(id, c, now_ms, true);
                            self.apply_ime(outs, el);
                        } else if !c.is_control() {
                            // ★ 모든 문자는 게이트 경유(배달 증거 봉인) — "가나다12233"
                            // 이중 입력의 진범이 이 직접 route 우회였다(증거 부재 →
                            // raw 대조가 정상 배달분을 소비된 키로 오판·재주입).
                            // Windows 한글 모드(한/영 키 토글): IME가 없어 라틴이 온다 —
                            // 두벌식 자모로 번역해 넣는다(대문자 = 시프트 반영·[docs/27 §8]).
                            // 숫자·기호는 None → 원문 그대로(한글 모드에서도 통과).
                            let c = if cfg!(windows)
                                && list_mode
                                && self.hangul_mode
                                && !self.primary_down
                            {
                                nbeep_ui::hangul::jamo_from_qwerty(c, c.is_ascii_uppercase())
                                    .unwrap_or(c)
                            } else {
                                c
                            };
                            // 게이트 경유(목록 = ime_on false → 즉시 방출·증거 봉인).
                            let outs = self.ime.route_char(id, c, now_ms, !list_mode);
                            self.apply_ime(outs, el);
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
/// 그룹 미전달 보관 상한(ADR-0012 §4 — 주체 = 송신자 · 사용자 확정 08-13).
/// 설정 `group.resync_keep` 관용 파싱 — 무효·0은 기본 200. **사용 시점에 읽어**
/// 설정 변경이 재시작 없이 반영된다(hot-swap 원칙).
fn group_resync_keep(settings: &SettingsState) -> usize {
    settings
        .get("group.resync_keep")
        .parse::<usize>()
        .ok()
        .filter(|n| *n > 0)
        .unwrap_or(200)
}

/// 캐러셀 스크롤 방향 해석(`ui.carousel_scroll` — 08-14 사용자 확정):
/// "auto" = OS 기본(**mac = 내추럴(반전)** · 그 외 = 정방향) · "fwd"/"rev" = 강제.
fn carousel_inverted(settings: &SettingsState) -> bool {
    match settings.get("ui.carousel_scroll") {
        "fwd" => false,
        "rev" => true,
        _ => cfg!(target_os = "macos"),
    }
}

/// 그룹 파일 팬아웃의 미연결 대기 경로 상한(13 §12-1 큐 상한 필수 — 텍스트와 달리
/// 파일은 오퍼·승인 비용이 커서 낮게 잡는다).
const GROUP_FILE_KEEP: usize = 16;

fn session_port_from(settings: &SettingsState) -> u16 {
    settings
        .get("net.session_port")
        .parse::<u16>()
        .ok()
        .unwrap_or(nbeep_net::DEFAULT_SESSION_PORT)
}

/// 데이터 디렉터리(FR-P-3 · DR-4) — 실행 파일 옆 `data/` 쓰기 가능(포터블)
/// → 사용자 설정 폴더 → **홈의 숨김 폴더**(최후 폴백 · M5-4e 08-20 확정).
/// ⚠️ 폴백 종점을 임시 폴더에 두지 않는다(DR-4 개정 08-19) — macOS
/// `tmp_cleaner`가 `identity.key`를 지워 **신원이 조용히 바뀌고**, 이전 키에
/// sealed된 격리물·핀을 영구히 못 연다(실기 사고). 홈조차 없는 극단에서만
/// 임시 폴더가 남는데, 그땐 신원 영속 자체가 성립하지 않는 환경이다.
/// 설정(`settings.cfg`)·신원 키(`identity.key`)·핀 세그먼트(`trust.seg`)가 전부
/// 여기 산다. 경로는 여기서 정해 **인자로 넘긴다**(소비 크레이트는 경로 비소유).
/// 동기화 폴더 감지(M2-5b · [17 §6] · 08-21) — 데이터 폴더가 클라우드 동기화
/// 폴더 안이면 제공자 이름을 돌려준다. 봉인이라 **내용 유출은 아니지만**, 동기화
/// 충돌·구버전 복원(지운 대화가 되살아남)·다중 기기 동시 실행이 세그먼트를
/// 조용히 망가뜨릴 수 있어 **경고만** 한다(막지 않는다 — DR-1 제로 컨피그).
/// 판정 = 경로 구성요소 이름(대소문자 무시) + Windows `OneDrive` env 접두.
pub(crate) fn sync_folder_hint(path: &std::path::Path) -> Option<&'static str> {
    let comp_is = |c: &str| -> Option<&'static str> {
        let l = c.to_ascii_lowercase();
        if l.contains("onedrive") {
            Some("OneDrive")
        } else if l.contains("dropbox") {
            Some("Dropbox")
        } else if l == "google drive" || l == "googledrive" || l == "my drive" {
            Some("Google Drive")
        } else if l.contains("icloud") || l == "com~apple~clouddocs" {
            Some("iCloud")
        } else if l.contains("nextcloud") {
            Some("Nextcloud")
        } else {
            None
        }
    };
    for c in path.components() {
        if let std::path::Component::Normal(os) = c {
            if let Some(p) = os.to_str().and_then(comp_is) {
                return Some(p);
            }
        }
    }
    // Windows — OneDrive는 사용자가 폴더명을 바꿀 수 있어 env 접두로도 본다.
    #[cfg(windows)]
    for key in ["OneDrive", "OneDriveConsumer", "OneDriveCommercial"] {
        if let Some(root) = std::env::var_os(key) {
            let root = std::path::PathBuf::from(root);
            if !root.as_os_str().is_empty() && path.starts_with(&root) {
                return Some("OneDrive");
            }
        }
    }
    None
}

/// 클립보드 스테이징 정리(SEAL-2 ③ · 08-21) — `data/clipboard/clip-*.png`는 보낸
/// 클립보드 이미지의 스테이징 사본이라 전송 뒤에도 남는다(스트리밍이 파일에서
/// 읽는 구조상 즉시 삭제 불가). 부팅마다 24h 지난 것을 지운다(part sweep 문법).
/// 사진 사본·와이어 축소본(①②)은 **의도된 경계로 확정** — 원본이 사용자 평문
/// 파일이라 사본 봉인은 보호 이득이 없다([17 §4-a] 관찰 종결).
pub(crate) fn sweep_clipboard_staging(dir: &std::path::Path) {
    const RETAIN_SECS: u64 = 24 * 3600;
    let Ok(rd) = std::fs::read_dir(dir) else {
        return;
    };
    let now = std::time::SystemTime::now();
    for e in rd.flatten() {
        let p = e.path();
        let named_clip = p
            .file_name()
            .and_then(|n| n.to_str())
            .is_some_and(|n| n.starts_with("clip-") && n.ends_with(".png"));
        if !named_clip {
            continue;
        }
        let expired = e
            .metadata()
            .ok()
            .and_then(|m| m.modified().ok())
            .is_some_and(|t| {
                now.duration_since(t)
                    .map(|d| d.as_secs() > RETAIN_SECS)
                    .unwrap_or(false)
            });
        if expired {
            let _ = std::fs::remove_file(&p);
        }
    }
}

pub(crate) fn data_dir() -> std::path::PathBuf {
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
    let home_key = if cfg!(windows) { "USERPROFILE" } else { "HOME" };
    if let Some(h) = std::env::var_os(home_key) {
        return std::path::PathBuf::from(h).join(".nexa-beep");
    }
    std::env::temp_dir().join("nexa-beep")
}

/// `--whoami`(진단) — 이 실행 파일이 **실제로 로드할** 신원을 **읽기 전용**으로 찍는다.
/// 지문·표시 이름·실행 파일 경로·데이터 경로. **키를 생성하지 않는다**(재기동마다
/// 신원이 바뀌는지 추적하려면 진단 자체가 새 키를 만들면 안 된다 — fail-closed 관찰).
pub(crate) fn print_whoami() {
    let exe = std::env::current_exe()
        .map(|p| p.display().to_string())
        .unwrap_or_else(|_| "?".into());
    let dir = data_dir();
    let key_path = dir.join("identity.key");
    // 읽기 전용 — 없거나 손상이면 그대로 보고(생성 금지).
    // 전체 64hex도 함께 — 서버 랑데부(`--chat-connect-via`·주소 모달 지문 입력)의
    // 대상 값이라 짧은 표기만으로는 쓸 수 없다(X-2b/2c · 08-22).
    let (fp, fp_full, key_state) = match std::fs::read(&key_path) {
        Ok(b) if b.len() == 68 && &b[..4] == b"NBK1" => {
            let mut k = [0u8; 64];
            k.copy_from_slice(&b[4..]);
            let id = nbeep_crypto::Identity::from_key_bytes(&k);
            (
                id.peer_id().short(),
                nbeep_relay::peer_hex(&id.peer_id()),
                "로드됨",
            )
        }
        Ok(_) => ("--------".into(), String::new(), "⚠ 손상(길이/매직)"),
        Err(_) => (
            "--------".into(),
            String::new(),
            "⚠ 없음(다음 기동에 새로 생성됨)",
        ),
    };
    // 표시 이름 — 부팅과 같은 규칙(settings.cfg의 profile.display_name · auto면 호스트/지문).
    let name = {
        let mut settings = SettingsState::with_defaults();
        let (mut conf, doc) = nexa_conf::Store::open(dir.join("settings.cfg"), 1_000, 10_000);
        for (k, v) in doc.pairs {
            if !settings.set_by_name(&k, &v) {
                conf.keep_unknown(k, v);
            }
        }
        // 지문 폴백을 위해 peer가 필요 — 로드 실패 시 0으로(이름만 폴백 라벨).
        let peer = match std::fs::read(&key_path) {
            Ok(b) if b.len() == 68 && &b[..4] == b"NBK1" => {
                let mut k = [0u8; 64];
                k.copy_from_slice(&b[4..]);
                nbeep_crypto::Identity::from_key_bytes(&k).peer_id()
            }
            _ => nbeep_core::PeerId::from_bytes([0u8; 32]),
        };
        effective_display_name(&settings, &peer)
            .as_str()
            .to_string()
    };
    println!("fingerprint = {fp}  ({key_state})");
    if !fp_full.is_empty() {
        println!("full        = {fp_full}");
    }
    println!("name        = {name}");
    println!("exe         = {exe}");
    println!("data        = {}", dir.display());
}

pub(crate) fn run(mode: WindowMode, live: bool, port_flag: Option<u16>) {
    // 컨트롤 내장 문자열 = 앱 i18n 연결(08-14 라이브러리화 — 컨트롤은 t()를 모른다.
    // 공급자가 t()를 부르므로 언어 전환도 자동 반영된다).
    nbeep_ui::controls::set_ctl_labels(|m| {
        use nbeep_core::{t, Msg};
        use nbeep_ui::controls::CtlMsg;
        t(match m {
            CtlMsg::CtxSelectAll => Msg::CtxSelectAll,
            CtlMsg::CtxCopy => Msg::CtxCopy,
            CtlMsg::CtxCut => Msg::CtxCut,
            CtlMsg::CtxPaste => Msg::CtxPaste,
        })
    });
    let (data, index) = nbeep_plat::font::system_ui_font().expect("시스템 UI 폰트 없음");
    let font = nbeep_gfx::Font::from_static(data, index).expect("폰트 파싱");
    let dir = data_dir();
    // 신원 영속(M2-5a) — 재시작해도 같은 PeerId. 키 파일 손상 시 **덮어쓰지 않고**
    // 임시 신원으로 강등(fail-closed — 조용히 새 키를 만들면 상대 핀에서 남이 된다).
    let (identity, id_note, id_persistent) =
        match nbeep_crypto::keyfile::load_or_generate(&dir.join("identity.key")) {
            Ok((id, created)) => (id, created.then_some("새 신원 키 생성"), true),
            Err(e) => {
                eprintln!("신원 키 파일 사용 불가({e}) — 이번 실행은 임시 신원");
                (
                    nbeep_crypto::Identity::generate(),
                    Some("⚠ 신원 키 파일 손상 — 임시 신원(재시작하면 바뀜)"),
                    false, // 임시 신원 — 세그먼트 보관 이동 금지(진짜 주인 보호)
                )
            }
        };
    let identity = std::sync::Arc::new(identity);
    // 핀·그룹 세그먼트(M2-5a·M5-1) — 래핑 원료 = 기기 신원 키(ADR-0005 §3 기본 A).
    // ★ 정상 신원 부팅은 open_or_archive(08-19): 현 신원으로 못 여는 잠긴 세그먼트를
    //   `<이름>.locked`로 **보관 이동**하고 새로 시작한다. 종전엔 잠긴 채 메모리
    //   전용으로 돌아 이번 세션의 핀·그룹이 재시작마다 조용히 증발했다(실기 —
    //   tmp 청소로 옛 신원이 사라진 뒤 옛 세그먼트가 영구 잠김). 임시 신원 부팅은
    //   종전대로 보존·메모리 전용(진짜 주인의 세그먼트를 밀어내면 안 된다).
    let (trust, trust_load) = if id_persistent {
        nbeep_store::FileTrustStore::open_or_archive(dir.join("trust.seg"), identity.wrap_secret())
    } else {
        nbeep_store::FileTrustStore::open(dir.join("trust.seg"), identity.wrap_secret())
    };
    let (groups, group_load) = if id_persistent {
        nbeep_store::FileGroupStore::open_or_archive(dir.join("groups.seg"), identity.wrap_secret())
    } else {
        nbeep_store::FileGroupStore::open(dir.join("groups.seg"), identity.wrap_secret())
    };
    use nbeep_net::Transport as _;

    // 이벤트 루프·프록시 먼저 — 인바운드 수락 펌프가 프록시를 필요로 한다(M2-7).
    let event_loop = EventLoop::<AppEvent>::with_user_event().build().unwrap();
    let proxy = event_loop.create_proxy();
    let shutdown = nbeep_plat::shutdown::install(); // R-16 — SIGINT/SIGTERM 포트
                                                    // 정식 macOS 알림 초기화(M3-8b · 사용자 확정 "정식 알림") — **번들(.app) 실행**이면
                                                    // UNUserNotificationCenter(권한 요청 + 클릭 delegate: 알림 클릭 = 창 복원 —
                                                    // 트레이 Open과 같은 이벤트). 비번들·타 OS는 false = 기존 폴백 그대로.
    {
        let nproxy = proxy.clone();
        let _ = nbeep_plat::notify::init(move |target| {
            let ev = match target {
                Some(t) => nbeep_plat::tray::TrayEvent::OpenTarget(t),
                None => nbeep_plat::tray::TrayEvent::Open,
            };
            let _ = nproxy.send_event(AppEvent::Tray(ev));
        });
    }

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
    // 폰트 크기 구 코드 이관(08-18 2차 — 절대 프리셋·전 슬롯 공통):
    // s→14 · m→16 · l→18 · xl→22 (사용자 확정 — 기본은 모두 Normal 16).
    for region in ["base", "peerlist", "message", "status"] {
        let key = format!("font.{region}.size");
        let px = match settings.get(&key) {
            "s" => Some("14"),
            "m" => Some("16"),
            "l" => Some("18"),
            "xl" => Some("22"),
            _ => None, // 이미 숫자(신) 또는 빈 값
        };
        if let Some(px) = px {
            settings.set_by_name(&key, px);
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
    // 발견 수신 강등(M1-13ⓔ) — 포트 점유 시 패닉 대신 발신 전용 + 상태바 고지.
    let mut discovery_degraded = false;
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
            .expect("LocalDirect 시작(송신 소켓·TCP 바인딩 — 발견 포트 점유는 오류 아님)");
            listen_port = Some(local.tcp_port());
            discovery_degraded = local.discovery_recv_degraded();
            let discovery = local.discovery();
            // 인바운드 수락 펌프 — 남이 나에게 연결하면 accept+에코(대칭 대화·비동기 GUI 펌프는 M2-7).
            spawn_inbound_accept(
                local.incoming(),
                std::sync::Arc::clone(&identity),
                proxy.clone(),
            );
            // L1 링크 구독(M1-2 · FR-D-5) — OS raw 이벤트(폭주)를 디바운스로 접어
            // LinkChanged 1회로. quiet 1000ms는 잠정(D-8b 실측 후 확정 — [08 §8]).
            // 구독 실패(None)는 조용히 폴백 — 주기 광고가 결국 따라잡는다(fail-soft).
            if let Some(raw) = nbeep_plat::linkwatch::spawn() {
                let lproxy = proxy.clone();
                std::thread::spawn(move || {
                    let mut deb = nbeep_core::linkwatch::Debouncer::new(1000);
                    let t0 = std::time::Instant::now();
                    let mono = |t0: &std::time::Instant| {
                        nbeep_core::MonoInstant(
                            u64::try_from(t0.elapsed().as_nanos()).unwrap_or(u64::MAX),
                        )
                    };
                    // 200ms 폴은 trailing 디바운스 마감 확인용(이벤트 없으면 통과) —
                    // 종료 = 채널 끊김(구독 스레드 소멸과 함께 자연 종료).
                    loop {
                        match raw.recv_timeout(std::time::Duration::from_millis(200)) {
                            Ok(()) => deb.observe(mono(&t0)),
                            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {}
                            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => return,
                        }
                        if deb.fire(mono(&t0)) && lproxy.send_event(AppEvent::LinkChanged).is_err()
                        {
                            return;
                        }
                    }
                });
            }
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
    // 하단 상태바 기동 안내(08-17 i18n).
    let net_hint = if !live {
        nbeep_core::t(nbeep_core::Msg::HintDemo)
    } else if discovery_degraded {
        // M1-13ⓔ — "왜 아무도 안 보이는지"를 화면이 말해야 한다(조용한 비호환 금지).
        nbeep_core::t(nbeep_core::Msg::HintDiscoveryDegraded)
    } else {
        nbeep_core::t(nbeep_core::Msg::HintDiscovery)
    };
    let mode_hint = match mode {
        WindowMode::Single => nbeep_core::t(nbeep_core::Msg::HintOpenChat),
        WindowMode::Separate => nbeep_core::t(nbeep_core::Msg::HintNewWindow),
    };
    // 신뢰 저장 상태 고지(M2-5a) — 잠김은 fail-closed라 반드시 사용자에게 보인다.
    let trust_hint = match trust_load {
        nbeep_store::TrustLoad::Locked => {
            format!(" · {}", nbeep_core::t(nbeep_core::Msg::HintTrustLocked))
        }
        nbeep_store::TrustLoad::Archived => {
            format!(" · {}", nbeep_core::t(nbeep_core::Msg::HintTrustArchived))
        }
        nbeep_store::TrustLoad::Loaded(_) | nbeep_store::TrustLoad::Fresh => String::new(),
    };
    // 그룹 저장 상태 고지(M5-1) — 같은 fail-closed 규약.
    let group_hint = match group_load {
        nbeep_store::GroupLoad::Locked => {
            format!(" · {}", nbeep_core::t(nbeep_core::Msg::HintGroupLocked))
        }
        nbeep_store::GroupLoad::Archived => {
            format!(" · {}", nbeep_core::t(nbeep_core::Msg::HintGroupArchived))
        }
        nbeep_store::GroupLoad::Loaded(_) | nbeep_store::GroupLoad::Fresh => String::new(),
    };
    let id_hint = id_note.map(|n| format!(" · {n}")).unwrap_or_default();
    let settings_sort_value = settings.get("ui.list_sort").to_string();
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
        relay: None,
        relay_connecting: false,
        relay_backoff: (0, 0),
        relay_gen: 0,
        relay_check_at: 0,
        discovery,
        table: nbeep_core::PeerTable::new(60_000),
        trust,
        groups,
        group_threads: HashMap::new(),
        single_open_group: None,
        gchats: HashMap::new(),
        name_prompt: None,
        name_prompt_for: None,
        pending_direct: HashMap::new(),
        pending_group_sends: HashMap::new(),
        pending_group_files: HashMap::new(),
        pending_invites: HashMap::new(),
        group_accepts: HashMap::new(),
        gunread: HashMap::new(),
        alert_ctx: None,
        pending_remote: None,
        pending_members: None,
        pending_peer_info: None,
        verify_hinted: std::collections::HashSet::new(),
        members_ctx: None,
        pending_reset: false,
        conversations: HashMap::new(),
        dedup: nbeep_core::DedupIndex::new(),
        started: Instant::now(),
        status: format!(
            "[{net_hint}] {mode_hint} · {} · {} · {}{trust_hint}{group_hint}{id_hint}",
            nbeep_core::t(nbeep_core::Msg::HintAddAddr),
            nbeep_core::t(nbeep_core::Msg::HintSettings),
            nbeep_core::t(nbeep_core::Msg::HintGallery),
        ),
        fonts: App::fonts_from_settings(&settings),
        settings,
        conf,
        settings_view: None,
        gallery_view: None,
        about_view: None,
        alert_view: None,
        pending_alert: None,
        pending_drops: Vec::new(),
        quarantine_view: None,
        convbox_view: None,
        xfer_progress: HashMap::new(),
        ledger: nbeep_core::ExchangeLedger::new(),
        approval: nbeep_core::ApprovalPolicy::default(),
        approval_window: nbeep_core::AutoWindow::Hour1,
        approval_started_unix: None,
        approval_footer_sec: 0,
        wait_timeout_sec: 60,
        approve_view: HashMap::new(),
        pending_approve_window: None,
        face_base: None,
        face_peerlist: None,
        face_message: None,
        face_status: None,
        face_mono: None,
        pending_offers: HashMap::new(),
        send_queue: HashMap::new(),
        qscan_gen: 0,
        qrows_raw: Vec::new(),
        peer_recv_cap: HashMap::new(),
        cap_req_deadline: HashMap::new(),
        preparing_send: std::collections::HashSet::new(),
        current_send: HashMap::new(),
        resend_offers: HashMap::new(),
        send_batch: HashMap::new(),
        awaiting_accept: HashMap::new(),
        active_send: HashMap::new(),
        active_recv: HashMap::new(),
        last_recv_seq: HashMap::new(),
        image_view: None,
        last_link_probe_ms: 0,
        send_avg: HashMap::new(),
        awaiting_ack: HashMap::new(),
        close_armed: false,
        send_wait: HashMap::new(),
        send_paused: HashMap::new(),
        send_excluded: HashMap::new(),
        paused_sends: HashMap::new(),
        recv_xids: HashMap::new(),
        recv_paused: HashMap::new(),
        recv_batch: HashMap::new(),
        recv_batch_sizes: HashMap::new(),
        xfer_pause_since: HashMap::new(),
        auto_cancel_limit_ms: 120_000,
        recv_manifest: HashMap::new(),
        batch_approved: HashMap::new(),
        batch_declined: HashMap::new(),
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
            // 대화함(M3-23) — 격리함과 동일 레벨(서랍 아이콘 · 사용자 확정 08-17).
            ToolItem::new(
                "convbox",
                ToolIcon::Mask {
                    w: nbeep_ui::icons::DRAWER_SIZE,
                    h: nbeep_ui::icons::DRAWER_SIZE,
                    alpha: nbeep_ui::icons::DRAWER_ALPHA,
                },
            ),
            // 프로필 버튼 = **내 얼굴 미니**(08-14 사용자 요청 — 우측 끝 배치).
            // 실제 아이콘은 부팅 직후 refresh_toolbar_avatar가 설정값으로 채운다.
            ToolItem::new(
                "profile",
                ToolIcon::Avatar {
                    img: None,
                    initials: String::new(),
                    seed: Vec::new(),
                    border: None,
                },
            )
            .align_right(),
            // 컨트롤 갤러리는 툴바에서 뺐다(사용자 요청 08-10) — 메뉴(보기 ▸ 컨트롤 갤러리)와
            // ⌘/Ctrl+G로 열 수 있으니, 상시 노출할 임시 검수용 항목은 툴바를 차지할 이유가 없다.
        ]),
        sort_drop: nbeep_ui::IconDropdown::new(
            App::sort_drop_items(),
            settings_sort_value.as_str(),
        ),
        closed_peers: std::collections::HashSet::new(),
        extra_peers: HashMap::new(),
        manual_addrs: HashMap::new(),
        unread: HashMap::new(),
        last_notify: HashMap::new(),
        notify_targets: HashMap::new(),
        last_read: HashMap::new(),
        reconnect: HashMap::new(),
        rows_dirty: false,
        last_rows_ms: 0,
        list_refresh_ms: 1500,
        icon: winit::window::Icon::from_rgba(
            nbeep_ui::brand::ICON_RGBA.to_vec(),
            nbeep_ui::brand::ICON_SIZE,
            nbeep_ui::brand::ICON_SIZE,
        )
        .ok(),
        picker_view: None,
        profile_view: None,
        peer_profiles: HashMap::new(),
        peer_profile_fp: HashMap::new(),
        resumed_recv: HashMap::new(),
        statuslog: None,
        netmon_log: None,
        netmon_prev: nbeep_net::netmon::NetSnapshot::default(),
        netmon_last_sec: 0,
        last_broadcast_ms: 0,
        enter_guard_until_ms: 0,
        datakeys: crate::keytable::KeyTable::empty(),
        actor_joins: Vec::new(),
        wire_gen: 0,
        wire_pending: false,
        wire_pending_ms: 0,
        builtin_avatars: nbeep_ui::avatar_assets::builtins()
            .into_iter()
            .map(|b| (b.key, Rc::new(b.image)))
            .collect(),
        my_avatar: None,
        tray: None,
        peer_info_view: None,
        picker_ctx: None,
        pending_picker: None,
        data_dir: dir,
        live,
        listen_port,
        addr_view: None,
        ime: crate::ime_gate::ImeGate::new(),
        parked_lines: HashMap::new(),
        qthumbs: HashMap::new(),
        ime_trace: std::env::var_os("NEXA_IME_TRACE").is_some(),
        os_focused: None,
        blink_anchor_ms: 0,
        blink_phase_seen: true,
        primary_down: false,
        shift_down: false,
        hangul_mode: false,
        proxy,
        shutdown,
    };
    app.reload_faces(); // 고정폭 등 슬롯 얼굴 초기 로드
    app.apply_boot_settings(); // 영속 설정 → 파생 런타임 상태(테마·정책 등 · M3-15)
    app.promote_local_groups(); // 구버전 로컬(동보) 그룹 → 그룹 대화(G4 마이그레이션)
    app.load_pii_sidecar(); // 연락처(PII) 봉인 사이드카 — cfg 구본보다 우선(08-17)
                            // 데이터 키 테이블(셰레딩 · D-18 §7) — 기록 복원보다 먼저(개봉 키의 원천).
    app.datakeys =
        crate::keytable::KeyTable::load(app.data_dir.join("keys.seg"), app.identity.wrap_secret());
    app.restore_history(); // 대화 기록 복원(M2-5b · parked_lines에 · 대화창 열면 뜬다)
    app.restore_pending(); // 1:1 오프라인 대기 복원(M4-6 — 대기 풍선+자동 전달 후보)
    app.restore_group_history(); // 그룹 기록 복원(08-19 · g-{uid}.seg → group_threads)
    app.restore_cached_profiles(); // 핀 상대의 캐시 프로필·목록 행 복원(08-14)
    app.ensure_wire_avatar(); // 상한 초과 사진의 와이어 축소본 보장(08-16 — 기존 사용자 자기 치유)
    crate::part::sweep_partials(crate::gate::CH_GUI); // 부분물 수명 정리(M4-10a — 72h·1GiB)
    sweep_clipboard_staging(&app.data_dir.join("clipboard")); // 스테이징 정리(SEAL-2 — 24h)
                                                              // 동기화 폴더 경고(M2-5b · 17 §6) — 부팅 1회 고지(막지 않는다).
    if let Some(provider) = sync_folder_hint(&app.data_dir) {
        app.set_status(nbeep_core::tf(
            nbeep_core::Msg::StfSyncFolderWarn,
            &[provider],
        ));
    }
    app.refresh_statuslog(); // 상태 로그(M3-22 — log.enabled면 여기서 기동)
    app.refresh_netmon(); // 네트워크 점검(08-21 — netmon.enabled면 여기서 기동)
    event_loop.run_app(&mut app).unwrap();
}

#[cfg(test)]
mod tests {
    /// M2-5b(08-21) — 동기화 폴더 판정: 구성요소 이름 기반(대소문자 무시) ·
    /// 일반 경로는 None. env 축(Windows OneDrive 접두)은 환경 의존이라 제외.
    #[test]
    fn sync_folder_hint_matches_known_providers_only() {
        use std::path::Path;
        // 전 OS 공통 단언은 `/` 구분자로 — Windows Path도 `/`를 갈라 읽는다.
        // (백슬래시는 유닉스에서 구분자가 아니라 경로 전체가 한 구성요소가 되고,
        //  등호 매처(google drive)가 못 본다 — CI 첫 노출이 잡은 OS 가정 · 08-21.)
        assert_eq!(
            super::sync_folder_hint(Path::new("/Users/u/OneDrive/문서/data")),
            Some("OneDrive")
        );
        assert_eq!(
            super::sync_folder_hint(Path::new("/Users/u/Dropbox/beep/data")),
            Some("Dropbox")
        );
        assert_eq!(
            super::sync_folder_hint(Path::new("/Users/u/Google Drive/beep")),
            Some("Google Drive")
        );
        assert_eq!(
            super::sync_folder_hint(Path::new(
                "/Users/u/Library/Mobile Documents/com~apple~CloudDocs/x"
            )),
            Some("iCloud")
        );
        assert_eq!(
            super::sync_folder_hint(Path::new("/home/u/projects/nexa-beep/data")),
            None,
            "일반 경로 오탐 금지"
        );
        // Windows 백슬래시 경로는 Windows에서만 구성요소로 갈라진다.
        #[cfg(windows)]
        {
            assert_eq!(
                super::sync_folder_hint(Path::new(r"C:\Users\u\OneDrive\문서\data")),
                Some("OneDrive")
            );
            assert_eq!(
                super::sync_folder_hint(Path::new(r"D:\Google Drive\beep")),
                Some("Google Drive")
            );
            assert_eq!(
                super::sync_folder_hint(Path::new(r"D:\Projects\nexa-beep\data")),
                None,
                "일반 경로 오탐 금지(Win)"
            );
        }
    }

    /// SEAL-2 ③(08-21) — 스테이징 정리는 `clip-*.png`만 본다(신선분·타 파일 보존).
    /// 만료 삭제 축은 mtime 주입 수단이 없어 여기선 필터 계약만 박제한다.
    #[test]
    fn clipboard_sweep_touches_only_clip_pngs() {
        let dir = std::env::temp_dir().join(format!("nb-clipsweep-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("clip-123.png"), b"png").unwrap(); // 신선 — 남는다
        std::fs::write(dir.join("keep.txt"), b"x").unwrap(); // 이름 불일치 — 남는다
        super::sweep_clipboard_staging(&dir);
        assert!(dir.join("clip-123.png").exists(), "신선분 보존");
        assert!(dir.join("keep.txt").exists(), "타 파일 무접촉");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// M4-2e 요청 단위 결정(08-20 사용자 시나리오 자동화) — 2.sna+3.hsh 배치.
    /// 거절 1회 = 배치 전체 거절, 승인 1회 = 배치 전체 승인이 **같은 헬퍼**로
    /// 성립함을 박제한다(승인은 배치·거절은 단건이던 실기 구멍의 회귀).
    #[test]
    fn request_level_decision_covers_whole_batch() {
        let manifest = vec![
            ("2.sna".to_string(), 491_900_000u64, false),
            ("3.hsh".to_string(), 58_000_000u64, false),
            ("skip.tmp".to_string(), 10u64, true), // 제외 파일은 결정 대상 아님
        ];
        // ① 거절 시나리오: 프롬프트(첫 파일 2.sna)에서 거절 → 잔여 = [3.hsh].
        let mut declined = super::batch_remainder(manifest.clone(), "2.sna", 491_900_000);
        assert_eq!(declined, vec![("3.hsh".to_string(), 58_000_000)]);
        //    3.hsh 오퍼 도착 = 자동 거절 소비 · 배치 비움(추가 프롬프트 없음).
        assert_eq!(
            super::batch_take(&mut declined, "3.hsh", 58_000_000),
            Some(true),
            "두 번째 파일은 재질문 없이 자동 거절"
        );
        //    낯선 파일(불일치)은 수동 폴백 — fail-closed.
        assert_eq!(super::batch_take(&mut declined, "3.hsh", 58_000_000), None);
        // ② 승인 시나리오: 같은 헬퍼 — 승인 1회로 잔여 자동 수락.
        let mut approved = super::batch_remainder(manifest, "2.sna", 491_900_000);
        assert_eq!(
            super::batch_take(&mut approved, "3.hsh", 58_000_000),
            Some(true),
            "두 번째 파일은 재질문 없이 자동 승인"
        );
        // ③ 같은 이름 2회 요청 — 항목 수만큼만 소비된다(초과분 = 수동).
        let dup = vec![
            ("a.bin".to_string(), 5u64, false),
            ("a.bin".to_string(), 5u64, false),
        ];
        let mut rem = super::batch_remainder(dup, "a.bin", 5);
        assert_eq!(rem.len(), 1, "결정한 자신은 1회만 제외");
        assert_eq!(super::batch_take(&mut rem, "a.bin", 5), Some(true));
        assert_eq!(super::batch_take(&mut rem, "a.bin", 5), None);
    }

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

    /// 목록 정렬 키(08-15 · 3차 개정) — 고정 → 접속 계층(세션>발견>오프라인) →
    /// 시각 사슬. ★ 최근 접속은 **분 버킷**: 비컨(800ms)이 ms를 계속 밀어 올려도
    /// 순서가 안 흔들린다(실기 — 갱신마다 순서가 뒤집히던 플리커의 회귀 방지).
    #[test]
    fn peer_order_key_pins_then_tier_then_stable_chain() {
        use super::peer_order_key as k;
        // 고정은 어떤 속성보다 먼저(전 모드).
        assert!(k("chat", true, 2, 0, 0) < k("chat", false, 0, u64::MAX - 1, u64::MAX - 1));
        // 기본(chat) — 계층 먼저, 그 안 ① 최근 대화 ② 최근 접속(사용자 확정).
        assert!(k("chat", false, 0, 0, 0) < k("chat", false, 1, u64::MAX - 1, u64::MAX - 1));
        assert!(k("chat", false, 0, 0, 200) < k("chat", false, 0, 999_999, 100));
        assert!(
            k("chat", false, 2, 0, 200) < k("chat", false, 2, 999_999, 100),
            "비접속 구획도 동일 기준"
        );
        assert!(
            k("chat", false, 0, 180_000, 50) < k("chat", false, 0, 60_000, 50),
            "대화 동률 = 최근 접속이 다음 기준"
        );
        // ★ 최근 접속은 분 버킷 — 같은 버킷 안 ms 차이는 동률(비컨 잡음 → 이름순 안정).
        assert_eq!(
            k("online", false, 1, 120_000, 0),
            k("online", false, 1, 179_999, 0)
        );
        assert!(k("online", false, 1, 180_000, 0) < k("online", false, 1, 119_999, 0));
        // seen — **온라인 여부 무관** 최근 접속순(4모드 확정 08-15).
        assert_eq!(
            k("seen", false, 0, 120_000, 7),
            k("seen", false, 2, 120_000, 7)
        );
        assert!(k("seen", false, 2, 180_000, 0) < k("seen", false, 0, 60_000, 0));
        // online — 계층 먼저, 그 안 최근 접속순.
        assert!(k("online", false, 0, 0, 0) < k("online", false, 1, u64::MAX - 1, 0));
        // 미지 저장값 = 기본(chat) 사슬로 관용 폴백.
        assert_eq!(k("whatever", false, 0, 5, 7), k("chat", false, 0, 5, 7));
        // name 모드 — 고정 외 속성 무시(이름 동률 비교는 호출자 몫).
        assert_eq!(k("name", false, 0, 5, 5), k("name", false, 2, 9, 1));
    }

    /// ★ M3-21 — 프로필 응답 구성이 읽는 **키 전부**가 전파 깔때기에 있어야 한다.
    /// 하나라도 빠지면 그 변경은 이미 연결된 상대에게 영영 안 닿는다(08-14 실기:
    /// 이메일 공유를 켰는데 상대 카드는 "(비공개)" — 그 구멍의 회귀 방지).
    #[test]
    fn every_profile_key_has_a_push_scope() {
        use super::{profile_push_scope, ProfileScope};
        // my_profile_frames_scoped가 읽는 설정 키 전수(빠짐 검사).
        for key in [
            "profile.share.basic",
            "profile.share.email",
            "profile.share.phone",
            "profile.display_name",
            "profile.email",
            "profile.phone",
            "profile.image_path",
            "profile.avatar",
            "profile.avatar_border",
        ] {
            assert!(
                profile_push_scope(key).is_some(),
                "{key} 변경이 전파되지 않는다 — profile_push_scope에 등록할 것"
            );
        }
        // 무게 배정 — 사진이 바뀔 수 있는 키만 Full(청크 동반), 나머지는 경량.
        assert_eq!(
            profile_push_scope("profile.image_path"),
            Some(ProfileScope::Full)
        );
        assert_eq!(
            profile_push_scope("profile.share.basic"),
            Some(ProfileScope::Full)
        );
        assert_eq!(
            profile_push_scope("profile.share.email"),
            Some(ProfileScope::Info)
        );
        // 프로필 밖 키는 전파하지 않는다(무관 변경이 트래픽을 만들면 안 된다).
        assert_eq!(profile_push_scope("ui.theme"), None);
        assert_eq!(profile_push_scope("net.session_port"), None);
    }

    /// 프로필 캐시 메타(08-14 부팅 복원) — 수신 시 쓴 것을 복원이 그대로 읽는다.
    #[test]
    fn profile_meta_round_trip() {
        use super::{encode_profile_meta, parse_profile_meta};
        let s = encode_profile_meta(Some("tiger"), Some((0x12, 0xAB, 0xFF)));
        assert_eq!(
            parse_profile_meta(&s),
            (Some("tiger".into()), Some((0x12, 0xAB, 0xFF)))
        );
        // 부분만 있어도 각자 산다(켠 필드만 오는 프로필과 같은 결).
        assert_eq!(
            parse_profile_meta(&encode_profile_meta(None, Some((1, 2, 3)))),
            (None, Some((1, 2, 3)))
        );
        assert!(
            encode_profile_meta(None, None).is_empty(),
            "빈 메타 = 파일 제거 신호"
        );
    }

    /// 캐시 파일은 사람이 고칠 수 있다 — 복원 경로도 수신 경로와 같은 검증을 태운다
    /// (12간지 밖 키·무효 색·미지 줄은 조용히 버림 = fail-closed·전방 관용).
    #[test]
    fn profile_meta_rejects_tampered_values() {
        use super::parse_profile_meta;
        let (k, b) = parse_profile_meta("avatar=dragon-lord\nborder=#ZZZZZZ\nfuture=1\n");
        assert_eq!(k, None, "12간지 밖 키는 채택하지 않는다");
        assert_eq!(b, None, "무효 색은 채택하지 않는다");
    }

    /// 자동 재연결 백오프(ⓑ 08-13) — 단계별 지연이 늘고, 다 쓰면 **중단**한다
    /// (상시 관찰이 아니라 복구 시도 — 포트 스캔처럼 보이면 안 된다).
    #[test]
    fn pending_direct_codec_roundtrips_and_survives_corruption() {
        use super::{decode_pending, encode_pending, PendingDirect};
        let q = vec![
            PendingDirect {
                text: "안녕 offline".into(),
                at_ms: 1_755_600_000_123,
                importance: 0,
            },
            PendingDirect {
                text: "공지야".into(),
                at_ms: 1_755_600_001_000,
                importance: 1,
            },
        ];
        let enc = encode_pending(&q);
        assert_eq!(decode_pending(&enc), q, "왕복 비트 동일");
        // 꼬리 손상 = 앞선 항목은 살린다(fail-soft — history 디코더와 같은 결).
        let cut = &enc[..enc.len() - 3];
        assert_eq!(decode_pending(cut), q[..1].to_vec());
        assert!(decode_pending(&[]).is_empty());
    }

    #[test]
    fn reconnect_backoff_grows_then_stops() {
        use super::reconnect_delay;
        assert_eq!(reconnect_delay(0), Some(5_000));
        assert_eq!(reconnect_delay(1), Some(15_000));
        assert_eq!(reconnect_delay(2), Some(60_000));
        assert_eq!(reconnect_delay(3), Some(300_000));
        assert_eq!(reconnect_delay(4), None, "상한 도달 = 중단(수동 재개만)");
    }

    /// X-2b — Managed 서버 목표 유도(설정 SSOT · 값 편집은 설정 화면이 이미 관용).
    #[test]
    fn server_target_from_settings() {
        use super::server_target;
        assert_eq!(server_target("unmanaged", "1.2.3.4", "47300"), None);
        assert_eq!(
            server_target("managed", "  ", "47300"),
            None,
            "주소 없음 = 미접속"
        );
        assert_eq!(
            server_target("managed", " relay.example ", "47301"),
            Some("relay.example:47301".into())
        );
        assert_eq!(
            server_target("managed", "relay.example", ""),
            Some("relay.example".into()),
            "포트 생략 = 해석 단계 기본 포트(47300)"
        );
        assert_eq!(
            server_target("managed", "10.0.0.5:47300", "47999"),
            Some("10.0.0.5:47300".into()),
            "주소에 명시한 포트가 설정 포트를 이긴다"
        );
    }

    /// X-2b — 서버 백오프는 300s 상한 **반복**(상대 재연결 ⓑ와 달리 중단 없음:
    /// 서버는 명시 등록한 인프라라 돌아오면 다시 붙어 있어야 한다).
    #[test]
    fn server_retry_ladder_caps_and_repeats() {
        use super::server_retry_delay;
        assert_eq!(server_retry_delay(0), 5_000);
        assert_eq!(server_retry_delay(1), 15_000);
        assert_eq!(server_retry_delay(2), 60_000);
        assert_eq!(server_retry_delay(3), 300_000);
        assert_eq!(server_retry_delay(200), 300_000, "상한 반복 — 중단 없음");
    }

    /// M2-5b — 대화 기록 왕복(텍스트·방향·시각 보존 · Xfer 줄은 제외).
    #[test]
    fn history_roundtrips_text_lines() {
        use super::{decode_history, encode_history, wall_from_ms, ChatLine};
        let w = wall_from_ms(1_700_000_000_000);
        let lines = vec![
            ChatLine::text(true, nbeep_core::sanitize_message("안녕 hello"), 1000, w),
            ChatLine::text(false, nbeep_core::sanitize_message("답장 reply"), 2000, w),
            // 파일 전송 줄 — 직렬화 대상 아님(텍스트만).
            ChatLine::xfer(true, nbeep_core::sanitize_message("a.bin"), 42, 3000, w),
            ChatLine::text(true, nbeep_core::sanitize_message("셋째"), 4000, w),
        ];
        let back = decode_history(&encode_history(&lines));
        assert_eq!(back.len(), 3, "텍스트 3줄만(Xfer 제외)");
        assert!(back[0].mine && back[0].at_ms == 1000);
        assert!(!back[1].mine && back[1].at_ms == 2000);
        assert_eq!(back[2].at_ms, 4000, "Xfer 건너뛰고 순서 보존");
        let nbeep_ui::ChatBody::Text(t0) = &back[0].body else {
            panic!("텍스트")
        };
        assert_eq!(t0.as_str(), "안녕 hello");
    }

    /// ★ 08-19 — 그룹 수신 풍선의 **발신자 라벨(from)** 왕복(tag 3). 라벨 없는
    /// 줄은 종전 tag 1 그대로(1:1 세그먼트 바이트 불변 — 전방 호환).
    #[test]
    fn history_roundtrips_group_from_label() {
        use super::{decode_history, encode_history, wall_from_ms, ChatLine};
        let w = wall_from_ms(1_700_000_000_000);
        let lines = vec![
            ChatLine::text(false, nbeep_core::sanitize_message("ㅎ22222"), 1000, w)
                .with_from("호랭이2"),
            ChatLine::text(true, nbeep_core::sanitize_message("내 말"), 2000, w),
        ];
        let back = decode_history(&encode_history(&lines));
        assert_eq!(back.len(), 2);
        assert_eq!(back[0].from.as_deref(), Some("호랭이2"), "발신자 라벨 보존");
        assert_eq!(back[1].from, None, "라벨 없는 줄은 그대로");
        // 라벨 없는 줄만 있는 인코딩 = 종전과 같은 tag 1 시작(전방 호환 근거).
        let plain = encode_history(&[ChatLine::text(
            true,
            nbeep_core::sanitize_message("x"),
            1,
            w,
        )]);
        assert_eq!(plain[0], 1u8);
    }

    /// M2-5b — 종결 전송(Done/Failed)은 기록되고, 진행 중(Active/Waiting)은 제외.
    #[test]
    fn history_persists_terminal_xfers_only() {
        use super::{decode_history, encode_history, wall_from_ms, ChatLine};
        use nbeep_ui::{ChatBody, XferLineState as St};
        let w = wall_from_ms(0);
        let done = {
            let mut l = ChatLine::xfer(true, nbeep_core::sanitize_message("a.jpg"), 100, 1, w);
            if let ChatBody::Xfer(x) = &mut l.body {
                x.state = St::Done {
                    note: nbeep_core::t(nbeep_core::Msg::XferQuarantined).into(),
                };
            }
            l
        };
        let active = {
            let mut l = ChatLine::xfer(false, nbeep_core::sanitize_message("b.bin"), 200, 2, w);
            if let ChatBody::Xfer(x) = &mut l.body {
                x.state = St::Active { done: 50 };
            }
            l
        };
        let back = decode_history(&encode_history(&[done, active]));
        assert_eq!(back.len(), 1, "종결만 — 진행 중 제외");
        let ChatBody::Xfer(x) = &back[0].body else {
            panic!("xfer")
        };
        assert_eq!(x.name.as_str(), "a.jpg");
        assert_eq!(x.size, 100);
        assert!(matches!(x.state, St::Done { .. }));
    }

    /// M2-5b — 손상은 fail-soft: 읽은 데까지 살리고 멈춘다(빈 봉투 안 만든다).
    #[test]
    fn history_decode_is_fail_soft() {
        use super::{decode_history, encode_history, wall_from_ms, ChatLine};
        let w = wall_from_ms(0);
        let good = encode_history(&[ChatLine::text(
            true,
            nbeep_core::sanitize_message("ok"),
            10,
            w,
        )]);
        let mut truncated = good;
        truncated.push(1); // 잘린 레코드 시작 — 헤더 미만이라 그 앞까지만
        assert_eq!(decode_history(&truncated).len(), 1, "온전한 1줄은 산다");
        assert!(decode_history(b"garbage").is_empty(), "쓰레기 = 빈 결과");
    }

    /// M2-5b — 스레드 상한(오래된 것부터 버림).
    #[test]
    fn history_caps_to_max_keeping_recent() {
        use super::{decode_history, encode_history, wall_from_ms, ChatLine, HISTORY_MAX};
        let w = wall_from_ms(0);
        let lines: Vec<ChatLine> = (0..HISTORY_MAX + 50)
            .map(|i| {
                ChatLine::text(
                    true,
                    nbeep_core::sanitize_message(&format!("m{i}")),
                    i as u64,
                    w,
                )
            })
            .collect();
        let back = decode_history(&encode_history(&lines));
        assert_eq!(back.len(), HISTORY_MAX, "상한 유지");
        assert_eq!(back[0].at_ms, 50, "오래된 50개 버려짐(최근 유지)");
    }
}
