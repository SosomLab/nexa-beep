# 55. 세션 액터 송수신 분리 — 메시지 왕복 지연 처방 📐 (LAT-1 · 2026-11-04)

> 상태: 📐 **설계**(11-04 저녁 · 코드 변경 없음) · 착수 = 11-05 첫 작업(개발 세션) · 판정 = `tools/ack-bench.sh` p95 ≤50ms.
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

- `NoiseSession::split(self) -> Option<(RecvHalf, SendHalf)>`
- snow 0.9.6 `HandshakeState::into_stateless_transport_mode()`를 쓴다. `StatelessTransportState`는 read/write가 `&self`이고 nonce를 호출자가 명시한다.
- 핸드셰이크 뒤 `into_transport_mode()` 대신 stateless로 들어가 `Arc<StatelessTransportState>`를 두 반쪽이 공유한다.
- **RecvHalf** = 링크 rx + recv nonce(u64).
- **SendHalf** = 링크 tx + `Mutex<u64>` send nonce.
- ★**불변식: 암호화와 send를 같은 잠금 안에서 한다.** 와이어 순서 = nonce 순서. 펌프 스레드와 쓰기 스레드가 같은 SendHalf를 공유해도 순서가 깨지지 않는다.
- snow 타입은 nbeep-crypto 안에 머문다(DR-21 · 포트 시그니처에 나오지 않는다).

### 2-2. nbeep-core — 포트 `Link::split`

- `Link::split(self: Box<Self>) -> Option<(Box<dyn Link>, Box<dyn Link>)>`
- **기본 구현 = `None`** → 종전 단일 루프로 폴백한다. 분리를 못 하는 링크도 지금처럼 돈다.
- `MuxSession` → `MuxRecv`(큐 · `recv_any`) / `MuxSend`(스트림 바이트 접두 + SendHalf).
- `TrustedSession` 래퍼도 양쪽에 씌운다.

### 2-3. nbeep-net — 링크별 구현

| 링크 | split |
|---|---|
| `TcpLink` | `TcpStream::try_clone` · recv 타임아웃은 rx 쪽에만 |
| `UdpLink` · relay · inmem | `None`(폴백) |

### 2-4. nexa-beep — 액터

세션 성립 뒤 split이 성공하면 두 갈래로 나눈다.

| 갈래 | 하는 일 |
|---|---|
| **읽기 루프** | 블로킹 recv(파일 발신 중에만 1ms 타임아웃) · 프레임 처리 · 상태 명령은 `actor_rx`로 받는다 · 파일 펌프는 여기 남고 같은 `MuxSend`로 청크를 보낸다(잠금으로 순서 보장) |
| **쓰기 스레드** | `out_rx` 소비 · Chat/Control/Group = **즉시** `MuxSend` · Offer/Accept/Reject/Cancel/Pause 등 상태 명령 = `actor_tx`로 읽기 루프에 넘긴다 |

split이 `None`이면 지금 루프 그대로다.

무변경: RL-13 인바운드 핸드셰이크 타임아웃 · RL-9 펌프 예산(PERF-9 20ms·64청크 포함) · netmon 계측.

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
| 편입 | 처방 뒤 ack-bench를 `tools/verify.sh`에 넣는다(지금은 의도된 ✗라 미편입) |

예상 규모 = 크레이트 4개(nbeep-crypto · nbeep-core · nbeep-net · nexa-beep) · 1일.

## 5. 위험·확인할 것

- **nonce 순서** — 2-1 불변식이 깨지면 상대가 복호 실패로 세션을 끊는다. 펌프·쓰기 두 스레드 동시 송신 시험이 필요하다.
- **폴백 경로** — split `None` 링크(UDP·relay·inmem)는 종전 지연을 그대로 가진다. 릴레이 경유는 v1 범위 밖(DR-8)이라 지금은 수용한다.
- **종료 순서** — 두 스레드 중 하나가 끊기면 다른 쪽도 정리돼야 한다(세션 닫힘 · 재연결 RL-3 경로와 맞물림).
- **종료 = `shutdown(Both)` 명시**(개발 세션 보충 11-04) — `TcpStream::try_clone`은 같은 소켓의 핸들 복제라 한쪽을 drop해도 블로킹 recv가 깨지지 않는다. 쓰기 스레드가 끝나거나 액터가 닫을 때 `shutdown(Both)`를 호출해 읽기 루프를 깨운다(Windows·Unix 공통). 지금 액터의 "대피(Closed) → join 2s 상한" 경로와 맞물린다.
- **recv 타임아웃은 rx 반쪽에만** — `set_recv_timeout`이 tx 핸들에도 걸리면 `write_all`이 타임아웃을 먹는다(Windows SO_SNDTIMEO 분리 확인). 파일 발신 중 1ms 타임아웃은 읽기 루프 쪽 교대용으로만 쓴다.
- **recv nonce는 단일 소비자** — RecvHalf는 읽기 루프 하나만 들고 있어 잠금이 필요 없다. 잠금은 쓰기 쪽(암호화+send 원자)에만 둔다.
