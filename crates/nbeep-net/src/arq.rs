//! ARQ 코어 — 신뢰성 UDP의 **순수 상태기계**(X-UDP-a 확정 ⓐ · 08-21).
//!
//! **결정(X-UDP-a)**: QUIC(quinn)이 아니라 **경량 자작 ARQ**다. quinn은 자체 TLS 1.3이
//! 우리 Noise 세션과 역할이 중복되고(암호화 두 겹 = 예산·복잡도 낭비), 의존 트리가
//! 산출물 수 MB를 먹는다(DR-5 ≤10MB·런타임 의존 0 지향). 우리가 필요한 건
//! "[`Link`](nbeep_core::link::Link) 계약(프레임 보존·순서·신뢰)을 UDP 위에서 만족"이
//! 전부라, 선택적 재전송 + 누적 ACK + 고정 창이면 충분하다.
//!
//! **왜 sans-io인가** — 소켓·시계를 직접 만지지 않고 `now_ms` 주입 + 데이터그램
//! in/out 큐로만 동작한다. 손실·재정렬·중복을 **결정적으로 주입**해 회귀를 박제할 수
//! 있다(이 저장소 공통 규약 — `ime_gate.rs`·`rate.rs`와 같은 이유). 실소켓 셸은
//! [`crate::udplink::UdpLink`].
//!
//! ## 와이어(데이터그램 · 전부 BE)
//!
//! ```text
//! SYN    [magic "NBU1"][1][nonce u32]                       — 동시 열기 허용(홀펀칭)
//! SYNACK [magic][2][내 nonce u32][상대 nonce u32]
//! DATA   [magic][3][conn u32][seq u32][flags u8][payload]    — flags bit0 = 프레임 끝
//! ACK    [magic][4][conn u32][ack u32][sack u32]             — ack = 다음 기대 seq · sack = ack+1.. 비트맵
//! FIN    [magic][5][conn u32][seq u32]                       — seq 슬롯 하나를 소비(순서 보장)
//! ```
//!
//! `conn = nonce_a ^ nonce_b` — 양쪽이 같은 값을 독립 계산한다(동시 열기 대칭).
//! 세그먼트 상한 [`MAX_SEG_PAYLOAD`]는 IPv6 최소 MTU(1280) 안쪽 — 경로 단편화 회피.

use std::collections::VecDeque;

/// 데이터그램 매직(v1).
pub const MAGIC: [u8; 4] = *b"NBU1";

/// 세그먼트 페이로드 상한 — IPv6 최소 MTU 1280 − IP/UDP 헤더(48) − ARQ 헤더(14) 여유.
pub const MAX_SEG_PAYLOAD: usize = 1152;

/// 링크 프레임 상한 — TCP 링크의 `MAX_FRAME`(Noise 상한)과 정합.
pub const MAX_FRAME: usize = 65_535;

/// 송신 창(세그먼트 수) — 64 × 1152B ≈ 72KiB in flight. 혼잡 제어 1차는
/// **고정 창 + RTO 백오프**다(상위에 M4-11 페이싱이 이미 있어 이중 제어를 피한다).
const WINDOW: u32 = 64;

/// 초기/최대 RTO(ms). LAN·릴레이 왕복 어느 쪽이든 250ms 초기값이면 과소 재전송이 없다.
const RTO_INIT_MS: u64 = 250;
const RTO_MAX_MS: u64 = 4_000;

/// SYN/SYNACK 재송 간격(ms).
const SYN_RTO_MS: u64 = 300;

/// ACK 진행 없이 미확인 데이터가 이만큼 방치되면 죽은 경로로 판정(ms).
const DEAD_MS: u64 = 15_000;

/// 유휴 킵얼라이브(ms) — NAT 매핑 유지(홀펀칭 경로). 데이터 없으면 ACK를 재송한다.
const KEEPALIVE_MS: u64 = 25_000;

/// 송신 대기 상한(바이트) — 이 위로는 [`ArqError::WouldBlock`](셸이 ACK를 기다린다).
const SEND_BUF_CAP: usize = 512 * 1024;

const K_SYN: u8 = 1;
const K_SYNACK: u8 = 2;
const K_DATA: u8 = 3;
const K_ACK: u8 = 4;
const K_FIN: u8 = 5;

const F_FRAME_END: u8 = 0b1;

