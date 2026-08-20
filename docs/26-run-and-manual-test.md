# 26. 실행·수동 테스트 가이드

> **목적** — 빌드해서 **띄울 준비**를 하고, **유형별로 어떻게 실행하는가**를 최소 절차로 정리한다.
> 빌드·게이트 SSOT는 [18](18-build-and-test.md)이고, 이 문서는 "**띄워서 직접 확인**"만 다룬다.
> 근거: 발견 사다리 [06 §4](06-network-stack.md) · 수동 연결 [19 ADR-0006](19-adr-0006-manual-endpoint.md).
> 실측 원본은 [journal/](journal/)에 날짜별로 있다 — 여기엔 **재현 절차와 결론**만 둔다.

**읽는 순서** — [§1 준비](#1-준비--빌드) → [§2 규칙 셋](#2-실행-유형-한눈에) → [§3 유형별 절차](#3-유형별-최소-절차)에서 필요한 것 하나.
막히면 [§6 자주 막히는 곳](#6-자주-막히는-곳--증상별)으로 간다.

---

## 1. 준비 — 빌드

### 1-1. 호스트(맥·Windows·Linux)

```bash
cargo build --release -p nexa-beep -p nbeep-imgdec
#   → target/release/{nexa-beep, nbeep-imgdec}
```

### 1-2. Linux 바이너리 — **상주 빌더**(권장)

컨테이너를 하나 띄워 두고 `exec`으로 빌드한다. 툴체인 다운로드와 컴파일 캐시가 살아 있어
**두 번째부터는 바뀐 크레이트만** 컴파일한다. Docker 시험을 두 번 이상 할 거면 이쪽이 맞다.

```bash
# ① 1회 준비 — 컨테이너를 남긴다(`--rm`이면 종료 시 사라져 캐시가 날아간다)
docker run -d --name beep-builder -v "$PWD":/src -w /src -e CARGO_TARGET_DIR=/target \
  rust:1-slim sleep infinity
docker exec beep-builder bash -c \
  'apt-get update -qq && apt-get install -y -qq pkg-config >/dev/null; rustup show >/dev/null'

# ② 매회 — 빌드와 반출을 **한 줄로** 묶는다(따로 두면 복사를 잊는다)
docker exec beep-builder bash -c \
  'cargo build --release -p nexa-beep -p nbeep-imgdec \
   && mkdir -p /src/.docker-target/release \
   && cp /target/release/nexa-beep /target/release/nbeep-imgdec /src/.docker-target/release/'
#   → .docker-target/release/{nexa-beep, nbeep-imgdec}  (linux/amd64)
```

`docker stop beep-builder`로 세워 두고 `docker start beep-builder` 후 다시 `exec`하면 된다.

**왜 `/target`(컨테이너 내부)인가 — 실측으로 갈랐다(08-13 · 맥 x64)**

| 산출 위치 | 무변경 재빌드 | 한 크레이트 변경 | 복사 단계 |
|---|---|---|---|
| `/target`(내부 FS) | **0.36초** | **34.9초** | 필요 |
| `/src/.docker-target`(바인드 마운트 직접) | 0.76초 | 37.2초 | 불필요 |

맥 바인드 마운트 I/O가 느려 내부 FS가 빠르지만, 차이는 **2.4초**다. 진짜 위험은 속도가 아니라
**복사를 잊고 옛 바이너리로 시험하는 것**이라, ②처럼 `&&`로 묶어 잊을 수 없게 만든다.
그게 싫으면 `-e CARGO_TARGET_DIR=/src/.docker-target`로 덮어써 직접 쓰면 된다(복사 불요).

콜드 스타트는 4분 29초. ⚠️ 그 몇 분간의 `debconf` 경고와 `downloading 9 components`는
**오류가 아니다**(실기 08-13 — 실패로 오인): TTY 없는 apt의 소음 + `rust-toolchain.toml`이
요구하는 툴체인·크로스 타깃 내려받기다. `Compiling …`부터가 빌드다.

**일회성으로 딱 한 번만**(CI·다른 PC) — 캐시를 남기지 않는다:

```bash
docker run --rm -v "$PWD":/src -w /src -e CARGO_TARGET_DIR=/src/.docker-target \
  rust:1-slim bash -c 'apt-get update -qq && apt-get install -y -qq pkg-config >/dev/null; \
  cargo build --release -p nexa-beep -p nbeep-imgdec'
```

`CARGO_TARGET_DIR`를 호스트 `target/`과 나누는 이유 — 섞으면 맥·리눅스 산출물이 충돌해
**매번 전체 재컴파일**이 된다. Docker 데몬이 꺼져 있으면 `open -a Docker` 후 `docker info`가
응답할 때까지 기다린다.

### 1-3. ★ `nbeep-imgdec`를 반드시 함께 — 빠뜨려도 **오류가 안 난다**

본체는 이미지 파서를 링크하지 않는다(R-5 · 격리 디코드). 미리보기·아바타는 전부 **자식 프로세스**가
처리하고, 본체는 그것을 `current_exe()`의 **바로 옆 디렉터리**에서 찾는다
([imgdec.rs](../crates/nexa-beep/src/imgdec.rs)). 없으면 **경고 없이 이니셜 폴백**한다.

| | |
|---|---|
| 없으면 죽는 것 | 이미지 미리보기 · 아바타 실사진 |
| 없어도 도는 것 | 채팅 · 파일 전송 · 발견 · 무해화 |
| 그래서 | 컨테이너엔 **파일 하나가 아니라 디렉터리째** 마운트한다 |

**실측(2026-08-13 · linux/amd64)** — `nexa-beep` 2.96MB · `nbeep-imgdec` 589KB. (빌드 시간은 [§1-2](#1-2-linux-바이너리--상주-빌더권장))

---

## 2. 실행 유형 한눈에

| 명령 | 화면 | 네트워크 | stdin | 용도 |
|---|---|---|:---:|---|
| `nexa-beep` (무인자) | **GUI** | **실물** | — | ★ **표준 실행**(08-13 확정 — DR-1 "실행 = 참여". `--window --live` 동등 · 그전엔 인자 안내 출력이었다) |
| `-h` / `--help` | 없음 | — | — | 모드·옵션 안내(08-13 신설 · 미지 인자는 안내 후 종료 — GUI로 삼키지 않음) |
| `-V` / `--version` | 없음 | — | — | 버전(배포 검증 26 §7이 사용 — 무인자 스캐폴드에서 분리) |
| `--window` | GUI | **InMemory**(에코 봇 3명) | — | 오프라인 데모 — **외부와 통신 불가**(개발용 · 명시했을 때만) |
| `--window --live` | GUI | 실물 | — | 무인자와 동일(명시형) |
| `--separate-windows` | GUI(상대별 창) | InMemory | — | 다중 창 데모(DR-26) |
| `--chat-live [이름]` | 터미널 | 실물(발견 광고) | **필요** | ★ **CLI 단말**(08-20 일습 — 발견 조회·능동 연결·수신 전용 채널 · [§2-1](#2-1--chat-live-cli-단말-명령-일람-08-20)) |
| `--chat-serve [port]` | 터미널 | 실물(고정 포트) | **필요** | 발견 없이 기다리는 쪽(1:1) |
| `--chat-connect <host[:port]>` | 터미널 | 실물(수동 IP) | **필요** | 발견 없이 거는 쪽(DR-19) |
| `--serve [port]` / `--connect` | 터미널(로그) | 실물 | — | 헤드리스 에코 서버/클라이언트 |
| `--live-echo [초]` | 터미널(로그) | 실물(발견) | — | 발견→연결→인사·에코 왕복 |
| `--discover-probe [초]` | 터미널(로그) | 실물(발견) | — | 발견만 관찰(`SAW`·복제 경고) |
| `--quarantine-demo <파일>` | 터미널(로그) | — | — | 무해화 게이트 실측 |

**공통 옵션** — `--port <N>`(세션 수신 포트 · `--window`·`--chat-live`) · `--xfer-limit-mib <N>` ·
`--xfer-rate-kb <N>`(대화 모드 파일 전송).

### 2-1. ★ `--chat-live` CLI 단말 명령 일람 (08-20)

08-20 연쇄로 chat-live가 **단순 인바운드 대기**에서 **조회·선택·능동 연결이 되는 단말**로 확장됐다.
구조 원칙(사용자 확정): **수신은 전 채널 · 상호 채팅은 명시적으로 연결한 상대 1명**.

**대기 중 명령**

| 명령 | 동작 |
|---|---|
| `/peers` (`/list`) | 발견된 상대 **번호 목록** 조회(등장·이탈은 실시간 `[발견]`/`[이탈]`로도 안내) |
| `/connect <번호>` | 목록에서 골라 **1:1 대화 시작**(경로는 전송 계층이 안다) |
| `/connect <host[:port]>` | **IP 직접 연결**(DR-19 · 포트 생략 = 47200 — GUI ⌘K와 같은 정규화) |
| `/quit` | 종료 |

**수신 전용 채널(구조)** — 인바운드는 **전부 수락해 세워 둔다**: 상대 메시지는
`{지문}(수신 전용)> …`로 표시되고 **전달/읽음 확인(N-2)** 도 되쏜다. 파일 오퍼는 정중히
거절(승인 UI는 활성 대화 몫), 프로필 요청엔 이름만 응답, **대화 모드로 자동 진입하지 않는다**.
⚠ 세워 두는 이유: 거절하면 상대 GUI의 재연결 백오프가 실패로 보고 **무한 재시도**한다(08-20 실기 —
로그 홍수). `/connect`로 승격하면 수신 채널을 내리고 정식 1:1로 전환된다.

**대화 중 명령** — 한 줄 = 전송 · `/send <파일…>`(공백 구분 **다중** · 요청당 최대 5 ·
공백 경로는 한 줄 전체가 실재 파일이면 1개) · `/accept`/`/reject` = **요청(배치) 전체** 승인/거절
(GUI M4-2e 미러 — 목록·총합 표시 후 1회 결정) · `/quit` = **대화만 종료 → 발견 목록 복귀**
(이후 그 상대가 다시 걸어오면 수신 전용으로 붙는다 — 자동 대화 재진입 없음).

**수신/읽음 확인** — 수신 즉시 Delivered, 출력 즉시 Read를 되쏜다(터미널은 "창 가시성"
구분이 없다 — 출력 = 봄). 내 발신분의 확인은 `[확인] 전달됨/읽음 seq=N`으로 표시.

### 2-2. ★ GUI 대화창 명령 일람 (`/…`)

대화창 입력줄에서 `/`로 시작하면 **메시지가 아니라 명령**이다. CLI(`--chat-live`)와 같은
문법을 쓰되 목록은 다르다 — 판정은 [`nbeep-core::command`](../crates/nbeep-core/src/command.rs)
**한 곳**이라 1:1·그룹 방이 갈리지 않는다.

| 명령 | 별칭 | 동작 |
|---|---|---|
| `/help` | `/?` · `/명령` | 이 목록을 대화창에 출력 |
| `/fingerprint` | `/fp` · `/fpr` · `/지문값` | **내 지문 + 상대 지문**을 함께 출력(대조용) |
| `/verify` | `/verified` · `/sas` · `/지문` · `/대조` | 이 상대를 **대조 완료로 표시**(파란 실 배지 · 영속) |
| `/unverify` | `/cancelverify` · `/대조취소` · `/지문취소` | 대조 완료 **취소**(핀 상태로 강등) |
| `/trust` | `/신뢰` | 이 상대의 신뢰 상태 한 줄 |
| `/close` | `/quit` · `/exit` · `/q` · `/닫기` | 대화창 닫기(대화는 유지) |

**판정 규칙 — 첫 글자 하나로 갈린다**(08-16 사용자 확정 · 회귀 테스트로 고정)

| 입력 | 결과 | 왜 |
|---|---|---|
| `/help` | **명령** | 원본 첫 글자가 `/`이고 한 줄이고 아는 이름 |
| `" /help"`(앞 공백) | 메시지 | ★ **trim 전 원본**의 첫 글자로 본다 — 규칙이 "첫 글자"면 판정도 거기서 끝나야 예측된다 |
| `/help`⏎`둘째 줄` | 메시지(**전문 보존**) | ★ **명령은 한 줄** — 아니면 Shift+Enter 멀티라인의 뒷줄이 **조용히 사라진다**(08-16 실측으로 잡은 데이터 손실) |
| `//안녕` | 메시지 `/안녕` | escape — 없으면 `/`로 시작하는 문장을 영영 못 보낸다 |
| `경로는 /usr/bin` | 메시지 | 중간의 `/`는 명령이 아니다 |
| `/verifed`(오타) | **아무것도 안 함** + 안내 | ⚠️ 상대에게 보내면 사용자는 **명령이 실행된 줄 안다**(fail-closed) |

> ★ **`/verify`가 대조를 대신해 주지 않는다.** 안전한 순서는 `/fingerprint`로 값을 보고 →
> **전화·대면 등 다른 채널**로 상대와 맞춰 본 뒤 → `/verify`다. 인증하려는 그 통로 안에서
> "확인했어?"를 주고받아 승격하면 **중간자가 그 문답까지 대신**할 수 있어 SAS가 무의미해진다
> ([§9 알려진 한계](#9-알려진-한계) · [26 §8](#8-아직-눈으로-못-본-것)).
> 명령은 전부 **로컬**이다 — 어떤 명령도 와이어로 나가지 않는다.

**추천 유도** — 아직 대조 전(`Pinned`)인 상대와 1:1 대화를 열면 안내 줄이 **대화당 1회** 뜬다
(매번 뜨면 소음이 되고, 소음이 되면 읽지 않는다).

### ★ 이 넷만 알면 대부분 안 막힌다

**① `--live`가 없으면 밖과 못 붙는다.** 기본 `--window`는 InMemory 에코 봇이라 ⌘K 수동 연결도 안 먹는다.

**② 대화 모드는 stdin이 곧 수명이다.** `--chat-*` 셋은 stdin을 한 줄씩 읽고, **EOF = 종료**다(`Ctrl+D`와 같다).
백그라운드·파이프로 띄우면 이 일이 **조용히** 일어나 "붙자마자 끊긴다"로 보인다. 버그가 아니다 → [§6](#6-자주-막히는-곳--증상별).

**③ `--chat-live`는 죽지 않는다(08-13부터).** 핸드셰이크가 실패해도(포트 스캔·오연결) 그 연결만
버리고 계속 기다리고, 대화가 끝나도 대기로 돌아간다. 그 전에는 **`nc -z` 한 번에 프로세스가
종료**돼 "컨테이너가 자꾸 죽는다"로 보였다. 반면 `--chat-serve`는 **accept 1회**가 설계다([§9](#9-알려진-한계)).

**④ 발견이 닿으면 포트를 맞출 필요가 없다.** 발견 패킷이 **실제 바인딩된 포트**(`tcp_port`)를 나른다
([29 §3](29-wire-security-audit.md) 실측). 포트가 문제되는 곳은 **발견이 닿지 않는 곳뿐**이고,
거기서는 `--port`로 고정하고 그 값을 사람이 알려준다. 세션 기본 포트는 **47200**(`DEFAULT_SESSION_PORT` ·
점유 시 임의 폴백 · 설정 `net.session_port`).

---

## 3. 유형별 최소 절차

### 3-1. GUI 혼자 — 화면만 본다

```bash
./target/release/nexa-beep --window          # InMemory 에코 봇 3명
```

컨트롤 갤러리 `⌘/Ctrl+G` · 설정 `⌘/Ctrl+,` · 주소 추가 `⌘/Ctrl+K`.

### 3-2. GUI ↔ 터미널, 같은 PC — **가장 쉬운 실물 왕복**

```bash
./target/release/nexa-beep --window --live              # ① GUI
./target/release/nexa-beep --chat-live 테스트단말        # ② 다른 터미널
```

1. ②가 `[대기] '테스트단말'(me=…) 로 발견 광고 중…` 출력
2. **① 목록에 "테스트단말"이 뜬다** → 클릭 → 대화창(Noise 핸드셰이크·TOFU 핀)
3. 양쪽에서 타이핑 → Enter → 실시간 상호 수신
4. ② 종료 = `/quit` · `Ctrl+D` · `Ctrl+C`

같은 호스트라 멀티캐스트 루프백으로 발견된다. 안 뜨면 3-3으로.
②는 대기 중 **`/peers` = 발견 목록 · `/connect <번호|host[:port]>` = 골라서 대화 시작**
(08-20 — 번호는 발견 상대·주소는 DR-19 수동 등록과 같은 정규화 · 성립하면 즉시 대화 루프).
ℹ️ **Windows에서 ②를 cmd/PowerShell로 부르면 새 콘솔 창이 열려 거기서 대화한다**
(08-20 — 그 셸들은 GUI 앱을 기다리지 않아 프롬프트와 입력이 섞이므로 앱이 자기
콘솔을 새로 연다). Git Bash·파이프는 그 자리 그대로 → [§7 유의](#7-배포본-실기-검증-windows).

### 3-3. IP로 직접 — 발견이 막힌 곳

```bash
./target/release/nexa-beep --chat-serve 47300           # ① 기다리는 쪽
./target/release/nexa-beep --chat-connect 127.0.0.1:47300   # ② 거는 쪽(또는 GUI ⌘K)
```

GUI에서는 `⌘/Ctrl+K` → 주소 입력 → Enter. **GUI는 포트를 생략하면 `:47200`을 붙인다**
(`10.0.0.5` = `10.0.0.5:47200`). ⚠️ CLI는 아직 보완하지 않는다 → [§6](#6-자주-막히는-곳--증상별).

### 3-4. ★ 08-20 신기능 3종 실기 절차 — 오프라인 큐 · 클립보드 이미지 · 등급/공지

> 준비 = GUI 2개(또는 GUI+chat-live). 같은 PC면 `./tools/relaunch.sh`가 3신원을 띄운다.

**① 오프라인 큐(M4-6 — 세션 없는 발신 = 보관·자동 전달 · 재시작 유지)**

1. A↔B 대화를 연 상태에서 **B를 종료**한다(세션 끊김 — A의 대화창은 열려 있음).
2. A에서 메시지 2~3개 입력 → 풍선 오른쪽에 **가로 막대**(= 대기 · 점(전달)·점2(읽음)와
   모양이 다르다) · 상태바 = "전송 대기 저장(N건) — **내 PC가 켜져 있고 상대가 나타나면**
   자동 전달됩니다"(이 한계 문구가 이 기능의 정직성이다 — Q-25-2).
3. **A를 재시작**한다 → 대화창을 다시 열면 대기 풍선이 그대로 살아 있다(영속 =
   `data/pending/{지문}.seg` 봉인 — 평문으로 저장되지 않는다).
4. **B를 다시 실행** → 발견 즉시 A가 자동 연결·전달: 막대가 **점(전달됨)으로 바뀌고**
   상태바 = "대기 메시지 N건 전달됨" · B에는 메시지가 정상 도착.
5. 확인 포인트: 순서 보존 · B 쪽 전달/읽음 마크 정상 · A 재시작 없이도(3 생략) 동일 동작.

**② 클립보드 이미지 전송(3-OS · Windows 실기 가능)**

1. `Win+Shift+S`(캡처 도구)로 화면 일부를 캡처(클립보드에 이미지).
2. 대화창 입력줄에서 **Ctrl+V**(또는 우클릭 ▸ 붙여넣기) — 텍스트가 클립보드에 없을 때만
   이미지 폴백이 발화한다(텍스트가 있으면 텍스트 붙여넣기 그대로).
3. 잠깐(imgdec 인코딩 워커) 뒤 **파일 전송 요청이 자동 시작** — 파일명 `clip-{ms}.png` ·
   수신측은 여느 파일과 같은 **승인 → 격리 → 미리보기**(imgdec 썸네일·확대) 흐름.
4. 확인 포인트: 4K 캡처도 UI 무정지(워커) · 수신 미리보기 색상 정상(BGR→RGB 변환) ·
   저장 위치 = `data/clipboard/`(발신측 사본).
5. mac/Linux: 같은 Ctrl(⌘)+V — mac은 스크린샷 후 붙여넣기, Linux는 wl-paste/xclip 필요.
   (구현 완료 · 실기는 해당 OS에서 — 잔여)

**③ 메시지 등급 + 공지(D-1 · FR-M-6 — docs/24 확정 매핑)**

1. **배지**: 대화창 입력줄 **오른쪽 "일반" 칩**을 클릭 = 일반→**알림**→**긴급** 순환.
   긴급 진입 시 상태바 경고(마찰 1단계) · 선택은 **다음 전송 1회에만** 적용 후 일반 복귀.
2. **명령**: `/notice 내용` · `/urgent 내용`(별칭 `/알림`·`/긴급`) — 빈 본문·여러 줄이면
   보내지 않고 사용법 안내(fail-closed).
3. 전송하면 **풍선에 등급 링**(알림 = 호박색 · 긴급 = 경고색 외곽) — 발신·수신 양쪽.
4. **수신 강도**(docs/24 §3-3): 미검증 상대 = 종전대로 **무음**(자동 강등) · 핀/검증
   상대의 **긴급 = 앱이 앞에 있어도 알림이 뜬다**(일반·알림은 뒤에 있을 때만).
5. **공지**: 메뉴 ▸ **공지 보내기** → 본문 입력 → 확인 = **발견된 전체**에게 알림 등급
   1:1 팬아웃. 연결 안 된 상대 몫은 **①의 오프라인 큐로 자동 편입**(상태바 "즉시 N ·
   대기 M") — 나중에 나타나면 그때 전달된다. 긴급 공지는 의도적으로 없다(팬아웃
   Urgent 강등 규칙의 발신측 준수).

### 3-4. ★ 여러 신원을 한 PC에서 동시에 — **폴더를 나눈다**

> 그룹(3인 이상) 실기는 전용 안내서가 있다 → **[33 그룹 채팅 테스트](33-group-chat-test-guide.md)**.
> 종료·재빌드·3신원 기동을 한 번에 하려면 **`./tools/relaunch.sh`**(기본 = 신원 보존).

한 대에서 2명(이상)을 흉내 내야 할 때가 있다 — 그룹 초대 왕복, 대화 상대 여럿, TOFU 핀 확인.
**같은 실행 파일을 폴더만 나눠 복사하면 각자 다른 신원이 된다.**

```bash
S=$HOME/.nexa-beep-multi; mkdir -p $S/A $S/B          # ★ /tmp 금지(아래 경고)
for d in A B; do cp target/release/{nexa-beep,nbeep-imgdec} $S/$d/; done   # imgdec 동거(§1-3)

( cd $S/A && ./nexa-beep --window --live ) &      # 신원 A
( cd $S/B && ./nexa-beep --window --live ) &      # 신원 B
```

> ⚠️ **data 폴더를 `/tmp`(= `/private/tmp`) 아래에 두지 말 것**(08-19 실기 진단). macOS
> `com.apple.tmp_cleaner`(`/usr/libexec/tmp_cleaner` · **매일 자정 · 3일 미접근 항목 삭제**)가
> `identity.key`를 조용히 지워, 다음 기동에 **새 신원이 생성**된다(지문이 바뀐다). 그러면
> 이전 신원 키에 sealed된 **격리물·핀을 현재 키로 못 연다**(도메인 분리·fail-closed = 크립토
> 셰레딩 → 격리함이 빈 목록으로 보인다). **홈 아래 durable 폴더**(`$HOME/.nexa-beep-multi`)를
> 쓴다. 각 실행 파일이 실제 로드할 신원은 **`nexa-beep --whoami`**(지문·이름·exe·data 경로 ·
> 읽기 전용 = 키 생성 안 함)로 확인한다. `relaunch.sh`는 이 durable 경로가 기본이며 기존
> `/tmp/beep-multi`를 1회 자동 이관한다.

두 창이 뜨고 **서로를 발견해 목록에 잡는다**(같은 호스트 = 멀티캐스트 루프백 · §3-2와 같은 원리).

**왜 갈리나 — 포터블 규칙의 부수 효과(DR-4)**

`data_dir()`는 **① `실행파일 옆/data`가 쓰기 가능하면 거기** → ② 사용자 설정 디렉터리 → ③ 임시 폴더
순으로 고른다([app.rs](../crates/nexa-beep/src/app.rs)). 폴더를 나누면 ①에서 갈리므로
**신원 키·핀·그룹·설정이 통째로 분리**된다.

> ⚠️ **③(임시 폴더)은 제품에도 남아 있는 같은 함정이다** — ①②가 모두 쓰기 불가면 앱이 스스로
> 임시 폴더를 고르는데, 그 자리는 위 경고대로 `identity.key`가 지워지는 곳이다(신원이 조용히
> 바뀐다). 실기 폴더만 옮겨서는 절반만 막은 것 → 정정 대상 **[TODO M5-4e](TODO.md)**.

| 파일 | 무엇 |
|---|---|
| `data/identity.key` | 신원(= `PeerId`) — 이게 갈려서 서로 남이 된다 |
| `data/trust.seg` | TOFU 핀 |
| `data/groups.seg` | 그룹 |
| `data/settings.cfg` · `data/profiles/` | 설정·프로필 |

> ⚠️ **CLI 대화·프로브 모드는 매 실행 새 신원이다**(임시). `--chat-live`·`--chat-connect`·
> `--discover-probe`는 `data/`를 만들지 않는다 — **폴더를 나눌 필요도 없고, 나눠도 영속되지 않는다.**
> **폴더 분리가 의미 있는 건 GUI(무인자·`--window --live`)뿐이다.**

**실측(2026-08-13)**

| 확인 | 결과 |
|---|---|
| 폴더 A·B GUI 기동 | 각자 `data/identity.key` 생성 — **해시 상이**(`bce8c448…` vs `5fd2006b…`) |
| `--discover-probe` 재실행 | `me=` 가 매번 바뀐다(`69859aa1` → `79b9b469`) = **임시 신원 확인** |

**정리** — `rm -rf $S`. ⚠️ 저장소 빌드(`target/release/`)도 **`target/release/data/`를 쓴다** —
그게 "기본 신원"이다. 초기화하려면 그 폴더를 지운다(핀·그룹·설정도 함께 사라진다).

### 3-5. 헤드리스 관찰 — 사람 없이 확인

```bash
./target/release/nexa-beep --discover-probe 8   # 누가 보이나 · ⚠️CLONE 경고(D-22)
./target/release/nexa-beep --live-echo 8        # 발견→Noise→에코 왕복
./target/release/nexa-beep --serve 47200        # GUI가 붙을 에코 서버
```

프로브 둘을 동시에 띄우면 서로 발견한다:

```bash
./target/release/nexa-beep --discover-probe 6 & ./target/release/nexa-beep --discover-probe 6 & wait
```

### 3-6. 파일 전송 — 대화 중 명령

3-2나 3-3으로 대화가 열린 상태에서:

| 명령 | 뜻 |
|---|---|
| `/send <파일>` | 전송 제안(협상 → 상대 승인 대기) |
| `/accept` · `/reject` | 수신측 결정 — 승인해야 `.beepq` 격리에서 실체화된다(DR-13) |
| `/help` · `/quit` | 도움말 · 종료 |

수신 상한은 **수신측**이 정한다(`--xfer-limit-mib` · 기본 256MiB). 무해화 게이트만 따로 보려면
`--quarantine-demo <파일>`.

---

## 4. 크로스플랫폼 — 맥 ↔ Docker Linux

⚠️ **맥 호스트와 컨테이너는 서로 발견되지 않는다.** Docker Desktop은 내부 Linux VM이라 브로드캐스트
도메인이 다르다. **포트 매핑 + IP 직접**으로 우회한다(컨테이너끼리는 발견된다 → 4-2).

### 4-1. ★ 표준 왕복 — 맥 GUI ↔ 리눅스 콘솔 (2026-08-13 실측)

`--chat-live`를 **포트 고정**으로 띄워, 발견이 닿지 않는 상대와 잇는 표준 케이스.

```bash
cd <저장소 루트>          # ★ $PWD 가 저장소여야 한다 → §6

# ① 맥 — GUI
./target/release/nexa-beep --window --live

# ② 리눅스 — 포트 고정 + 인터랙티브. 디렉터리째 마운트(imgdec 동거 · §1-3)
docker run -dit --name beep_lin --init -p 43211:43211 \
  -v "$PWD/.docker-target/release:/opt/beep:ro" \
  debian:stable-slim /opt/beep/nexa-beep --chat-live --port 43211 테스트단말

docker attach beep_lin    # 콘솔 접속 — 여기서 타이핑하면 전송
```

②가 내야 하는 두 줄:

```
[대기] '테스트단말'(me=9c37bba8) 로 발견 광고 중 …
[포트] 세션 수신 43211 — 발견이 닿지 않는 상대에겐 `--chat-connect <내IP>:43211` 로 알려준다
```

> ★ **두 번째 줄이 요점** — 실제 리슨 포트를 찍는다. 발견이 안 닿는 상대에겐 **사람이 이 값을 전해야**
> 수동 연결이 성립한다(ADR-0006 §3-1). 옵션이 이름보다 앞에 와도 된다(`--port 43211 테스트단말`).

**③ 잇는다** — 맥 GUI `⌘/Ctrl+K` → `127.0.0.1:43211`(`-p`로 게시했으므로 루프백으로 닿는다).
반대 방향은 컨테이너에서 `--chat-connect host.docker.internal:47200`.

**실측 결과**

| 확인 | 결과 |
|---|---|
| 자식 바이너리 동거 | `docker exec beep_lin ls /opt/beep` → 둘 다 존재 |
| 포트 고정 | `[포트] 세션 수신 43211` |
| 맥 → 리눅스 | `4b00d4d1> 맥에서 보냄` |
| 리눅스 → 맥 | `a3ae162a> 리눅스 콘솔에서 보냄` |
| 프로필 교환 | 양방향 자동 프리페치 · 이미지 70,000B |

정리: `docker rm -f beep_lin`

### 4-2. Linux ↔ Linux — 발견 테스트베드

컨테이너끼리는 같은 브리지에서 **멀티캐스트 발견이 된다**.

```bash
docker network create beepnet
for n in a b; do
  docker run -d --name node_$n --network beepnet \
    -v "$PWD/.docker-target/release:/opt/beep:ro" \
    debian:stable-slim /opt/beep/nexa-beep --live-echo 15
done
sleep 12; docker logs node_a; docker logs node_b     # CLIENT got reply / SERVER recv
docker rm -f node_a node_b; docker network rm beepnet
```

**실측** — 시작 즉시 상호 발견 → 인사 → 에코 왕복(양방향 로그).

### 4-3. 포트 대조군 — "포트가 진짜로 쓰이는가"

세 줄을 **같이** 봐야 증명된다(2026-08-13 실측).

| 시험 | 명령 | 결과 | 뜻 |
|---|---|---|---|
| 명시 포트 | `--chat-connect $IP:47999` | ✅ 연결 | 어떤 포트든 붙는다 |
| 포트 생략 | `--chat-connect $IP` | ❌ `BadAddress` | ⚠️ CLI 미보완 → [§6](#6-자주-막히는-곳--증상별) |
| 발견 경유 | 한쪽 `--chat-live`, 한쪽 `--live-echo` | ✅ 연결 | **임의 포트인데도 찾아 붙는다** = §2 규칙 ④ |

---

## 5. 종료

| 대상 | 종료 방법 |
|---|---|
| GUI 창 | 창 닫기 · `SIGTERM`(실측 0.28초 — GOODBYE·정리) |
| `--serve`/`--live-echo`/`--discover-probe` | `Ctrl+C` |
| `--chat-*`(인터랙티브) | `/quit`(`/exit`·`/q`) · `Ctrl+D` · `Ctrl+C` — **상대를 기다리는 중에도 된다**(08-11) |
| 컨테이너 | **`docker run --init` 필수** — 없으면 `docker stop`이 10초(SIGKILL 대기). `--init` 있으면 **0.38초** |

> 대기 구간이 `accept()`/`recv()`에 갇혀 **키를 읽는 쪽이 없던** 것이 과거 "종료가 안 된다"의 실체였다.
> 남은 `Ctrl+C`가 `Drop`을 건너뛰어 터미널을 raw 모드로 남겼다. 지금은 종료 포트(`plat::shutdown`)를
> 채팅 모드에도 걸어 `Ctrl+C`도 `Drop`을 거쳐 터미널을 복원한다(R-16 · FR-P-7).
>
> ⚠️ **파이프 입력(비-TTY)에서는 대기 중 `/quit`을 읽지 않는다** — 자동화가 보낸 줄은 대화용이지
> 대기용이 아니다. 그 경우엔 `SIGTERM`/`SIGINT`로 끝낸다.

---

## 6. 자주 막히는 곳 — 증상별

| 증상 | 원인 | 조치 |
|---|---|---|
| **붙자마자 "상대와의 세션이 종료됨"** | 대화 모드를 `-d`(백그라운드)·파이프로 띄웠다. **stdin EOF = 종료**(§2 규칙 ②) | 컨테이너는 `-it`(사람) / `-i`(스크립트). 자동화는 아래 FIFO |
| **`exec /nexa-beep failed: Permission denied`** | `$PWD`가 저장소가 아니다. Docker는 **없는 호스트 경로를 마운트하면 빈 디렉터리를 만든다** → 파일이 아니라 디렉터리를 exec | `rm -rf ~/.docker-target` 후 **저장소 루트에서** 재실행. 바이너리 문제가 아니다 |
| **`cannot attach stdin to a TTY-enabled container`** | `-dit` 컨테이너에 **파이프로** 붙으려 했다 | 사람은 `docker attach`를 직접 친다. 자동화는 `-t`를 빼고 **`-di`** 로 띄운다 |
| **`--chat-connect 10.0.0.5` → `BadAddress`** | ⚠️ **포트 생략 보완이 GUI 모달에만 있다** | CLI는 `:47200`을 직접 쓴다. *(미해결 — CLI에도 같은 정규화 필요)* |
| **프로브의 `from=`으로 연결 실패** | `from=`은 **UDP 발신 주소**지 TCP 포트가 아니다 | `tcp=` 필드(M1-15 · 08-15 해소 — 광고된 세션 수신 포트)를 쓴다: 연결 주소 = **from의 IP + tcp의 포트** |
| **GUI에서 밖이 안 보인다** | `--live`가 빠졌다(§2 규칙 ①) | `--window --live` |
| **컨테이너에서 창이 안 뜬다** | 헤드리스라 디스플레이·폰트가 없다. ⚠️ **무인자도 GUI다**(08-13) | 08-13부터 **사유와 대안을 콘솔에 안내하고 exit 3** 으로 끝난다(그전엔 `Exited (134)`). 컨테이너는 `--chat-live --port N`(§4-1) |
| **SSH·원격 PowerShell에서 창이 안 뜬다** | 창 서버(mac)·디스플레이(Linux)·대화형 데스크톱(Windows)이 없는 세션 | 안내대로 터미널 단말을 쓰거나, Linux는 `ssh -X`, Windows는 **RDP**(RDP는 창이 뜬다) |
| **맥 GUI에 컨테이너가 안 뜬다** | 정상 — 브로드캐스트 도메인이 다르다(§4) | IP 직접(4-1) |
| **이미지가 안 보인다** | `nbeep-imgdec`이 본체 옆에 없다. **경고가 안 난다**(§1-3) | 같이 빌드 + 디렉터리째 마운트 |
| **컨테이너가 `Ctrl+C`로 안 죽는다** | `--init` 없음(PID 1은 기본 동작을 건너뛴다) | `docker run --init` |
| **`docker attach`에서 나오려다 앱을 죽였다** | attach 중 `Ctrl+C`는 앱에 `SIGINT`로 간다 | 떼기만 하려면 **`Ctrl+P` → `Ctrl+Q`**(컨테이너는 계속 돈다) |
| **빌드했는데 옛 동작 그대로** | 상주 빌더의 산출물을 `.docker-target`으로 **복사하지 않았다** | 빌드와 복사를 `&&`로 묶는다([§1-2](#1-2-linux-바이너리--상주-빌더권장) ②) |

**자동화에서 세션을 유지하려면** — stdin 쓰기 끝을 붙잡아 EOF를 막는다.

```bash
mkfifo /tmp/beep.in
( sleep 3600 > /tmp/beep.in & )                    # ← 쓰기 끝 유지
./target/release/nexa-beep --chat-serve 47200 < /tmp/beep.in &
echo "한 줄 보낸다" > /tmp/beep.in                  # 아무 때나 밀어 넣는다
```

컨테이너 자동화는 `-di` + `docker attach`가 더 단순하다:

```bash
printf '리눅스 콘솔에서 보냄\n' | docker attach --sig-proxy=false beep_lin
```

---

## 7. 배포본 실기 검증 (Windows)

릴리스 자산은 **CI가 만들기만 하고 실행 안 해 본** 상태로 나간다 — 실기 검증이 게시(M5-4b)의 선행 조건이다.

```powershell
# ① 무결성 — 자산 + SHA256SUMS.txt 대조(Get-FileHash)
gh release download <태그> -R SosomLab/nexa-beep -p "*windows-x64-*" -p "SHA256SUMS.txt"

# ② 포터블 — 셸(Explorer) 해제로 MotW 전파 확인 (Expand-Archive는 MotW를 안 옮긴다)
#    exe·문서에 Zone.Identifier(ZoneId=3)가 붙는지 → --version 실행

# ③ 설치본 — 무인 설치·업그레이드·완전 제거 (전부 사용자 단위 HKCU·무권한)
Start-Process <구버전>-setup.exe /S -Wait
Start-Process <신버전>-setup.exe /S -Wait
Start-Process "$env:LOCALAPPDATA\Programs\NexaBeep\uninstall.exe" /S -Wait
#    확인: HKCU\…\Uninstall\NexaBeep(DisplayVersion) · 시작 메뉴 .lnk · 제거 후 잔재 0

# ④ 매니페스트 정합 — winget 스키마 + 해시
winget validate --manifest manifests\winget\manifests\s\SosomLab\NexaBeep\<버전>
```

> ★ **08-20 — Windows exe는 windows 서브시스템**(콘솔 창 근절 — Windows Terminal에선
> `FreeConsole`로도 빈 터미널 창이 남던 실기 · main.rs `windows_subsystem` + `attach_parent_console`).
> 검증 시 유의: **PowerShell은 GUI 앱을 기다리지 않아 `--version > 파일`이 비어 나온다** —
> 파이프(`| Out-String`)·`cmd /c "… > 파일"`·bash 리다이렉트는 전부 정상(실측 08-20).
> 인터랙티브 CLI(`--chat-live`·`--chat-serve`·`--chat-connect`)를 cmd/pwsh에서 부르면
> **자기 콘솔 창을 새로 열어 계속한다**(입력 경합 차단 — 08-20 실기 후속). Git Bash·
> 파이프/스크립트는 그 자리 그대로(bash는 서브시스템 무관하게 자식을 기다린다).

**실측(v0.1.2 · win x64)** — 체크섬 3/3 · MotW 전파 정상 · 설치/업그레이드/제거 통과 · PATH 미등록(설계) ·
winget validate 통과 · 해시 일치.
🔴 **결함 1건**: 포터블 exe가 콘솔 서브시스템 + 인자 없음이라 **더블클릭 시 콘솔 번쩍 후 종료**
(시작 메뉴 바로가기 동일 · macOS 번들 버그의 쌍둥이 — M5-4d).
⏸ SmartScreen 마찰은 헤드리스로 관측 불가(M5-4a 실기 날) · arm64 실행 미검증.

---

## 7-1. 08-15 배치 Windows 실기 체크리스트 (사용자 실기 예정)

> 08-14~15 이틀 배치(프로필 캐시·전파 → 목록 정렬·핀 → IME 설정화 → IconDropdown →
> 설정 고급 → 보안 4건)의 **Windows 확인 항목**. mac·CI(3-OS)는 검증 완료 —
> 여기는 Windows 실기에서만 보이는 것들이다. 문제 발견 시 증상만 기록하면
> mac 세션이 수정한다.

| # | 항목 | 확인 방법 | 기대 |
|---|---|---|---|
| **W-A** ✅ | ★ **저장소 마이그레이션**(최우선 — 기존 데이터 지우기 전에) | 기존 `data/`를 가진 채 새 빌드 **첫 실행** | **통과(08-15)** — 헤더 실측 `NBGS 03`(v2→**v3**)·`NBTS 02`(v1→**v2**) 3신원 전부 · `identity.key` **무변경**(mtime 08-11/08-13 · PeerId가 발견 결과와 일치) · trust.seg 828→**1179B**(초기화면 줄었을 값 · fail-closed 잠김 아님) |
| **W-B** ✅ | imgdec 권한 강등 실기(R-5) | 프로필 실사진·수신 이미지 미리보기 | **통과(08-15 · 실바이너리)** — stderr `lockdown = windows-mitigation(dynamic-code·image-load)` 확인 · PNG→NIMG 256²/64² **바이트 정합** · **쓰레기·빈 입력 = exit 2 · 출력 0B**(fail-closed). ⇒ 강등이 디코드를 막지 않는다 |
| **W-C** ✅ | 툴바 정렬 드롭다운(IconDropdown) | 4번째 슬롯(호박색 ▼) 클릭 → 4항목 선택 | **통과(08-15 · 사용자)** — 팝업 렌더·선택 즉시 재정렬 확인 · 영속은 `ui.list_sort=chat` 실측 |
| **W-D** ✅ | 목록 정렬·핀 | 1~2분 관찰 + 우클릭 "목록 상단에 고정" | **통과(08-15 · 사용자)** — 순서 변경·**깜빡임 없음** 확인. 창 위치 영속도 실측(`ui.win_x/y/w/h` — 08-14 HIDDEN_KEYS 구멍이 막혔다) |
| **W-E** ⏸ | 2-PC 프로필 전파(M3-21 · Win↔Mac) | Windows에서 이메일 공유 토글 | **미확인 — Mac이 같은 망에 있어야 한다**(유일한 잔여). Mac 상대 카드 **즉시 반영**(역방향도) · 카드에 보더 색·최근 접속/대화 표시 · **비연결 상대 카드 = 조용한 연결**(대화창이 열리면 안 됨) |
| **W-F** ✅ | 캐러셀 스크롤 방향 | 프로필 스와치/최근 이미지 가로 스크롤 | **통과(08-15 · 사용자)** — Windows 기본 방향이 기획대로 동작 |
| **W-G** ✅ | 그룹 구성원 모달(G4) | 그룹 아이콘/방 헤더 클릭 · 소유자면 행 클릭 | **통과(08-15 · 사용자)** — 3신원으로 그룹 생성·메시지 전달 확인 |
| **W-H** ✅ | 설정 신설 3종 | 설정 열기 | **통과(08-15)** — 저장 실측: `ime.*` **9종**+토글 2 · `ui.tooltip_ms`·`ui.carousel_scroll`·`ui.list_refresh_scroll` · 키 68개 **중복 0** |

> **결과(08-15 · 빌드 `2512beb`)** — **8건 중 7건 통과 · 잔여는 W-E 하나**(2-PC라 Windows 단독 불가).
> W-A·W-B·W-H는 헤드리스 실측(파일 헤더·실바이너리·settings.cfg), W-C·W-D·W-F·W-G는 사용자 육안.
> ⚠️ 실기 후 `8d1b046`·`86b045a`(모달 위치·소유) 3커밋이 더 들어왔다 — **모달 z순서는 재확인 대상**.

기존 백로그(여유 시): [WIME-2·7~11](TODO.md)(조합 전 첫 키 유출·후보창·빠른 연타) ·
[WGUI-1~6](TODO.md)(GUI 세션 판정 — RDP·서비스 세션 안내 후 종료) · 방화벽 프롬프트.

## 8. 아직 눈으로 못 본 것

> **"코드가 있다"와 "동작을 봤다"는 다르다.** 확인된 것은 [DEVLOG](DEVLOG.md)·[journal/](journal/)에
> 날짜별로 있으니, 여기엔 **남은 것만** 둔다. 살아 있는 목록은 [TODO](TODO.md)다.

**🖥 육안(GUI) 확인 필요** — 헤드리스로 대행 불가

| 항목 | 비고 |
|---|---|
| `FingerprintVerified` 배지(3번째) | ✅ **도달 가능**(v0.1.7 카드 대조 완료 버튼 → 08-18 **지문 비교로 전환**): `/fingerprint` 로 키 지문 확인 → 다른 채널 대조 → `/verify` **직접 대조 완료**(파란 실 배지) · `/unverify` 강등 |
| ~~상대별 별도 창(`--separate-windows`)~~ | ✅ **육안 확인(08-13 · 사용자 · Windows)** — 다중 창 동작(M3-12 완결). 같은 날 관찰: 주소 입력 창은 **모달리스**(AlwaysOnTop뿐 · P3 — M3-16) |
| ~~Windows 목록 타입어헤드 **한/영 토글·한글 조합**([27 §8](27-typeahead-hangul-composition.md))~~ | ✅ **실기 확인(08-13 · 사용자)** — 한/영 키 도달(IME 비연결)·상태바 고지·조합·복귀 동작 |
| 설정 화면(트리·검색·즉시 적용) · 컨트롤 갤러리 | M3-11 · M3-1 |
| i18n 전환(영/한/중/일) · 라이트 테마 · 고DPI 배율 | M3-4 · FR-U-5/6 |
| 한글 IME **조합 중** 프리에딧 | 한글 입력·전송 자체는 실증(FR-U-7) |

**🔬 시나리오 재현 필요**

| 항목 | 왜 아직 못 봤나 |
|---|---|
| 클론 탐지 U-P1 · 자기 `PeerId` 거부 U-P2 | 신원 키를 실제로 복제한 환경이 필요(D-22 · R-12) |
| TOFU 이름 재사용 경고(`name_conflict`) | 같은 이름·다른 키를 순차로 만나야 한다 |
| SAS 60자리 양쪽 일치 | 구현·단위 테스트됐으나 **출력 지점이 없다**(M3-6) |
| 백프레셔·프레임 상한(M2-3) · 중복 제거(FR-M-9) | 부하·다중 경로 시나리오 미구성 |

**⏸ 실기 필요(대행 불가)** — D-8b 타이밍(E-1~E-9 · 2대 이상) · 클라이언트 격리 AP에서 S4(R-2) ·
Windows IME 실기(M3-3b · WIME-1~11) · 24h 누수(E-9) · **mac 유휴 RSS 재실측**(R-8 잔여 축).

---

## 9. 알려진 한계

| 한계 | 내용 · 참조 |
|---|---|
| **맥 ↔ Docker 발견 불가** | Docker Desktop 내부 VM 구조. IP 직접(§4-1)으로 우회 · 맥↔실제 리눅스는 같은 공유기면 발견된다(D-8b) |
| **타이밍은 잠정치** | announce 주기·타임아웃·그룹/포트/TTL은 D-8b 실기 실측 전 잠정값([08 §8](08-adr-0002-discovery-transport.md)) |
| **CLI 채팅은 1:1** | `--chat-serve`는 accept 1회 · 인터랙티브는 사람 타이핑 전제([18 §2-2](18-build-and-test.md)) |
| **SAS 미배선** | 60자리 대조 출력 지점이 없다. 화면의 `me`↔`상대` 교차 대조는 **MITM 방어가 아니다**(M3-6) |
| **대화 기록 미영속** | 신원·핀은 영속된다(M2-5a — 재시작해도 `Pinned` 유지). **대화 내용 저장은 M2-5b**(🔴 D-18 §4~ 대기) |
