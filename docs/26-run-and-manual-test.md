# 26. 실행·수동 테스트 가이드

> **목적**: `nexa-beep` 바이너리의 **실행 모드 전체**와, 사람이 직접 돌려 보는 **수동 테스트 시나리오**를 한 곳에. 명령·빌드 SSOT는 [18](18-build-and-test.md), CLI 대화 상세·제약은 [18 §2-2](18-build-and-test.md). 이 문서는 "무엇을 어떻게 띄워 테스트하나"의 지도다.
> 실측 기록은 [journal/2026-08-08](journal/2026-08-08.md). 발견 사다리 근거는 [06 §4](06-network-stack.md), 수동 연결은 [19 ADR-0006](19-adr-0006-manual-endpoint.md).

## 0. 준비 — 빌드

```bash
# 맥(호스트) 릴리스
cargo build --release -p nexa-beep          # → target/release/nexa-beep

# Linux(컨테이너) 릴리스 — Docker 테스트용(별도 타깃 디렉터리로 호스트 캐시와 분리)
docker run --rm -v "$PWD":/src -w /src -e CARGO_TARGET_DIR=/src/.docker-target \
  rust:1-slim bash -c 'apt-get update -qq && apt-get install -y -qq pkg-config >/dev/null; \
  cargo build --release -p nexa-beep'
#   → .docker-target/release/nexa-beep  (linux/amd64)
```

## 1. 실행 모드 한눈에

| 명령 | 화면 | 네트워크 | 용도 |
|---|---|---|---|
| `nexa-beep` | 없음(스캐폴드 출력) | — | 인자 안내 |
| `nexa-beep --window` | GUI 창 | **InMemory**(에코 봇 3명) | 오프라인 데모 — 외부와 통신 불가 |
| `nexa-beep --window --live` | GUI 창 | **실물**(LocalDirect) | 같은 LAN·컨테이너의 실제 상대와 대화 |
| `nexa-beep --separate-windows` | GUI(상대별 창) | InMemory | 동시 다중 대화 데모(창 모드 옵션 DR-26) |
| `nexa-beep --chat-live [이름]` | 터미널(인터랙티브) | 실물(발견 광고) | **GUI 목록에 뜨는 터미널 클라이언트** |
| `nexa-beep --chat-serve [port]` | 터미널(인터랙티브) | 실물(고정 포트) | 발견 없이 기다리는 쪽(1:1) |
| `nexa-beep --chat-connect <host:port>` | 터미널(인터랙티브) | 실물(수동 IP) | 발견 없이 IP로 거는 쪽(DR-19) |
| `nexa-beep --serve [port]` | 터미널(로그) | 실물 | 헤드리스 **에코 서버**(자동 응답) |
| `nexa-beep --live-echo [초]` | 터미널(로그) | 실물(발견) | 헤드리스 — 발견 즉시 인사 + 에코 |
| `nexa-beep --discover-probe [초]` | 터미널(로그) | 실물(발견) | 발견만 관찰(SAW 로그·복제 경고) |

> **`--live`가 관건** — 실물 통신은 전부 `--live`/`--chat-*`/`--serve`/`--connect`/`--discover-probe`/`--live-echo`가 쓴다. 기본 `--window`는 InMemory라 **외부와 못 붙는다**(⌘K 수동 연결도 InMemory에선 미지원).

## 2. 시나리오 A — GUI ↔ 터미널 클라이언트 (같은 맥, 권장)

가장 쉬운 실물 대화 테스트. 터미널 클라이언트가 **GUI 목록에 자동으로 뜨고** 클릭만 하면 된다.

```bash
# ① GUI (실물 네트워크)
./target/release/nexa-beep --window --live

# ② 다른 터미널 — 발견 가능한 인터랙티브 클라이언트
./target/release/nexa-beep --chat-live 테스트단말
```

1. 터미널이 `[대기] '테스트단말'(me=…) 로 발견 광고 중…` 을 출력한다.
2. **GUI 목록에 "테스트단말"이 뜬다** → 클릭 → 대화창이 열린다(Noise 핸드셰이크·TOFU 핀).
3. **GUI에서 타이핑 → Enter** 하면 터미널에 `<지문>> 메시지` 로 뜬다.
4. **터미널에서 타이핑 → Enter** 하면 GUI 대화창에 실시간으로 뜬다(M2-7 수신 펌프).
5. 터미널은 `Ctrl+D`로 종료(⚠️ `Ctrl+C`는 아직 안 됨 — [18 §2-2](18-build-and-test.md)).

> 같은 호스트라 멀티캐스트 루프백으로 서로 발견된다. 발견이 안 뜨면 시나리오 B(IP 직접)로.

## 3. 시나리오 B — GUI ↔ 터미널, IP 직접 (발견 없이)

발견이 막힌 환경·다른 세그먼트일 때. GUI의 **⌘/Ctrl+K 수동 연결**(DR-19)을 쓴다.