/// [`ArqCore`] 오류.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ArqError {
    /// 프레임이 [`MAX_FRAME`] 초과.
    TooLarge,
    /// 연결이 끝났다(FIN·죽은 경로).
    Closed,
    /// 송신 버퍼 가득 — ACK가 돌아올 때까지 기다렸다 다시 넣어야 한다.
    WouldBlock,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum State {
    /// SYN을 쏘는 중(상대 nonce 미확보).
    SynSent,
    /// conn 확정(데이터 송수신 가능). `confirmed`가 false면 SYNACK 재송 중.
    Established,
    /// 죽은 경로(재전송 소진) 또는 프로토콜 위반.
    Dead,
}

/// 송신 중 세그먼트.
#[derive(Debug)]
struct InFlight {
    seq: u32,
    /// K_DATA 페이로드(flags 포함 재구성용) — FIN은 `None`.
    payload: Option<(u8, Vec<u8>)>,
    sent_at: u64,
    /// 재전송 여부(Karn — RTT 표본 제외).
    rexmit: bool,
    /// SACK로 도착 확인됨(재전송 불필요 · 누적 ack 전진 대기).
    sacked: bool,
}

/// 신뢰성 UDP의 순수 상태기계. 소켓·시계를 모른다 — 셸이 `now_ms`와 데이터그램을 나른다.
#[derive(Debug)]
pub struct ArqCore {
    state: State,
    nonce: u32,
    peer_nonce: Option<u32>,
    conn: u32,
    /// 상대가 내 nonce를 알았다는 증거(conn 스탬프 패킷 수신)를 봤는가.
    confirmed: bool,

    // ── 송신 ──
    next_seq: u32,
    inflight: VecDeque<InFlight>,
    /// 창 밖 대기 세그먼트(flags, payload).
    send_queue: VecDeque<(u8, Vec<u8>)>,
    queued_bytes: usize,
    srtt_ms: Option<u64>,
    rttvar_ms: u64,
    rto_ms: u64,
    rto_deadline: Option<u64>,
    /// 마지막 누적 ack 전진 시각 — 죽은 경로 판정의 기준.
    last_progress: u64,
    fin_queued: bool,

    // ── 수신 ──
    recv_next: u32,
    /// 순서 밖 세그먼트: (seq → (flags, payload)). FIN은 payload 없이 flags bit7로 구분.
    ooo: std::collections::BTreeMap<u32, (u8, Option<Vec<u8>>)>,
    frame_buf: Vec<u8>,
    ready: VecDeque<Vec<u8>>,
    ack_pending: bool,
    /// 상대 FIN이 순서상 도달함 — ready를 다 비우면 Closed.
    peer_fin: bool,
    fin_seq_sent: Option<u32>,

    // ── 타이머 ──
    hs_deadline: u64,
    last_tx: u64,

    out: VecDeque<Vec<u8>>,
}

/// FIN을 ooo 맵에 구분 저장하기 위한 내부 플래그(와이어에 나가지 않는다).
const F_INTERNAL_FIN: u8 = 0b1000_0000;

impl ArqCore {
    /// 새 연결 시도(동시 열기 허용 — 양쪽이 똑같이 `new`로 시작해도 성립한다).
    /// `nonce`는 셸이 주는 무작위 값(자기 예측 불가면 충분 — 보안은 위층 Noise 소관).
    #[must_use]
    pub fn new(nonce: u32, now_ms: u64) -> Self {
        Self {
            state: State::SynSent,
            nonce,
            peer_nonce: None,
            conn: 0,
            confirmed: false,
            next_seq: 0,
            inflight: VecDeque::new(),
            send_queue: VecDeque::new(),
            queued_bytes: 0,
            srtt_ms: None,
            rttvar_ms: 0,
            rto_ms: RTO_INIT_MS,
            rto_deadline: None,
            last_progress: now_ms,
            fin_queued: false,
            recv_next: 0,
            ooo: std::collections::BTreeMap::new(),
            frame_buf: Vec::new(),
            ready: VecDeque::new(),
            ack_pending: false,
            peer_fin: false,
            fin_seq_sent: None,
            hs_deadline: now_ms, // 즉시 첫 SYN
            last_tx: now_ms,
            out: VecDeque::new(),
        }
    }

    /// conn 확정 후 데이터가 오갈 수 있는가.
    #[must_use]
    pub fn is_established(&self) -> bool {
        self.state == State::Established
    }

