# 55. 세션 액터 송수신 분리 — 메시지 왕복 지연 처방 ✅ (LAT-1 · 2026-11-04)

> 상태: ✅ **구현 완료** `b4d723f`(11-04 · 루프백 p95 210 → **5ms** · 게이트 ≤50ms ✓ · 루프백 목표 ≤10ms ✓ · 결과 = §4 · 설계 대비 변경 = 2-5) · 잔여 = 실 LAN 2-PC 실측(지금 수치는 Win Debug 루프백).
> 내용 결정 = 개발 세션(원문 = 개발 세션 스크래치패드 `lat1-design.md`) · 작성 = 협업 세션([53 §3](53-collab-session-operation.md)).
> 관련: [49 F-1](49-inspection-framework.md)(메시지 왕복 지연 ≤50ms P95 · NFR-B-9) · [05 NFR-B-9](05-requirements.md) · [13 §2-4 규칙 7](13-code-design-standards.md)(DR-21 — 외부 타입은 포트 뒤) · TODO **LAT-1** · INS-6 `9849c70`.

## 0. 배경 — INS-6 첫 실측

메시지를 보낸 뒤 상대의 Delivered ack가 돌아오기까지의 시간을 처음 쟀다(INS-6 `NEXA_ACK_TRACE` + `tools/ack-bench.sh`).

| 조건 | n | p50 | p95 | max | ack 못 받음 | 게이트 |
|---|---|---|---|---|---|---|
| Win Debug · 루프백 · 100ms 간격 100줄 | 100 | 158ms | **210ms** | 224ms | 0 | ≤50ms P95 ✗ |

루프백인데도 왕복이 수백 ms다. 네트워크가 아니라 앱 안에서 기다린다는 뜻이다.

## 1. 원인 — 액터 1스레드가 수신 대기에 묶여 있다

세션 액터는 스레드 하나로 아래를 돈다.

```
loop {
    session.recv()        // 유휴 100ms 타임아웃 · 파일 발신 중 1ms
    → 프레임 처리
    out_rx 명령 drain     // 메인 스레드가 넣은 Chat·ack 프레임
    파일 펌프
}
```

메인 스레드가 큐에 넣은 Chat·ack 프레임은 **지금 걸린 recv 타임아웃이 끝나야** 나간다.

- 발신 쪽 대기 = U(0,100)ms
- 수신 쪽이 ack를 내보내기까지 대기 = U(0,100)ms
- 합 = 평균 ≈100ms + 메인 스레드 홉 → 실측 p50 158 · p95 210ms와 맞는다.

파일 전송 중에는 recv 타임아웃이 1ms라 이 지연이 없다. RL-12 수신 펌프 교대 설계가 유휴 때 치르는 대가다.

## 2. 처방 A(권고) — 읽기/쓰기 분리

### 2-1. nbeep-crypto — `NoiseSession::split`

- `NoiseSession` 전송 상태를 stateless로 바꾸고 `split_halves`로 `NoiseRecv` / `NoiseSend`를 얻는다(nonce는 우리가 센다 · 단일 소유 동작은 종전과 같다).
- snow 0.9.6 `HandshakeState::into_stateless_transport_mode()`를 쓴다. `StatelessTransportState`는 read/write가 `&self`이고 nonce를 호출자가 명시한다.
- 핸드셰이크 뒤 `into_transport_mode()` 대신 stateless로 들어가 `Arc<StatelessTransportState>`를 두 반쪽이 공유한다.
- **NoiseRecv** = 링크 rx + recv nonce(잠금 없음).
- **NoiseSend** = 복제 링크 tx + send nonce를 **한 Mutex** 안에 둔다.
- ★**불변식: 암호화와 send를 같은 잠금 안에서 한다.** 와이어 순서 = nonce 순서. 펌프와 쓰기 스레드가 같은 `NoiseSend`를 공유해도 순서가 깨지지 않는다.
- snow 타입은 nbeep-crypto 안에 머문다(DR-21 · 포트 시그니처에 나오지 않는다).

### 2-2. nbeep-core — 포트 `Link::split_send`

- `Link::split_send(&mut self) -> Option<Box<dyn Link>>`(초안의 `split(self: Box<Self>)`에서 변경)
- session.rs = `RecvHalf`·`SendHalf`·`Splittable` + `Halves`.
- **기본 구현 = `None`** → 종전 단일 루프로 폴백한다. 분리를 못 하는 링크도 지금처럼 돈다.
- `MuxSession::split` → `MuxRecv`(`recv_any` · 큐 승계) / `MuxSend`(`&self` · `Arc` 공유).
- `TrustedSession`은 분리를 위임한다.

### 2-3. nbeep-net — 링크별 구현

| 링크 | `split_send` |
|---|---|
| `TcpLink` | `TcpStream::try_clone` · 수신 타임아웃 SO_RCVTIMEO는 rx 핸들에만 |
| `InMemoryLink` | Sender 복제(분리됨) |
| `UdpLink` · relay | `None`(폴백 · v1 범위 밖 DR-8) |

### 2-4. nexa-beep — 액터

세션 성립 뒤 분리가 성공하면 두 갈래로 나눈다. `ActorIo{Split, Whole}`이 둘을 감춘다. 액터 본문은 `send`·`recv_any`·`set_recv_timeout` 세 메서드만 본다.

