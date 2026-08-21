# 18 · 빌드 & 테스트 (SSOT)

> **이 문서가 빌드·테스트·검증 절차의 SSOT다.** 절차를 바꾼 **그 커밋에서 이 문서를 같이 고친다**(사후 정리 금지).
> 규약 출처 [16 §2-4](16-doc-git-conventions.md#2-문서-작성--필수-규칙-8).

## 0-1. 재기동 한 방 — `tools/relaunch.sh`

실기 반복에서 매번 한 단계씩 빠졌다(빌드를 잊거나 · 옛 프로세스가 포트를 물고 있거나 ·
`nbeep-imgdec` 복사를 빠뜨려 아바타만 죽거나). 스크립트로 고정했다.

```bash
./tools/relaunch.sh            # ① 전부 종료 ② 릴리스 빌드 ③ 3신원 준비 ④ 기동 ⑤ 발견 확인
./tools/relaunch.sh --fresh    # A·B 신원 초기화(⚠️ 핀·그룹·프로필 삭제)
./tools/relaunch.sh --gate     # 빌드 대신 전체 게이트
./tools/relaunch.sh --no-build # 재기동만
```

**기본은 신원 보존**이고, 마지막에 **상호 발견까지 확인**한 뒤에야 성공으로 친다
(창만 떠도 발견이 막히면 아무것도 못 한다). 3신원의 근거는 [33 §2](33-group-chat-test-guide.md).

> ⚠️ **A·B 신원 폴더 = `$HOME/.nexa-beep-multi`**(durable · `BEEP_MULTI`로 덮어쓰기 가능 ·
> Windows는 `%USERPROFILE%\.nexa-beep-multi`). **`/tmp`·`%TEMP%` 금지** — macOS
> `com.apple.tmp_cleaner`(매일 자정 · 3일 미접근 삭제)가 `identity.key`를 지워 **재기동마다
> 새 신원**이 생기고, 이전 키에 sealed된 격리물·핀을 못 연다(08-19 진단 · [26 §3-4b](26-run-and-manual-test.md)).
> 그 실행 파일이 **실제로 로드할** 신원 확인 = **`nexa-beep --whoami`**(지문·이름·exe·data 경로 ·
> 읽기 전용 = 키를 만들지 않는다). 스크립트가 기존 `/tmp/beep-multi`를 1회 자동 이관한다.

## 1. 명령 (SSOT) — Rust 워크스페이스([07](07-adr-0001-stack.md))

> 로컬·CI 동일. `rust-toolchain.toml`이 stable·컴포넌트(rustfmt/clippy)·4타깃을 자동 고정한다.

| 목적 | 명령 |
|---|---|
| 개발 빌드 | `cargo build --workspace` |
| 릴리스 빌드 | `cargo build --release -p nexa-beep -p nbeep-imgdec` → `target/release/{nexa-beep,nbeep-imgdec}` |
| **서버 릴리스 빌드** | `cargo build --release -p nexa-beepd` → `target/release/nexa-beepd` — **클라와 별도 산출물**(DR-9 개정 · W-1: 클라 빌드에 절대 포함 금지 · 예산 게이트 무관 W-2). 배포 = `beepd-v*` 태그 → `release-server.yml` |
| **서버 e2e** | `cargo test -p nexa-beepd --test relay_e2e` — 종단 Noise 관통·핀 불일치·루프백 홀펀칭 배관(위 워크스페이스 테스트에 포함 — 별도 실행은 서버만 만졌을 때) |
| **테스트**(green 기준) | `cargo test --workspace --features nbeep-core/testkit,nbeep-net/testkit,nbeep-crypto/testkit` — **전부 통과 + 0 ignored 예상 밖 없음** |
| 포맷 | `cargo fmt --all --check` (수정은 `--check` 없이) |
| 린트 | `cargo clippy --workspace --all-targets --features nbeep-core/testkit,nbeep-net/testkit,nbeep-crypto/testkit -- -D warnings` |
| rustdoc 링크 | `RUSTDOCFLAGS="-D warnings" cargo doc --workspace --no-deps --features nbeep-core/testkit,nbeep-net/testkit,nbeep-crypto/testkit` |
| 크로스 빌드 | `cargo build --workspace --target <TARGET>` (예: `aarch64-pc-windows-msvc`) |
| **크로스 타입검사** | `cargo check --workspace --all-targets --target <TARGET>` — **링크가 필요 없어 맥/리눅스에서도 Windows 타깃을 검사할 수 있다** |
| 산출물 크기 | 릴리스 빌드 후 `wc -c target/release/<bin>` — **≤10MB/바이너리**(NFR-B-3) |
| **아이콘 자산 굽기** | `tools/mkicons.sh [이름 …]` — `assets/icons-src/<name>.svg` → `crates/nbeep-ui/assets/icon-<name>-96.alpha`(9,216B 검증). 인자 없으면 전체 |

> **아이콘 자산 절차**([14 §12-7](14-control-ux-architecture.md) · 원장 [10 §4](10-decision-record.md)): 본체는 **런타임 SVG 파서를 링크하지 않는다** — 모양을 빌드 전에 굽는다(`rsvg-convert` 96×96 → `magick -alpha extract` → 1채널 알파). 필요 도구는 macOS `brew install librsvg imagemagick` · Ubuntu `apt install librsvg2-bin imagemagick`. **CI에는 없다**(구운 결과물을 커밋하므로 빌드에 불필요). 구운 뒤 [`nbeep_ui::icons`](../crates/nbeep-ui/src/lib.rs)에 상수를 추가해야 실제로 쓰인다.

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
| ★ **와이어 평문 점검**([29](29-wire-security-audit.md) W-1~W-12) | **자동**: 발견 인코더 골든 레이아웃 · **금칙어 스캔**(프로필 키·이메일·전화 패턴) · 세션 **tap 평문 부재** · 로그/`Debug` 마스킹. **수동**: 멀티캐스트 그룹 조인 후 hex+ASCII 덤프([29 §4-2](29-wire-security-audit.md) — **`sudo` 불필요**) · 여러 패킷의 고정 필드 비교로 **재식별자 탐색** · 세션 구간 `tcpdump` |
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

> ⚠️ **대화 모드는 `-it`(또는 최소 `-i`)가 필요하다** — `-d`·파이프로 띄우면 **stdin EOF = 종료**라 붙자마자 세션이 끊긴다. 자동화는 FIFO로 stdin을 열어둔다 → [26 §4-1](26-run-and-manual-test.md).
>
> 실측 2026-08-08([journal](journal/2026-08-08.md)). ⚠️ **`--init`은 우회이지 해결이 아니다** — 앱 자체의 종료 경로 부재는 **FR-P-7 · R-16**으로 등록돼 있다. 정리가 안 될 때는 `docker stop <id>`(10초) 또는 `docker kill <id>`(즉시).

### 2-2. 터미널로 사람 대 사람 대화 — `--chat-serve` / `--chat-connect`

> GUI 없이 **stdin/stdout으로 실제 대화**한다. 세션 스택은 GUI와 **완전히 동일**(Noise_XX → 신원 확정 → 다중화 `StreamId::Chat`)하고 **프레젠테이션만 터미널**이다.

| 역할 | 명령 | 하는 일 |
|---|---|---|
| **기다리는 쪽** | `nexa-beep --chat-serve [port]`(기본 47200) | `0.0.0.0:port` 바인딩 → **한 명** accept → Noise **responder** |
| **거는 쪽** | `nexa-beep --chat-connect <host:port>` | DR-19 **수동 엔드포인트**(`add_endpoint`)로 직접 연결 → Noise **initiator** |

연결 후 경로는 양쪽이 같다. **한 줄 입력 = 전송**(Enter) · 수신은 `<상대지문>> 메시지`로 실시간 출력 · 종료는 **`/quit`**(`/exit`·`/q`) · `Ctrl+D` · `Ctrl+C`(`/help`로 명령 목록).
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
| ~~`Ctrl+C`로 끝나지 않는다~~ | ✅ **해소(08-11)** — 채팅 모드도 종료 포트(`plat::shutdown`)를 쓴다. `/quit`·`Ctrl+D`·`Ctrl+C`가 **같은 정리 절차**를 타고 터미널 설정을 복원한다(그전엔 `Ctrl+C`가 `Drop`을 건너뛰어 raw 모드를 남겨, 셸이 먹통처럼 보였다). **상대를 기다리는 구간에서도** 듣는다. 컨테이너는 여전히 `--init` 권장 |
| **파이프 입력은 부적합** | stdin EOF가 즉시 오면 수신 스레드가 먼저 끝나 **받은 메시지를 못 본다**(실측). 사람이 직접 타이핑하는 전제 |
| **발견을 거치지 않는다** | 주소 직접 연결(DR-19)이라 발견 목록에서 **이름으로 고르는 건 GUI**(`--window --live`)의 몫이다. 원격 신뢰 등급·SAS 전 파일 차단(FR-S-24)도 CLI에는 아직 없다 |

## 3. 규칙

- **main은 항상 green** — 테스트·린트 통과 전 병합 금지.
- **[13 §12 코드 리뷰 체크리스트](13-code-design-standards.md) 통과**가 병합 조건이다(파이프라인 우회 없음·포트 주입·민감 타입 마스킹·상한 있는 큐 등).
- 네트워크 기능은 단위 테스트만으로 신뢰하지 않는다. **2대 이상 실기 확인**을 병합 조건에 포함하고, 결과 수치를 journal에 남긴다.
- ★ **조건부 컴파일(`#[cfg(...)]`)을 건드렸으면 반대편 타깃을 타입검사한다** — 내 OS의 게이트는 그 코드를 **보지도 않는다**. 08-11에 `use std::io::Write as _;`를 `#[cfg(unix)]`로 묶었다가 main이 red가 됐다: 그 트레이트를 쓰는 `Drop`은 **모든 플랫폼에서 컴파일된다**(Windows에선 실행만 안 될 뿐). *"안 도는 코드"와 "컴파일 안 되는 코드"는 다르다.*
  ```bash
  cargo check --workspace --all-targets --target x86_64-pc-windows-msvc
  cargo check --workspace --all-targets --target aarch64-pc-windows-msvc
  ```
- **로컬 게이트는 4종 전부**(`fmt --check` · `clippy` · `test` · **`doc`**) + 위 크로스 타입검사. rustdoc을 빼먹으면 한국어 `[대괄호]`가 intra-doc 링크로 해석돼 CI가 red가 된다.

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

## 5. 배포 — `.github/workflows/release.yml`

> 포장 상세(채널·타깃·매니페스트·게시 스위치)는 [`packaging/README.md`](../packaging/README.md)가 SSOT.
> 여기서는 **개발자가 밟는 절차**만 적는다.

```bash
# 1) main이 green인지 먼저 확인한다 — red 상태로 태그를 밀지 않는다.
gh run list --branch main --workflow ci --limit 1
# 2) 버전을 올린다(워크스페이스 단일 버전).
#    태그와 Cargo 버전이 어긋나면 brew formula의 test가 잡는다.
$EDITOR Cargo.toml       # [workspace.package] version
# 3) 태그를 밀면 끝 — 빌드·릴리스 공개·brew 탭 반영이 자동으로 이어진다.
git tag -a v0.1.2 -m "..." && git push origin v0.1.2
```

| 단계 | 자동 | 잠금 |
|---|---|---|
| 5타깃 포장 + GitHub Release **공개** | ✅ 태그 push | — |
| Homebrew 탭 반영 | ✅ 릴리스 직후 | `TAP_TOKEN` 시크릿 유무 |
| winget · Chocolatey 제출 | 실행되나 **기본 꺼짐** | 저장소 변수 `WINGET_PUBLISH`/`CHOCO_PUSH` + 시크릿 |

- 태그 없이 확인만 하려면 Actions ▸ **release** ▸ *Run workflow*(그 경로는 **초안**을 만든다).
- 제출만 다시 하려면 **homebrew** / **publish-windows-packages** 워크플로를 태그를 주고 실행한다.
- ★ **공개된 버전의 자산은 덮어쓰지 않는다.** 같은 버전의 파일이 바뀌면 `SHA256SUMS.txt`가
  지키려던 것을 스스로 깨는 셈이다 — 고칠 것이 생기면 **버전을 올려 새로 낸다**.

### 배포 후 실기 확인(사람이 한다)

CI가 통과해도 *"설치해서 실제로 뜨는가"* 는 다른 질문이다. 08-11 첫 배포에서 실제로
깨져 있었던 것들이다.

```bash
brew update && brew install --cask kiros33/tap/nexa-beep
xattr "/Applications/Nexa Beep.app"          # 격리 속성이 남아 있으면 실행 즉시 죽는다
open -a "/Applications/Nexa Beep.app"        # ★ Finder 실행 = 인자 없음 경로
brew install kiros33/tap/nexa-beep-portable && nexa-beep --version
```

⚠️ **macOS 격리** — 서명·공증이 없는 앱은 격리 표식이 붙어 있으면 **`exit 137`(SIGKILL)**
되고 macOS가 앱을 삭제한다. 애드혹 서명으로도 넘지 못한다(08-11 실측). Cask가
`postflight`에서 표식을 떼는 것이 **임시방편**이며, 제대로 된 해결은 서명·공증이다(M5-4 잔여).