    /// 더 이상 어떤 데이터도 오가지 않는다(죽음·상대 FIN 소진).
    #[must_use]
    pub fn is_terminated(&self) -> bool {
        self.state == State::Dead || (self.peer_fin && self.ready.is_empty())
    }

    /// 송신 대기 총량(창 밖 큐 + in flight) — 셸의 배압 판단용.
    #[must_use]
    pub fn pending_bytes(&self) -> usize {
        self.queued_bytes
    }

    /// 아직 창에 못 들어가 1차 송신조차 안 된 세그먼트 수 — 셸의 `send`가 이걸 0으로
    /// 만들 때까지 IO를 구동한다(송신 후 아무도 recv를 안 불러도 프레임이 나가게).
    #[must_use]
    pub fn unpumped(&self) -> usize {
        self.send_queue.len() + usize::from(self.fin_queued && self.fin_seq_sent.is_none())
    }

    /// 링크 프레임 하나를 송신 큐에 넣는다(세그먼트 분할 포함).
    ///
    /// # Errors
    /// [`ArqError::TooLarge`]·[`ArqError::Closed`]·버퍼 가득이면 [`ArqError::WouldBlock`].
    pub fn push_frame(&mut self, frame: &[u8], now_ms: u64) -> Result<(), ArqError> {
        if self.state == State::Dead || self.fin_queued || self.peer_fin {
            return Err(ArqError::Closed);
        }
        if frame.len() > MAX_FRAME {
            return Err(ArqError::TooLarge);
        }
        if self.queued_bytes + frame.len() > SEND_BUF_CAP {
            return Err(ArqError::WouldBlock);
        }
        // 분할: 마지막 조각에만 FRAME_END. 빈 프레임도 세그먼트 하나(END)로 나간다.
        let mut rest = frame;
        loop {
            let take = rest.len().min(MAX_SEG_PAYLOAD);
            let (chunk, tail) = rest.split_at(take);
            let flags = if tail.is_empty() { F_FRAME_END } else { 0 };
            self.queued_bytes += chunk.len();
            self.send_queue.push_back((flags, chunk.to_vec()));
            if tail.is_empty() {
                break;
            }
            rest = tail;
        }
        self.pump_window(now_ms);
        Ok(())
    }

    /// 우아한 종료 — 남은 큐 뒤에 FIN을 붙인다(이후 `push_frame`은 `Closed`).
    pub fn close(&mut self, now_ms: u64) {
        if self.state == State::Dead || self.fin_queued {
            return;
        }
        self.fin_queued = true;
        self.pump_window(now_ms);
    }

    /// 완성된 수신 프레임 하나.
    pub fn take_frame(&mut self) -> Option<Vec<u8>> {
        self.ready.pop_front()
    }

    /// 내보낼 데이터그램 하나(셸이 소켓으로 쏜다).
    pub fn take_outgoing(&mut self) -> Option<Vec<u8>> {
        self.out.pop_front()
    }

    /// 다음 타이머 만기까지 남은 시간(ms) — 셸의 소켓 대기 상한 힌트.
    #[must_use]
    pub fn next_wake_in(&self, now_ms: u64) -> u64 {
        let mut next = u64::MAX;
        if self.state == State::SynSent || !self.confirmed {
            next = next.min(self.hs_deadline.saturating_sub(now_ms));
        }
        if let Some(d) = self.rto_deadline {
            next = next.min(d.saturating_sub(now_ms));
        }
        if self.state == State::Established {
            next = next.min((self.last_tx + KEEPALIVE_MS).saturating_sub(now_ms));
        }
        next.min(1_000)
    }

