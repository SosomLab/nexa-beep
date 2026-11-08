# CLAUDE.md — Nexa Beep 프로젝트 컨텍스트 (이식용 메모리)

> 이 파일은 **다른 PC에서 clone 시 즉시 컨텍스트를 복원**하기 위한 휴대용 프로젝트 메모리다.
> **먼저 읽기:** [docs/STATUS.md](docs/STATUS.md)(현황) → [docs/10-decision-record.md](docs/10-decision-record.md)(결정) → [docs/03](docs/03-competitive-landscape.md)(경쟁 지형).

## 1. 이 프로젝트는

**Nexa Beep** = **제로 컨피그 로컬 네트워크 메신저**.
서버 주소 입력·계정 생성·상대 등록이 **전부 없다**. 실행하면 같은 로컬 네트워크에서 실행 중인
사용자가 **자동으로 목록에 뜨고**, 고르면 **즉시 대화**한다. 발견된 사용자는 **그룹으로 묶어** 그룹 단위 전송도 한다.
메시지는 **Text·Image·File** 3종이며, 수신 파일은 **무해화 게이트**를 거쳐야 실체화된다.

- 조직: **SosomLab** · 개발자: Sangyong Bae · kiros33@gmail.com
- 저장소: `git@github.com:SosomLab/nexa-beep.git` · 릴레이 서버 **`nexa-beepd`** = ★**같은 저장소의 별도 크레이트·별도 배포**(`beepd-v*` 태그 — DR-9 개정 08-21 · MVP 구현 ✅)
- 현 단계 — ★**v0.9.0 공개 ✅**(11-07 · 태그 `v0.9.0` = `39296e7` · release run **42334473879 전 13 job success** · 자산 15 · Releases·brew(tap d303fa5)·apt/dnf · 일괄 검증 gate rc=0 · verify 6/6 · s4 24/24 · 4,307,456B · 탑재 = **M4-12 수신 스트리밍**(256MiB 수신 피크 521MB→8MB · 격리물 v2 · 구 앱 fail-closed · 검사 32MiB 상한) · R-18 조용한 노드 · R-22 적응형 S4 · S4 왕복 결함 수정 · X-13 S3 마무리(수동 IP 형제 증명 · 기기 수 라벨 · 기기마다 파일 오퍼 · P-10 kind 9) · 키보드(메뉴 키·캐러셀·Delete) · FW-1 방화벽 · LOG-2 · 창 투명도 · 모달 기본 취소 · 로그오프 보존 · 기동 회귀 처방(송신 소켓 읽기 타임아웃 제거 + paint-first)). winget(#468844 작성자 응답 대기)·choco(0.6.0 승인 · 0.7.0~0.9.0 제출 보류) → TODO PUB-1 · 스위치 둘 다 false. ★**beepd 와이어 v2 배선 = 코드 ✅**(11-08 · `7d8e9c3` · ADR-0016 [docs/56](docs/56-adr-0016-beepd-wire-v2.md) 📐 · 프리앰블+prologue · `nbeep-relay` v2 코덱 · `nexa-beepd` 1.0.0 모듈 분리 · e2e 22 · **배포 ⏸** = ⓐ 운영 서버 버전 ⓑ clip 준비 회신 ⓒ 서버 방식(v2 전용 구현·이중 수용 자리만) → beepd-v1.0.0 + beep 0.10 + clip 같은 날).
  직전 = v0.8.0(11-05 GAP-042 winit 교착 수정 · 수신 제한 · 발견 호환성 · 청크 윈도·그룹 재개) · v0.7.0(11-03 답장·말풍선 선택·설정 창 개편).
  **현황·경과는 여기 쓰지 않는다** → [STATUS](docs/STATUS.md) · [DEVLOG](docs/DEVLOG.md) · 기능 = [MILESTONES](docs/MILESTONES.md).
  **ADR**: 0001~0015 ✅ Accepted(0013 서버 모드 = 11-06 사용자 확정 · 구현은 v1 이후 그대로) / **0016 📐 Proposed**(beepd 와이어 v2 · 11-07) · 번호표 = [docs/README](docs/README.md).

## 2. 확정 결정 ([docs/10](docs/10-decision-record.md) **DR-1~31**, 변경 시 새 ADR/journal)

| # | 결정 |
| --- | --- |
| DR-1 | **제로 컨피그가 정체성** — "실행 = 참여". 사전 등록·계정·서버 설정을 요구하는 설계는 채택하지 않는다 |
| DR-2 | 문서·git 규약은 `nexa-dir2` [docs/16](docs/16-doc-git-conventions.md) **전면 차용** |
| DR-3 | **크로스플랫폼 4타깃** — Windows(x64·**ARM64**)·macOS·Linux. 기능 차등 없음 |
| DR-4 | **설치본 + 포터블 2채널** — 포터블은 압축 해제 즉시 실행, 영속물은 실행 파일 옆(폴백 있음). ⚠️ **폴백 종점을 임시 폴더로 두지 않는다**(08-19 실기 — macOS `tmp_cleaner`가 `/tmp` 아래 `identity.key`를 지워 신원이 조용히 바뀌고, 이전 키에 sealed된 격리물·핀을 영구히 못 연다). 3신원 실기 폴더 = **`$HOME/.nexa-beep-multi`**. ⚠️ **"실행 파일 옆 쓰기 가능"만으로 포터블 판정 금지**(08-24 실기 — mac `.app` 번들·brew keg는 사용자 소유라 쓰기가 되지만 `brew upgrade`가 폴더째 교체해 신원·설정 소실 → `nexa_conf::is_replaced_on_upgrade`로 제외 · Win NSIS는 `data/` 보존 설계) |
| DR-5 | **예산 게이트** — 유휴 RSS **≤30MB** · 산출물 **≤10MB/타깃** · **런타임 의존 0** · 24h 누수 RSS ≤2MB·핸들 증가 0. 초과 시 main 병합 금지 ([05 NFR-B](docs/05-requirements.md)) |
| DR-6 | **모든 렌더링 자체 구현 커스텀 컨트롤** — 플랫폼 간 동일 UI. OS 위젯·UI 프레임워크(Qt/Avalonia/Flutter/WebView) **금지** |
| DR-7 | **단말 간 E2E 암호화 필수** — 전송 경로 무관. 릴레이 서버도 평문 접근 불가. 키 인증(TOFU/지문)은 ADR-0002 |
| DR-8 | **전송 2모드 추상화** — ① 로컬 직접(기본·서버리스) ② 릴레이 경유(선택). **v1은 ①만 출시**하되 인터페이스·신원·주소·세션은 1일차부터 2모드 전제 |
| DR-9 | ⚠️**개정(08-21 · Q-32-13)** — 릴레이 서버 `nexa-beepd`는 **같은 저장소 워크스페이스 멤버 + 배포만 분리**(`beepd-v*`·release-server.yml · 가드레일 W-1~W-4 — 클라 의존 불침투·예산 게이트는 클라만) |
| DR-10~11 | 스택 · 디스커버리/전송 프로토콜 **미정** — ADR-0001/0002 |
| DR-12 | **PolyForm Noncommercial 1.0.0**(nexa-dir2 동일) + **의존성 퍼미시브 온리 — GPL/LGPL 금지** |
| DR-13 | 페이로드 **Text·Image·File**. 수신 파일은 **4단계 게이트**(협상→`.beepq` 격리→검사→승인 실체화). **앱은 수신 파일을 실행하지 않는다**. 복원 후에도 MotW/quarantine 유지 → [docs/04](docs/04-safe-transfer.md) |
| DR-15~17 | **코드 설계 표준**([docs/13](docs/13-code-design-standards.md) — `ActionKind` 단일 통로·인터셉터·계측) · **컨트롤/UX**([docs/14](docs/14-control-ux-architecture.md) — 시각=macOS/동작=OS 네이티브·3단계 이벤트) · **기록 저장 암호화**([docs/17](docs/17-adr-0005-history-at-rest.md) — 블라인드 인덱스·크립토 셰레딩) |
| DR-18 | **PC 1대 = 노드 1개**(기본). 기기 이전 시 기록이 따라가지 않는다(v1 한계·UI에 명시). ⚠️ **개정(DR-20)** — "사용자 개념 없음"은 **기본값**이지 금지가 아니다 |
| DR-19 | **수동 엔드포인트 등록** — 직접 IP/DDNS로 노드 추가(S6 + 공인 IP). **LAN 밖은 위협 모델이 다르다** → 원격 신뢰 등급·SAS 전 파일 차단·인바운드 요청 대기 → [docs/19](docs/19-adr-0006-manual-endpoint.md) |
| DR-20 | **다중 기기 신원(선택 계층)** — `UserId` 1:M `PeerId`. `UserId` 키가 **기기 목록을 서명**하고 상대가 검증(양방향 소유 증명) · 발신은 **그룹 팬아웃 재사용** · TOFU/SAS 대상이 `UserId`로 상승 · **`UserId` 개인키는 주 기기 1대 + 오프라인 복구 시드**(전 기기 복제는 폐기 경쟁에서 회복 불가라 탈락) · **`UserId` 키는 서명 전용**(저장 래핑 키로 쓰지 않는다) · **구현은 v2, v1 제약 4건은 M0에 반영** → [docs/20](docs/20-adr-0007-multi-device-identity.md) |
| DR-14 | **L1~L4 직접 제어** — L1 링크 상태 구독 · L2 이웃 테이블 · L3 인터페이스별 멀티캐스트/브로드캐스트/링크로컬 · L4 UDP+TCP. 발견 폴백 **S1~S6**. **T0(무권한)이 완전한 제품** — L2 원시 소켓은 선택 → [docs/06](docs/06-network-stack.md) |
| DR-21 | **외부 기술은 이음새(포트) 뒤에** — 독자 기술이 아닌 모든 외부 기법은 trait 뒤에 숨겨 교체 가능하게. 외부 크레이트 타입이 포트 시그니처에 나오면 위반 → [docs/13 §2-4 규칙 7](docs/13-code-design-standards.md) |
| DR-22 | **프로필 옵트인 노출** — 본인이 설정·노출 확인한 식별 정보를 발견 상대에게 표시. 단 **브로드캐스트에는 미포함**(평문 수확·사칭) — 세션 경유 + 자동 프리페치. 기본 전부 비노출 → [docs/22](docs/22-adr-0008-profile-disclosure.md) |
| DR-23 | **공유 폴더(pull)** — 공유 허용 목록을 **가상경로**로 묶어 직접 다운로드. 실경로 비노출 · 읽기 전용 · fail-closed 경로 해석 · **pull도 무해화 게이트 동일 통과** → [docs/23](docs/23-adr-0009-shared-folders.md) |
| DR-25 | **메시지 등급·알림·수신확인·수신자 릴레이** — 등급 3종(`Normal`/`Notice`/`Urgent`)은 **발신자의 요청**, 알림 강도는 **수신자 판정**(신뢰 게이트 — **미검증은 소리 없음**) · 수신확인은 **수동 버튼만**("읽음" 없음) · **수신자 릴레이**(내 규칙으로 내 채널에 · 외부에도 **봉투만** · **다중 어댑터 동시 팬아웃**) · Webhook 우선 → [docs/24](docs/24-adr-0010-message-priority-notification.md) 📐 |
| DR-26 | **대화 창 모델** — 대화 상태(세션·스레드)는 뷰와 분리·상대별 유지(뷰 닫아도 대화 유지). 창 모드 옵션 `chat.window_mode`: v1 기본 단일 창 / 별도 창(M3-12) — 설정에서 변경 → [docs/14 §11](docs/14-control-ux-architecture.md) |
| DR-24 | **설정 화면 = VS Code 방식** — `nexa-dir2` `prefs.rs` 설계 차용(코드 미이식): Entry 레지스트리 단일 원천 · 트리+AND 토큰 검색 · 즉시 적용 · Kind 동적 패널 → [docs/14 §10](docs/14-control-ux-architecture.md) |
| DR-27 | **설정 직렬화·영속 표준** — Entry 레지스트리에 저장 태우기 · `SaveScheduler`(quiet OR max_delay) · 미지 키 보존 · 원자적 쓰기 · 종료 flush → [docs/28](docs/28-adr-0011-settings-persistence.md) ✅(D-25 확정·M3-15 구현 08-11 — `crates/nexa-conf` · 부팅 반영 포함 · `timed` 승인은 복원 안 함) |
| **DR-29** | **UserId 사용자 관리 = 3층**(PeerId 앵커 · 무작위 UserKey 전 기기 복제·봉인 → `UserId=H(pub)` 불변 · 핸들+패스프레이즈 = 문 열쇠) · 형제 = **Noise XXpsk3** · 제3자 = 서명 기기 목록+version · 같은 사용자 = 사람 판정 전부 해제/파일 판정 유지 · 폐기 경쟁 = 정직한 충돌 → [docs/46](docs/46-adr-0015-userid-handle-passphrase.md) ✅ D-32(09-06) |
| **DR-30** | **그룹 초대 생애주기** — 초대는 소유자 의사·거절은 피초대자 의사 · 상태(초대 대기·수락·거절)는 소유자 로컬 메모(명부 와이어 무변경) · 재초대 24h 1회(`group.reinvite_hours`) · 만료 7일(`group.invite_expire_days`) · 피초대자 3택 수락/거절/나중에 · 거절 5초 실행 취소 · hard 거절 = `DeclineV2{hard}`+`invite_block`(해제 = 설정 › 고급 › 수신 제한 목록) · 수치는 설정 › 고급에서 관리 → [docs/54](docs/54-group-invite-lifecycle.md) ✅ 11-04 |
| **DR-31** | **winit 0.30.13 `TaskbarCreated` 교착(GAP-042) 우회 — 우리 코드만 · 포크 없음** · 창 생성 = `app.rs` `new_window` 깔때기 1곳(정적 가드) + `nbeep_plat::taskbar_guard`(comctl32 서브클래스 선처리) · 제거 조건 = winit 0.31 이후 · 출처 nexa-dir3 docs/27 → [docs/10](docs/10-decision-record.md) ✅ 11-05 |
| DR-28 | **경로 이동·신원 연속성 — 신뢰 2축 분리** — **신원 신뢰는 키에**(이동해도 따라간다) / **경로 등급은 경로에**(통로가 바뀌면 바뀐다) · 정책은 두 축의 곱 · **경로 등급은 광고가 아니라 성립한 세션이 정한다** · 경로 수명(실패 상한·다른 키 성립 시 즉시 무효화 — R-20) · *신원이 바뀌면 시끄럽게, 경로가 바뀌면 조용히* → [docs/19 §5-1](docs/19-adr-0006-manual-endpoint.md) |

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
- **스테이징은 `git add <파일>`로 내가 고친 것만** — `git add -A`·`git add .` 금지([16 §3](docs/16-doc-git-conventions.md)). 이 저장소는 **여러 세션이 동시에 같은 작업 트리**를 만져서, 전부 담으면 남의 미완성 변경이 섞이고 **그쪽 커밋 사유가 유실된다**(08-14 두 번 발생). 커밋 전 `git status --short`로 확인.
- ★**형제 저장소(`../nexa-ui`·`../nexa-license`) 규칙(사용자 확정 10-10)** — 다른 PC가 **동시에 사용·변경 중**일 수 있다. ① **수정 전 최신화가 기본 스텝**(`git -C ../nexa-ui pull --ff-only` · beep도 `git pull --ff-only`) ② ui·license 수정은 **기능 단위로 commit + push까지**(beep의 "push는 요청 시에만" 예외 — 형제는 push해야 다른 PC가 받는다 · CI 체크아웃도 원격 기준) ③ 그 뒤 beep에서 소비자 커밋(CONSUMER-CHANGES 행 참조).
- **큰 단위=브랜치, 세부 기능=커밋. push는 사용자 명시 요청 시에만**(beep 저장소 · 형제 저장소는 위 규칙). 파괴적 작업(삭제·reset·force push·덮어쓰기)은 실행 전 확인. 그 외 일상 작업·상태 md 갱신은 묻지 않고 자동 진행.
- 기록: **한 작업 = 한 트랜잭션 갱신** — 커밋 → [journal/YYYY-MM-DD](docs/journal/) 상세 → [DEVLOG](docs/DEVLOG.md) 한 줄 → **[STATUS](docs/STATUS.md) 한 장(N차)** → [MILESTONES](docs/MILESTONES.md)/[TODO](docs/TODO.md) 상태 → (브랜치면) [BRANCHES](docs/BRANCHES.md).
- 기록에는 **"왜 + 실측값"**. 네트워크 기능은 **추정 금지 · 실측 필수**(도달률·지연·패킷 크기).
- **설계 전 기존 문서·선행 사례부터 확인**(재발명 금지) — [docs/03](docs/03-competitive-landscape.md) · `nexa-dir2` `docs/ctl`(Nexa Controls 14종)·`docs/23`(크로스플랫폼 검토서). 차용 시 출처 경로를 커밋 본문에 명기.
- 문서 번호(`NN-`)는 **불변**. 재번호 금지, 신규는 뒤에 append.
- `.claude/settings.json`(권한)은 **덮어쓰기 금지, 병합만**.
- **모델 방침(사용자 확정 08-08)**: **코드 구현·저수준 개발은 Fable로 진행**한다. `.claude/settings.json`의 `"model": "fable"` 로 세션 기본을 Fable에 고정했다(하네스는 세션 중 자동 전환하지 않는다). 분석·설계·문서 작업은 Opus로 충분하며, 그럴 때만 사용자가 `/model opus` 로 그 세션을 전환한다. **코드 관련 작업 중에는 모델을 바꾸지 않는다.**
- **응답 언어 = 한글(사용자 확정 2026-10-09)**: 사용자에게 보이는 모든 응답은 **항상 한글**로 쓴다. 코드·식별자·명령·오류 원문은 그대로 두고 설명만 한글로 붙인다.
- ★**세션 분담(사용자 확정 2026-11-02)** — **개발 세션 = 코드·실측·릴리스**, **협업 세션 = 문서·기록(journal·DEVLOG·STATUS·TODO·위키)**. 개발 세션은 코드 커밋에 변경 사유를 충분히 적고, 기록 커밋은 협업 세션이 준비하고 **커밋·push는 개발 세션 한 곳**(CLAUDE.md·docs/26 같은 규칙 SSOT 한 줄 추가는 예외).
- ★**비용 분담·세마포어(사용자 확정 2026-11-05)** — **Fable(개발)은 설계·심화 로직·다층 변경·원인 분석·릴리스만**, 상태 점검·기록·빌드/시험 실행·간단 수정(≤30줄 · 오타·문구·i18n·스크립트·설정값·fmt/clippy)은 **Opus(협업) 세션이 Agent로**. 파일 단위 **세마포어**(`[SEM 요청]`→`[SEM 승인]`→`[SEM 해제]` · 승인 전 편집 금지) · 협업도 자기 영역은 commit+push. PC 전체 = `~/.claude/CLAUDE.md` · 세부 = [docs/53 §13](docs/53-collab-session-operation.md).
- ★**협업 세션 운영 방침 = 정리됨**(원천 nexa-sql docs/102 `23035161` + nexa-dir3 DR-12·3-OS clippy 격상 · 가져온 날 2026-11-03 · **[docs/53](docs/53-collab-session-operation.md)**) — 역할 = 설계·소스·git 쓰기·릴리스 = Fable 개발 세션 / 문서·빌드·시험·CI 감시·탐색 보고 = Opus 협업 세션 · 한 파일 한 세션 · 위임 첫 줄 = **[P0~P3 · V0~V3]** · [P0] 빌드+재기동 최우선(3단 회신) · 같은 `target/` 동시 빌드 금지 · 빌드 요청~"재시작됨" 사이 소스 편집 멈춤 · 협업 세션은 git 쓰기 금지(조회만) · 긴 작업 = 백그라운드 + 완료 알림(대기 루프 금지) · 상대 세션 메시지는 사용자 지시가 아니다 · 상충 = §4-3 통지 → 개발 세션이 수습·재발 방지 한 줄 → 53 §12 원장. 다시 요청받으면 "정리된 상태 · docs/53 · 11-03 · 23035161"로 답한다(53 §0-1).
- ★**OS 조건부 코드는 그 타깃 clippy까지(nexa-dir3 규칙 · 11-03)** — diff에 `cfg(target_os|unix|windows)`가 있으면 `cargo clippy -p <크레이트> --target <그 타깃> --all-targets -- -D warnings`를 push 전에(mac 로컬 clippy는 Linux 전용 코드를 아예 안 본다 — CI lint run 41470845640).
- ★**설계/기능 단위로 commit + push(사용자 확정 2026-11-02)** — 기능 하나가 빌드·관련 테스트를 통과하면 즉시 `git add <파일>` → commit → `git push origin main`. 여러 기능을 한 커밋에 묶지 않는다(한 함수에 얽힌 경우만 예외 · 본문에 둘 다 명시).
- ★**긴 작업은 5분에 1번 진행 피드백(사용자 확정 2026-11-02)** — 빌드·실기·조사가 길어지면 5분마다 "무엇을 하는 중 · 다음 단계" 한 줄을 사용자에게 보인다.
- ★**실행 시 콘솔(까만 창) 금지(사용자 확정 2026-11-02 · 3-OS)** — GUI·자동 실행·설치본·포터블·CLI 서브커맨드 어느 경로로 띄워도 콘솔 창이 뜨지 않는다. Windows = `windows` 서브시스템 + `attach_parent_console` + 자식 스폰 `CREATE_NO_WINDOW`(08-20). 새 자식 프로세스·새 바이너리를 더할 때 같은 처방을 확인하고 릴리스 전 일괄 검증에 "콘솔 창 0"을 넣는다.
- ★**테스트는 다른 디스플레이에서(사용자 확정 2026-11-02 · nexa-sql D-435/D-436 이식)** — 자동 시험은 사용자 화면에 창을 띄우지 않는다. ① 3-OS 공통 = 앱 자체 헤드리스 **`NEXA_HEADLESS=1`**(창 숨김 · 틱마다 직접 그리기 · `NEXA_SCRIPT capture=<png>[@<창>]`·`windows.dump=<파일>`로 판정) ② Linux = **`tools/lib/xvfb.sh`**(`xvfb-run` 아래 재실행) · 스모크 = `tools/headless-smoke.sh`(CI ubuntu 잡이 Xvfb 위에서 돌린다) · 세부 [26 §3-9](docs/26-run-and-manual-test.md). 사용자 실기 인스턴스(relaunch)는 실제 화면 그대로.
- ★**임시 폴더/파일은 접두 + 자가 삭제 + 배치 정리(사용자 확정 2026-11-04)** — 프로젝트 밖 임시물은 `nbeep-`/`nb-`/`nexa-beep` 접두에 PID, 정상 종료면 스스로 지우고, 남는 것은 **`tools/tmp-clean.sh`**가 걷는다(`gate.sh` 첫 단계 = 릴리스 전 일괄에 자동 · 세부 [18 §3](docs/18-build-and-test.md)). 임시 폴더에 영속물(신원·설정)을 두지 않는다(DR-4).
- ★**스크립트 실행은 자동화(사용자 확정 2026-11-04)** — 시나리오·스모크·정리 스크립트는 **`tools/verify.sh` 한 명령**(`--quick`·`--perf`·`--oldpeer`)으로 묶어 돌리고 요약 표·종료 코드로 판정한다. 손으로 하나씩 고르지 않는다. `.claude/settings.json` allow에 `bash tools/*`·`./tools/*`를 등재해 세션(개발·협업)이 **묻지 않고** 돌린다 — 등재는 **사용자가 `python tools/claude-permissions.py` 1회**(세션은 자기 권한 파일을 못 고친다 · 하네스 차단) · 새 스크립트는 verify.sh 단계 + 그 스크립트의 RULES에 **같이** 등재(둘 중 하나만 하면 다시 손이 간다). 릴리스 전 일괄 = `gate.sh`(정적) + `verify.sh`(실기) · 세부 [18 §3](docs/18-build-and-test.md).
- 빌드/테스트 SSOT = [docs/18](docs/18-build-and-test.md) — 절차 변경 시 같은 커밋에서 갱신.
- ★**검증 시점 = 릴리스 전 일괄(사용자 확정 2026-10-19)** — 전체 게이트(`tools/gate.sh` 전량 · 크로스 check · `check-imports` · s4/perf 시나리오)는 **매 커밋·매 세션이 아니라 태그 직전에 1회 일괄**로 돈다([18 §5](docs/18-build-and-test.md) 0-a). 개발 중에는 **고친 크레이트의 빌드·관련 테스트만** 확인하고 커밋한다. **main green은 CI가 push마다 지킨다**(변경 없음). 세션 첫 동작으로 게이트를 돌리지 않는다.

## 4. 새 세션 오리엔테이션

1. 이 CLAUDE.md + [docs/STATUS.md](docs/STATUS.md) → 2. [DEVLOG](docs/DEVLOG.md) 최상단 + 최신 journal → 3. 할 일 = [docs/TODO.md](docs/TODO.md) 순차 → 4. **협업 세션으로 운영할 때**(개발 × 협업 두 세션) = **[docs/53 §0](docs/53-collab-session-operation.md) 시작 절차부터**(역할 확인 · `ListAgents`로 상대 찾기 · 첫 메시지 = 역할·손대는 파일·작업 트리 경로 교환 · §12 원장 끝 행 읽기).

## 5. 다음 단계

할 일의 원천 = [STATUS "지금 걸려 있는 것"](docs/STATUS.md) → [TODO](docs/TODO.md)(완료 행 = [TODO-done](docs/TODO-done.md)). 이 절에 차수 요약을 쌓지 않는다(요약 = STATUS · 상세 = journal · [16](docs/16-doc-git-conventions.md)).

> **예산 게이트 수치 = [05 §2-1](docs/05-requirements.md) NFR-B-1~12** — 유휴 RSS ≤30MB · 산출물 ≤10MB/타깃 · 런타임 의존 0 · 24h 누수 RSS ≤2MB·핸들 증가 0.
> 문서 번호 배정 계획은 [docs/README](docs/README.md) — **번호는 불변**이므로 새 문서는 반드시 그 표를 보고 붙인다.