| 갈래 | 하는 일 |
|---|---|
| **읽기 루프** | recv 폴 유지(유휴 100ms · 전송 중 1ms — 2-5) · 프레임 처리 · 상태 명령은 `actor_rx`로 받는다 · 파일 펌프는 여기 남고 같은 `MuxSend`로 청크를 보낸다(잠금으로 순서 보장) |
| **쓰기 스레드**(`spawn_session_writer`) | `out_rx` 소비 · Chat/Control/Group = **즉시** `MuxSend` · 파일 상태 명령(Offer·Accept·Cancel·Pause·Resume·Reject·SetRecvMax) = `actor_tx`로 읽기 루프에 넘긴다 |

분리가 `None`이면 `ActorIo::Whole`로 종전 루프 그대로다.

종료 경로 = `out_tx` drop → 쓰기 스레드 종료 → `actor_tx` drop → 읽기 루프 Disconnected(종전 "대화 닫힘" 경로 그대로).

송신 순서 확인 = Control 표식·manifest는 메인이 OfferFile보다 먼저 보내고 즉시 경로가 더 빠르다. "표식 → 목록 → 오퍼" 순서는 보존된다.

쓰기 스레드의 송신 실패는 내부 명령 `SessionCmd::WriterFailed`로 액터에 넘긴다. Closed는 액터 한 곳이 낸다. 늦은 Closed가 재수립된 대화를 대피시키는 경합을 막기 위해서다.

무변경: RL-13 인바운드 핸드셰이크 타임아웃 · RL-9 펌프 예산(PERF-9 20ms·64청크 포함) · netmon 계측.

### 2-5. 구현 착수 뒤 변경 — 읽기 루프의 폴은 유지한다(11-04 개발 세션 결정)

초안은 읽기 루프를 블로킹 recv로 두었다. 구현에서는 **recv 폴(유휴 100ms · 전송 중 1ms)을 그대로 둔다.**

- 파일 상태 명령(Offer·Accept·Cancel·Pause 등)은 읽기 루프가 깨어나야 처리되는데, 블로킹 recv를 깨울 이식성 있는 수단이 없다.
- 그 명령들의 100ms 지연은 종전과 같다.
- Chat·Control·Group은 쓰기 스레드가 즉시 보내므로 왕복 지연은 해소된다.

## 3. 처방 B(비권고) — 유휴 recv 타임아웃 100 → 20ms

대화 창이 열린 상대에게만 적용하는 안이다.

- 예상 p50 ≈40 · p95 ≈60ms로 **여전히 게이트 ✗**.
- 세션당 50Hz 웨이크업 = RL-10 유휴 바닥(1.08Hz)에 역행한다.

그래서 탈락이다.

## 4. 검증

| 항목 | 기준 |
|---|---|
| `tools/ack-bench.sh` | p95 ≤50ms(루프백 목표 ≤10ms) |
| 회귀 시나리오 | reply 8/8 · group 28/28 · s4 24/24 · mem-bench PASS · oldpeer 5/5 |
| 정적 | clippy 3-OS 타깃(cfg 조건부 코드 없음 예상) |
| 편입 | 처방 뒤 ack-bench를 `tools/verify.sh`에 넣는다 ✅(s4 뒤) |

예상 규모 = 크레이트 4개(nbeep-crypto · nbeep-core · nbeep-net · nexa-beep) · 1일.

### 4-1. 결과(11-04 `b4d723f`)

| 항목 | 결과 |
|---|---|
| `tools/ack-bench.sh`(Win Debug · 루프백 · 100줄) | n=100 · p50 4ms · p95 **5ms** · max 9ms · unacked 0(종전 158/210/224) ✓ |
| `tools/verify.sh --oldpeer` | **10통과 0실패**(build debug·release · smoke · reply · scan-plaintext · group · s4 · ack-bench · oldpeer · tmp-clean) |
| 정적 | fmt ✓ · clippy `-D warnings`(core·crypto·net·nexa-beep `--all-targets`) ✓ · rustdoc ✓ · cfg 조건부 코드 없음 |
| 단위 | core 200 · crypto 36(+2 = 동시 송신 300×2 nonce 순서 · 분리 불가 폴백) · net 54(+1 = TCP 분리 핸들 송신/rx 폴) ✓ |

⚠ 수치는 루프백이다. 게이트 F-1의 조건인 실 LAN 1:1(2-PC)은 아직 재지 않았다.

## 5. 위험·확인할 것

- **nonce 순서** — 2-1 불변식이 깨지면 상대가 복호 실패로 세션을 끊는다. 동시 송신 300×2 단위 시험으로 고정했다 ✅.
- **폴백 경로** — 분리 `None` 링크(UDP·relay)는 종전 지연을 그대로 가진다. 릴레이 경유는 v1 범위 밖(DR-8)이라 지금은 수용한다.
- **종료 순서** — 두 스레드 중 하나가 끊기면 다른 쪽도 정리돼야 한다. 쓰기 실패 = `WriterFailed`로 액터 한 곳이 Closed를 낸다 ✅(2-4).
- ~~종료 = `shutdown(Both)` 명시~~ — **불필요**(11-04 개정). 읽기 루프가 폴을 유지하므로(2-5) 블로킹 recv를 깨울 일이 없다.
- **recv 타임아웃은 rx 반쪽에만** — `set_recv_timeout`이 tx 핸들에도 걸리면 `write_all`이 타임아웃을 먹는다(Windows SO_SNDTIMEO 분리 확인). 파일 발신 중 1ms 타임아웃은 읽기 루프 쪽 교대용으로만 쓴다.
- **recv nonce는 단일 소비자** — RecvHalf는 읽기 루프 하나만 들고 있어 잠금이 필요 없다. 잠금은 쓰기 쪽(암호화+send 원자)에만 둔다.
