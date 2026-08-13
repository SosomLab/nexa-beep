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
| `--chat-live [이름]` | 터미널 | 실물(발견 광고) | **필요** | GUI 목록에 뜨는 터미널 단말 |
| `--chat-serve [port]` | 터미널 | 실물(고정 포트) | **필요** | 발견 없이 기다리는 쪽(1:1) |
| `--chat-connect <host[:port]>` | 터미널 | 실물(수동 IP) | **필요** | 발견 없이 거는 쪽(DR-19) |
| `--serve [port]` / `--connect` | 터미널(로그) | 실물 | — | 헤드리스 에코 서버/클라이언트 |
| `--live-echo [초]` | 터미널(로그) | 실물(발견) | — | 발견→연결→인사·에코 왕복 |
| `--discover-probe [초]` | 터미널(로그) | 실물(발견) | — | 발견만 관찰(`SAW`·복제 경고) |
| `--quarantine-demo <파일>` | 터미널(로그) | — | — | 무해화 게이트 실측 |

**공통 옵션** — `--port <N>`(세션 수신 포트 · `--window`·`--chat-live`) · `--xfer-limit-mib <N>` ·
`--xfer-rate-kb <N>`(대화 모드 파일 전송).

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

### 3-3. IP로 직접 — 발견이 막힌 곳

```bash
./target/release/nexa-beep --chat-serve 47300           # ① 기다리는 쪽
./target/release/nexa-beep --chat-connect 127.0.0.1:47300   # ② 거는 쪽(또는 GUI ⌘K)
```

GUI에서는 `⌘/Ctrl+K` → 주소 입력 → Enter. **GUI는 포트를 생략하면 `:47200`을 붙인다**
(`10.0.0.5` = `10.0.0.5:47200`). ⚠️ CLI는 아직 보완하지 않는다 → [§6](#6-자주-막히는-곳--증상별).

### 3-4. 헤드리스 관찰 — 사람 없이 확인

```bash
./target/release/nexa-beep --discover-probe 8   # 누가 보이나 · ⚠️CLONE 경고(D-22)
./target/release/nexa-beep --live-echo 8        # 발견→Noise→에코 왕복
./target/release/nexa-beep --serve 47200        # GUI가 붙을 에코 서버
```

프로브 둘을 동시에 띄우면 서로 발견한다:

```bash
./target/release/nexa-beep --discover-probe 6 & ./target/release/nexa-beep --discover-probe 6 & wait
```

### 3-5. 파일 전송 — 대화 중 명령

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
| **프로브의 `from=`으로 연결 실패** | `from=`은 **UDP 발신 주소**지 TCP 포트가 아니다 | 상대의 `[포트] 세션 수신 N` 값을 쓴다. *(미해결 — 프로브가 `tcp_port`를 아직 출력 안 함)* |
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

**실측(v0.1.2 · win x64)** — 체크섬 3/3 · MotW 전파 정상 · 설치/업그레이드/제거 통과 · PATH 미등록(설계) ·
winget validate 통과 · 해시 일치.
🔴 **결함 1건**: 포터블 exe가 콘솔 서브시스템 + 인자 없음이라 **더블클릭 시 콘솔 번쩍 후 종료**
(시작 메뉴 바로가기 동일 · macOS 번들 버그의 쌍둥이 — M5-4d).
⏸ SmartScreen 마찰은 헤드리스로 관측 불가(M5-4a 실기 날) · arm64 실행 미검증.

---

## 8. 아직 눈으로 못 본 것

> **"코드가 있다"와 "동작을 봤다"는 다르다.** 확인된 것은 [DEVLOG](DEVLOG.md)·[journal/](journal/)에
> 날짜별로 있으니, 여기엔 **남은 것만** 둔다. 살아 있는 목록은 [TODO](TODO.md)다.

**🖥 육안(GUI) 확인 필요** — 헤드리스로 대행 불가

| 항목 | 비고 |
|---|---|
| `FingerprintVerified` 배지(3번째) | `Unverified`·`Pinned` 2종은 실증. **3번째는 SAS 대조 UX가 없어 도달 불가**(확인 문제가 아니라 미구현 — M3-6) |
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
