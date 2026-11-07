# 56 · ADR-0016 — `nexa-beepd` 와이어 v2: 호환 포기 · 확장 가능한 최소 구조

> **상태**: 📐 Proposed(2026-11-07 · 사용자 지시 "기존 버전 호환성을 포기하고 이후 확장성을 가질 수 있도록 최소한의 구조적 설계 및 준비")
> **범위**: 릴레이 서버 `nexa-beepd`와 클라 어댑터 `nbeep-relay`의 **와이어·핸드셰이크·서버 모듈 경계**. 앱 기능(X-2b 타입 협상 · X-2d 비공개 프레즌스 · X-13 S5 컨텐츠 · docs/44 I-2/I-3/I-6)은 이 구조 **위에서** 각자 결정한다.
> **준비물(이 커밋)**: 코덱 크레이트 [`crates/nbeep-relay-proto`](../crates/nbeep-relay-proto/src/lib.rs)(I/O·크립토 0 · 골든 벡터) + 이 문서. **배선(서버·클라 교체)은 다음 단계**(§7) — v0.9.0 릴리스에는 영향 없음(어느 바이너리도 아직 의존하지 않는다).
> **관련**: ADR-0003(docs/09 사다리 전송) · ADR-0013(docs/32 서버 모드) · docs/41(운영) · docs/44(nexa-clip 연계) · DR-9(같은 저장소 · 배포 분리 · W-1~W-4).

---

## 1. 문제 — v1 와이어에는 "확장"이라는 통로가 없다

조사(11-07 · `nbeep-relay/src/lib.rs` · `nexa-beepd/src/lib.rs`) 결과:

| 빈틈 | 근거 | 결과 |
|---|---|---|
| **버전·능력 필드 없음** | `Register`에 버전 없음 · Noise prologue 없음(`noise.rs:18`) · "v2"는 `AnnounceCard` kind를 보냈는지로 추정(`beepd:680`) | 서버·클라 버전이 어긋나면 **기능만 조용히 빠지거나 경고 없이 깨진다** |
| **kind u8 + 고정 오프셋 본문** | `[kind u8][고정 바이트]`(`:125~129`) · "나머지 전부"를 먹는 필드 뒤는 확장 불가(`:401 Data` · `:416 card`) | 새 필드 = 새 kind · 구조 변경 = 비호환 |
| **모르는 kind = 조용히 무시** | `beepd:456 continue` · 클라 `:840` | 오류를 **알릴 통로가 없다**(Error/Goaway kind 없음) |
| **상한 하드코딩** | 프레임 65535(`tcp.rs:16`) · RID 8 · 채널 64 · 큐 8MiB · 카드 1024 | 협상 없이 양쪽 상수가 같아야 한다 |
| **인증 1종 · 이름공간 없음** | Noise만 성립하면 수락(`beepd:391`) · 앱 격리는 RID 파생 문자열에만 의존(docs/44 §1-1) | beep·clip이 같은 서버에서 **관례**로만 분리 |
| **모듈 경계 없음** | 코덱·세션·라우팅·roster·UDP가 `lib.rs` 한 파일 · `conn_thread` 440줄 단일 함수(`:378~817`) · 연결당 OS 스레드 + 15ms 폴 | 모드 추가(컨텐츠·관리)가 곧 그 함수 비대화 |
| **운영 노출 없음** | 서버 버전·종류를 알리는 kind 없음(X-2b) · 계측·헬스·시그널 종료 없음(`main.rs:97`) | 배포 뒤 "무엇이 돌고 있나"를 클라가 모른다 |
| **코덱 위치** | 와이어 코덱이 **클라 어댑터 크레이트 안**에 섞여 있고 clip은 그 사본(`nclip-sync/relay.rs` 1818줄 "이식 사본") | 동기 = 사람이 고지하는 절차(docs/44 §7-4)뿐 |

> **결정 전제(사용자 11-07)**: **구버전 호환을 포기한다.** 운영 서버 `beepd.sosomlab.com`(OCI · 0.2.5 배포 기록)과 beep ≤0.9 · clip 현행은 v2 서버에 붙지 못한다 — **의도된 비호환**이며 §6의 전환 절차로 한 번에 넘어간다.

## 2. 결정 — 봉투 + TLV · major는 prologue에 · 앱 이름공간 · 오류 통로

### 2-1. 바이트 형식(`nbeep-relay-proto`)

```text
링크 프레임  = [len u32 BE][Envelope]          len ≤ limits.max_frame(Welcome이 알린다 · 기본 65535)
Envelope     = kind u16 BE ‖ flags u8 ‖ Field*
Field(TLV)   = tag u8 ‖ len u32 BE ‖ value      같은 tag 반복 = 목록 · 값이 TLV면 중첩
```

