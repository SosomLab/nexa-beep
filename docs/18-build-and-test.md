# 18 · 빌드 & 테스트 (SSOT)

> **이 문서가 빌드·테스트·검증 절차의 SSOT다.** 절차를 바꾼 **그 커밋에서 이 문서를 같이 고친다**(사후 정리 금지).
> 규약 출처 [16 §2-4](16-doc-git-conventions.md#2-문서-작성--필수-규칙-8).

## 1. 명령 (SSOT) — Rust 워크스페이스([07](07-adr-0001-stack.md))

> 로컬·CI 동일. `rust-toolchain.toml`이 stable·컴포넌트(rustfmt/clippy)·4타깃을 자동 고정한다.

| 목적 | 명령 |
|---|---|
| 개발 빌드 | `cargo build --workspace` |
| 릴리스 빌드 | `cargo build --release -p nexa-beep -p nbeep-imgdec` → `target/release/{nexa-beep,nbeep-imgdec}` |
| **테스트**(green 기준) | `cargo test --workspace --features nbeep-core/testkit,nbeep-net/testkit,nbeep-crypto/testkit` — **전부 통과 + 0 ignored 예상 밖 없음** |
| 포맷 | `cargo fmt --all --check` (수정은 `--check` 없이) |
| 린트 | `cargo clippy --workspace --all-targets --features nbeep-core/testkit,nbeep-net/testkit,nbeep-crypto/testkit -- -D warnings` |
| rustdoc 링크 | `RUSTDOCFLAGS="-D warnings" cargo doc --workspace --no-deps --features nbeep-core/testkit,nbeep-net/testkit,nbeep-crypto/testkit` |
| 크로스 빌드 | `cargo build --workspace --target <TARGET>` (예: `aarch64-pc-windows-msvc`) |
| 산출물 크기 | 릴리스 빌드 후 `wc -c target/release/<bin>` — **≤10MB/바이너리**(NFR-B-3) |

**4타깃**: `x86_64-pc-windows-msvc` · `aarch64-pc-windows-msvc` · `x86_64-apple-darwin` · `aarch64-apple-darwin` · `x86_64-unknown-linux-gnu`.
(테스트 실행은 네이티브 3-OS, ARM/타OS는 **빌드 검증만** — [CI](#4-ci--githubworkflowsciyml).)

> **테스트 더블**([13 §11-1](13-code-design-standards.md)): 미완 협력자는 `testkit`(feature 게이트)의 fake/spy로 대체해 단위별로 검증한다. 릴리스엔 미포함.
>
> ★ **테스트 의무 + CI 회귀 게이트(사용자 확정 08-08)** — **모든 목적 기능은 테스트 로직과 함께**만 완료로 친다(테스트 없는 기능 커밋 금지). 완료된 기능의 테스트는 CI `test` 잡(3-OS)이 **매 push·PR마다 전부** 실행한다 — 다른 기능 구현이 기존 동작을 되돌리면 CI가 red가 되어 main 병합이 막힌다(main 항상 green). **새 크레이트에 testkit feature를 추가하면 CI·이 표의 `--features` 목록에 같은 커밋으로 추가**한다 — 빠뜨리면 그 표면이 회귀 게이트 밖에 놓인다.

## 2. 검증 항목 (기능 도착 시 절차화)

> **실행 모드 전체·수동 테스트 시나리오(GUI·터미널·Docker)** 는 [26 실행·수동 테스트 가이드](26-run-and-manual-test.md). 아래는 검증 절차의 SSOT 세부.

| 구분 | 내용 |
| --- | --- |
| 빌드 | 개발 빌드 · 릴리스 빌드 명령, 산출물 경로 |
| 테스트 | 단위 테스트 · 통합 테스트 명령, green 기준 |
| 린트/포맷 | 명령과 CI 강제 여부 |
| **네트워크 검증** | [06 §7](06-network-stack.md) **E-1~E-9** 절차화 — 동일 서브넷 / 무선 혼재 / 클라이언트 격리 AP / 다중 VLAN / 다중 NIC / 링크로컬 직결 / 방화벽 3종 / 100대 / **24h 상주 누수** |
| **종료 경로 검증**(FR-P-7 · R-16) | `SIGINT`/`SIGTERM`(Win 콘솔 종료)으로 **GOODBYE가 실제로 나가는가**(FR-D-8 절반) · 세션·소켓·워커·타이머가 **전부 회수되는가**(NFR-B-6) · 평문 버퍼 zeroize(FR-S-22) · **블로킹 accept/recv에 갇히지 않고 깨어나는가** · 컨테이너는 **`--init` 없이도** 정상 종료되는가 |
| **예산 게이트** | 산출물 크기 · 유휴 RSS · **장시간 상주 후 RSS 증가량 0**(DR-5) — 초과 시 병합 금지 |
| **안전 송수신 검증** | [04](04-safe-transfer.md) 회귀 — 격리물이 실행 불가한가 · 복원 후 MotW/quarantine이 남는가 · Zip Slip/압축폭탄 거부 · RLO 파일명 무해화 |
| **원격 연결 검증** | [19](19-adr-0006-manual-endpoint.md) — 서브넷 밖 인바운드가 **자동 등록되지 않는가** · 대조 전 파일 수신 차단 · 잘못된 핸드셰이크에 **침묵**하는가(포트 스캔 무응답) · 백오프가 유휴 CPU 예산을 깨지 않는가 |
| **저장 암호화 검증** | [17](17-adr-0005-history-at-rest.md) 회귀 — **데이터 폴더 전체를 문자열 스캔해 평문이 한 조각도 없는가**(H-3) · 임시 폴더·썸네일에 평문 없는가 · 키 폐기 후 복호 불가 · 논스 재사용 없음 · 동기화 폴더 감지 동작 |
| **다중 기기 신원 검증** | [20](20-adr-0007-multi-device-identity.md) — **v1**: 대화 스레드·기록·차단 키가 `UserId`인가(`PeerId` 사용 시 컴파일 실패) · 저장 마스터 키가 **래핑 구조**인가(래핑 키 교체 후 재암호화 없이 열리는가) · `sender_device` + 시퀀스로 **중복 제거**되는가. **v2**: 서명 없는 "같은 사용자" 주장 **거부** · 폐기된 기기 키 핸드셰이크 거부 · **옛 목록 재생(version 롤백) 거부** · 목록에 없는 기기의 편입 거부 · **보조 기기에서 서명 시도 실패**(주 기기 전용) · **현재 기기 자기 폐기 불가** · **주 기기 양도 전 폐기 불가** · 복구 시드로 승격 후 **이전 주 기기 폐기 가능** · **상대 기기 구성 변경 시 시스템 라인이 남는가**(끌 수 없음) · `UserId` 키로 **저장 기록이 열리지 않는가**(서명/복호 분리) |
### 2-1. 컨테이너로 실행할 때 — **`--init` 필수**

> DR-19 수동 엔드포인트 실증처럼 **리눅스 노드를 컨테이너로 띄울 때**의 규약.

```bash
docker run --rm --init -p 47200:47200 \
  -v "$PWD/.docker-target/release/nexa-beep:/nexa-beep:ro" \
  debian:stable-slim /nexa-beep --serve 47200
```

**`--init`을 빠뜨리면 Ctrl+C로 종료되지 않는다.** 리눅스 커널은 **PID 1에게 시그널 기본 동작을 적용하지 않아서**, 핸들러가 없는 프로세스가 PID 1이면 `SIGINT`·`SIGTERM`을 **그대로 무시**한다. `--rm`은 컨테이너가 종료돼야 발동하므로 컨테이너도 남는다.

| 조건 | `docker stop` 소요 | 해석 |
|---|---:|---|
| `--init` 없음 | **10.26초** | SIGTERM 무시 → 10초 타임아웃 → **SIGKILL** |
| **`--init`** | **0초** | tini가 시그널 중계 → 자식은 PID 1이 아니므로 정상 종료 |

> 실측 2026-08-08([journal](journal/2026-08-08.md)). ⚠️ **`--init`은 우회이지 해결이 아니다** — 앱 자체의 종료 경로 부재는 **FR-P-7 · R-16**으로 등록돼 있다. 정리가 안 될 때는 `docker stop <id>`(10초) 또는 `docker kill <id>`(즉시).

### 2-2. 터미널로 사람 대 사람 대화 — `--chat-serve` / `--chat-connect`

> GUI 없이 **stdin/stdout으로 실제 대화**한다. 세션 스택은 GUI와 **완전히 동일**(Noise_XX → 신원 확정 → 다중화 `StreamId::Chat`)하고 **프레젠테이션만 터미널**이다.

| 역할 | 명령 | 하는 일 |
|---|---|---|
| **기다리는 쪽** | `nexa-beep --chat-serve [port]`(기본 47200) | `0.0.0.0:port` 바인딩 → **한 명** accept → Noise **responder** |
| **거는 쪽** | `nexa-beep --chat-connect <host:port>` | DR-19 **수동 엔드포인트**(`add_endpoint`)로 직접 연결 → Noise **initiator** |

연결 후 경로는 양쪽이 같다. **한 줄 입력 = 전송**(Enter) · 수신은 `<상대지문>> 메시지`로 실시간 출력 · **`Ctrl+D` = 종료**.
본문은 출력 전 `sanitize_message`를 통과한다(RLO·제어문자 — FR-S-13).

#### 상대가 "그 사람"인지 확인하는 법

주소로 거는 것이지 **신원을 지정하는 게 아니다** — 신원은 **핸드셰이크가 확정**한다([21 P-5](21-identity-spec.md)). 양쪽 화면의 값을 **교차 대조**한다:

```
A쪽:  [대기] 47210 …  (me=5e578c85)      ← A의 지문
      [대화 시작] 상대=8fd1123f           ← A가 본 B
B쪽:  [연결] … (me=8fd1123f)             ← B의 지문
      [대화 시작] 상대=5e578c85           ← B가 본 A
```
**A의 `me` == B의 `상대`** 이고 **B의 `me` == A의 `상대`** 면 서로가 의도한 상대와 붙은 것이다.

> ⚠️ **이건 MITM 방어가 아니다.** `short()`는 앞 4바이트라 **표시용**이고([21 §3-1](21-identity-spec.md)) 육안 대조 수단은 **SAS 60자리**인데 **CLI 채팅에는 아직 배선되지 않았다**(M3-6).

#### 컨테이너와 대화 (실측 2026-08-08)

```bash
# ① 컨테이너를 "기다리는 쪽"으로            -it = stdin · --init = R-16
docker run --rm -it --init -p 47200:47200 \
  -v "$PWD/.docker-target/release/nexa-beep:/nexa-beep:ro" \
  debian:stable-slim /nexa-beep --chat-serve 47200

# ② 맥에서 건다 (게시된 포트는 로컬호스트로 보인다)
./target/release/nexa-beep --chat-connect 127.0.0.1:47200
```
반대 방향(맥이 기다리고 컨테이너가 걸기)이면 컨테이너에서 `--chat-connect host.docker.internal:<port>`.

#### 알려진 제약

| 제약 | 내용 |
|---|---|
| **1:1 전용** | `--chat-serve`는 `accept`를 **한 번만** 한다. 두 번째 상대는 붙지 못한다 |
| **`Ctrl+C`로 끝나지 않는다** | 채팅 모드는 아직 **종료 포트(`plat::shutdown`)를 쓰지 않는다**(현재 `--discover-probe`·`--serve`·`--live-echo`만). **`Ctrl+D`로 종료**하고, 컨테이너는 `--init` 필수 → **R-16 · FR-P-7 · M3-13** |
| **파이프 입력은 부적합** | stdin EOF가 즉시 오면 수신 스레드가 먼저 끝나 **받은 메시지를 못 본다**(실측). 사람이 직접 타이핑하는 전제 |
| **발견을 거치지 않는다** | 주소 직접 연결(DR-19)이라 발견 목록에서 **이름으로 고르는 건 GUI**(`--window --live`)의 몫이다. 원격 신뢰 등급·SAS 전 파일 차단(FR-S-24)도 CLI에는 아직 없다 |

## 3. 규칙

- **main은 항상 green** — 테스트·린트 통과 전 병합 금지.
- **[13 §12 코드 리뷰 체크리스트](13-code-design-standards.md) 통과**가 병합 조건이다(파이프라인 우회 없음·포트 주입·민감 타입 마스킹·상한 있는 큐 등).
- 네트워크 기능은 단위 테스트만으로 신뢰하지 않는다. **2대 이상 실기 확인**을 병합 조건에 포함하고, 결과 수치를 journal에 남긴다.

## 4. CI — `.github/workflows/ci.yml`

> main push·모든 PR에서 실행. 하나라도 실패하면 **병합 금지**(§3).

| 잡 | 실행 | 게이트 |
|---|---|---|
| **lint** | ubuntu | `fmt --check` · `clippy -D warnings` · `rustdoc -D warnings` |
| **test** | ubuntu·macOS·windows(네이티브) | `build` + `test`(testkit). 3-OS = 중립 크레이트 4타깃 검증의 실행부 |
| **cross-build** | Win ARM64 · Intel mac | 실행 불가 타깃은 **컴파일 검증만** |
| **budget** | ubuntu | 릴리스 빌드 후 **바이너리 ≤10MB**(NFR-B-3) 초과 시 실패 |

- 전역 `RUSTFLAGS=-D warnings` · `RUSTDOCFLAGS=-D warnings` — 경고 = 실패.
- ⏳ **미발효(실물 도착 후 추가)**: 임포트 화이트리스트(NFR-B-5 · OS 인박스만 — SP-1/M0-2) · 유휴 RSS·24h 누수(NFR-B-1/6 — M1+) · 안전 회귀([§2](#2-검증-항목-기능-도착-시-절차화)) · 발견 실측 E-1~E-9(실기).
