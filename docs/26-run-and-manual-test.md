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

## 7-1. ★ 검증 현황 매트릭스 (2026-08-09 전수 점검)

> **"코드가 있다"와 "동작을 봤다"는 다르다.** 아래는 구현된 기능을 **어떤 수준까지 확인했는지** 한 장으로 본 것이다.
> 등급: **실증** = 실제 실행으로 확인 · **단위** = 테스트만 · **미확인** = 실행 확인 기록 없음.

### ✅ 실증 완료 (2026-08-09 직접 실행)

| # | 기능 | 확인 방법 | 결과 |
|:--:|---|---|---|
| 1 | **실물 멀티캐스트 발견**(S2 · FR-D-2) | 맥 로컬 2노드 `--discover-probe` | **양방향 상호 발견** — `Hello`·`Announce` 수신 |
| 2 | **실물 종단 왕복**(발견→Noise→대화) | `--live-echo` 2노드 | `SERVER recv` → `CLIENT got reply` **한글 왕복** |
| 3 | **정상 종료**(FR-P-7 · R-16) | `SIGTERM`/`SIGINT` 실측 | **0.09초 / 0.11초** · `DONE` 정상 출력 |
| 4 | ★ **GOODBYE 실제 발신**(FR-D-8 절반) | 관찰 노드가 이탈 노드의 종료를 관측 | **`kind=Goodbye` 4회 수신** — 비어 있던 절반이 채워짐 |
| 5 | **수동 엔드포인트**(DR-19) | `--serve` ↔ `--connect` | `peer=…(지문 확정)` — 핸드셰이크로 신원 확정 |
| 6 | **인터랙티브 채팅**(M1-3c) | `--chat-serve` ↔ `--chat-connect` · **맥↔Docker** | 양방향 · 한글 · 지문 교차 대조 일치 |
| 7 | **`--chat-live` 발견 광고** | 프로브가 이름 관측 | `name=테스트단말` — **한글 이름 정상** |
| 8 | **빌드 게이트 전항목** | `fmt`/`clippy`/`test`/`rustdoc` | fmt ✅ · clippy 0 · **테스트 243** · rustdoc ✅ |

### ✅ 육안 실증 (2026-08-09 · 사용자 확인)

> `--window --live` ↔ `--chat-live 테스트단말` 조합으로 확인. **스크린샷이 증명한 범위만** 적는다.

| 기능 | 증거 |
|---|---|
| **실물 발견 → 목록 → 대화 성립** | 터미널이 `[대화 시작] 상대=1f9ed7d0`, GUI 헤더가 `테스트단말` — **발견 광고를 GUI가 받아 대화까지** 갔다 |
| **양방향 실시간 수신**(M2-7) | 터미널에서 보낸 `실측`·`간다`·`이봐`·`보이나`가 GUI에 `Peer:` 로, GUI에서 친 `응 잘 보이네`가 터미널에 `1f9ed7d0>` 로 |
| **대화 화면**(M3 ChatViewWidget) | 스레드 `Me:`/`Peer:` 구분 · 입력창 안내(`Enter 전송 · Esc 목록`) · 하단 정렬 |
| **한글 입력·렌더**(FR-U-7 일부) | GUI에서 한글 직접 입력·전송 성공 · 스레드 한글 표시 정상 |
| **다크 테마**(FR-U-5 일부) | 다크 배경 · 파랑 `Peer:` / 흰색 `Me:` 대비 |
| ★ **피어 목록 + 신뢰 배지 전이**(M1-6 · FR-S-3) | 첫 발견 = **`Unverified`**(황토 pill) → 대화 후 Esc = **`Pinned`**(파랑 pill). **TOFU 전이가 화면에 그대로 드러났다** |
| **타입어헤드 안내** | 상태바 `↑↓ 이동 · 타이핑 = 이름 점프(한글 가능) · Enter = 대화 열기` |
| **다중 피어 목록** | `테스트단말`·`테스트단말2` 2행 동시 표시 · 선택 행 강조 |
| **정상 종료 이스케이프** | `got 3 SIGTERM/SIGINTs, forcefully exiting` — 강제 탈출 경로 동작 |

### 🖥 여전히 미확인 — **육안(GUI) 확인 필요** (헤드리스로 대행 불가)

| 기능 | 근거 | 확인 방법 |
|---|---|---|
| **`FingerprintVerified` 배지(3번째)** | FR-S-4 · M3-6 | `Unverified`·`Pinned` 2종은 ✅ 실증. **3번째는 SAS 대조 UX가 없어 도달 자체가 불가**(확인 문제가 아니라 미구현) |
| **상대별 별도 창**(`--separate-windows`) | M3-12 · DR-26 | 구현·설정 연동은 완료. 다중 창 동작만 미확인 |
| **수동 추가 `⌘K` 오버레이** | M1-3b | CLI 경로는 실증 · GUI 오버레이만 미확인 |
| **설정 화면**(트리·검색·즉시 적용) | M3-11 · DR-24 | 설정 버튼 |
| **컨트롤 갤러리**(Checkbox·Radio·TextBox·Combo·Choose·Tree·스크롤바) | M3-1 | 갤러리 버튼 |
| **이미지 아이콘**(투명 배경 RGBA) | — | 콤보·Choose 항목 |
| **i18n 전환**(영/한/중/일) | M3-4 | 설정 → 언어 |
| 고DPI 배율 · **한글 IME 조합 중 표시** · **라이트 테마** | M3-3 · FR-U-6/7 | 한글 입력·다크는 08-09 확인 · 조합 중 프리에딧과 라이트는 미확인 |