    /// 타이머 구동 — 핸드셰이크 재송·재전송·킵얼라이브·죽음 판정. 셸이 주기적으로 부른다.
    pub fn poll(&mut self, now_ms: u64) {
        match self.state {
            State::Dead => return,
            State::SynSent => {
                if now_ms >= self.hs_deadline {
                    self.emit_syn(now_ms);
                    self.hs_deadline = now_ms + SYN_RTO_MS;
                }
            }
            State::Established => {
                if !self.confirmed && now_ms >= self.hs_deadline {
                    self.emit_synack(now_ms);
                    self.hs_deadline = now_ms + SYN_RTO_MS;
                }
                // 재전송: 만기 시 **첫 미확인(unsacked) 세그먼트 하나**만 다시 보내고
                // RTO를 지수 백오프한다(버스트 재전송은 혼잡을 키운다).
                if let Some(deadline) = self.rto_deadline {
                    if now_ms >= deadline {
                        if now_ms.saturating_sub(self.last_progress) >= DEAD_MS {
                            self.state = State::Dead; // 진행 없는 재전송 소진 = 죽은 경로
                            return;
                        }
                        self.retransmit_first(now_ms);
                        self.rto_ms = (self.rto_ms * 2).min(RTO_MAX_MS);
                        self.rto_deadline = Some(now_ms + self.rto_ms);
                    }
                }
                if self.ack_pending {
                    self.emit_ack(now_ms);
                }
                // 유휴 킵얼라이브 — NAT 매핑 유지(펀칭 경로). ACK 재송이면 충분하다.
                if now_ms.saturating_sub(self.last_tx) >= KEEPALIVE_MS {
                    self.emit_ack(now_ms);
                }
            }
        }
        if self.ack_pending {
            self.emit_ack(now_ms);
        }
    }

    /// 수신 데이터그램 처리. 매직·conn 불일치는 조용히 버린다(스트레이·다른 프로토콜).
    pub fn handle_datagram(&mut self, pkt: &[u8], now_ms: u64) {
        if self.state == State::Dead || pkt.len() < 9 || pkt[..4] != MAGIC {
            return;
        }
        let kind = pkt[4];
        let word = u32::from_be_bytes([pkt[5], pkt[6], pkt[7], pkt[8]]);
        match kind {
            K_SYN => self.on_syn(word, now_ms),
            K_SYNACK => {
                if pkt.len() < 13 {
                    return;
                }
                let echoed = u32::from_be_bytes([pkt[9], pkt[10], pkt[11], pkt[12]]);
                self.on_synack(word, echoed, now_ms);
            }
            K_DATA | K_ACK | K_FIN => {
                if self.state != State::Established || word != self.conn {
                    return;
                }
                self.confirmed = true; // conn 스탬프 = 상대가 내 nonce를 안다
                match kind {
                    K_DATA => self.on_data(pkt, now_ms),
                    K_ACK => self.on_ack(pkt, now_ms),
                    K_FIN => self.on_fin(pkt),
                    _ => unreachable!(),
                }
            }
            _ => {}
        }
    }

    // ── 핸드셰이크 ──

    fn on_syn(&mut self, peer_nonce: u32, now_ms: u64) {
        // 이미 아는 상대의 SYN 재송이면 SYNACK 재송. 다른 nonce는 무시(MVP — 재연결은 새 소켓).
        match self.peer_nonce {
            Some(n) if n != peer_nonce => return,
            _ => {}
        }
        self.peer_nonce = Some(peer_nonce);
        self.conn = self.nonce ^ peer_nonce;
        if self.state == State::SynSent {
            self.state = State::Established;
            self.last_progress = now_ms;
        }
        self.emit_synack(now_ms);
        self.hs_deadline = now_ms + SYN_RTO_MS;
        self.pump_window(now_ms); // 성립 전 큐된 프레임을 즉시 흘린다(동시 열기·응답자 경로)
    }

    fn on_synack(&mut self, peer_nonce: u32, echoed: u32, now_ms: u64) {
        if echoed != self.nonce {
            return; // 내게 온 응답이 아니다
        }
        match self.peer_nonce {
            Some(n) if n != peer_nonce => return,
            _ => {}
        }
        self.peer_nonce = Some(peer_nonce);
        self.conn = self.nonce ^ peer_nonce;
        if self.state == State::SynSent {
            self.state = State::Established;
            self.last_progress = now_ms;
        }
        self.confirmed = true; // 상대가 내 nonce를 되울렸다
        self.ack_pending = true; // 상대의 SYNACK 재송을 멈추게 즉시 확인
        self.pump_window(now_ms);
    }

    // ── 수신 경로 ──

    fn on_data(&mut self, pkt: &[u8], now_ms: u64) {
        if pkt.len() < 14 {
            return;
        }
        let seq = u32::from_be_bytes([pkt[9], pkt[10], pkt[11], pkt[12]]);
        let flags = pkt[13];
        let payload = &pkt[14..];
        self.ack_pending = true; // 중복이어도 ACK(상대의 재전송을 멈춘다)
        if seq.wrapping_sub(self.recv_next) >= WINDOW * 4 {
            return; // 과거(이미 소비) 또는 터무니없는 미래 — 버림
        }
        self.ooo.insert(seq, (flags, Some(payload.to_vec())));
        self.drain_in_order();
        let _ = now_ms;
    }

