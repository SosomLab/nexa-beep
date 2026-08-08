# BRANCHES — 브랜치 기록 (Branch History, 시간 역순)

> **목적**: 병합 후 삭제되는 작업 브랜치의 이력을 남긴다. **정렬: 시간 역순(최신이 위)** — 새 브랜치는 표·상세 모두 맨 위에 추가. 시각 = 커밋 committer date(KST).
> **규약**: 브랜치는 main 병합·green 확인 후 삭제, 이력은 이 문서 + journal에 보존. push는 사용자 명시 요청 시에만([16 §4](16-doc-git-conventions.md#4-브랜치--푸시--반드시-준수)).
> **삭제 열** = 로컬 ref를 실제로 지운 날. 병합 후 ref가 남아 있었다면 정리한 날짜로 적고 `(정정)`을 붙인다.

## 요약 (시간 역순)

| 브랜치 | 생성 | 병합(커밋) | 삭제 | 커밋수 | 작업 요약 | 상세 |
| --- | --- | --- | --- | --- | --- | --- |
| `feat/m3-widgets` | 2026-08-08 | _진행 중_ | — | 2 | **M3-1 슬라이스 1·2** — `nexa-gui` 인프라 이식(geom/event/widget/theme/draw/typeahead) · **`RasterCtx`**(CPU 래스터 백엔드 — SDF AA) · **`PeerListWidget`**(첫 실물 위젯 — 부분 무효화 단언·타입어헤드) · bin 인터랙티브 배선 | [journal/2026-08-08](journal/2026-08-08.md) |
| `feat/m2-mux` | 2026-08-08 | 2026-08-08 (840d778 --no-ff) | 2026-08-08 | 12 | **M2 잔여 + M1 선행 + 텍스트 스택** — M2-2b(안전번호 60자리) · CI 회귀 게이트(테스트 의무) · M2-3(`MuxSession` 다중화·백프레셔) · M2-4(`ChatMessage` 봉투·시퀀스·중복 제거·팬아웃) · M2-6(`safetext` 무해화·링크) · M1-5(`PeerTable` 지문 병합·경로 단위 이탈) · SP-1c 실측→**ab_glyph 채택** · M1-6(gfx `Surface`/`Font`·plat 폰트·ui 피어 목록+배지 3종·실창 데모) · ADR-0010 개정·25 기능 범위(병행 세션) | [journal/2026-08-08](journal/2026-08-08.md) |
| `feat/m2-session` | 2026-08-08 | 2026-08-08 | a52eac1 | ✅ 병합 | **M2-1 세션** — a) `Link` core 이관·`Session` 트레이트·`PlainSession` 스텁·`duplex` fake  b) **실물 `NoiseSession`(snow · Noise_XX)**·`Identity`(X25519=PeerId)·평문 미노출 검증. snow 라이선스 퍼미시브  c) **M2-2 TOFU** — `TrustStore`·`TrustedSession` 데코레이터·SAS `safety_number` | [journal/2026-08-08](journal/2026-08-08.md) |
| `feat/m1-transport` | 2026-08-08 | 2026-08-08 (0c6e20c --no-ff) | 2026-08-08 | 2 | **M1-1 net 골격** — ADR-0003 Transport 경계(`Transport`/`Link`/`PeerHint`/`DiscoveryEvent`)·`InMemoryTransport` fake(소켓 없이 발견·연결·이탈)·core `DisplayName`(RLO 무해화 FR-S-13). ADR-0003 Accepted | [journal/2026-08-08](journal/2026-08-08.md) |
| `spike/sp1-budget` | 2026-08-08 | 2026-08-08 (da98c96 --no-ff) | 2026-08-08 | 2 | **M0-2 SP-1 예산 검증 + P2 채택** — winit+softbuffer: 빈 창 0.40MB·퍼미시브 100%·의존성 Win15/mac32/Linux75. R-8 크기 해소. CI 4타깃 크기 게이트. 잔여 SP-1b/d/e=실기 | [journal/2026-08-08](journal/2026-08-08.md) |
| `feat/m0-scaffold` | 2026-08-08 | 2026-08-08 (`95ea048` --no-ff) | 2026-08-08 | 6 | **M0 기반** — 스캐폴딩(9크레이트·의존성 역전)·횡단 골격(ActionKind 파이프라인·포트)·안정 코드·마스킹·**CI 4잡+빌드/테스트 SSOT+의존성 원장**. 29테스트 green·280KB. 남은 것 = M0-2 SP-1(실물 창 필요) | [journal/2026-08-08](journal/2026-08-08.md) |