| 규칙 | 내용 |
|---|---|
| **kind 공간** | `0x0000..=0x3FFF` **핵심** — 모르면 `Error{UnknownKind, ref_kind}`로 **응답**(조용히 버리지 않는다) · `0x4000..=0x7FFF` **확장** — 모르면 무시 · `0x8000..` 예약. 방향 = 번호(C2S `0x00xx` · S2C `0x01xx`) |
| **flags** | bit0 `OPTIONAL` = 핵심 kind라도 "모르면 버려도 됨"(발신자가 선언) · 나머지 예약 |
| **tag 규칙** | 모르는 tag = **건너뜀**(같은 major 안 전방 호환) · 필수 tag 누락 = `Error{Malformed}` · 번호 불변 · append만 |
| **major** | Noise **prologue** `"nexa-beepd/2"`에 묶인다 → major가 다르면 핸드셰이크가 **명시적으로** 실패(TOFU 핀은 그대로). 클라는 이를 "서버가 다른 세대 — 앱 업데이트 필요"로 표시 |
| **minor·능력** | `Hello`(클라) / `Welcome`(서버)로 교환 · 세션 능력 = 비트 교집합(`caps::ROSTER/SHARED_BIND/UDP_OBSERVE/CONTENT`) |
| **앱 이름공간** | `Hello.app`(`"nexa-beep"`·`"nexa-clip"`)이 연결을 앱에 묶고 서버는 **`(app, rid)`** 로 라우팅 · 교차 `Open`은 `Forbidden`. RID 파생 문자열 격리(docs/44 §1-1)는 **2차 방어**로 유지 |
| **상한 협상** | `Welcome.limits{max_frame, max_rids, max_chans, queue_bytes, rate_bps, ping_interval_s, card_max}` — 클라는 이 값을 따른다(하드코딩 제거) |
| **오류·종료** | `Error{code, reason, ref_kind}` · `Reject{code, reason, min_proto}`(Hello 거절 뒤 끊음) · `Goaway{code, reason, retry_after_s}` 양방향(우아한 종료 · 재접속 힌트) |
| **바인딩 모드** | `Register.bind = ExclusiveLastWins(v1 동작) \| Shared`(docs/44 I-2 "한 RID에 여러 기기" — 능력 `SHARED_BIND`일 때) · `RegisterResult`는 **RID마다 status** |
| **봉투 원리** | 서버는 `kind·rid·ch·길이`만 본다 · `Data.bytes`·`Announce.card`는 불투명 그대로 |

### 2-2. 핸드셰이크 순서

```text
TCP → Noise_XX(prologue "nexa-beepd/2" · 서버 키 TOFU 핀 그대로)
  → C2S Hello{major 2, minor, app, app_version, caps}
  ← S2C Welcome{major, minor, server_version, server_type(relay/content/admin), caps, limits, session_id, udp_port, observed_ep}
     | Reject{code=VersionMismatch|Forbidden, reason, min_proto} → 끊음
  → C2S Register{rid×n, bind}
  ← S2C RegisterResult{(rid,status)×n, udp_token}
  … 이후 v1과 같은 채널 모델(Open → Incoming → Accept → Data ↔ Data → CloseCh/ChClosed) · Ping/Pong(nonce) · Announce/PeerUp/PeerDown
```

- Hello 전 어떤 요청도 `Error{NotRegistered}`. Hello는 세션당 1회.
- `Welcome.server_type`이 **X-2b "타입 협상(auto)"의 와이어**다(설정 `net.server.type=auto`는 이 값을 따른다).
- `Welcome.observed_ep`가 v1 `RegisterOk.ep`를 대체(UDP 관측 프로브는 `UDP_OBSERVE` 능력일 때만 · 프로브 형식도 봉투로 통일 — §7).

### 2-3. 서버 모듈 경계(`nexa-beepd` · 목표 구조)

```text
src/main.rs      설정(CLI + 환경변수 BEEPD_*) · 시그널(SIGTERM/CTRL_C) → Goaway 방송 → 우아한 종료 · 헬스 응답
src/session.rs   Noise + Hello/Welcome + 능력·상한 확정 · 세션 상태기계(Pre/Ready/Closing)
src/router.rs    (app, rid) → 바인딩(Exclusive/Shared) · 채널 표 · Open/Accept/Data 중계 · 상한·토큰 버킷
src/roster.rs    앱별 명부(Announce/PeerUp/PeerDown) — 능력 ROSTER
src/udp_obs.rs   UDP 관측 프로브(능력 UDP_OBSERVE)
src/metrics.rs   카운터(세션·채널·바이트·오류 코드별) · 로그는 상태만(봉투 원리)
src/modes/       content.rs(ADR-0013 S5 · 예약) · admin.rs(예약)
```