    fn on_fin(&mut self, pkt: &[u8]) {
        if pkt.len() < 13 {
            return;
        }
        let seq = u32::from_be_bytes([pkt[9], pkt[10], pkt[11], pkt[12]]);
        self.ack_pending = true;
        if seq.wrapping_sub(self.recv_next) >= WINDOW * 4 {
            return;
        }
        self.ooo.insert(seq, (F_INTERNAL_FIN, None));
        self.drain_in_order();
    }

    /// 순서 도달분을 프레임으로 조립한다.
    fn drain_in_order(&mut self) {
        while let Some((flags, payload)) = self.ooo.remove(&self.recv_next) {
            self.recv_next = self.recv_next.wrapping_add(1);
            if flags & F_INTERNAL_FIN != 0 {
                self.peer_fin = true;
                self.ooo.clear(); // FIN 뒤 데이터는 없다(송신이 순서를 보장)
                return;
            }
            if let Some(bytes) = payload {
                if self.frame_buf.len() + bytes.len() > MAX_FRAME {
                    self.state = State::Dead; // 프레임 상한 위반 = 프로토콜 오류(fail-closed)
                    return;
                }
                self.frame_buf.extend_from_slice(&bytes);
            }
            if flags & F_FRAME_END != 0 {
                self.ready.push_back(core::mem::take(&mut self.frame_buf));
            }
        }
    }

    // ── 송신 경로 ──

    fn on_ack(&mut self, pkt: &[u8], now_ms: u64) {
        if pkt.len() < 17 {
            return;
        }
        let ack = u32::from_be_bytes([pkt[9], pkt[10], pkt[11], pkt[12]]);
        let sack = u32::from_be_bytes([pkt[13], pkt[14], pkt[15], pkt[16]]);
        let mut progressed = false;
        while let Some(front) = self.inflight.front() {
            // ack = 다음 기대 seq — 그보다 앞(seq < ack)은 전부 도착.
            if front.seq.wrapping_sub(ack) < WINDOW * 4 {
                break; // front.seq >= ack (랩 고려)
            }
            let seg = self.inflight.pop_front().expect("front 확인됨");
            if !seg.rexmit {
                self.update_rtt(now_ms.saturating_sub(seg.sent_at));
            }
            if let Some((_, p)) = seg.payload {
                self.queued_bytes = self.queued_bytes.saturating_sub(p.len());
            }
            progressed = true;
        }
        // SACK: ack+1+i 도착 표시 — 재전송 대상에서 제외.
        for seg in &mut self.inflight {
            let off = seg.seq.wrapping_sub(ack);
            if (1..=32).contains(&off) && (sack >> (off - 1)) & 1 == 1 && !seg.sacked {
                seg.sacked = true;
                progressed = true;
            }
        }
        if progressed {
            self.last_progress = now_ms;
            self.rto_ms = self.current_rto();
            self.rto_deadline = if self.inflight.is_empty() {
                None
            } else {
                Some(now_ms + self.rto_ms)
            };
        }
        self.pump_window(now_ms);
    }

    fn update_rtt(&mut self, sample_ms: u64) {
        // RFC 6298 축약 — srtt/rttvar 지수 이동 평균.
        match self.srtt_ms {
            None => {
                self.srtt_ms = Some(sample_ms);
                self.rttvar_ms = sample_ms / 2;
            }
            Some(srtt) => {
                let diff = srtt.abs_diff(sample_ms);
                self.rttvar_ms = (self.rttvar_ms * 3 + diff) / 4;
                self.srtt_ms = Some((srtt * 7 + sample_ms) / 8);
            }
        }
    }

    fn current_rto(&self) -> u64 {
        match self.srtt_ms {
            None => RTO_INIT_MS,
            Some(srtt) => (srtt + (4 * self.rttvar_ms).max(10)).clamp(RTO_INIT_MS, RTO_MAX_MS),
        }
    }

