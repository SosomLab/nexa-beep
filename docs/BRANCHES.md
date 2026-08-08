# BRANCHES — 브랜치 기록 (Branch History, 시간 역순)

> **목적**: 병합 후 삭제되는 작업 브랜치의 이력을 남긴다. **정렬: 시간 역순(최신이 위)** — 새 브랜치는 표·상세 모두 맨 위에 추가. 시각 = 커밋 committer date(KST).
> **규약**: 브랜치는 main 병합·green 확인 후 삭제, 이력은 이 문서 + journal에 보존. push는 사용자 명시 요청 시에만([16 §4](16-doc-git-conventions.md#4-브랜치--푸시--반드시-준수)).
> **삭제 열** = 로컬 ref를 실제로 지운 날. 병합 후 ref가 남아 있었다면 정리한 날짜로 적고 `(정정)`을 붙인다.

## 요약 (시간 역순)

| 브랜치 | 생성 | 병합(커밋) | 삭제 | 커밋수 | 작업 요약 | 상세 |
| --- | --- | --- | --- | --- | --- | --- |
| `feat/m1-wire` | 2026-08-08 | 2026-08-08 (1ab72a5 --no-ff) | 2026-08-08 | 2 | **D-22 확정 구현 + M1-3 와이어 포맷** — `instance` 16B·`CloneWatch`(U-P1) · `SelfPeer` 거부(U-P2) · 512B 강제·전방 호환·골든 레이아웃 | [journal/2026-08-08](journal/2026-08-08.md) |
| `feat/m1-linkwatch`(+`feat/m3-settings` 스택) | 2026-08-08 | 2026-08-08 (625c725 --no-ff) | 2026-08-08 | 7 | **M3-11 설정 화면 1·2**(Entry 레지스트리·`⌘/Ctrl+,`·좌측 카테고리 트리·`chat.window_mode`/`ui.theme` 즉시 적용) · **M1-2 슬라이스 1**(LinkEvent·trailing Debouncer) · **Docker 테스트베드 실증**(Linux 146 green·멀티캐스트 도달) · **Windows VM 불요 확정**(D-8a 재정의) · 문서 현행화(병렬) | [journal/2026-08-08](journal/2026-08-08.md) |
| `feat/m3-multiwin` | 2026-08-08 | 2026-08-08 (96218ab --no-ff) | 2026-08-08 | 2 | **M3-12 다중 대화 창** — `WinEntry` 창 단위 라우팅·Separate/Single 모드·재활성화=포커스·창 닫기=뷰만(DR-26 분리 회수 — 도메인 변경 0) | [journal/2026-08-08](journal/2026-08-08.md) |
| `feat/m3-inmem-e2e` | 2026-08-08 | 2026-08-08 (b694c26 --no-ff) | 2026-08-08 | 3 | **InMemory 종단 데모**(실물 발견→Noise→TOFU→다중화→대화 왕복 — M2 게이트 GUI 실증) · **DR-26**(대화 상태-뷰 분리 즉시 반영·창 모드 설정 옵션·[14 §11]) | [journal/2026-08-08](journal/2026-08-08.md) |
| `feat/m3-chatview` | 2026-08-08 | 2026-08-08 (e346e5f --no-ff) | 2026-08-08 | 2 | **대화 화면 첫 슬라이스** — `ChatViewWidget`(스레드=`SafeText` 타입 강제·임시 한 줄 입력)·목록↔대화 전환·발신 실물 도메인 경로(Identity·Sequencer·봉투)·snow 첫 bin 링크(+131KB) | [journal/2026-08-08](journal/2026-08-08.md) |
| `feat/m3-fontmap` | 2026-08-08 | 2026-08-08 (c78e0d1 --no-ff) | 2026-08-08 | 2 | **폰트 메모리 매핑(R-15)** — memmap2 채택(사용자 확정)·`Box::leak` 프로세스 수명·gfx `FontRef<'static>` 전환·**RSS −51MB 실측**·plat→gfx 의존 제거 | [journal/2026-08-08](journal/2026-08-08.md) |
| `feat/m3-dpi` | 2026-08-08 | 2026-08-08 (99d1304 --no-ff) | 2026-08-08 | 2 | **실기 피드백 3건** — 고DPI 배율(RasterCtx/위젯 scale·히트테스트 일관·`ScaleFactorChanged`) · 한글 타입어헤드(IME Commit 라우팅) · Enter 피드백(하단 상태바) | [journal/2026-08-08](journal/2026-08-08.md) |
| `feat/m3-widgets` | 2026-08-08 | 2026-08-08 (a2a0425 --no-ff) | 2026-08-08 | 3 | **M3-1 슬라이스 1·2** — `nexa-gui` 인프라 이식(geom/event/widget/theme/draw/typeahead) · **`RasterCtx`**(CPU 래스터 백엔드 — SDF AA) · **`PeerListWidget`**(첫 실물 위젯 — 부분 무효화 단언·타입어헤드) · bin 인터랙티브 배선 | [journal/2026-08-08](journal/2026-08-08.md) |
| `feat/m2-mux` | 2026-08-08 | 2026-08-08 (840d778 --no-ff) | 2026-08-08 | 12 | **M2 잔여 + M1 선행 + 텍스트 스택** — M2-2b(안전번호 60자리) · CI 회귀 게이트(테스트 의무) · M2-3(`MuxSession` 다중화·백프레셔) · M2-4(`ChatMessage` 봉투·시퀀스·중복 제거·팬아웃) · M2-6(`safetext` 무해화·링크) · M1-5(`PeerTable` 지문 병합·경로 단위 이탈) · SP-1c 실측→**ab_glyph 채택** · M1-6(gfx `Surface`/`Font`·plat 폰트·ui 피어 목록+배지 3종·실창 데모) · ADR-0010 개정·25 기능 범위(병행 세션) | [journal/2026-08-08](journal/2026-08-08.md) |
| `feat/m2-session` | 2026-08-08 | 2026-08-08 | a52eac1 | ✅ 병합 | **M2-1 세션** — a) `Link` core 이관·`Session` 트레이트·`PlainSession` 스텁·`duplex` fake  b) **실물 `NoiseSession`(snow · Noise_XX)**·`Identity`(X25519=PeerId)·평문 미노출 검증. snow 라이선스 퍼미시브  c) **M2-2 TOFU** — `TrustStore`·`TrustedSession` 데코레이터·SAS `safety_number` | [journal/2026-08-08](journal/2026-08-08.md) |
| `feat/m1-transport` | 2026-08-08 | 2026-08-08 (0c6e20c --no-ff) | 2026-08-08 | 2 | **M1-1 net 골격** — ADR-0003 Transport 경계(`Transport`/`Link`/`PeerHint`/`DiscoveryEvent`)·`InMemoryTransport` fake(소켓 없이 발견·연결·이탈)·core `DisplayName`(RLO 무해화 FR-S-13). ADR-0003 Accepted | [journal/2026-08-08](journal/2026-08-08.md) |
| `spike/sp1-budget` | 2026-08-08 | 2026-08-08 (da98c96 --no-ff) | 2026-08-08 | 2 | **M0-2 SP-1 예산 검증 + P2 채택** — winit+softbuffer: 빈 창 0.40MB·퍼미시브 100%·의존성 Win15/mac32/Linux75. R-8 크기 해소. CI 4타깃 크기 게이트. 잔여 SP-1b/d/e=실기 | [journal/2026-08-08](journal/2026-08-08.md) |
| `feat/m0-scaffold` | 2026-08-08 | 2026-08-08 (`95ea048` --no-ff) | 2026-08-08 | 6 | **M0 기반** — 스캐폴딩(9크레이트·의존성 역전)·횡단 골격(ActionKind 파이프라인·포트)·안정 코드·마스킹·**CI 4잡+빌드/테스트 SSOT+의존성 원장**. 29테스트 green·280KB. 남은 것 = M0-2 SP-1(실물 창 필요) | [journal/2026-08-08](journal/2026-08-08.md) |
