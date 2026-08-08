# CLAUDE.md — Nexa Beep 프로젝트 컨텍스트 (이식용 메모리)

> 이 파일은 **다른 PC에서 clone 시 즉시 컨텍스트를 복원**하기 위한 휴대용 프로젝트 메모리다.
> **먼저 읽기:** [docs/STATUS.md](docs/STATUS.md)(현황) → [docs/10-decision-record.md](docs/10-decision-record.md)(결정) → [docs/03](docs/03-competitive-landscape.md)(경쟁 지형).

## 1. 이 프로젝트는

**Nexa Beep** = **제로 컨피그 로컬 네트워크 메신저**.
서버 주소 입력·계정 생성·상대 등록이 **전부 없다**. 실행하면 같은 로컬 네트워크에서 실행 중인
사용자가 **자동으로 목록에 뜨고**, 고르면 **즉시 대화**한다. 발견된 사용자는 **그룹으로 묶어** 그룹 단위 전송도 한다.
메시지는 **Text·Image·File** 3종이며, 수신 파일은 **무해화 게이트**를 거쳐야 실체화된다.

- 조직: **SosomLab** · 개발자: Sangyong Bae · kiros33@gmail.com
- 저장소: `git@github.com:SosomLab/nexa-beep.git` · 릴레이 서버는 별도 **`nexa-beepd`**(v1 이후)
- 현 단계: **M-1 설계 사실상 완료** — 문서 15종 · **ADR 7종**(0001·0002 Accepted / 0003~0007 Proposed) · DR-1~20. 다음 관문은 **M0 스캐폴딩 + SP-1 예산 스파이크**.

## 2. 확정 결정 ([docs/10](docs/10-decision-record.md) DR-1~20, 변경 시 새 ADR/journal)

| # | 결정 |
| --- | --- |
| DR-1 | **제로 컨피그가 정체성** — "실행 = 참여". 사전 등록·계정·서버 설정을 요구하는 설계는 채택하지 않는다 |
| DR-2 | 문서·git 규약은 `nexa-dir2` [docs/16](docs/16-doc-git-conventions.md) **전면 차용** |
| DR-3 | **크로스플랫폼 4타깃** — Windows(x64·**ARM64**)·macOS·Linux. 기능 차등 없음 |
| DR-4 | **설치본 + 포터블 2채널** — 포터블은 압축 해제 즉시 실행, 영속물은 실행 파일 옆(폴백 있음) |
| DR-5 | **예산 게이트** — 유휴 RSS **≤30MB** · 산출물 **≤10MB/타깃** · **런타임 의존 0** · 24h 누수 RSS ≤2MB·핸들 증가 0. 초과 시 main 병합 금지 ([05 NFR-B](docs/05-requirements.md)) |
| DR-6 | **모든 렌더링 자체 구현 커스텀 컨트롤** — 플랫폼 간 동일 UI. OS 위젯·UI 프레임워크(Qt/Avalonia/Flutter/WebView) **금지** |
| DR-7 | **단말 간 E2E 암호화 필수** — 전송 경로 무관. 릴레이 서버도 평문 접근 불가. 키 인증(TOFU/지문)은 ADR-0002 |
| DR-8 | **전송 2모드 추상화** — ① 로컬 직접(기본·서버리스) ② 릴레이 경유(선택). **v1은 ①만 출시**하되 인터페이스·신원·주소·세션은 1일차부터 2모드 전제 |
| DR-9 | 릴레이 서버는 **별도 저장소 `nexa-beepd`** (v1 이후 착수) |
| DR-10~11 | 스택 · 디스커버리/전송 프로토콜 **미정** — ADR-0001/0002 |
| DR-12 | **PolyForm Noncommercial 1.0.0**(nexa-dir2 동일) + **의존성 퍼미시브 온리 — GPL/LGPL 금지** |
| DR-13 | 페이로드 **Text·Image·File**. 수신 파일은 **4단계 게이트**(협상→`.beepq` 격리→검사→승인 실체화). **앱은 수신 파일을 실행하지 않는다**. 복원 후에도 MotW/quarantine 유지 → [docs/04](docs/04-safe-transfer.md) |
| DR-15~17 | **코드 설계 표준**([docs/13](docs/13-code-design-standards.md) — `ActionKind` 단일 통로·인터셉터·계측) · **컨트롤/UX**([docs/14](docs/14-control-ux-architecture.md) — 시각=macOS/동작=OS 네이티브·3단계 이벤트) · **기록 저장 암호화**([docs/17](docs/17-adr-0005-history-at-rest.md) — 블라인드 인덱스·크립토 셰레딩) |
| DR-18 | **PC 1대 = 노드 1개**(기본). 기기 이전 시 기록이 따라가지 않는다(v1 한계·UI에 명시). ⚠️ **개정(DR-20)** — "사용자 개념 없음"은 **기본값**이지 금지가 아니다 |
| DR-19 | **수동 엔드포인트 등록** — 직접 IP/DDNS로 노드 추가(S6 + 공인 IP). **LAN 밖은 위협 모델이 다르다** → 원격 신뢰 등급·SAS 전 파일 차단·인바운드 요청 대기 → [docs/19](docs/19-adr-0006-manual-endpoint.md) |
| DR-20 | **다중 기기 신원(선택 계층)** — `UserId` 1:M `PeerId`. `UserId` 키가 **기기 목록을 서명**하고 상대가 검증(양방향 소유 증명) · 발신은 **그룹 팬아웃 재사용** · TOFU/SAS 대상이 `UserId`로 상승 · **`UserId` 개인키는 주 기기 1대 + 오프라인 복구 시드**(전 기기 복제는 폐기 경쟁에서 회복 불가라 탈락) · **`UserId` 키는 서명 전용**(저장 래핑 키로 쓰지 않는다) · **구현은 v2, v1 제약 4건은 M0에 반영** → [docs/20](docs/20-adr-0007-multi-device-identity.md) |
| DR-14 | **L1~L4 직접 제어** — L1 링크 상태 구독 · L2 이웃 테이블 · L3 인터페이스별 멀티캐스트/브로드캐스트/링크로컬 · L4 UDP+TCP. 발견 폴백 **S1~S6**. **T0(무권한)이 완전한 제품** — L2 원시 소켓은 선택 → [docs/06](docs/06-network-stack.md) |