- 코덱은 **`nbeep-relay-proto`** 한 곳(서버·클라·clip 공유 정의). `nbeep-relay`는 어댑터(연결·세션·사다리)만 남긴다.
- 동시성은 지금의 "연결당 스레드 + 폴"을 유지한다(의존 0 원칙 · 규모는 OCI 1인스턴스). 모듈 경계가 먼저고 런타임 교체는 별도 결정.

## 3. 대안과 기각

| 대안 | 기각 이유 |
|---|---|
| v1에 `Version` kind만 추가(호환 유지) | 고정 오프셋 본문·u8 kind·무시 규칙이 그대로 — 확장 통로가 생기지 않는다. 사용자 결정 = 호환 포기 |
| 외부 직렬화(protobuf/CBOR 크레이트) | DR-5 런타임 의존 0 · W-3(서버 전용 의존도 최소) · 수제 TLV 300줄이면 충분하고 clip 사본 이식이 쉽다 |
| major도 Hello로(prologue 미사용) | 구클라가 Noise를 성립시킨 뒤 Hello 없이 `Register`를 보내는 경로가 남아 "조용한 비호환"이 재발 — prologue가 가장 이른 지점에서 명시적으로 끊는다 |
| 서버 포트 분리(v1 47300 · v2 47301 병행) | 호환 포기 결정과 어긋남 · 운영 2배. 전환은 §6처럼 **한 번에** |

## 4. 결과(영향)

- **클라(nexa-beep ≥0.10)**: `nbeep-relay` 어댑터가 proto 크레이트로 교체 · 설정 `net.server.type=auto`가 실제 협상 · "서버 세대 불일치" 안내 문구 · 상한은 Welcome 값.
- **서버(beepd-v1.0.0)**: v2 전용. OCI 재배포 필요(docs/41 §9-0 · ★ 이 ADR 배선이 **beepd 재배포가 필요한 첫 변경**).
- **nexa-clip**: `nclip-sync/relay.rs` 사본을 v2로 이식해야 같은 서버에 붙는다(§5 프롬프트). 전환 전까지는 **v1 서버**(현 OCI)를 그대로 쓴다.
- **docs/44 I-2(한 RID 여러 기기)**: `Shared` 바인딩으로 구조적 답 · I-6(앱·버전 태그) = `Hello.app/app_version` · I-3(오프라인 큐)는 컨텐츠 모드 몫(미정).
- 예산: proto 크레이트 = 의존 0 · 클라 산출물 영향 ≈ 0(코덱 대체).

## 5. `nexa-clip` 세션용 프롬프트(사용자가 clip 세션에 붙여 넣는다)

> clip은 코드 의존은 없지만 **와이어 사본 + 같은 운영 서버**를 쓴다(docs/44). 서버가 v2로 바뀌는 날 clip도 같이 바뀌어야 한다.

```text
[nexa-beep → nexa-clip 사전 고지 · beepd 와이어 v2(ADR-0016 · nexa-beep docs/56)]

배경: 릴레이 서버 nexa-beepd가 구버전 호환을 포기하고 와이어 v2로 간다(사용자 결정 2026-11-07).
clip은 crates/nclip-sync/relay.rs가 nbeep-relay v1의 이식 사본이고 beepd.sosomlab.com:47300을 같이 쓰므로,
서버 전환일에 같이 바뀌지 않으면 접속이 (의도적으로·명시적으로) 끊긴다. 지금은 **준비**만 한다 — 전환일 전까지
v1 서버는 그대로 돈다.

해야 할 준비(설계·코드 준비 · 전환 스위치는 아직 켜지 않는다):
1. 새 코덱 사본 이식: nexa-beep crates/nbeep-relay-proto/src/lib.rs(I/O·크립토 0 · 외부 의존 0)를
   crates/nclip-sync/relay_v2.rs(또는 nclip-relay-proto)로 이식. 골든 벡터 테스트(golden_vectors_are_stable)가
   그대로 통과해야 한다 — 바이트가 다르면 이식 오류.
2. 핸드셰이크 변경 지점 정리: Noise prologue = b"nexa-beepd/2"(지금 clip은 서버 제어 세션에 prologue를
   안 넣는다 · 종단 prologue "nexa-clip/1"은 그대로) → Hello{app="nexa-clip", app_version=<clip 버전>,
   caps=ROSTER|UDP_OBSERVE(쓰는 것만)} → Welcome.limits를 따르고(프레임 상한·RID 수·카드 상한 하드코딩 제거)
   → Register{rids, bind=ExclusiveLastWins}(I-2를 원하면 Shared + 능력 SHARED_BIND).
3. 오류 처리: Reject(VersionMismatch)와 Noise 실패를 "서버 세대 불일치 — 앱 업데이트 필요"로 사용자에게 보여 준다
   (v1처럼 조용히 기능만 빠지는 경로를 없앤다). Error{code,reason,ref_kind}·Goaway{retry_after_s}를 로그·재접속에 반영.
4. 앱 이름공간: 서버가 (app, rid)로 라우팅하므로 RID 파생 문자열 "nclip-rid-v1"은 2차 방어로 유지(바꾸지 않는다).
5. 전환 스위치: 설정 또는 빌드 플래그로 v1/v2 어댑터를 고를 수 있게 두고 기본은 v1 유지. beep 쪽 "beepd-v1.0.0
   배포 + beep 0.10 릴리스" 고지가 오면 기본을 v2로 바꾸고 v1 코드를 지운다.
6. clip docs/22-upstream-beep-liaison.md에 이 항목(ADR-0016 · 전환일 미정 · 준비 완료 여부)을 적고,
   beep docs/44 §7-4 [clip-sync] 절차로 회신(준비 완료 커밋 해시).

참고 경로(beep 저장소): docs/56-adr-0016-beepd-wire-v2.md · crates/nbeep-relay-proto/src/lib.rs ·
docs/44-nexa-clip-liaison.md §1·§7-4 · docs/41-beepd-operations.md §9-0.
질문은 beep 개발 세션으로(와이어 번호·tag·능력 비트는 beep이 단일 원천 · clip은 append 요청만).
```