```bash
# ① 터미널 — 고정 포트로 기다림
./target/release/nexa-beep --chat-serve 47300

# ② GUI: --window --live 실행 → ⌘/Ctrl+K → 127.0.0.1:47300 → Enter
```

GUI 하단 상태바가 주소 입력창이 되고, Enter 하면 대화창이 열린다. 이후는 시나리오 A와 동일.

## 4. 시나리오 C — 맥 ↔ Docker Linux (크로스플랫폼, IP)

⚠️ **맥 호스트와 Docker Desktop 컨테이너는 멀티캐스트 발견이 안 된다**(컨테이너의 network는 내부 Linux VM — VM 경계를 못 넘음). **포트 매핑 + 수동 연결**로 우회한다.

```bash
# ① 컨테이너를 에코 서버로(포트 매핑)         --init = 정상 종료(R-16)
docker run --rm -it --init -p 47200:47200 \
  -v "$PWD/.docker-target/release/nexa-beep:/nexa-beep:ro" \
  debian:stable-slim /nexa-beep --chat-serve 47200

# ② 맥에서 붙는다(터미널 또는 GUI ⌘K)
./target/release/nexa-beep --chat-connect 127.0.0.1:47200
```

**실측(2026-08-08)**: 맥(arm64) ↔ 컨테이너(amd64) 사이에서 발견→TCP→Noise→대화 왕복 성공. `--chat-serve`↔`--chat-connect` 로 양쪽 사람이 타이핑하면 실시간 상호 수신(예: 맥 "맥에서 크로스플랫폼 인사" ↔ 리눅스 "나는 리눅스 컨테이너다").

## 5. 시나리오 D — Docker 2노드 (Linux↔Linux 발견, 자동)

컨테이너끼리는 같은 브리지 네트워크에서 **멀티캐스트 발견이 된다**(발견 테스트베드).

```bash
docker network create beepnet
for n in a b; do
  docker run -d --name node_$n --network beepnet \
    -v "$PWD/.docker-target/release/nexa-beep:/nexa-beep:ro" \
    debian:stable-slim /nexa-beep --live-echo 15
done
sleep 12
docker logs node_a; docker logs node_b        # CLIENT got reply / SERVER recv 확인
docker rm -f node_a node_b; docker network rm beepnet
```

**실측**: 두 컨테이너가 시작 즉시 서로 발견 → 인사 → 에코 왕복(양방향 로그 확인).

## 6. 헤드리스 검증 도구

| 명령 | 확인하는 것 |
|---|---|
| `nexa-beep --discover-probe 8` | 발견만 — 누가 보이나(`SAW peer=… kind=…`), **복제 경고**(`⚠️CLONE` — D-22) |
| `nexa-beep --live-echo 8` | 발견→연결→Noise→인사·에코 왕복(`CLIENT got reply` / `SERVER recv`) |
| `nexa-beep --serve 47200` | 헤드리스 에코 서버(자동 응답 — GUI가 붙는 상대) |

두 프로브를 동시에 띄우면 서로 발견한다(맥 2프로세스):
```bash
./target/release/nexa-beep --discover-probe 6 &  ./target/release/nexa-beep --discover-probe 6 &  wait
```

## 7. 정상 종료 (R-16 · FR-P-7)

| 대상 | 종료 |
|---|---|
| GUI 창 | 창 닫기 · `SIGTERM`(실측 0.28초 — GOODBYE·정리) |
| `--serve`/`--live-echo`/`--discover-probe` | `Ctrl+C`(`SIGINT`) — 정상 종료 |
| `--chat-*`(인터랙티브) | **`Ctrl+D`**(⚠️ `Ctrl+C` 미배선 — [18 §2-2](18-build-and-test.md)) |
| 컨테이너 | **`docker run --init` 필수** — 없으면 `docker stop`이 10초(SIGKILL 대기). 실측: `--init`+시그널 핸들러 = **0.38초** |

## 8. 알려진 한계

| 한계 | 내용 · 참조 |
|---|---|
| **맥↔Docker 발견 불가** | Docker Desktop 구조(내부 VM). IP 직접(시나리오 C)로 우회 · 맥↔실제 리눅스는 같은 공유기/브리지 VM(D-8b) |
| **타이밍은 잠정치** | announce 주기·타임아웃·그룹/포트/TTL은 D-8b 실기 실측 전 잠정값([08 §8](08-adr-0002-discovery-transport.md)) |
| **CLI 채팅 1:1·`Ctrl+D`** | `--chat-serve`는 accept 1회 · 인터랙티브는 파이프 입력 부적합(사람 타이핑 전제) — [18 §2-2](18-build-and-test.md) |
| **SAS 미배선** | 지문 육안 대조(SAS 60자리)는 CLI에 아직 없음 — 화면의 `me`↔`상대` 교차 대조는 MITM 방어가 아님(M3-6) |
| **파일·기록** | 파일 전송(M4)·기록 저장(M2-5)은 미착수 — 현재 텍스트 대화만 |