    /// 창이 허용하는 만큼 대기 큐를 in flight로 옮겨 송신한다. FIN은 큐가 빈 뒤에.
    fn pump_window(&mut self, now_ms: u64) {
        if self.state != State::Established {
            return;
        }
        while (self.inflight.len() as u32) < WINDOW {
            if let Some((flags, payload)) = self.send_queue.pop_front() {
                let seq = self.next_seq;
                self.next_seq = self.next_seq.wrapping_add(1);
                self.emit_data(seq, flags, &payload, now_ms);
                self.inflight.push_back(InFlight {
                    seq,
                    payload: Some((flags, payload)),
                    sent_at: now_ms,
                    rexmit: false,
                    sacked: false,
                });
            } else if self.fin_queued && self.fin_seq_sent.is_none() {
                let seq = self.next_seq;
                self.next_seq = self.next_seq.wrapping_add(1);
                self.fin_seq_sent = Some(seq);
                self.emit_fin(seq, now_ms);
                self.inflight.push_back(InFlight {
                    seq,
                    payload: None,
                    sent_at: now_ms,
                    rexmit: false,
                    sacked: false,
                });
            } else {
                break;
            }
            if self.rto_deadline.is_none() {
                self.rto_deadline = Some(now_ms + self.rto_ms);
            }
        }
    }

    fn retransmit_first(&mut self, now_ms: u64) {
        type SegInfo = (u32, Option<(u8, Vec<u8>)>); // seq + (flags, payload) — FIN은 None
        let mut seg_info: Option<SegInfo> = None;
        for seg in &mut self.inflight {
            if !seg.sacked {
                seg.rexmit = true;
                seg.sent_at = now_ms;
                seg_info = Some((seg.seq, seg.payload.clone()));
                break;
            }
        }
        match seg_info {
            Some((seq, Some((flags, payload)))) => self.emit_data(seq, flags, &payload, now_ms),
            Some((seq, None)) => self.emit_fin(seq, now_ms),
            None => {}
        }
    }

    // ── 인코더 ──

    fn emit_syn(&mut self, now_ms: u64) {
        let mut p = Vec::with_capacity(9);
        p.extend_from_slice(&MAGIC);
        p.push(K_SYN);
        p.extend_from_slice(&self.nonce.to_be_bytes());
        self.out.push_back(p);
        self.last_tx = now_ms;
    }

    fn emit_synack(&mut self, now_ms: u64) {
        let Some(peer) = self.peer_nonce else { return };
        let mut p = Vec::with_capacity(13);
        p.extend_from_slice(&MAGIC);
        p.push(K_SYNACK);
        p.extend_from_slice(&self.nonce.to_be_bytes());
        p.extend_from_slice(&peer.to_be_bytes());
        self.out.push_back(p);
        self.last_tx = now_ms;
    }

    fn emit_data(&mut self, seq: u32, flags: u8, payload: &[u8], now_ms: u64) {
        let mut p = Vec::with_capacity(14 + payload.len());
        p.extend_from_slice(&MAGIC);
        p.push(K_DATA);
        p.extend_from_slice(&self.conn.to_be_bytes());
        p.extend_from_slice(&seq.to_be_bytes());
        p.push(flags);
        p.extend_from_slice(payload);
        self.out.push_back(p);
        self.last_tx = now_ms;
    }

    fn emit_fin(&mut self, seq: u32, now_ms: u64) {
        let mut p = Vec::with_capacity(13);
        p.extend_from_slice(&MAGIC);
        p.push(K_FIN);
        p.extend_from_slice(&self.conn.to_be_bytes());
        p.extend_from_slice(&seq.to_be_bytes());
        self.out.push_back(p);
        self.last_tx = now_ms;
    }