## 6. 전환 절차(순서 고정 · 한 번에)

1. **beep**: `nbeep-relay` 어댑터를 proto 크레이트로 교체 · `nexa-beepd` 모듈 분리(§2-3) · e2e 시험(relay_e2e·transport_e2e) v2로 · 클라 "세대 불일치" 안내.
2. **clip**: §5 준비 완료 회신(사본 이식 + 스위치).
3. **배포**: `beepd-v1.0.0` 태그 → OCI 재배포(docs/41 §9-0) → **같은 날** beep `v0.10.0` + clip 릴리스. 서버 핀(TOFU)은 바뀌지 않는다(키 동일).
4. v1 코드(`C2s/S2c` · clip 사본) 삭제 · docs/44 갱신.

## 7. 남은 설계 항목(배선 단계에서 결정)

- UDP 관측 프로브(`OBS_MAGIC "NBOB"`)의 v2 형식 — 봉투로 통일할지(토큰 u64 + ep) · 능력 `UDP_OBSERVE`.
- `Shared` 바인딩의 Incoming 팬아웃 규칙(전부 vs 최근 활성 1대) · 채널 표의 키.
- 환경변수 설정 집합(`BEEPD_PORT/BIND/KEY/RATE/LOG`) · 헬스 응답 경로(TCP 같은 포트의 `Hello{app="probe"}` 응답 vs 별도 포트).
- 컨텐츠 모드(S5) 봉투(ADR-0013 §0 · docs/46 §6) — 능력 `CONTENT`로 예약만.

## 8. 골든 벡터(clip 사본 검증용 · `nbeep-relay-proto` 테스트와 동일)

```text
Hello{major 2, minor 0, app "nexa-beep", app_version "0.10.0", caps ROSTER|UDP_OBSERVE} =
  0001 00
  01 00000002 0002
  02 00000002 0000
  03 00000009 6e6578612d62656570
  04 00000006 302e31302e30
  05 00000008 0000000000000005
Ping{nonce 0x0102030405060708} = 0007 00 19 00000008 0102030405060708
```

| 상수 | 값 |
|---|---|
| `PROLOGUE` | `b"nexa-beepd/2"` |
| `PROTO_MAJOR` / `PROTO_MINOR` | 2 / 0 |
| 핵심 kind | Hello 0x0001 · Register 0x0002 · Open 0x0003 · Accept 0x0004 · Data 0x0005 · CloseCh 0x0006 · Ping 0x0007 · Announce 0x0008 · Goaway 0x0009 · Welcome 0x0101 · Reject 0x0102 · RegisterResult 0x0103 · OpenResult 0x0104 · Incoming 0x0105 · Data(S2C) 0x0106 · ChClosed 0x0107 · Pong 0x0108 · PeerUp 0x0109 · PeerDown 0x010A · Error 0x010B · Goaway(S2C) 0x010C |
| 오류 코드 | UnknownKind 1 · Malformed 2 · Unsupported 3 · LimitExceeded 4 · NotRegistered 5 · NoSuchPeer 6 · Busy 7 · Forbidden 8 · Shutdown 9 · VersionMismatch 10 |
| 능력 비트 | ROSTER 1<<0 · SHARED_BIND 1<<1 · UDP_OBSERVE 1<<2 · CONTENT 1<<3 |