### 🔬 미확인 — **시나리오 재현이 필요**

| 기능 | 왜 아직 못 봤나 |
|---|---|
| **U-P1 클론 탐지 · U-P2 자기 `PeerId` 거부**(D-22 · R-12) | 신원 키 파일을 실제로 복제한 환경이 필요 — 매 실행마다 키를 새로 만들어 재현되지 않는다 |
| **TOFU 이름 재사용 경고**(`name_conflict`) | 같은 이름·다른 키를 순차로 만나는 시나리오 필요 |
| **SAS 60자리 양쪽 일치** | `safety_number`는 구현·단위 테스트됐으나 **CLI/GUI에 출력 지점이 없다**(M3-6) |
| **백프레셔 · 프레임 상한**(M2-3) | 부하를 거는 시나리오가 없다 — 단위 테스트만 |
| **중복 제거**(FR-M-9) | 다중 경로로 같은 메시지가 들어오는 상황을 아직 못 만든다 |

### ⏸ 미확인 — **실기 필요**(대행 불가)

D-8b 타이밍 실측(E-1~E-9 · **2대 이상**) · **클라이언트 격리 AP에서 S4**(R-2) · Windows·Linux 실기 · **24h 누수**(E-9) · **유휴 RSS 재실측**(R-8 잔여 축).

### ⚠️ 아직 main에 없는 것

**`.beepq` 격리 컨테이너 코덱**(M4-1 슬라이스 1)은 `feat/m4-quarantine` **브랜치에만** 있고 main에 병합되지 않았다.

### 🐛 점검 중 발견한 것

| 발견 | 조치 |
|---|---|
| **CI가 main에서 3회 연속 red**(rustdoc `-D warnings` 6건 — private 항목 링크·링크 문법 오용) | ✅ **해소**(2026-08-09) — 코드 동작과 무관한 문서 주석 링크였다 |
| ★ **`Pinned`가 재시작하면 사라진다** — 현재 신뢰 저장소는 `MemoryTrustStore`(메모리 전용)이고 `nbeep-store`는 아직 골격(10줄)뿐이다 | ⚠️ **TOFU의 핵심은 "기억"인데 그 기억이 프로세스 수명뿐**이다. 재실행하면 아는 상대가 다시 `Unverified`로 돌아간다 → **M2-5(영속)까지는 이 성질을 문서에 명시**. FR-S-3의 절반이 아직 비어 있는 셈 |
| 목록 상태바에 **`연결 실패: Unreachable`** | 상대가 아직 세션 수락 준비 전이거나 이탈 직후일 때 나오는 정상 경로(`ConnectError::Unreachable`). 다만 **사용자에게 다음 행동을 주지 않는 문구**라 개선 여지 |
| **상태바가 `전송 seq=0 (응답 대기)`에 머문다** — 상대 응답을 받았는데도 "응답 대기" 표시 | ⚠️ 확인 필요 — **수신확인(ack)은 ADR-0010/D-23 미확정으로 아직 없다**. 없는 기능을 기다리는 문구라면 오해를 준다(M2-4b·M3-9 전까지 문구 조정 검토) |
| `--live-echo` 2노드 동시 실행 시 **stdout 라인 인터리빙** | 사소(검증 도구 한정) — 출력 잠금 미적용. 제품 경로 아님 |

## 8. 알려진 한계

| 한계 | 내용 · 참조 |
|---|---|
| **맥↔Docker 발견 불가** | Docker Desktop 구조(내부 VM). IP 직접(시나리오 C)로 우회 · 맥↔실제 리눅스는 같은 공유기/브리지 VM(D-8b) |
| **타이밍은 잠정치** | announce 주기·타임아웃·그룹/포트/TTL은 D-8b 실기 실측 전 잠정값([08 §8](08-adr-0002-discovery-transport.md)) |
| **CLI 채팅 1:1·`Ctrl+D`** | `--chat-serve`는 accept 1회 · 인터랙티브는 파이프 입력 부적합(사람 타이핑 전제) — [18 §2-2](18-build-and-test.md) |
| **SAS 미배선** | 지문 육안 대조(SAS 60자리)는 CLI에 아직 없음 — 화면의 `me`↔`상대` 교차 대조는 MITM 방어가 아님(M3-6) |
| **파일·기록** | 파일 전송(M4)·기록 저장(M2-5)은 미착수 — 현재 텍스트 대화만 |