    fn emit_ack(&mut self, now_ms: u64) {
        self.ack_pending = false;
        let mut sack = 0u32;
        for &seq in self.ooo.keys() {
            let off = seq.wrapping_sub(self.recv_next);
            if (1..=32).contains(&off) {
                sack |= 1 << (off - 1);
            }
        }
        let mut p = Vec::with_capacity(17);
        p.extend_from_slice(&MAGIC);
        p.push(K_ACK);
        p.extend_from_slice(&self.conn.to_be_bytes());
        p.extend_from_slice(&self.recv_next.to_be_bytes());
        p.extend_from_slice(&sack.to_be_bytes());
        self.out.push_back(p);
        self.last_tx = now_ms;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 결정적 유사 난수(LCG) — 손실·재정렬 주입의 재현성.
    struct Lcg(u64);
    impl Lcg {
        fn next(&mut self) -> u64 {
            self.0 = self
                .0
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            self.0 >> 33
        }
        /// 0..1000 미만 값이 `permille`보다 작으면 true.
        fn chance(&mut self, permille: u64) -> bool {
            self.next() % 1000 < permille
        }
    }

    /// 가상 망 — 양방향 큐에 손실·중복·재정렬을 결정적으로 주입하며 두 코어를 돌린다.
    fn run_network(
        a: &mut ArqCore,
        b: &mut ArqCore,
        seed: u64,
        loss_permille: u64,
        dup_permille: u64,
        reorder: bool,
        ticks: u64,
    ) -> (Vec<Vec<u8>>, Vec<Vec<u8>>) {
        let mut rng = Lcg(seed);
        let mut got_a = Vec::new();
        let mut got_b = Vec::new();
        let mut wire_ab: Vec<Vec<u8>> = Vec::new();
        let mut wire_ba: Vec<Vec<u8>> = Vec::new();
        for tick in 0..ticks {
            let now = tick * 10; // 10ms 틱
            a.poll(now);
            b.poll(now);
            while let Some(p) = a.take_outgoing() {
                if !rng.chance(loss_permille) {
                    if rng.chance(dup_permille) {
                        wire_ab.push(p.clone());
                    }
                    wire_ab.push(p);
                }
            }
            while let Some(p) = b.take_outgoing() {
                if !rng.chance(loss_permille) {
                    if rng.chance(dup_permille) {
                        wire_ba.push(p.clone());
                    }
                    wire_ba.push(p);
                }
            }
            if reorder && wire_ab.len() >= 2 && rng.chance(300) {
                let n = wire_ab.len();
                wire_ab.swap(n - 1, n - 2);
            }
            if reorder && wire_ba.len() >= 2 && rng.chance(300) {
                let n = wire_ba.len();
                wire_ba.swap(n - 1, n - 2);
            }
            for p in wire_ab.drain(..) {
                b.handle_datagram(&p, now);
            }
            for p in wire_ba.drain(..) {
                a.handle_datagram(&p, now);
            }
            while let Some(f) = a.take_frame() {
                got_a.push(f);
            }
            while let Some(f) = b.take_frame() {
                got_b.push(f);
            }
        }
        (got_a, got_b)
    }

    #[test]
    fn establishes_and_roundtrips_clean() {
        let mut a = ArqCore::new(0x1111, 0);
        let mut b = ArqCore::new(0x2222, 0);
        a.push_frame(b"hello", 0).unwrap();
        // 미성립 상태에서 push해도 성립 후 흘러야 한다(핸드셰이크 대기 큐).
        let (_, got_b) = run_network(&mut a, &mut b, 1, 0, 0, false, 200);
        assert!(a.is_established() && b.is_established());
        assert_eq!(got_b, vec![b"hello".to_vec()]);
    }

    #[test]
    fn simultaneous_open_converges_to_same_conn() {
        let mut a = ArqCore::new(7, 0);
        let mut b = ArqCore::new(9, 0);
        // 동시 열기: 양쪽 다 SYN부터 — 홀펀칭의 전제.
        let (_, _) = run_network(&mut a, &mut b, 2, 0, 0, false, 50);
        assert!(a.is_established() && b.is_established());
        assert_eq!(a.conn, b.conn, "conn = nonce XOR — 대칭 계산");
    }

    #[test]
    fn big_frame_survives_segmentation() {
        let mut a = ArqCore::new(1, 0);
        let mut b = ArqCore::new(2, 0);
        let big: Vec<u8> = (0..60_000u32).map(|i| (i % 251) as u8).collect();
        a.push_frame(&big, 0).unwrap();
        let (_, got_b) = run_network(&mut a, &mut b, 3, 0, 0, false, 2_000);
        assert_eq!(got_b.len(), 1);
        assert_eq!(
            got_b[0], big,
            "60KB 프레임이 세그먼트 분할·조립 후 비트 동일"
        );
    }

    #[test]
    fn survives_loss_reorder_dup() {
        let mut a = ArqCore::new(3, 0);
        let mut b = ArqCore::new(4, 0);
        let frames: Vec<Vec<u8>> = (0..20u8).map(|i| vec![i; (i as usize + 1) * 100]).collect();
        for f in &frames {
            a.push_frame(f, 0).unwrap();
        }
        // 15% 손실 + 5% 중복 + 재정렬 — 그래도 순서·완전성 보존이 계약이다.
        let (_, got_b) = run_network(&mut a, &mut b, 42, 150, 50, true, 20_000);
        assert_eq!(got_b, frames, "손실·재정렬·중복 아래에서도 순서·완전성");
    }

    #[test]
    fn bidirectional_under_loss() {
        let mut a = ArqCore::new(5, 0);
        let mut b = ArqCore::new(6, 0);
        for i in 0..10u8 {
            a.push_frame(&[i; 500], 0).unwrap();
            b.push_frame(&[100 + i; 700], 0).unwrap();
        }
        let (got_a, got_b) = run_network(&mut a, &mut b, 77, 100, 0, true, 20_000);
        assert_eq!(got_b.len(), 10);
        assert_eq!(got_a.len(), 10);
        assert_eq!(got_a[9], vec![109u8; 700]);
    }

    #[test]
    fn fin_closes_after_drain() {
        let mut a = ArqCore::new(11, 0);
        let mut b = ArqCore::new(12, 0);
        a.push_frame(b"last words", 0).unwrap();
        a.close(0);
        let (_, got_b) = run_network(&mut a, &mut b, 8, 0, 0, false, 300);
        assert_eq!(
            got_b,
            vec![b"last words".to_vec()],
            "FIN 전 데이터는 전부 배달"
        );
        assert!(b.is_terminated(), "FIN 도달 + ready 소진 = 종료");
        assert_eq!(b.push_frame(b"x", 0), Err(ArqError::Closed));
    }

    #[test]
    fn dead_path_detected_without_acks() {
        let mut a = ArqCore::new(21, 0);
        let mut b = ArqCore::new(22, 0);
        // 성립까지만 통신시키고, 이후 b를 절단(패킷 전달 없음).
        let (_, _) = run_network(&mut a, &mut b, 9, 0, 0, false, 30);
        assert!(a.is_established());
        a.push_frame(b"into the void", 300).unwrap();
        let mut now = 300;
        while now < 300 + DEAD_MS + RTO_MAX_MS * 2 {
            a.poll(now);
            while a.take_outgoing().is_some() {} // 전부 유실
            now += 100;
        }
        assert!(
            a.is_terminated(),
            "ACK 진행 없음 {DEAD_MS}ms = 죽은 경로 판정"
        );
    }

    #[test]
    fn send_buffer_backpressure() {
        let mut a = ArqCore::new(31, 0);
        let chunk = vec![0u8; 60_000];
        let mut pushed = 0;
        loop {
            match a.push_frame(&chunk, 0) {
                Ok(()) => pushed += 1,
                Err(ArqError::WouldBlock) => break,
                Err(e) => panic!("예상 밖 오류 {e:?}"),
            }
            assert!(pushed < 100, "상한 없는 큐 = 메모리 무한");
        }
        assert!(a.pending_bytes() <= SEND_BUF_CAP);
    }

    #[test]
    fn stray_and_garbage_datagrams_ignored() {
        let mut a = ArqCore::new(41, 0);
        let mut b = ArqCore::new(42, 0);
        let (_, _) = run_network(&mut a, &mut b, 10, 0, 0, false, 30);
        a.handle_datagram(b"garbage", 100);
        a.handle_datagram(&[0u8; 64], 100);
        let mut wrong_conn = Vec::new();
        wrong_conn.extend_from_slice(&MAGIC);
        wrong_conn.push(K_DATA);
        wrong_conn.extend_from_slice(&0xdead_beefu32.to_be_bytes());
        wrong_conn.extend_from_slice(&0u32.to_be_bytes());
        wrong_conn.push(F_FRAME_END);
        wrong_conn.extend_from_slice(b"evil");
        a.handle_datagram(&wrong_conn, 100);
        assert!(a.take_frame().is_none(), "conn 불일치 데이터는 버려진다");
        assert!(a.is_established());
    }

    #[test]
    fn empty_frame_preserved() {
        let mut a = ArqCore::new(51, 0);
        let mut b = ArqCore::new(52, 0);
        a.push_frame(b"", 0).unwrap();
        a.push_frame(b"after", 0).unwrap();
        let (_, got_b) = run_network(&mut a, &mut b, 11, 0, 0, false, 100);
        assert_eq!(
            got_b,
            vec![Vec::new(), b"after".to_vec()],
            "빈 프레임도 경계 보존"
        );
    }
}