### ★ 관통 원리 — "봉투만 본다"

> **어떤 계층도 자기 일에 필요한 만큼만 보고, 내용은 보지 않는다.**
> 릴레이는 목적지만 · 발견 패킷은 존재만 · 격리물은 메타만 · 저장 세그먼트는 헤더만 · 계측은 횟수만 · 로그는 상태만.
> 계층별 대조표 [docs/10 §0](docs/10-decision-record.md). **새 기능 설계의 첫 질문 = "이 계층의 봉투는 무엇인가?"**

### 설계 시 절대 놓치면 안 되는 것

- **신원 = 기기 키 지문. PC 1대 = 노드 1개**(DR-18 기본). **연결은 `PeerId`에, 대화·기록·차단은 `UserId`에** 붙인다(DR-20 V1-1 — v1은 둘이 1:1).
- **병합의 근거는 언제나 암호학적 증거** — 같은 노드의 여러 경로는 병합(FR-D-6), 다른 PC는 별개. **서명된 같은 `UserId`면 접어 표시**(v2). 이름·IP·사용자 주장은 근거가 아니다.
- **`UserId`는 그룹과 다르다** — 그룹은 **로컬 개념**(FR-G-3)이지만 `UserId`는 **상대가 믿어야 하는 주장**이라 서명·검증·폐기(롤백 방지)가 필수([docs/20 §2](docs/20-adr-0007-multi-device-identity.md)).
- **다중 기기는 릴레이로 풀리지 않는다** — 릴레이는 "닿는가"만 푼다. 기여는 오프라인 큐 하나.
- **차단은 기기가 아니라 사람(`UserId`) 단위** — 기기 단위 차단은 상대가 새 기기를 추가하면 곧바로 우회된다.
- **권한 설계의 판단 기준 = "잘못됐을 때 되돌릴 수 있는가"**(Q-1이 이걸로 갈렸다). 예방만 보면 회복 불가 설계를 고르게 된다.
- **암호화는 전송 계층 "위"** 에 둔다 — 릴레이는 봉투만 본다.
- **발견은 다단 폴백 필수** — 기업 무선망 클라이언트 격리·VLAN이 멀티캐스트를 차단한다([06 §4](docs/06-network-stack.md)).
- **T0(무권한)에서 전 기능이 돌아야 한다** — 관리자 권한·드라이버를 요구하는 순간 DR-1·DR-4가 깨진다.
- **수신 파일은 보낸 사람이 누구든 믿지 않는다** — 무해화는 신원과 무관하게 항상 적용([04 T-7](docs/04-safe-transfer.md)).
- **스팸 방어는 1차 범위** — 무인증 즉시 발신 UX의 필연적 공격면(국내 소프트메신저 서비스 종료 실사례).
- **GPL 코드 차용 금지** — 참고 가능한 선행 제품은 MIT(Squiggle)·Apache-2.0(LocalSend)·표준(XEP-0174)뿐.
- **네트워크는 추정 금지, 실측 필수** — 소켓 옵션 의미는 OS마다 다르고, "보냈다"는 "닿았다"가 아니다.

## 3. 작업 규약

- **문서·커밋/푸시 규약 SSOT = [docs/16](docs/16-doc-git-conventions.md)** — 4층 문서 체계·작성 규칙 8·커밋/브랜치/푸시 필수 규칙.
- 개발 규율([docs/15](docs/15-dev-methodology.md)): **수직 슬라이스 · 단위=커밋 1개 · 초안→확장 · main 항상 green · Conventional Commits**.
- **큰 단위=브랜치, 세부 기능=커밋. push는 사용자 명시 요청 시에만.** 파괴적 작업(삭제·reset·force push·덮어쓰기)은 실행 전 확인. 그 외 일상 작업·상태 md 갱신은 묻지 않고 자동 진행.
- 기록: **한 작업 = 한 트랜잭션 갱신** — 커밋 → [journal/YYYY-MM-DD](docs/journal/) 상세 → [DEVLOG](docs/DEVLOG.md) 한 줄 → [MILESTONES](docs/MILESTONES.md)/[TODO](docs/TODO.md) 상태 → (브랜치면) [BRANCHES](docs/BRANCHES.md).
- 기록에는 **"왜 + 실측값"**. 네트워크 기능은 **추정 금지 · 실측 필수**(도달률·지연·패킷 크기).
- **설계 전 기존 문서·선행 사례부터 확인**(재발명 금지) — [docs/03](docs/03-competitive-landscape.md) · `nexa-dir2` `docs/ctl`(Nexa Controls 14종)·`docs/23`(크로스플랫폼 검토서). 차용 시 출처 경로를 커밋 본문에 명기.
- 문서 번호(`NN-`)는 **불변**. 재번호 금지, 신규는 뒤에 append.
- `.claude/settings.json`(권한)은 **덮어쓰기 금지, 병합만**.
- 빌드/테스트 SSOT = [docs/18](docs/18-build-and-test.md) — 절차 변경 시 같은 커밋에서 갱신.

## 4. 새 세션 오리엔테이션

1. 이 CLAUDE.md + [docs/STATUS.md](docs/STATUS.md) → 2. [DEVLOG](docs/DEVLOG.md) 최상단 + 최신 journal → 3. 할 일 = [docs/TODO.md](docs/TODO.md) 순차.

## 5. 다음 단계 (2026-08-08 5차 기준)

1. 🔴 **사용자 확정 대기** — ADR-0003·0004·0006(Proposed). **ADR-0007은 08-08 Accepted**(Q-1 = 주 기기 + 복구 시드 / 기록 키 분리).
2. **M0-1 스캐폴딩** — Cargo 워크스페이스·MSRV·최소 지원 OS·4타깃. 이어 **M0-1b 횡단 골격**([13](docs/13-code-design-standards.md))과 **D-21 v1 제약 4건**(DR-20)을 **같이** 세운다 — 나중에 넣으면 마이그레이션이다.
3. **SP-1 예산 검증 스파이크**(D-15) — M0-1 직후 최우선(R-8 해소).
4. **D-8 발견 도달 스파이크**(E-1~E-9) — 🔴 **실기 2대 이상 필요, 대행 불가.** ADR-0002 타이밍 6종이 여기 묶여 있다.

> ADR 상태: **0001·0002·0007 ✅ Accepted** / **0003·0004·0005·0006 📐 Proposed**.
> 완료된 설계 문서: [00 비전](docs/00-vision.md) · [01 아키텍처](docs/01-architecture.md) · [02 로드맵](docs/02-roadmap.md) · [03 경쟁 조사](docs/03-competitive-landscape.md) · [04 안전 송수신](docs/04-safe-transfer.md) · [05 요구사항](docs/05-requirements.md) · [06 네트워크 스택](docs/06-network-stack.md) · [12](docs/12-asset-reuse.md)·[13](docs/13-code-design-standards.md)·[14](docs/14-control-ux-architecture.md).
> **예산 게이트 수치 = [05 §2-1](docs/05-requirements.md) NFR-B-1~12** — 유휴 RSS ≤30MB · 산출물 ≤10MB/타깃 · 런타임 의존 0 · 24h 누수 RSS ≤2MB·핸들 증가 0.
> 문서 번호 배정 계획은 [docs/README](docs/README.md) — **번호는 불변**이므로 새 문서는 반드시 그 표를 보고 붙인다.
