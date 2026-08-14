# TODO — 할 일 백로그 (목표 순차 · living)

> **목표를 향한 순차 진행 순서.** 새 항목은 해당 단계 섹션에 append.
> **규모**: 소=반나절/1커밋 · 중=1~2일/1~3슬라이스 · 대=3일+/설계(ADR) 동반.
> **상태**: ☐ 대기 · 🚧 진행 · ✅ 완료(커밋) · ⏸ 보류. **우선**: P0(정체성·게이트) · P1 · P2.
> 단계별 원안·게이트 근거는 [02 로드맵](02-roadmap.md), 기능 관점은 [MILESTONES](MILESTONES.md).

---

## 🗺️ 개발 전체 흐름 (한눈에)

```
M-1 설계 ──► M0 기반 ──► M1 발견 ──► M2 대화 ──► M3 셸·UI ──► M4 전송·무해화 ──► M5 그룹·마감·배포 ──► v1 출시
 (거의 완료)   예산검증     서로 보임    암호화 대화    매일 쓸 만함    파일 안전 송수신     그룹·스팸방어·배포        │
                 │                                                                                        ▼
                 └── SP-1 실패 시 스택/예산 재검토(R-8)                                            v2: 릴레이·다중기기·백업
```

| 단계 | 목표(끝나면 되는 것) | 병합 게이트 | 해소 리스크 | 진행 |
|---|---|---|:--:|:--:|
| **M-1 설계** | 무엇을·어떻게 만들지 확정 | **ADR 11종** + 요구/아키텍처 확정 | — | 🚧 **거의 완료** |
| **M0 기반** | "이 예산이 가능한가"에 답 | **SP-1 통과** 또는 NFR-B 정정 | **R-8** | **🚧** |
| **M1 발견** | 두 대 켜면 서로 목록에 뜬다 | 발견 ≤3s(P95) · **격리 AP에서 발견**(S-3) | **R-2·R-7** | **🚧** |
| **M2 대화** | 암호화된 1:1 대화 + 기록 | 경로 변경에도 스레드 연속 · InMemory 테스트 green | R-1 완화 | **🚧** |
| **M3 셸·UI** | 매일 켜 둘 만한 프로그램 | **OS 구분 불가**(S-7) · 60fps · 유휴 RSS | **R-3·R-9** | **🚧** |
| **M4 전송·무해화** | 파일을 안전하게 | 안전 회귀 전항목 · 전송 링크 70% | **R-5** | ☐ |
| **M5 그룹·마감·배포** | **v1 출시** | **성공기준 S-1~S-8** · 예산 전항목 | R-4 | ☐ |
| **v2** | 릴레이 · 다중 기기 · 백업 | 별도 계획 | — | ⏸ |

**크리티컬 패스(08-12 현행)** — ~~M0-1 → M0-1b → SP-1(크기 축)~~ ✅ → ~~D-22 확정~~ ✅ → ~~M1-3 와이어 포맷~~ ✅ → ~~M2-7 비동기 펌프~~ ✅ → ~~M4-9 종단 ack~~ ✅ → ~~D-25 → M3-15 설정 영속~~ ✅ → ~~D-18 §3 → M2-5a 신원·핀 영속~~ ✅ → ~~M5-4 배포 2채널~~ ✅(v0.1.4 공개) → **지금 = 🔴 D-23 확정(M2-4b → M3-8/9/10 연쇄) · 🔴 D-24 범위 확정 · M4-5ⓒ 권한 강등(R-5 종결) · M1 잔여(M1-2·M1-8·M1-9) · M5-1 그룹(v1 핵심 미착수)** → M2-5b 기록 저장(D-18 §4~ 대기) → M5-5 24h 누수 → v1.
> ⚠️ **배포가 앞서 나갔다** — M5-4는 끝났지만 **M5-1 그룹 전송이 미착수**다. v1 약속("발견된 사용자를 그룹으로 묶어 전송")의 절반이 아직 없다.
**신규 항목 의존** — `D-23 → M2-4b → {M3-8, M3-9} → M3-10` · `M0-2b(SP-2) → M3-10` · `ADR-0009 → M4-7 → M4-8` · `D-24 → v1.0/v1.1 재정렬`.
**병렬 가능** — D-13(L1 구독)·D-14(소켓 검증표)는 M0 직후 M1과 나란히. M3 UI 시안(M3-1c)은 M2 중 착수 가능.
🔐 **보안은 마일스톤이 아니라 상시다** — 열린 보안 항목과 점검 트리거는 **[§9 보안 상시 관찰](#9--보안-상시-관찰-sec--끝나지-않는-항목)**. 기준은 [29](29-wire-security-audit.md) W-1~W-12.
**막힌 것** — **D-8 발견 스파이크는 실기 2대+ 필요**(대행 불가). ADR-0002 타이밍 6종이 여기 묶여 M1 완결을 잠근다.

### 설계 확정 현황 (ADR)

| ADR | 주제 | 상태 |
|---|---|:--:|
| [0001](07-adr-0001-stack.md) 스택 | Rust·자체 CPU 래스터라이저·P2·Wayland+X11·시스템폰트 | ✅ Accepted |
| [0002](08-adr-0002-discovery-transport.md) 디스커버리/전송/암호 | 자체 UDP·TCP·Noise_XX·TOFU | ✅ Accepted |
| [0007](20-adr-0007-multi-device-identity.md) 다중 기기 신원 | UserId 1:M PeerId·주 기기+복구 시드 | ✅ Accepted(구현 v2) |
| [0003](09-adr-0003-transport-abstraction.md) 전송 추상화 | 4계층 경계·InMemory | ✅ Accepted(구현으로 비준) |
| [0004](11-adr-0004-quarantine.md) 수신 무해화 | `.beepq`·등급·상태기계 | 📐 **Proposed — 확정 대기** |
| [0005](17-adr-0005-history-at-rest.md) 기록 저장 암호화 | 블라인드 인덱스·크립토 셰레딩 | 📐 **부분 확정** — §3 키 관리 ✅(08-11 · M2-5a 구현) / **§4~ 확정 대기**(M2-5b 선행) |
| [0006](19-adr-0006-manual-endpoint.md) 수동 엔드포인트 | `Locator::Manual`·원격 신뢰등급 | 📐 **Proposed — 확정 대기** |
| [0008](22-adr-0008-profile-disclosure.md) 프로필 옵트인 노출 | 브로드캐스트 미포함·세션 경유 프리페치 | ✅ Accepted |
| [0009](23-adr-0009-shared-folders.md) 공유 폴더 | 가상경로·pull·fail-closed | ✅ Accepted |
| [0010](24-adr-0010-message-priority-notification.md) 등급·알림·수신확인·수신자 릴레이 | 등급=요청 / 강도=수신자 판정 · 릴레이 팬아웃 | 📐 **Proposed — 확정 대기**(D-23) |
| [0011](28-adr-0011-settings-persistence.md) 설정 직렬화·영속 | Entry 레지스트리=저장 스키마 · coalescing 저장 · 미지 키 보존 | ✅ **Accepted**(08-11 · D-25 — `crates/nexa-conf` 구현) |
| [0012](31-adr-0012-shared-group-chat.md) 공유 그룹 채팅 | 소유자 roster 서명 · 초대 수락제 · pairwise 팬아웃 | 📐 **Proposed — 확정 대기**(D-28) |
| [0013](32-adr-0013-server-modes.md) 서버 모드(릴레이·컨텐츠·관리) | 서버가 **아는 것**의 3단계 · S-0~S-3 불변식 · **§5 그룹 결합**(P-9~P-11 선행 제약) | 📐 **Proposed — 확정 대기**(D-29) · **방향성만**(구현은 별도 저장소·v1 이후 — DR-9) |

---

## §1. M-1 — 설계

| ID | 항목 | 우선 | 규모 | 의존 | 상태 |
|---|---|---|---|---|---|
| D-1 | 문서 표준 차용 + 4층 골격 생성([16](16-doc-git-conventions.md) §6) | P0 | 소 | — | ✅ (08-08) |
| D-2 | **경쟁 프로그램 전수 조사** — 국내/국외 목록 + 기능 매트릭스 → [03](03-competitive-landscape.md) | P0 | 중 | D-1 | ✅ (08-08 — 37종·A~D 4계층·매트릭스 23기능×7제품) |
| D-3 | 경쟁 프로그램 **장단점** + 차별화 지점 도출 → [03 §5·§6](03-competitive-landscape.md) | P0 | 중 | D-2 | ✅ (08-08 — 시장 공백·보안 충돌·발견 제약·크기 축 4건) |
| D-4 | [00 비전](00-vision.md) — 왜 만드나·차별화·성공 기준 | P0 | 소 | D-3 | ✅ (08-08 — 경쟁 좌표표·성공기준 S-1~8·긴장 V-1~7) |
| D-5 | [05 요구사항](05-requirements.md) — FR/NFR(**예산 수치 확정**)/제약/리스크 | P0 | 중 | D-4 | ✅ (08-08 — FR·NFR-B 예산 게이트·제약 C-1~6·리스크 R-1~15) |
| D-6 | **ADR-0001 스택** → [07](07-adr-0001-stack.md) | P0 | 대 | D-5 | ✅ **Accepted** (Rust·자체 CPU 래스터라이저·P2·Wayland+X11·시스템폰트) |
| D-7 | **ADR-0002 디스커버리·전송·암호** → [08](08-adr-0002-discovery-transport.md) | P0 | 대 | D-6 | ✅ **Accepted** (자체 UDP·TCP·Noise_XX·TOFU·발견은 미검증 힌트) |
| D-9 | **ADR-0003 전송 추상화** → [09](09-adr-0003-transport-abstraction.md) | P0 | 중 | D-7 | ✅ **Accepted** (08-08 · M1-1에서 구현) |
| D-10 | [01 아키텍처](01-architecture.md) · [02 로드맵](02-roadmap.md) | P0 | 중 | D-9 | ✅ (08-08 — 9크레이트·단방향 의존·스레딩·데이터흐름·M0~M5) |
| D-11 | **차용 자산 실측 평가** → [12](12-asset-reuse.md) | P0 | 중 | D-6 | ✅ (08-08 — **`ctl` 재사용 불가**, 실제 자산 `nexa-gui` 1,187 LOC. R-3·M3 정정) |
| D-12 | **ADR-0004 수신 무해화** → [11](11-adr-0004-quarantine.md) | P0 | 대 | D-6 | 🚧 **Proposed** |
| D-16 | **[13 코드 설계 표준](13-code-design-standards.md)** — 모듈화·포트&어댑터·`ActionKind`·인터셉터·계측 | P0 | 대 | D-10 | ✅ (08-08 — 충돌 2건 절충·리뷰 체크리스트 13항) |
| D-17 | **[14 컨트롤·UX](14-control-ux-architecture.md)** — 시각=macOS/동작=OS·3단계 이벤트 | P0 | 대 | D-16 | ✅ (08-08 — DR-16 · FR-U-9~14) |
| D-18 | **ADR-0005 기록 저장 암호화** → [17](17-adr-0005-history-at-rest.md) | P0 | 대 | D-16 | 🚧 **Proposed** — ★ **§3(키 관리)은 사용자 확정(08-11)**: A 기본(기기 키 파생 래핑)+B/C 선택 승격 → M2-5a 구현 완료. 잔여 = §4~ 전체 확정(M2-5b 전) |
| D-19 | **ADR-0006 수동 엔드포인트 등록** → [19](19-adr-0006-manual-endpoint.md) | P1 | 중 | D-9 | 🚧 **Proposed** |
| D-20 | **ADR-0007 다중 기기 신원** → [20](20-adr-0007-multi-device-identity.md) | P1 | 대 | D-9 · D-18 | ✅ **Accepted** (Q-1: 주 기기 + 복구 시드 / 기록 키 분리) |
| D-21 | **v1 제약 4건 반영**([20 §7](20-adr-0007-multi-device-identity.md)) — `UserId` 타입 자리 · 발신 경로 `PeerId` 집합 일반화 · `sender_device` 필드 · **저장 마스터 키 한 겹 래핑** | P0 | 소 | D-20 | ✅ (08-08 — M0-1b UserId·Recipients 타입 / **M2-4 `sender_device`+`fanout` 배선 완료** / 저장 키 래핑은 store doc 명시 → M2-5 구현) |

| D-22 | **신원 키 파일 복제 대응 확정**([21 §5](21-identity-spec.md) · R-12) — U-P1 `instance` 16B + U-P2 자기 `PeerId` 세션 거부 | P0 | 소 | D-7 | ✅ (08-08 — **사용자 승인(추천안 수용) · 둘 다 채택 + 구현 완료**: U-P1 = `wire.rs` `instance` 필드+`CloneWatch`(창 내 공존 탐지·재시작 무오탐), U-P2 = `NoiseSession` `SelfPeer` 거부. **탐지이지 방지 아님** 명시) |
| D-23 | 🔴 **ADR-0010 메시지 등급·알림·수신확인·수신자 릴레이 확정**([24](24-adr-0010-message-priority-notification.md) · DR-25) — ⓐ 등급 3종 + **신뢰 게이트 강등표**([24 §3-3](24-adr-0010-message-priority-notification.md)) 승인 ⓑ **수신자 릴레이를 v1에 넣을지**(R-14 TLS 크기 · SP-2 결과에 종속) — 규칙 `조건→채널 집합` · **다중 어댑터 동시 팬아웃** · 채널별 노출 수준 포함 ⓒ **N-1~N-4 프로토콜 자리**(등급 1바이트·ack 제어 타입·기록 상태 필드·ActionKind 5종)를 **M2-3/M2-4에 같이 넣는다** — 나중이면 포맷 변경 | P0 | 중 | D-7 | 🔴 **사용자 확정 대기** |
| D-24 | 🔴 **기능 범위 확정**([25](25-feature-scope.md)) — 경쟁 매트릭스 × FR 119건 대조로 **🅐입장권/🅑정체성/🅒경쟁채택/🅓사용자확장/🅔제외** 5등급 분류. **v1.0 커트라인 = 🅐+🅑+🅒4+🅓4 (94 → 약 65건)** · v1.1로 미룸 5건(검색·브로드캐스트·상태/타이핑·**공유 폴더·수신자 릴레이**) — ⚠️ **미루는 건 구현이지 설계가 아니다**(모델·프로토콜 자리는 v1.0에 함께). 서브넷 너머는 오히려 **앞당김**(S6과 같은 코드) | P0 | 중 | D-2 · D-5 | 🔴 **사용자 확정 대기** |
| ~~D-25~~ | **ADR-0011 설정 직렬화·영속 표준 확정**([28](28-adr-0011-settings-persistence.md) · DR-27) — ⓐ 설계 승인 ⓑ Q-28-1 크레이트 위치 ⓒ 기본값 | P0 | 중 | D-17 | ✅ **확정(08-11)** — ⓐ Accepted ⓑ **저장소 안 `crates/nexa-conf`**(안정화 후 분리) ⓒ quiet 1s·max 10s 유지(Q-28-2에서 실사용 조정). **M3-15 구현 완료** |

> **설계에 남은 것**: **ADR-0004·0005(§4~)·0006·0010 사용자 확정** → `Accepted`. ⚠️ **지금 실제로 잠겨 있는 것은 D-23(→ M2-4b → M3-8/9/10 알림·수신확인·릴레이 연쇄)과 D-18 §4~(→ M2-5b 기록 저장)** 둘이다. **D-12·D-19는 구현이 먼저 끝나 확정만 남았고**(M4-1 도메인 · `add_endpoint`), D-22·D-25·D-18 §3는 ✅ 확정·구현 완료.
> ⚠️ **표기 규약(08-12 정정)** — 상태 칸은 **확정 여부**, 본문은 **구현 여부**를 적는다. 둘을 섞어 적어 `Proposed`인 ADR이 `Accepted`로, 해소된 리스크가 미해결로 남아 있었다.
> **실기·코드 필요 항목**(D-8 발견 스파이크 · D-13 L1 구독 · D-14 소켓 검증표 · D-15 SP-1)은 아래 M0/M1에서 실행한다.

---

## §2. M0 — 기반 · 예산 검증

> **목표: "이 예산이 가능한가"에 답한다.** 게이트 — SP-1 통과 또는 [05 NFR-B](05-requirements.md) 정정 확정(**R-8 해소**).

| ID | 항목 | 우선 | 규모 | 의존 | 상태 |
|---|---|---|---|---|---|
| M0-1 | 프로젝트 스캐폴딩 — Cargo 워크스페이스·`rust-toolchain.toml`(stable 고정)·release 프로파일·9크레이트 골격·4타깃·린트/테스트 러너. **최소 지원 OS·MSRV 확정**([07 §8](07-adr-0001-stack.md)) | P0 | 중 | D-6 | ✅ (08-08 — 9크레이트·의존성 역전·CRT정적·크기 프로파일. build/clippy/fmt green. MSRV 1.82 잠정) |
| M0-1b | **★ 횡단 골격 선구축**([13](13-code-design-standards.md)) — `ActionKind`·`ActionCtx`·`Interceptor` 파이프라인 · 포트 스켈레톤(`Clock`/`Rng`/`Meter`/`Tracer`) · **D-21 v1 제약 4건 동시 반영**. **기능보다 먼저** | P0 | 중 | M0-1 | ✅ (08-08 — ActionKind 24종·인터셉터 파이프라인·포트 4종·신원(PeerId/UserId/Recipients)·testkit. 20테스트 green) |
| M0-1c | `ActionKind` ↔ `u16` **안정 코드 매핑표** 초판 · 민감 타입 `Debug` 마스킹 규약 적용 | P0 | 소 | M0-1b | ✅ (08-08 — stable_code 24종·골든/유일성 테스트 · `redact::Redacted/RedactedText` 헬퍼. 29테스트 green) |
| M0-2 | **★ SP-1 예산 검증 스파이크**(D-15) — 빈 창 4타깃 크기(≤3MB)·RSS(≤15MB) · 텍스트 스택 증가분 · **한글 IME** · 500행 60fps · **의존성 라이선스 전수**([07 §6](07-adr-0001-stack.md)). **R-8 해소 지점** | P0 | 대 | M0-1 | 🚧 (08-08 — **크기 차원 해소·P2 확정**. CI가 4타깃 크기 게이트. 잔여 SP-1b/d/e = 실기(디스플레이 필요). ★ **08-12 Windows RSS 관측** — 릴리스 GUI **17.1MB** · 테스트단말 8.5MB(WorkingSet · 발견만 도는 상태)로 **NFR-B-1(30MB) 안쪽 첫 관측** ⇒ R-8의 남은 축은 **mac 재실측**(85.7MB · phys_footprint)) |
| M0-2b | **SP-2 TLS 스택 크기 실측 스파이크**(Q-24-1 · R-14) — `rustls`+`webpki-roots` vs OS TLS(schannel/Security.framework/Linux?) 4타깃 릴리스 증가분. **결과가 FR-S-43(대체 알림)의 v1 포함 여부를 가른다** | P1 | 소 | M0-2 | ☐ (D-23 확정 후) |
| M0-3 | CI — 4타깃 build/test/lint + **예산 게이트**(크기·RSS·임포트 화이트리스트 · **중립 크레이트 4타깃 테스트** [12 §6](12-asset-reuse.md)) | P0 | 중 | M0-1 | ✅ (08-08 — ci.yml 4잡: lint/test(3-OS)/cross-build(ARM·Intel)/budget(≤10MB). RSS·임포트 게이트는 SP-1 후 발효) |
| M0-4 | [18 빌드&테스트](18-build-and-test.md) SSOT 실내용 · [10 §3 의존성 원장](10-decision-record.md) 개시(건별 라이선스·크기) | P0 | 소 | M0-1 | ✅ (08-08 — 명령 SSOT·CI 표·원장 개시(외부 crate 0)) |

---

## §3. M1 — 발견 (첫 수직 슬라이스)

> **목표: 두 대를 켜면 서로 목록에 뜬다.**(대화는 M2) 게이트 — 발견 ≤3s(P95, NFR-B-8) · **클라이언트 격리 AP에서 S4 성공**(S-3) · 유휴 RSS·CPU 예산(**R-2·R-7 해소**).

| ID | 항목 | 우선 | 규모 | 의존 | 상태 |
|---|---|---|---|---|---|
| M1-1 | `net` 골격 — `Transport` 경계 · **`InMemoryTransport`**(테스트용·[09](09-adr-0003-transport-abstraction.md)) | P0 | 중 | M0-1b · D-9확정 | ✅ (08-08 — Transport/Link/PeerHint/DiscoveryEvent · InMemoryTransport fake(발견·연결·이탈) · core `DisplayName`(RLO 무해화). net 5 + name 6 테스트 green) |
| M1-2 | **L1 링크 상태 구독 3구현**(D-13) — Win `NotifyIpInterfaceChange`/mac `PF_ROUTE`/Linux netlink + 디바운스([06 §2](06-network-stack.md)) | P0 | 중 | M1-1 | ☐ |
| M1-3c | **인터랙티브 헤드리스 채팅**(사람 대 사람 검증 도구) — `--chat-serve`/`--chat-connect`(stdin 타이핑·수신 실시간). GUI와 동일 세션 스택(Noise→TOFU→다중화). **맥↔docker-linux 양방향 대화 실증**(arm64↔amd64) | P2 | 소 | M1-3b | ✅ (08-08) |
| M1-3b | **수동 엔드포인트**(DR-19 · [19](19-adr-0006-manual-endpoint.md)) — `Transport::add_endpoint`(발견 우회 IP/호스트명 직접 연결·신원은 핸드셰이크 확정) · bin `--serve`/`--connect` · **맥↔docker-linux IP 대화 실증**(발견 경계 우회). 잔여 = 원격 신뢰 등급(M4 파일 차단)·인바운드 요청 대기·재연결 백오프 | P1 | 중 | M1-4 | ✅ (08-08 — 핵심+**GUI 수동 추가 UI 완료**: `⌘/Ctrl+K` 주소 입력 오버레이(상태바)→add_endpoint→대화 자동 오픈) · **✅ CLI 실증**(08-09 `--serve`↔`--connect` 지문 확정 · 맥↔Docker) · ⚠️ GUI `⌘K` 오버레이는 육안 미확인 |
| M1-3 | 와이어 포맷 확정([08 §2](08-adr-0002-discovery-transport.md)) · 인터페이스별 바인딩(`IP_MULTICAST_IF`/`IP_PKTINFO`) · **소켓 옵션 크로스플랫폼 검증표**(D-14 · [06 §5-2](06-network-stack.md)) | P0 | 중 | M1-1 | 🚧 (08-08 — **와이어 포맷 확정·구현**: `net/wire.rs` — 고정부 71B(+`instance` 16B D-22)·512B 강제·미지 버전/종류 무시(전방 호환)·이름 무해화+지문 폴백·**골든 레이아웃 테스트 고정** · `CloneWatch`. 잔여 = 소켓 바인딩·옵션 검증표(M1-4와 함께 — socket2 의존 결정 필요)) |
| M1-4 | 발견 **S1~S3**(IPv6/IPv4 멀티캐스트 · 브로드캐스트) **동시 시도** · 자기 패킷 키 필터 + **`LocalDirect` Transport 통합**(발견+TCP 세션) | P0 | 중 | M1-3 | 🚧 (08-08 — **S2+S3 실물 + `LocalDirect` 통합 + 실물 종단 실증** · **S1(IPv6)+S2(멀티캐스트)+S3(브로드캐스트) 동시 발신** — S1은 best-effort(ff02::beb·기본 인터페이스·미지원 환경 조용히 IPv4만) · 수신 소켓 공용/전용 · 잔여 S4(유니캐스트·D-8b): `TcpLink`(길이 접두 프레이밍·폴 타임아웃) · `LocalDirect`(UdpDiscovery+TCP → `Transport` — InMemory 자리에 그대로) · `--live-echo` 헤드리스 종단. **발견→TCP→Noise→암호화 대화 왕복**을 맥 2프로세스 + **Docker 컨테이너 2노드** 양쪽에서 실증. 잔여 = S1(IPv6)·S3(브로드캐스트) 동시 시도·인터페이스별 바인딩 · **GUI LocalDirect 배선 완료(08-08)** — `--window --live`로 실물 발견·대화(창이 `Box<dyn Transport>`라 InMemory↔LocalDirect 한 지점 교체)) — 원 S2 실증 3건: `UdpDiscovery`(socket2·REUSEPORT·자기 패킷 **키** 필터·HELLO/ANNOUNCE 주기·GOODBYE 2회·타이밍 주입) ① 맥 2인스턴스 상호 발견(테스트 `--ignored`) ② 맥 프로브 2프로세스(실 LAN 주소) ③ **Docker 컨테이너 2노드 교차 발견**(`--discover-probe` · 172.18.0.2↔0.3 Hello/Announce 수신 — D-8a-Linux 프로토콜 실증). 잔여 = S1(IPv6)·S3(브로드캐스트) 동시 시도·인터페이스별 바인딩·Transport 통합) |
| **M1-4b** | 🔴 **주소 계열 비대칭 수정 — 발견된 상대에게 연결이 실패한다**(**R-21** · Windows 실측 08-10). 수신 TCP가 `0.0.0.0`(IPv4 전용)인데 광고는 v6까지 3중 발송이고, 주소록이 **마지막 관측 IP로 덮어써서** v6 링크로컬이 기록되면 `Unreachable`. 권장안 = **주소를 후보 목록으로 보관해 순차 시도**(v4 우선·성공 경로 승격 — [06 §4](06-network-stack.md) 폴백 사다리 취지). 듀얼스택 바인딩(`[::]`+`IPV6_V6ONLY`)은 **R-7(OS별 기본값 상이)** 를 함께 검증해야 한다. **회귀 테스트 필수** — 주소록에 v6가 섞여도 연결이 성립하는가 | **P0** | 중 | M1-4 | ✅ (08-10 — **권장안 ⓒ 구현**: `PeerAddrs` 상대별 후보 목록(관측 순·중복 제거·상한 8) + 성공 경로 승격 · 시도 순서 = **성공 경로 → v4 → v6** · `add_endpoint` 다중 해석도 v4 우선 순차(같은 비대칭 재발 방지) · **회귀 6건**(핵심: 죽은 v6 혼입에도 성립 — 루프백 실소켓·CI 가능) · **Windows 실측**: `--live-echo` 2노드 동시·시차(15s) 양방향 왕복 성공·연결 실패 0 · 371 green) |
| M1-5 | `core` 피어 목록 — **PeerId 병합**(FR-D-6 · 다중 경로) · 이탈 판정(goodbye+타임아웃 FR-D-8) · 표시 이름 호스트명 기본값(FR-D-9) | P0 | 중 | M1-4 | ✅ (08-08 — `PeerTable`: 병합 키=오직 PeerId · **goodbye는 경로 단위**(마지막 경로만 이탈) · 타임아웃은 마지막 관측 기준 · **타임아웃 수치는 주입**(D-8 실측 대기 — 하드코딩 금지) · 결정적 정렬 목록. 호스트명 기본값은 힌트 생성 측(M1-4)에서. 순수 로직이라 M1-4보다 먼저 완결 — 실물 발견이 여기 꽂힌다) |
| M1-6 | 최소 UI — 피어 목록 표시(gfx/ui 최소 경로) · **미검증 배지** | P0 | 중 | M1-5 | ✅ (08-08 — gfx `Surface`(클립 보장)+`Font`(ab_glyph·시스템 폰트) · plat `system_ui_font`(3-OS 후보) · ui `peer_list::render`(신뢰 배지 3종 상시·다중 경로 ×N) · bin `--window` = **PeerTable+TrustStore 실물 도메인 경로**의 데모 목록. 3-OS 통합 테스트(한글은 mac/win 단언) · 릴리스 0.53MB. 실물 발견 배선은 M1-4) · **✅ 08-09 육안 실증**(사용자) — 목록 2행 · **배지 전이 `Unverified`→`Pinned` 확인**(FR-S-3) · 타입어헤드 안내. ⚠️ `FingerprintVerified`는 SAS UX(M3-6) 없어 미도달 · ⚠️ **핀이 메모리 전용**(영속은 M2-5) |
| M1-7a′ | **D-8a-Linux Docker 테스트베드**(08-08 실증) — ✅ **검증 완료 2건**: ① Linux 컨테이너 전체 테스트 **146 green**(맥 147 − 한글 래스터 cfg 1건 — CJK 폰트 없는 환경 제외 설계 그대로) ② **컨테이너 2개 간 UDP 멀티캐스트 실도달**(브리지 네트워크 · `239.255.77.77` → RECV 확인) — **M1-4 발견을 실기 없이 컨테이너 2~N개로 테스트 가능**. ⚠️ 가상 스위치 손실≈0 — 타이밍 실측 대체 불가(D-8b 유지). Windows는 Docker 불가(Windows 컨테이너 = Windows 호스트 전용) → VM | P0 | 소 | — | ✅ (08-08) |
| M1-7a | **D-8a 상호운용 검증**(08-08 재정의 — **Windows VM 불요, 사용자 확정**) — ① Linux↔Linux: **Docker 컨테이너 N개** — ✅ **실 프로토콜 교차 발견 완료**(08-08: NXBP 와이어로 컨테이너 2노드 상호 SAW) ⚠️ **실측(08-08)**: **맥 호스트 ↔ Docker Desktop 컨테이너는 멀티캐스트 상호 발견 불가**(`--network host`로도 — 컨테이너의 host = 내부 Linux VM). 맥↔리눅스 실물 검증은 **실기(같은 공유기)** 또는 **브리지 VM(UTM macvlan)** 필요 · 로드맵 **DR-19 수동 엔드포인트**(IP 직접) 구현 시 발견 없이 연결 가능. Linux↔Linux는 컨테이너 2개로 완결. ② Windows 단위·회귀: **CI windows-latest**(매 push 자동) ③ Windows 실행·방화벽·IME: **원격 Windows PC**(망 달라도 무관) ④ **맥↔Win LAN 상호운용만 D-8b 실기 날로 이월**(같은 망에 물리는 날 타이밍 실측과 함께). 타이밍 확정은 여전히 불가(가상 손실≈0) | P0 | 중 | M1-4 | ☐ (M1-4 후 Docker로) |
| M1-7b | **D-8b 실망 타이밍 실측 E-1~E-9**([06 §7](06-network-stack.md)) → 타이밍 6종 확정 → [ADR-0002 §8](08-adr-0002-discovery-transport.md) 정정. 🔴 **실기 2대 + 같은 망**(무선 AP 멀티캐스트 손실·클라이언트 격리·IGMP 스누핑은 VM 재현 불가). 그 전까지 타이밍은 **잠정치 표기** | P0 | 대 | M1-7a | ⏸ **실기 2대 같은 망 필요** |
| M1-8 | **S4**(ARP/NDP 이웃 유니캐스트 프로브 — 클라이언트 격리 대응) · E-3 결과 기반 | P0 | 중 | M1-7 | ☐ |
| M1-9 | 링크로컬 직결(DHCP 없음) 발견(FR-D-7) · 가상/터널 인터페이스 제외 | P1 | 소 | M1-4 | ☐ |
| **M1-10** | **표시 이름 기본값 정정**(FR-S-50 · **R-19**) — 호스트명 **직접 사용 금지**(macOS 기본은 대개 `{실명}의 MacBook`). 정제/중립 라벨로 교체 · 실명은 **옵트인 + "LAN 전체에 방송된다" 고지** | P0 | 소 | M1-3 | ✅ (08-11 — **Q-29-1 확정 = ⓐ 호스트명 정제 + 지문 라벨 폴백** · 적용 = **즉시 재공지**. `core::neutral_from_host`(장치 단어부터 유지 · 없으면 **fail-closed** → `beep-{지문}`) + `default_display_name` · `plat::host::hostname`(원시 — 정제 필수 주석) · `Transport::set_display_name` 기본 no-op + `LocalDirect`→`UdpDiscovery::set_name`(템플릿 Arc<Mutex> 공유 · **즉시 ANNOUNCE**) · 설정 `profile.display_name`(CatProfile · "auto"/직접 입력 · desc = **LAN 평문 방송 고지** 4어) · 상대 목록은 기존 `PeerTable Renamed`로 갱신. **450 green** · 재공지 실측(60s 주기 전 도달) · ★ **이 PC 실측: 방송 이름 = `beep-6808efd4`** — 사용자명(KIROS33) 미노출. ⏸ 2-PC 상대 목록 rename 실기) |
| **M1-11** | **와이어 평문 자동 회귀**(FR-S-51 · [29 §4-1](29-wire-security-audit.md)) — ① 발견 인코더 **골든 레이아웃**(오프셋 고정) ② **금칙어 스캔**(프로필 키·이메일·전화 패턴이 인코딩 결과에 없음) ③ 세션 **tap 평문 부재**를 금칙어 목록 기반으로 일반화(기존 `ciphertext_on_the_wire_is_not_plaintext` 확장) ④ 로그·`Debug` 마스킹 단언. **CI에 상시** | P0 | 중 | M1-3 | ☐ |
| **M1-12** | **와이어 수동 캡처 절차 상시화**([29 §4-2](29-wire-security-audit.md)) — 멀티캐스트 조인 덤프 스크립트를 저장소에 두고(`sudo` 불필요) **발견 포맷을 바꾼 커밋에서 재실행**. 판정 = ASCII에 사람이 읽을 것이 없는가 · 여러 패킷의 **고정 필드 = 재식별자** 탐색 | P0 | 소 | M1-11 | ☐ |
| **M1-13** | **발견 호환성 표면 — 버전·세대·포트**(사용자 검토 요청 08-11) — 현재 실태: ① 와이어 `ver` 1B는 있으나 **미래 버전 = 조용히 무시** → 신·구버전이 서로 **투명 인간**(왜 안 보이는지 아무도 모름) ② `flags` u16은 예약만 ③ **발견 포트 47100 고정** — 다른 앱이 배타 점유하면 `UdpDiscovery::spawn` 실패 → **GUI `expect` 패닉**(기동 불가). 설계안(검토 08-11): ⓐ **포트·매직은 프로토콜 헌법으로 영구 동결**(포트 변경 = 다른 제품 취급 — "들리는 채널"을 영구 보장) ⓑ **신버전이 구버전 와이어를 병행 발신**(dual-announce · N-1 하위 발신 정책 — 구버전 목록에도 뜬다) ⓒ **flags에 세션 세대(gen) 비트 배정** → 비호환 상대를 목록에서 **제외가 아니라 "버전 불일치 — 대화 불가" 표시**(조용한 비호환 금지) ⓓ 세부 능력 협상은 세션 성립 후 첫 프레임(capability exchange — `Caps` 자리 기존) ⓔ **포트 점유 시 발신 전용 강등**(수신 소켓 실패해도 송신은 임의 포트라 가능 — 상대 목록에 나는 뜨고, 상대가 나를 클릭하면 인바운드로 대화 성립 · 상태바 "발견 수신 불가 — 단방향" 고지 · 패닉 제거) · 포트 사다리는 발신 배수·스캔 오인 비용으로 **보류**. 착수 시 ADR-0002 §2 개정 필요 | P1 | 중 | M1-3 | ☐ **설계 등록 — 구현 대기**(ⓔ 패닉 제거는 P1 선행 가능) |
| **M1-14** | **수동 연결 주소 정규화를 CLI에도**(08-13 실기 · [26 §6](26-run-and-manual-test.md)) — GUI 모달은 포트를 생략하면 `:47200`을 붙이는데(`nbeep_ui::addr_prompt::normalize_endpoint`) **CLI `--chat-connect 10.0.0.5`는 `BadAddress`** 로 떨어진다. 같은 입력이 경로에 따라 갈리는 건 규약이 아니라 누락 — 정규화를 **공용(코어/net)으로 내리고** `--chat-connect`·`--connect`에 적용. ⚠️ 비-IP 문자열은 `to_socket_addrs()`가 **DNS로 흘러** 오해를 주는 `Unreachable`을 낸다(실기: 오타 `10.60.218.517`) → 형식 검증을 먼저 태운다 | P1 | 소 | M1-3 | ☐ |
| **M1-15** | **`--discover-probe`가 실제 세션 포트를 안 찍는다**(08-13 실기 · [26 §6](26-run-and-manual-test.md)) — 출력의 `from=`은 **UDP 발신 주소**지 TCP 세션 포트가 아닌데, 그걸 그대로 `--chat-connect`에 넣어 실패했다(실기에서 실제로 밟음). 패킷은 `tcp_port`를 이미 나르므로 **`tcp=<N>` 한 필드 추가**면 끝 — 발견이 닿지 않는 상대에게 알려줄 값이 프로브 출력에 없다는 게 문제의 핵심 | P1 | 소 | M1-4 | ☐ |

---

## §4. M2 — 대화 (신원 확정 + 암호화 기록)

> **목표: 목록에서 고르면 암호화된 1:1 대화가 되고 기록이 남는다.** 게이트 — 경로 변경에도 **대화 스레드 연속**([09](09-adr-0003-transport-abstraction.md) 규칙 3) · InMemory로 상위 계층 테스트 green.

| ID | 항목 | 우선 | 규모 | 의존 | 상태 |
|---|---|---|---|---|---|
| M2-1a | **세션 추상화** — `Link` core 이관 · `Session` 트레이트 · **`PlainSession` 스텁**(crypto testkit) · core `duplex` 링크 fake | P0 | 중 | M1-1 | ✅ (08-08 — Session 경계+스텁으로 상위 계층을 암호 없이 검증. crypto→net 없음) |
| M2-1b | `crypto` — **실물 Noise_XX** 핸드셰이크 · AEAD([08 §4](08-adr-0002-discovery-transport.md)) — **snow 채택** | P0 | 대 | M2-1a | ✅ (08-08 — NoiseSession(snow)·Identity(X25519=PeerId)·평문 미노출 검증. 라이선스 퍼미시브 100%·릴리스 0.40MB 불변) |
| M2-2 | **TOFU** — 핀 저장 · 차단(fail-closed) · 이름 재사용 경고·이력 · SAS 안전번호([08 §4](08-adr-0002-discovery-transport.md)) | P0 | 중 | M2-1 | ✅ (08-08 — `TrustStore`/`MemoryTrustStore`·`TrustedSession` 데코레이터·`safety_number`(BLAKE2s, 새 의존성 0). **"지문 불일치 차단"은 v1에 해당 없음** — PeerId=공개키라 키가 다르면 다른 항목. v1 보호는 **신뢰 불상속** + 이름 재사용 경고이며, 원래 시나리오는 v2 `UserId`에서 실체화 |
| M2-2b | **안전번호 자릿수 상향**([21 §3-1](21-identity-spec.md) · Q-21-5) — Signal/WhatsApp **60자리** 기준 상향 + 다이제스트 전체 사용 | P0 | 소 | M2-2 | ✅ (08-08 — 사용자 승인(추천안 수용). 5자리 12묶음·카운터 도메인 2회 해싱 64B 전체 사용·형식/전폭 사용 회귀 테스트) |
| M2-3 | `Session` 다중화(제어/대화 논리 스트림) · 프레임 상한 · **백프레셔**([08 §3](08-adr-0002-discovery-transport.md)) | P0 | 중 | M2-1 | ✅ (08-08 — `MuxSession`/`StreamId`(Control·Chat) · `[stream 1B][payload]` 봉투 · 미지 스트림 무시(전방 호환) · `MAX_PAYLOAD` 65,518 · 큐 64 초과 시 `Backpressure` fail-closed. 실물 Noise 위 통합 테스트) |
| M2-4 | 1:1 텍스트 송수신 · **논리 시퀀스 · 중복 제거**(발신 경로 = PeerId 집합 일반화, D-21) | P0 | 중 | M2-3 | ✅ (08-08 — `ChatMessage` 봉투(`[ver][sender_device 32B][seq][kind][utf8]` · 골든 테스트) · `Sequencer`(재시작 복원 API) · `DedupIndex`(키=(기기,seq)·창 1024·창 밖 과거=중복 간주 fail-closed) · `fanout`(1:1=그룹=같은 경로·개별 전달 보고 FR-G-4) · **sender_device≠세션 인증 상대 = 거부** · 미지 kind는 Unsupported로 순서 보존. 실물 Noise 종단 통합) |
| M2-4b | **등급·확인 프로토콜 자리**(N-1~N-4 · [24 §7](24-adr-0010-message-priority-notification.md)) — 메시지 봉투 `importance` 1B(미지 값=`Normal`) · **`ack` 제어 메시지**(`Delivered`/`Acknowledged`) · 기록 레코드 상태·시각 필드 · `ActionKind` 5종 + `stable_code` 대역. **M2-4와 같은 슬라이스** — 나중이면 포맷 변경. ★ **`Bye`(정중한 종료) 제어 자리도 함께 검토**(08-13 실기 — 의도적 종료와 장애 끊김을 와이어가 구분 못 해, 상대 자동 재연결이 /quit한 대화를 계속 다시 연다) | P0 | 소 | M2-4 · D-23확정 | ☐ |
| **M2-5a** | **신뢰 핀 영속 최소 슬라이스**(FR-S-47 · R-17) — 데이터 경로 + 키 계층 래핑(FR-S-32) + 핀 세그먼트 · fail-closed | P0 | 중 | D-18 §3 | ✅ (08-11 — **D-18 §3 확정(A 기본+B/C 승격) 즉시 구현**. ① **신원 키 영속 선행**(`crypto::keyfile` — `identity.key` 68B·Unix 0600·손상 시 **덮어쓰지 않고** 임시 신원 강등 — 핀 영속은 내 신원 영속과 한 몸) ② **`nbeep-store::FileTrustStore`** — `MemoryTrustStore` 위임(도메인 변경 = 추가 API `PinRecord`/`export`/`from_records`뿐) · **키 계층**: 마스터 32B 무작위 + 래핑 키(SHA-256(솔트‖기기 키) — 256비트 무작위 원료라 메모리-하드 불요·승격②만 해당) AEAD(ChaCha20Poly1305 — snow 계열·**트리 증가 0**) · **핀 세그먼트 `trust.seg`**(기록과 분리·이름·PeerId 평문 미노출 테스트) · **변경 즉시 저장**(write-through — 핀은 드물어 디바운스 불요·크래시에 마지막 핀 보존) · 원자적 교체 ③ **fail-closed**: 손상·키 불일치 = **잠김**(전부 Unverified + 상태바 고지 + **원본 파일 보존**·영속 중단 — 조용한 재핀 오염 방지) ④ **458 green**(+7) · ★ 실측: 재시작 전후 **같은 PeerId**(`17c470c2`) 방송 확인. ⏸ 2-PC 핀 유지 실기(같은 망) · CLI 도구는 의도적 임시 신원 유지) |
| M2-5b | `store` 나머지 — 데이터 경로 결정(포터블/폴백 FR-P-3) · **암호화 대화 기록 저장**([17](17-adr-0005-history-at-rest.md)) · 키 계층(마스터 래핑) · **동기화 폴더 감지 경고** · 크립토 셰레딩 | P0 | 대 | M2-4 · D-18확정 | ☐ |
| M2-6 | 텍스트 **무해화**(RLO·제어문자 FR-S-13 — `core`/`safe` 경계) · 링크 자동 열기 금지(FR-S-14) | P0 | 소 | M2-4 | ✅ (08-08 — `safetext`: `sanitize_message`(이름과 달리 **개행·탭 보존**·CRLF 정규화·상한 16,384자 잘림 보고) · `find_links`(http/https 화이트리스트·범위만 보고 — **열기 API 부재가 구조 보증**) |
| M2-7 | UI 신뢰 배지 · **비동기 수신 펌프**(실시간 수신 GUI 반영) | P0 | 중 | M2-2 · M1-6 | ✅ (08-08 — **비동기 수신 펌프 완료**: 세션을 액터 스레드로 이전(snow TransportState가 read/write에 &mut 요구 — 한 세션=한 스레드) · `set_recv_timeout` 전 계층 위임(Link→Session→Noise/Plain/Trusted/Mux) · winit `EventLoopProxy`+`AppEvent`로 수신 실시간 반영 · 수신도 `DedupIndex` 통과. 신뢰 배지는 M1-6에서 완료. **인바운드→GUI 대화 자동 생성 완료(08-08)** — 남이 나에게 연결하면 accept(핸드셰이크)→TOFU 판정(메인 스레드)→대화·창 자동 생성(Separate=새 창·Single=목록 화면이면 열고 아니면 알림). **양방향 실시간 대화 성립**) · **✅ 08-09 GUI 육안 실증**(사용자) — 터미널 `--chat-live`↔GUI **실시간 양방향**·한글 표시·`Me:`/`Peer:` 구분 |
| **M2-8** | 🔴 **아웃바운드 연결 수립을 워커 스레드로**(사용자 실기 08-10 — **죽은 상대를 더블클릭하면 GUI 전체가 "응답 없음"**). 원인 = `open_session`(`connect`+Noise 핸드셰이크 블로킹)이 **winit 이벤트 루프 스레드에서 실행** · M1-4b 후보 순차 시도(후보당 3초)가 최악 대기를 키움(강제 종료된 상대는 GOODBYE가 없어 목록에 잔존 — FR-D-8 타임아웃 이탈 미배선과 결합). 설계 = **인바운드와 대칭**: 워커 스레드에서 connect+initiate → `AppEvent::Connected/ConnectFailed`로 복귀(TOFU 판정은 지금처럼 메인) · 목록 행 "연결 중…" 표시 · **중복 클릭 가드** · 후보당 타임아웃 단축(워커라 급하진 않음). **프로세스 분리는 불채택**(IPC·상태 공유 비용 · NFR-B 예산 — 액터 모델이 이미 스레드 기반이라 스레드로 충분) | **P0** | 중 | M2-7 | ✅ (08-10 — **인바운드 대칭 워커 구현**: `start_connect` 워커 스폰 → `AppEvent::Outbound/ConnectFailed` 복귀 · TOFU는 메인 유지 · `connecting` 중복 클릭 가드 · 인바운드 선성립 경합 처리 · `transport = Arc<dyn Transport+Send+Sync>` · 392 green. ✅ **무정지 사용자 실기 확인(08-10)** · **잔여 소진(08-11)** — `LinkState::Connecting`(강조색 점) + `add_endpoint`(⌘K 모달) 워커 이관(성공=`Outbound` 합류·실패=`AddFailed` · 동기 `open_session_addr` 제거 = **블로킹 연결 경로 0**)) |

---

## §5. M3 — 셸 · UI 본체

> **목표: 매일 켜 둘 수 있는 프로그램이 된다.** 게이트 — **S-7(스크린샷으로 OS 구분 불가)** · 60fps · 유휴 RSS(**R-3·R-9 해소**).

| ID | 항목 | 우선 | 규모 | 의존 | 상태 |
|---|---|---|---|---|---|
| M3-1 | **위젯 세트 신규 구현** — `nexa-gui` 인프라 이식(`DrawCtx`/`Widget`/`event`/`geom`/`theme`/**`edit`**(08-08 이식 — 캐럿·선택·삽입/삭제·IME 연결점)/`typeahead`) + `WidgetBase` 컴포지션 + 트레이트 기본 메서드 전파([14 §2](14-control-ux-architecture.md)). ⚠️ `ctl` 코드 재사용 안 함 | P0 | 대 | M2-7 | 🚧 (08-08 — **슬라이스 1 완료**: `geom`/`event`(휠 누적기·now_ms 주입)/`widget`(Invalidations 병합)/`theme`(토큰+danger)/`draw`(DrawCtx — dir2 전용 어휘 제외) 이식 + **`RasterCtx` 신규**(우리 gfx 위 백엔드 — SDF 커버리지 AA 라운드 사각형·타원·폴리라인·클립 텍스트). 슬라이스 2(08-08): **`typeahead` 이식** + **`PeerListWidget`**(첫 실물 Widget — 캐럿 탐색·클릭·휠 분수 노치·타입어헤드 cycle·Enter 활성화 폴링·부분 무효화) · bin 인터랙티브 배선(winit→InputEvent 번역). 잔여 = `edit` 이식(M3-3 IME와 함께)·WidgetBase·버튼/입력 위젯) |
| M3-1b | **OS 동작 어댑터**(`PlatformConventions`) — 수정 키·표준 단축키(`StdAccel`)·스크롤 관성·컨텍스트 메뉴·창 닫기([14 §6](14-control-ux-architecture.md)) | P0 | 중 | M3-1 | 🚧 (08-11 — **클립보드 3-OS 실물 완료**: mac `pbcopy`/`pbpaste` · Linux Wayland `wl-copy` → X11 `xclip` 폴백 · Windows CF_UNICODETEXT(08-10). ★ 그전까지 **비-Windows는 통짜 스텁**이라 ⌘V가 조용히 무동작이었다. **컨텍스트 메뉴 컨트롤 완료**(`ContextMenu` — 대화 창 배선). ★ **08-13 텍스트 에디터 일습**(2-PC 실기 피드백 ①): ⌘/Ctrl+C·X·V **전 뷰 일반화**(그전엔 대화 입력창만 — TextBox copy/cut/paste 신설 → AddrPrompt·Profile·Settings 위임 → 창 역할 라우팅) · TextBox **가로 스크롤**(긴 텍스트 캐럿 상실 해소) · **영역 밖 드래그 자동 진행**. 잔여 = 수정 키·`StdAccel` 표준화 · 스크롤 관성 · 창 닫기 관례(M3-2와 함께) · 설정 색상 hex·직접 입력 칸 클립보드 위임 · 드래그 정지 연속 스크롤(타이머)) |
| M3-1c | **macOS 시각 언어 수치표 확정** — 라운드 반경·간격·타이포·상태 6종 시각 규약. 시안 대조([14 §5·§9](14-control-ux-architecture.md)) | P0 | 중 | M3-1 | ☐ |
| M3-1d | **3단계 이벤트 전파**(캡처→타겟→버블) · 포인터 캡처 · hover 합성 · IME 조합 중 단축키 가로채기 금지([14 §3](14-control-ux-architecture.md)) | P0 | 중 | M3-1 | ☐ |
| M3-1e | **입력 스택 단일화**(사용자 요청 08-13 — "문제가 퍼지지 않고 한 군데서 해결") — ① **`TextInput` 공용 코어**: EditState+프리에딧 표시+선택 반전+드래그/자동 스크롤+우클릭 편집 메뉴+⌘단축키를 한 컴포넌트로, 단일 행(TextBox)·다중 행(대화 입력)·콤보 편집 칸이 **위임**해서 쓴다(지금은 TextBox와 ChatView가 각자 구현 — 08-13 전수 검사에서 같은 기능이 창마다 따로 뚫려 있었다) ② **IME 중재 상태기계 추출**: app.rs의 조합 게이트·보류-판정·유출 조합기·잔향 억제를 순수 모듈로 빼고 **08-13 트레이스 실측 순서를 회귀 테스트로 박제**(첫 키 유출·낱개 Commit·이중 배달·조합 중 이동 키) | P1 | 대 | M3-1b | ☐ |
| H-26c | ★ **한글 1byte 첫 타 유실 — ✅ 해소(08-14 · 사용자 실타건 확인)** — 절차 = 수집·분석 먼저(원인 실측: *조합 세션 종료 후 첫 1byte keydown 1개, 세션당 1회, winit 경계 소비*) → **ImeGate 상태기계 + 재생 14종 박제**(G3) → **keytap 보충 주입**(G1 — 개정 2차: raw 순서 대조 + 배달 증거 링 · 실타건 2회가 순서 붕괴/이중 입력을 잡아 다듬음) → **G2 저장 트리거 조합 확정 + H-25 선택 대체**. 잔여 없음(Windows 무영향 — keytap cfg 격리) · 상세 [34 §2-8·§4-4](34-hangul-input-issues.md) | P0 | 중 | M3-1e | ✅ |
| M3-2 | 트레이/메뉴막대 상주 · 알림 팝업/사운드(FR-U-2) · 창 닫기 = 상주(맥 앱 계속 실행) | P0 | 중 | M3-1b | 🔴 **의존 판단 대기**(사용자 요청 08-10: 3-OS 상주 기본 + `ui.close_to_tray` + 트레이 종료 메뉴 + 수신 OS 알림 + `ui.autostart`(기본 해제)). ★ **막는 것 = 트레이 아이콘 의존**(`tray-icon` MIT — DR-12 퍼미시브 적합, 그러나 DR-5 "런타임 의존 0"·DR-6 자체 렌더 원칙과의 정합 판단 필요). **아이콘 없이 창만 숨기면 사용자가 앱을 되찾을 방법이 없다** — 창 닫기=상주가 기본이면 더 심각하므로 아이콘 확보 전에는 착수하지 않는다 |
| M3-3 | **IME 완성**(FR-U-7 한/중/일) · 고DPI 배율(FR-U-6) · 다크/라이트(FR-U-5) · **영역별 글꼴**(크기·굵기·기울임) | P0 | 대 | M3-1d | 🚧 (08-08 — 배율·조합 프리에딧·다크/라이트(설정)·**영역별 글꼴 설정**(기본/메시지/상태 × 크기·faux 볼드·faux 이탤릭 · 기본 크기 상향 13/15/11→16/18/13) 완료. 잔여 = 글꼴 패밀리 선택(폰트 열거)·중일 검증 · **Windows IME 실기 = M3-3b**) |
| **M3-3b** | **Windows IME 실기 검증 — 상세 체크리스트**(FR-U-7 · [27 §6~7](27-typeahead-hangul-composition.md)) — mac에서 확정한 IME 탈피 설계(목록 = IME off 직접 조합 · 대화 = IME 유지 + 자모 보류-판정)가 **Windows IME 이벤트 순서에서도 성립하는지** 실측. 체크리스트 = 아래 WIME-1~9. 기록엔 실측값 필수(추정 금지) | P1 | 중 | M3-3 | ☐ **Windows 실기(이 PC 가능)** |
| M3-4 | 다국어 한/영(FR-U-3) · 키보드 우선 조작·타입어헤드(FR-U-4) | P0 | 중 | M3-1 | 🚧 (08-08 — **영어 기본 + 한/중/일 언어팩** 도입. ★ **08-13 Windows 목록 한글**([27 §8](27-typeahead-hangul-composition.md)) — 목록 창(IME off)은 Windows에서 라틴만 와 한글 불가였다(실기) → `jamo_from_qwerty`(QWERTY→두벌식)+`hangul_mode` 토글(한/영 키 — 드라이버 수준이라 IME 없이 도달)·상태바 고지·`cfg!(windows)`. 501 green · ✅ **실기 확인(08-13 사용자)**. 잔여 = 키보드 우선 조작 잔여·문자열 전수 이관) |
| M3-5 | 상태(자리비움)·타이핑 표시(FR-M-5) · **블라인드 인덱스 전문 검색**(FR-M-4/S-19 — 한국어 음절 n-gram·후보 복호 대조) | P1 | 대 | M2-5 · M3-1 | ☐ |
| M3-6 | **SAS 지문 대조 UX**(FR-S-4) — 결정적 순간 유도 | P1 | 중 | M2-2 | ☐ (파생은 완료 — `safety_number`. 남은 건 UX) |
| M3-7 | **프로필**(FR-D-15/16 · [22](22-adr-0008-profile-disclosure.md)) — 필드 설정·**노출 확인 UX**(미리보기) · 세션 경유 조회 + 자동 프리페치(속도 제한) · 목록에 신뢰 배지와 함께 표시 · 이름 재사용 경고 확장 | P1 | 중 | M2-3 · M3-1 | ☐ |
| M3-8 | **알림 강도 UI**(FR-U-15~17 · FR-S-41/42) — 배지/토스트/**최상위 창(포커스 비강탈)** · 미리보기 무해화·이미지/파일명/링크 금지 · 잠금·화면공유 시 내용 숨김 · **소리 포트 3어댑터**(Win winmm · mac NSSound · **Linux D-Bus 알림 데몬**, 없으면 무음 폴백 표시) · 신뢰 등급 강등표 · 24h `Urgent` 횟수 표시 | P0 | 대 | M2-4b · M3-2 | ☐ |
| M3-9 | **수신 확인 UX**(FR-M-11/12) — 수신자 **확인 버튼**(자동 금지) · 발신자 쪽 전달/확인 상태 표시 · `Urgent` 미전달 강조 · 그룹은 구성원별(FR-G-4 합류) | P0 | 중 | M2-4b | ☐ |
| M3-10 | **수신자 릴레이**(FR-S-43~46 · R-13) — `RelayChannel` 포트(DR-21) + **Webhook 어댑터**(P0)·SMTP(P1) · **다중 어댑터 동시 팬아웃**(병렬·개별 타임아웃·부분 실패·채널별 결과·중복 억제·채널별 묶음·큐 상한) · **규칙 `조건 → 채널 집합`** + 1회성 수동 릴레이 · **채널별 노출 수준 L0~L2**(기본 L0) · 안전한 기본값 + **끌 수 없는 상한** · 자격 증명 암호화 저장([17](17-adr-0005-history-at-rest.md))·로그 마스킹 · 채널별 계측 | P1 | 대 | M3-9 · M0-2b | ☐ (SP-2 결과에 종속) |
| M3-12 | **상대별 별도 대화 창**(DR-26 · FR-U-18 · [14 §11](14-control-ux-architecture.md)) — winit 다중 창(`WindowId`→Conversation 라우팅) · 같은 상대 재활성화 = 기존 창 포커스 · 창 닫기 = 뷰만 · `chat.window_mode` 설정 연동(M3-11 Entry) | P1 | 중 | M3-1 · M3-11 | ✅ (08-08 — **다중 창 구현 완료**: `WinEntry` 맵·역할(`Main`/`Chat(peer)`) 라우팅·창별 배율·재활성화=포커스·창 닫기=뷰만(대화 유지)·동시 대화 N개. **설정 연동 완료(08-08 M3-11 슬라이스 1)** — 설정 창에서 `chat.window_mode` 즉시 변경(새 대화부터). 실행 인자는 초기값 지정용으로 유지 · 잔여 없음 → **완료**(✅ 육안 확인 08-13 — 다중 창 동작·사용자) |
| M1-8x | **정상 종료 경로**(FR-P-7 · R-16) — `plat::shutdown`(SIGINT/SIGTERM 포트·DR-21) + 헤드리스 루프 깨우기 + **GUI 종료 훅**(SIGTERM 0.28s·Drop 체인 GOODBYE·유휴 폴 ~5Hz). docker stop 10.26→0.38s. ★ **08-13 터미널 복원 3중 방어**(kitty `ESC[>1u` 누수 실기 — 대화 루프 폴링+플래그 · 전역 `restore_now()` 멱등 · 패닉 훅(release `panic=abort` 대응) · PTY 실측 pop 확인). 잔여 = zeroize(M2-5)·Windows 콘솔 핸들러 | P0 | 소 | — | 🚧 (핵심+GUI 완료) |
| M1-8y | ★ **kitty 프로토콜 누수 재발 — 3중 방어를 뚫는 경로 재점검**(08-13 실기 **2회차**: 방어 구현 후에도 Ctrl+C에 이상 문자 → `reset` 수동 복구) — 재현 경로 후보를 하나씩 실측: ① **SIGKILL**(`kill -9`/`pkill -9` — 어떤 훅도 못 돈다 → 방어 불가능한 경로면 **다음 실행 시작 시 선제 `ESC[<u` pop**(잔존 상태 청소)으로 전환) ② `--window --live`(GUI 모드)가 raw 터미널을 만졌는가 — GUI 경로는 RawTerm을 아예 안 켜야 정상 ③ 다중 인스턴스가 같은 TTY에 pop을 중복/경합 ④ nohup·리다이렉트 상태에서 복원 시퀀스가 TTY가 아닌 파일로 새는가(isatty 확인). **수용 기준 = 어떤 종료 방식이든 다음 프롬프트에서 Ctrl+C 정상**(선제 청소 포함) | P0 | 소 | M1-8x | ☐ |
| ~~M3-13~~ | ⚠️ **M1-8x로 통합**(같은 FR-P-7 · R-16 — 중복 등록이었다). 진행 상태는 M1-8x 참조 | — | — | — | ↩︎ 통합(08-08) |
| M3-11 | **설정 화면**(DR-24 · [14 §10](14-control-ux-architecture.md)) — Entry 레지스트리 · 계층 트리 + AND 토큰 검색(조상 보존·매치 수) · Kind 동적 패널 · 즉시 적용 · `Cmd/Ctrl+,` · 보안 항목(프로필 노출·공유 scope·알림 정책) 확인 훅. ⚠️ 원래 M3-8이었으나 ADR-0010 태스크와 번호 충돌로 재배정(08-08) | P0 | 대 | M3-1 · M3-1b | 🚧 (08-08 — **슬라이스 1**: `registry` 단일 원천(렌더=검색 — 테스트 단언) · AND 토큰 검색 · 카테고리 헤더 · 값 칩 순환(즉시 적용·폴링) · `⌘/Ctrl+,` 별도 창 · **`chat.window_mode` 설정 연동 완료**(M3-12 잔여 해소) + `ui.theme`(다크/라이트 전 창 즉시). 슬라이스 2(08-08 사용자 요청): **좌측 카테고리 트리 사이드바** — 선택 하이라이트·검색 중 매치 카테고리만+매치 수 "(N)"(X-10 ①)·사이드바 클릭=카테고리 이동+검색 해제·단일 원천 불변식 재정의(사이드바 합=레지스트리 전체, 테스트 단언). 슬라이스 3(08-11 사용자 요청): **VS Code식 그룹 구성** — 직속 설정 먼저 → 하위 그룹 순(안정 정렬) · 하위 섹션 제목 · **상단 고정 밴드**(윗줄 상위/아랫줄 현재 하위 · 높이 고정 · 클릭 삼킴) · 제목 위계 +2/+1 굵게(`select_font_sized` 증분) · 스크롤바 자동 숨김 항목 `ui.scrollbar_hide` 추가. **다단 계층 해소** — 잔여 = 확인 훅(값 영속은 **M3-15 ✅ 08-11**)) |
| **M3-14** | **신뢰 배지를 원형 상태 아이콘으로**(FR-U-19 · 사용자 요구 08-09) — 글자 배지 → 이미지 아이콘(**색만으로 구분 금지** · 툴팁으로 등급명). ★ **설계 완료 — 규격 = [14 §13](14-control-ux-architecture.md)**(08-14): ⓐ **표시 가능한 상태 전수 6종**(`Unverified`·`Pinned`·`FingerprintVerified` + **화면에 없던 3종** = `Blocked`·**이름 충돌**(`name_conflict` — v1에서 사칭을 드러내는 **유일한** 가시 신호)·`FirstContact`) ⓑ 아이콘 = **Lucide `badge-*` 한 가족**(`badge-question-mark`/`badge`/`badge-check`/`badge-x`/`badge-alert`/`badge-plus` · ISC · **방패는 쓰지 않는다 — `shield-check`가 이미 격리함**) ⓒ **경로 등급 축**(`house`/`globe`/`waypoints` · DR-28)은 신뢰 오른쪽 별도 자리 ⓓ ★ **기본 상태는 조용히** — `Pinned`는 흐리게·`Local`은 생략(안 그러면 목록 전체가 시각 소음) ⓔ 자산 미생성 = `tools/mkicons.sh badge badge-check …` 한 줄(굽기 전 14px 실루엣 생존 확인). ⚠️ **착수 전 [14 §12-6](14-control-ux-architecture.md)·[§13-3](14-control-ux-architecture.md) 필독** — M3-19까지 들어오면 한 행에 표식이 **셋**이 된다(세션 11px 파냄 / 신뢰 14px 윤곽 / 경로 12px). **자리·문법·색 팔레트를 갈라 놓을 것** | P1 | 소 | M1-6 | ☐ |
| **M3-15** | **설정 영속 구현**(FR-P-8~12 · FR-S-48 · [28](28-adr-0011-settings-persistence.md)) — ~~현재 설정값이 전혀 저장되지 않는다~~(해소 08-11). `Entry` 레지스트리에 저장 태우기 · **`SaveScheduler`**(`Debouncer` 확장 — quiet OR max_delay · 호스트 `tick` 주입) · 미지 키 보존 · 원자적 쓰기(PID temp·fsync·덮어쓰기 rename) · 관용 파싱+`Entry::range` 클램프 · **종료 flush**(M1-8x 훅) · 불변식 T-1~T-9 테스트 | P0 | 중 | D-25 · M3-11 | ✅ (08-11 — **D-25 확정 → 즉시 구현**. **`crates/nexa-conf` 신설**(Q-28-1 확정: 이 저장소 안·분리 가능 상태): 관용 파싱(`_schema` 키)·미지 키 보존·`SaveScheduler`(mark/tick/flush — quiet 1s OR max 10s)·원자적 쓰기(PID temp·sync_all·덮어쓰기 rename·부모 fsync(Unix))·`dir_writable`/`user_config_dir`. 앱 배선: 부팅 로드(`SettingsState::set_by_name` 관용 검증 — 무효 값은 기본값 유지)+`apply_boot_settings`(테마·타입어헤드·툴바·전송 정책) · 변경 = `conf_mark` · `about_to_wait` tick · winit `exiting` flush · 경로 = exe 옆 `data/`(포터블)→사용자 폴더→임시. ★ **`timed` 승인은 저장 시점에 복귀 대상으로 치환**(사용자 확정 08-09 그대로) · 파일에 timed가 있어도 복원 안 함(manual 정규화 — 기간 연장 방지). 테스트 T-1~T-9(+2) · **447 green** · 실측: 부팅 왕복 스냅샷 35키·미지 키 보존·손상 파일 정상 기동) |
| **M3-17** | **프로필 화면 + 프로필 교환**(사용자 요청 08-11 · **DR-22/ADR-0008이 이미 설계** — [22](22-adr-0008-profile-disclosure.md)) — ① **별도 프로필 화면**(Role::Profile): 프로필 이미지·표시 이름·이메일·전화번호 편집(이미지 파일 선택은 탐색형 피커 재사용 · 저장 위치·암호화 여부는 착수 시 결정 — PII라 평문 settings.cfg 부적합 후보) ② **교환 프로토콜**: 연결(세션 성립)된 상대에게만 `StreamId::Profile`(신설)로 **요청-응답**(ADR-0008 — 브로드캐스트 미포함·자동 프리페치) · 수신 이미지는 크기 상한+`imgdec` 경로(R-5) ③ **목록·대화에 상대 이미지·이름 표시**(프로필 이름은 발견 이름과 구분 표기 — 신원은 여전히 키) ④ 공개 게이트 = `profile.share.*` 설정(**08-11 골격 구현됨** — 기본정보/이메일/전화 개별 토글 · 기본 전부 off). ⚠ 응답은 **공개 on인 필드만** · 미공개는 필드 자체 미전송(fail-closed) | P1 | 대 | M2-4 · ADR-0008 | 🚧 (**08-11 — 공개 토글 ✅ + 화면부 ✅ + ★교환 와이어 ✅**: `core::profile::ProfileMsg`(Control 스트림 — Request/Info/ImageChunk · **켠 필드만·미공개는 필드 부재** · 이미지 256KiB 상한·32KiB 청크·순서 어긋남 이미지만 폐기) · **자동 프리페치**(성립 합류점 1회) · 정책 판단 메인 단일 지점 · **프로필 이름 목록·제목 우선 표시**(무해화 통과분·`record_name` 이력 연동) · 이미지는 `data/profiles/` **바이트 캐시만**(픽셀 렌더 = M4-5 imgdec 후 — R-5). 468 green. ★ **08-14 기본 아바타 일습**(다른 세션 WIP 인계 완주): 12간지 내장(NBAV1 160KB)·`AvatarChoice`(`profile.avatar` 단일 문자열·부팅 시드 기본값)·프로필 스와치 14개(사진과 배타)·`draw_builtin` 렌더(프로필·목록)·와이어 Info.avatar(키만·전방 호환·변경 시 능동 재전송). **잔여 = `c:` 관리 복사(사용자 이미지 data/profiles/custom 이관)·연락처 상세 UI·⏸ 실기(스와치·얼굴 왕복)**) |
| **M3-18** | **프로필 화면 적용/취소 버튼**(사용자 확정 08-13 — 절차 변경) — 현재는 필드별 Enter = 즉시 적용(설정 Face 규약 차용)인데, **적용(Apply)/취소(Cancel) 버튼을 두고 저장 전에는 반영되지 않는 절차로 변경**한다: ① 화면 진입 시 스냅샷 → 편집은 로컬 상태만 ② **적용** = 그때 일괄 `apply_settings`(표시 이름 재공지·프로필 재교환 arm도 이 시점 1회) ③ **취소/Esc** = 스냅샷 복원·무반영 ④ 미저장 변경이 있는 채 닫기 = 확인(버리기/적용) ⑤ 공개 토글·이미지 선택도 동일 절차(현재 토글은 클릭 즉시 적용이라 함께 이관). ⚠ 설정 창(M3-11)의 "즉시 적용" 원칙(DR-24)과 **의도적으로 다른** 화면별 규약 — 프로필은 여러 필드가 한 신원 표현을 이루므로 원자적 저장이 맞다(사용자 확정) | P2 | 소 | M3-17 | ☐ |
| **M3-19** | **연결 상태 배지 = 색 + 실루엣 2중 부호화**(사용자 확정 08-14 · 규격 = [14 §12](14-control-ux-architecture.md)) — 현행 11px **채운 원 1개 + 색만**으로 `LinkState` 4값을 나르고 있어 적록 색각·저대비에서 무너진다. 색 토큰 4종은 **그대로 두고** 디스크 안쪽을 `theme.panel_bg`로 **파내(knockout) 실루엣을 넷으로 가른다**: `Idle`=빈 링 · `Connecting`=갭 링(90°·회전) · `Active`=꽉 찬 원(현행) · `Lost`=가로 막대(통행금지). 기하는 배지 지름 `D` **비율로 고정**([14 §12-3](14-control-ux-architecture.md) — 구멍 0.53D · 막대 0.56D×0.19D · 갭 90°) → 배율 무관. **회전은 캐럿 530ms 틱 재사용**(90°×4스텝 · 새 타이머 0 · `reduce_motion`이면 정지·갭은 유지). 끄기 스위치 `ui.link_badge_shape`(bool·기본 on · Entry 등록·핫 스왑). ⚠️ **M3-14와 한 행에 원형 표식 2개** → [14 §12-6](14-control-ux-architecture.md) 표대로 자리·문법 분리. 테스트 = 상태별 파냄 픽셀 단언 + 배율 2종 비율 불변 + 스위치 off = 현행 그림. **자산·의존·산출물 증가 0**(수식 렌더 — `avatar.rs` 방식) | P1 | 소 | M3-17(아바타 배지 겹침 자리) | ☐ **대기** — 목록 프로필 쪽을 만지는 다른 세션 작업이 끝난 뒤 착수(08-14 사용자 지시). 착수 지점 = [`peer_list.rs`](../crates/nbeep-ui/src/peer_list.rs) `LinkState` 분기의 `fill_ellipse` 1회 → 상태별 분기 |
| **M3-20** | **큰 자리 연결 상태 아이콘**(2층 · [14 §12-7](14-control-ux-architecture.md)) — 20px 이상 자리(대화 창 헤더·피어 정보 카드·툴팁)에 **Lucide 플러그 가족**(ISC · **사용자 확정 08-14 = 한 세트로 통일**): `plug`(Idle) · `plug-zap`(Connecting) · **`cable`**(Active) · `unplug`(Lost). ✅ **자산은 이미 구워 커밋됐다**(08-14 — `icon-{plug,plug-zap,cable,unplug}-96.alpha` 각 9,216B · 상수 `icons::link::*` · 원본 `assets/icons-src/` · 절차 [`tools/mkicons.sh`](../tools/mkicons.sh)) → **잔여 = 배선뿐**(`ToolIcon::Mask` 테마 틴트 · 새 코드 없음). ⚠️ **지금은 붙일 자리가 없다** — `chat_view.rs`·`peer_info.rs` 어디에도 `LinkState`가 넘어오지 않는다. **자리(대화 헤더 상태 줄)를 먼저 만드는 것이 선행**이며 M3-19와 별개 슬라이스. ⚠️ `plug-zap`·`unplug`는 실루엣 60% 공유 — 범례에 나란히 놓을 땐 라벨 동반. 탈락 근거(Wi-Fi 호 = 유선 상대에 거짓말 · 사슬 = Idle/Lost 수렴 · 방송 = 동심원 겹침 · Phosphor = 세트 혼용 비용)는 [14 §12-7](14-control-ux-architecture.md)에 박제 — 재검토 시 되풀이 금지 | P3 | 소 | M3-19 · (대화 헤더 상태 줄) | ☐ |
| **M3-16** | **주소(ip:port) 입력 모달 창**(사용자 요청 08-10) — 상태바 인라인 입력(⌘/Ctrl+K·툴바 +)을 **별도 모달 입력창**으로 승격: TextBox(Beam 캐럿·클립보드) · host:port/[v6]:port 형식 검증 · Enter 연결·Esc 취소 · (후속) 최근 주소 목록 | P2 | 소 | M3-1b | ✅ (08-11 — `AddrPromptWidget` 신설: 제목+TextBox(Beam·선택·클립보드)+**형식 검증**(host:port·[v6]:port·포트 1~65535)+Connect/Cancel · `Role::AddEndpoint` 모달(⌘K·툴바 + 진입) · **인라인 adding 경로 완전 제거** · 테스트 4 · 436 green. 잔여 = 최근 주소 목록(P3)·연결 워커 이관은 M2-8. ★ **08-13 후속 — 기본 포트 일습**: `DEFAULT_SESSION_PORT=47200` 우선 바인딩(점유 시 임의 폴백) · 설정 `net.session_port`(ⓐ 듣는 포트 = 거는 기본 포트 · 핫 스왑) · 포트 생략 입력(`10.0.0.5`→`:47200`) · 상태바 `수신 :N` 상시 표시 — 포트가 매 실행 바뀌면 수동 등록이 성립하지 않는다(실측). ⚠️ **08-13 육안: 모달리스로 동작** — AlwaysOnTop일 뿐 부모 비활성화 없음(winit 한계 · 진짜 모달은 plat `EnableWindow` 어댑터 필요 · P3) |

**M3-3b 상세 체크리스트 (WIME)** — Windows 실기(이 PC 가능) · 근거 = [27 §6~7](27-typeahead-hangul-composition.md) · 기록엔 **왜 + 실측값** 필수:

- [x] **WIME-1 대화(IME on) 한글 조합 기본** — ✅ **실기 확인(08-13 · 사용자)**: preedit 단계 표시·Commit 삽입·**조합 중 Backspace 자소 단위** 동작
- [ ] **WIME-2 조합 전 첫 키 유출** — 한글 IME 켠 직후 첫 키가 `Character`로 새는지 — mac에서 만든 **자모 보류-판정 ④**([27 §5](27-typeahead-hangul-composition.md))가 Windows 이벤트 순서(KeyboardInput ↔ Ime 도착 순)에서도 걸리는가
- [x] **WIME-3 목록 모드(IME off) 자모 도착** — ★ **실측 완료(08-13 · 사용자 실기): 자모가 오지 않는다**(라틴만 · 한/영 키 무력) — 우려대로 목록 한글 직접 조합이 Windows 무동작이었다. **처방 구현 = [27 §8](27-typeahead-hangul-composition.md)**: `jamo_from_qwerty`(QWERTY→두벌식) + 앱 소유 `hangul_mode` 토글 · 잔여 = 한글 이름 상대 **자소 접두 매칭 실동작**(GUI 실기)
- [ ] **WIME-4 한/영 전환 키** — winit 소스 확인 + ★ **실기 실증(08-13 · 사용자)**: VK_HANGUL → `NamedKey::HangulMode`(물리 `Lang1`)가 **IME 비연결 목록 창에도 도달** — [27 §8](27-typeahead-hangul-composition.md) 앱 토글·상태바 고지·조합·복귀 동작 확인. 잔여 = 대화 창(IME on) 전환 직후 **첫 키 손실**(mac R2 유형) 재현 여부만
- [x] **WIME-5 목록↔대화 전환 IME 토글** — ✅ **실기 확인(08-13 · 사용자)**: 전환 경계 묵은 조합 없음·첫 입력 정상
- [x] **WIME-6 조합 중 Enter — ★ 사용자 확정(08-13): 조합 확정+전송이 한 번에** — Windows IME는 Commit 후 같은 키의 KeyboardInput이 **또** 온다(macOS는 IME가 삼킴). 처음엔 잔향으로 보고 삼켰으나(2단 전송) **사용자 확정 = 통과가 정답**: Commit이 버퍼에 먼저 들어가고 Enter가 전송 → "확정+전송 동시"(Windows 메신저 관례 · DR-16 동작=OS 네이티브 · chat-live 콘솔과 동일). **Esc 잔향만 차단**(조합 취소가 화면 닫기로 새는 것 — `ime_cleared_ms` 1회성 · `cfg!(windows)`). mac은 IME가 삼켜 확정만 되는 2단 — OS별 관례 차이 그대로. 잔여 = 조합 중 Ctrl+K 등 단축키 오동작 여부만(→ WIME-6b로 관찰)
- [ ] **WIME-7 후보창·고DPI** — 한자 변환 후보창이 캐럿 근처에 뜨는가(Preedit cursor 위치 전달) · **배율 125%/150%**(FR-U-6)에서 preedit·후보창 위치가 캐럿에 붙는가
- [ ] **WIME-8 한글 선택·클립보드 왕복** — 드래그 선택 → Ctrl+C → 메모장 붙여넣기 → 다시 Ctrl+V(**CF_UNICODETEXT 어댑터 08-10 구현분의 한글 실증** — M3-1b 연계)
- [ ] **WIME-9 빠른 연속 타이핑** — 자소 유실·순서 뒤바뀜 없는가(이벤트 큐 경합·보류-판정 타이밍)
- [ ] **WIME-10 설정 검색 TextBox** — 아직 레거시 IME 경로([27 §7](27-typeahead-hangul-composition.md)) — stale(묵은 조합) 증상 Windows 재현 여부(재현되면 조합기 재사용 검토) · **한글 AND 토큰 검색** 실동작
- [ ] **WIME-11 신/구 MS IME 양쪽** — Windows 11 신형 IME + "이전 버전의 Microsoft IME" 호환 모드 각 1회전(구현이 달라 이벤트 순서·preedit 통지가 다를 수 있다) · 일/중 IME 첫 키 유출은 X-IME-CJK와 공유


**GUI 세션 판정 실기 (WGUI)** — Windows 실기 · 근거 = `nbeep-plat::gui`([e230e58](../crates/nbeep-plat/src/gui.rs)) ·
맥(콘솔·SSH)과 Linux(컨테이너 2종)는 08-13 실측 완료, **Windows만 실행 검증이 비어 있다**(크로스 컴파일만 통과):

- [ ] **WGUI-1 로컬 콘솔** — 데스크톱에서 무인자 `nexa-beep` = **창이 뜬다**(게이트가 오탐으로 막지 않는가 — 가장 중요)
- [ ] **WGUI-2 원격 PowerShell(WinRM)** — `Enter-PSSession`에서 무인자 실행 = **사유 안내 + exit 3**. 안내에 관측된 윈도우 스테이션 이름이 찍히는가(`Service-0x0-…$` 꼴)
- [ ] **WGUI-3 OpenSSH** — Windows OpenSSH 서버로 접속해 무인자 실행 = 안내 + exit 3
- [ ] **WGUI-4 ★ RDP는 통과해야 한다** — 원격 데스크톱 접속 후 무인자 실행 = **창이 뜬다**. RDP도 `WinSta0`이라 통과가 정답 — 여기서 막히면 **되는 걸 막는 것**이라 판정 기준을 고쳐야 한다
- [ ] **WGUI-5 터미널 모드 비간섭** — 위 어느 세션에서든 `--chat-live`·`--discover-probe`·`--help`는 게이트에 걸리지 않는다(exit 0)
- [ ] **WGUI-6 API 실패 폴백** — 스테이션 이름을 못 얻는 상황이 실제로 있는지(있다면 SSH 아닐 때 통과가 맞는지 재검토)

---

## §6. M4 — 전송 · 무해화

> **목표: 파일을 안전하게 주고받는다.** 게이트 — [18](18-build-and-test.md) **안전 회귀 전항목**(격리물 실행 불가·복원 후 표식 유지·Zip Slip/압축폭탄 거부·데이터 폴더 문자열 스캔 평문 0) · 전송 처리량 링크 70%(NFR-B-10)(**R-5 해소**).

| ID | 항목 | 우선 | 규모 | 의존 | 상태 |
|---|---|---|---|---|---|
| M4-1 | **ADR-0004 확정 반영**([11](11-adr-0004-quarantine.md)) — `.beepq` 컨테이너 포맷 구현·위험 등급 4단계 판정·상태 기계(fail-closed) | P0 | 대 | M2-4 · D-12확정 | 🚧 (08-09 — **도메인 전 구간 완료**: 코덱·위험 등급(합집합 fail-closed)·상태 기계(자동 승인 부재)·이름 정규화(RLO)·격리 저장소(`HashPort`/`MarkPort` 포트·materialize 재검증→원자 rename→표식 후행). safe 37 green. 잔여 = **OS 표식 실물 어댑터**(macOS xattr·Win IAttachmentExecute/ADS — 실기)·검사(§6) 어댑터·[18] 안전 회귀 게이트 편입) |
| M4-2 | 파일 **드래그앤드롭** · **폴더 전송**(FR-X-1/2 구조 유지) · **스트리밍**·진행률·취소(FR-X-4/5) · 협상→수락→전송(FR-X-3) · SHA-256 검증(FR-X-6) | P0 | 대 | M4-1 | 🚧 (08-09 — **정책·UX 완성**: 와이어·수신 조립기 · CLI/GUI 전송 · **전송 자격**(핀+양방향 대화 1회) · **승인 4방식**(자동/기간/수동/거부 · 오퍼당 1승인) · **수신 승인 화면**(정보 4종·즉석 자동승인·타임아웃) · **다중 파일 큐** · **진행률**(목록 막대·대화창·CLI) · **스레드 전송 항목**(08-10 — `ChatBody::Xfer`: 승인 대기→진행 막대→완료/실패가 **대화 기록으로 잔존**·뷰 재열기 복원·FIFO 갱신·자동 거부 미기록) · **대역폭 제어**(자동 50%·쌍방 협상 · 실측 19.9s/이론 20.0s) · TimeoutButton. 잔여 = **수락 후 취소 UX**·폴더 전송·재개·드롭 발신 육안 검증) |
| M4-3 | **무해화 게이트 4단계**(FR-S-5~10) — 격리 수신·헤더 봉인·검사 · **OS 격리 표식**(MotW/quarantine, 복원 후 유지 FR-S-8) · **AMSI**(Win, FR-S-15) · 승인 UX 등급별 마찰 | P0 | 대 | M4-1 | 🚧 (08-09 — **승인 UX 완료**: 격리함(등급별 마찰·🔴 2단계·키보드 배제) + **수신 승인 화면** · 다운로드 폴더 실체화 + OS 표식(실패도 명시) · 사용자 실기 확인. ★ **08-11 Windows MotW 표식 완료(이 PC 실측)** — `Zone.Identifier` ADS 직접 기록(`ZoneId=3` · IAttachmentExecute 대신 ADS = 의존 0·T0 무권한·내용 결정적) · FAT/exFAT은 오류 명시(macOS xattr 정책과 동일) · `--quarantine-demo` 종단에서 `표식=Ok(Applied)` + `Get-Item -Stream` 실물 확인 · **DR-13 "복원 후에도 MotW 유지"가 3-OS 중 mac·Win 성립**. 잔여 = **검사(§6) 어댑터·AMSI**(Win 실기) · Linux xattr) |
| **M4-10** | **전송 파이프라인 동기·이탈 내성 — 설계 검토**(사용자 요청 08-11 · 방향 결정 대기) — 실태: 전송은 이미 **스트리밍**(송신 청크 → 수신 즉시 소비)이지만 ① **송신 진행률이 소켓 쓰기 기준**이라 로컬·저지연 망에선 버퍼가 즉시 삼켜 "송신 완료 → 그 뒤 수신 진행"처럼 **보인다**(M4-9가 최종 완료 표시는 ack로 고쳤으나 **중간 진행률은 여전히 낙관**) ② 수신측 격리(.beepq 봉인)는 전량 수신 후 수행(마지막 구간 지연). 설계안: **ⓐ 청크 윈도 흐름 제어**(미확인 N청크 상한 — 자연 배압 + 송신 진행률 = 수신 확인 기준 + 메모리 상한 · M4-9 ack의 자연 확장 · 권장) ⓑ 주기 진행 통지만 추가(윈도 없이 표시만 동기 — 트래픽 최소·배압 없음) ⓒ 현행 유지+표기 변경("보냄"/"전달됨" 구분 — 이미 절반 적용). **이탈 내성(fail-over)**: ⓓ **전송 재개**(수신측이 받은 오프셋 영속 → 재연결 시 오프셋부터 재요청 — M4-2 "재개" 실체) ⓔ **지연 ack** — 전량 수신 후 송신자가 떠났으면 격리 완결은 진행하고 ack는 다음 세션에 전달(ack 대기 큐 영속) ⓕ 부분 수신물 보존 상한·만료 정책. 결정 지점 = ⓐvsⓑvsⓒ · 재개 단위 · 보존 기간 | P1 | 중 | M4-9 | ☐ **설계 등록 — 방향 결정 대기** |
| **M4-3b** | **세션 구간 실소켓 캡처**(W-8·W-9 · [29 §4-3](29-wire-security-audit.md)) — `tcpdump`로 대화·**파일 전송 중** 평문 부재 확인(단위 tap만으로는 실소켓을 증명 못 한다). 파일 바이트가 **별도 평문 경로로 새지 않는가** | P0 | 소 | M4-2 | ⏸ 실기(권한 필요) |
| M4-4 | 아카이브 안전 처리(X-5) — **자동 해제 금지** · Zip Slip · 압축폭탄(FR-S-11) | P1 | 중 | M4-1 | 🚧 (08-09 — **정책 계층 완료**: `safe_entry_path`/`check_entry`/`check_archive` — Zip Slip(`..` 상쇄 없이 거부·절대경로·UNC·링크) · 이름 일관성 · 압축률/총량/개수/깊이 · 전체 거부. 잔여 = **포맷 파서 어댑터**(zip 중앙 디렉터리 읽기)와 목록 UI) |
| M4-5 | **`imgdec` 별도 프로세스**(X-4) — 권한 강등 이미지 격리 디코드 · **재인코딩본만 표시**(FR-S-12, SVG 미지원). **R-5 해소** | P1 | 대 | M4-1 | 🚧 (08-11 — **코어 실물화**: 프로토콜 v1(stdin→`NIMG`+RGBA) · 디코더(`png`·`jpeg-decoder`)는 **격리 bin에만**(본체 미링크 — R-5 격리 성립) · 상한(원본 1MiB·픽셀 2048²·박스 축소 256·**부모 3초 kill**) · 본체 재검증·부재 시 이니셜 폴백 · **아바타 실사진 개통**(목록·카드·편집 미리보기 — 원형 마스크). 실측 PNG/JPEG 왕복·쓰레기 fail-closed. **잔여 = ~~ⓐ 포장 동봉~~ ✅(08-11 · **CI 검증 08-12 v0.1.4** — 포터블 zip에 imgdec 362KB 실재) · ~~ⓑ 수신 파일 미리보기~~ ✅(08-12 — 격리함 36px·스레드 인라인 18px · `.beepq` 재조립→imgdec·1MiB 상한)** ⓒ 권한 강등(잡오브젝트/seccomp) · 확대 미리보기 — ⓒ 완료 시 R-5 해소 표기) |
| M4-6 | 오프라인 큐(FR-M-3) — 부재 시 **송신자 로컬 보관** 후 재접속 전달 · 보관 상한(그룹 `pending_group_sends` 문법의 1:1판) — ★ 사용자 재확인(08-14): **서버리스 한계 = 송신자도 꺼져 있으면 못 받는다** → 완전한 미수신 해소는 서버 모드 ②([32 §5·GS-2](32-adr-0013-server-modes.md) — 암호문 대리 보관·오프라인 큐 TTL·발신자 보관과 겹침 규칙)가 담당. 설계는 이미 32에 반영돼 있고 v1 구현 아님 | P1 | 중 | M2-4 | ☐ |
| **M4-9** | 🔴 **전송 완료의 종단 확인(ack) 부재 — 발신 "전송 완료"가 거짓일 수 있다**(사용자 점검 요청 08-10). 현재 `XferSendDone`은 마지막 Chunk+Done을 **소켓에 쓴 시점**에 발화 — 수신측 SHA 재검증·격리 성공과 무관("보냈다≠닿았다" 위반 상태). 와이어에 수신 완료 메시지가 없다(`XferMsg` = Offer/Accept/Reject/Chunk/Done/Cancel — 6종 전부 확인). 정상 종료 직후엔 OS가 송신 버퍼를 마저 흘려보내 대체로 도달하지만(페이싱 덕에 잔량이 소켓 버퍼 수준) **커널 동작에 기댄 우연한 안전**이고, 강제 종료·크래시면 수신측 부분 수신→폐기(fail-closed·안전)인데 발신 UI는 이미 "완료"로 남는다 · 수신측 해시 불일치·저장 실패도 발신자는 영원히 모른다. 처방: ① **`XferMsg::Received{id}`/`Failed{id,why}` ack 신설**(전방 호환 — 미지 종류 무시 규약 유지) — 발신 스레드 항목은 ack 후에만 "완료", 그 전엔 "전달 중 · 확인 대기" ② **종료 가드** — 미확인 전송이 있으면 종료 시 확인·짧은 flush 대기. ★ DR-25의 수신확인(사람·수동 버튼)과 별개인 **전송 계층의 기술적 확인** — 봉투 원리 준수(완료 사실만) | **P0** | 중 | M4-2 | ✅ (08-11 — ① **`XferMsg::Received{id}`(kind 7)·`Failed{id}`(kind 8)** 신설 · 수신측 격리 성공→Received/실패→Failed 반송(actor·CLI) · 발신 UI `XferLineState::AwaitingAck`("전달됨·확인 대기")→ack로 완료/실패 종결 · ★ `update_xfer_in`이 AwaitingAck 건너뛰고 `update_xfer_ack`가 확인 대기만 닫아 **다중 파일에서 다음 진행이 앞 확인 대기를 안 덮음** ② **종료 가드** — 미확인 전송 시 첫 닫기 경고·둘째 확정(2단계) · ack 완료 시 자동 해제 · 432 green. ⏸ 2노드 실물 왕복 GUI 실기 확인 대기) |
| M4-7 | **공유 목록 + 가상경로 서버**(FR-X-8/9/11 · [23](23-adr-0009-shared-folders.md)) — 등록·범위 확인 UX · `ShareList`/`ShareGet` · **fail-closed 경로 해석기**(traversal·심링크 안전 회귀 [18](18-build-and-test.md) 추가) · 계측 4종 | P1 | 대 | M2-3 · M4-2 | ☐ |
| M4-8 | **pull 다운로드 수신 경로**(FR-X-10) — 상대 공유 브라우즈 UI · 다운로드→**동일 4단계 게이트** 합류 | P1 | 중 | M4-3 · M4-7 | ☐ |

---

## §7. M5 — 그룹 · 마감 · 배포 (v1 출시)

> **목표: v1 출시.** 게이트 — [00 §5](00-vision.md) **성공 기준 S-1~S-8 전항목** · [05 NFR-B](05-requirements.md) **예산 전항목**.

| ID | 항목 | 우선 | 규모 | 의존 | 상태 |
|---|---|---|---|---|---|
| M5-1 | **그룹 관리 + 그룹 전송**(FR-G-1~4) — 로컬 그룹 생성/편입/제외·재시작 유지 · 팬아웃 + 단일 스레드 UI · 개별 전달 상태 | P0 | 대 | M4-2 | 🚧 (08-13 — **기반 일습 커밋**: `groups.seg` 암호화 영속(FR-G-1 ✅) · 목록 상단 그룹 섹션 + ⌘클릭 다중 선택 + 우클릭 생성/개명/편입/제외/삭제 · `TextPromptWidget` 이름 모달 · **동보 스레드 + 팬아웃 발신**(세션 즉시·미연결 자동 연결 후 이어 전달·백오프 소진 실패 라인 — FR-G-2·G-6 ✅ · FR-G-4 초판) · 527 green. ★ **방향 확정(사용자 08-13) = 진짜 그룹(한 방 대화)** → [31 ADR-0012](31-adr-0012-shared-group-chat.md) 설계 → 잔여 = 🔴 **D-28 확정** 후 G1~G4 구현 · ⏸ 동보 기반 2-PC 실기) |
| **M5-1g** | **공유 그룹 채팅 G1~G4**([31 ADR-0012](31-adr-0012-shared-group-chat.md) — 한 방 대화·발신자 라벨·roster 서명·초대 수락·구성원 관리 모달·재동기) · ⚠️ **[32 §8 P-9~P-13](32-adr-0013-server-modes.md) 선행 제약을 함께 지킨다**(콘텐츠 키 봉투 수용 포맷 · 중복 제거 키 고정 · roster 정책 필드 자리 · **`blob_id=H(암호문)` 내용 주소 자리** · **오퍼/수락 UX를 경로 무관하게**) — 나중에 넣으면 **와이어·서명 포맷 변경** | P0 | 대 | ~~D-28~~ ✅ | 🚧 (08-13 — **✅ D-28 완결 · G1~G3 구현**: `sgroup.rs`(Roster 수용 규칙·SGroupMsg 6종·P-11 확장 영역)+`StreamId::Group`+groups.seg v2 · 그룹 만들기=전원 초대(미연결 대기+자동 연결)·수락/거절 카드 · 방 대화(Msg 팬아웃·발신자 라벨·unread 배지·명부 운영·제외 수신=방 닫힘) · 서명은 세션 인증 대체(DR-20에서 얹음) · 532 green. **잔여 = G4**(구성원 관리 모달·재동기 확장·로컬 승격) · **Q-32-7**(P-9 콘텐츠 봉투) · ⏸ 2-PC 실기(초대 왕복·방 대화)) |
| M5-2 | 전체 **브로드캐스트 공지**(FR-M-6) | P1 | 소 | M2-4 | ☐ |
| M5-3 | **스팸·플러딩 방어**(FR-S-16 · X-3) — 발신자별 속도 제한·차단 목록·미확인 발신자 격리. **R-4 해소** | P1 | 중 | M2-4 | ☐ |
| M5-3b | **수동 엔드포인트 등록**([19](19-adr-0006-manual-endpoint.md)) — IPv4/IPv6/DDNS 등록 · 원격 신뢰 등급(대조 전 파일 차단 FR-S-24) · **인바운드 요청 대기**(FR-S-25) · 백오프 재연결 · 병합(FR-D-13) | P1 | 중 | M5-3 · D-19확정 | ☐ |
| **M5-3c** | **경로 축 분리 + 경로 수명**(FR-D-17~19 · FR-S-52 · **R-20**) — **신원 신뢰 / 경로 등급 2축 모델**과 정책 매트릭스 · **경로 등급은 성립한 세션이 정한다**(가짜 LAN 광고로 원격 제약 우회 차단) · 경로 수명(실패 상한·**다른 키 성립 시 즉시 무효화**·보존 상한) · 경로 전환 시 **조용한 표시 변경**(신원 변화만 시끄럽게) | P0 | 중 | M1-3b · M5-3b | ☐ |
| M5-4 | **배포 2채널**(DR-4) — 포터블 + 설치본, 4타깃 · 데이터 경로 폴백 | P0 | 대 | M0-3 | 🚧 (08-11 — **포장 파이프라인 완료**: `release.yml`이 5타깃(win x64·arm64 / mac arm64·x64 / linux x64) × 2채널을 만든다. 설치본 = NSIS `.exe`(**사용자 단위·무권한** · `/S` 무인) · `.dmg`(.app+icns) · `.deb`(desktop+아이콘). **설치본도 zip 동봉**(실행 확장자 차단 대비 · 사용자 요청) · `SHA256SUMS.txt` · 릴리스 산출물에도 ≤10MB 게이트. **winget·choco 매니페스트 4종**(설치본/포터블 × 2) 자동 생성 — 실제 산출물 SHA로 채우고 자리표시자 잔존 시 실패. **Homebrew 추가**(08-11 · 참고: `sosomlab-tauri-test1` 탭 운영 기록) — Cask `nexa-beep`(.app) + Formula `nexa-beep-portable`, `kiros33/homebrew-tap`. ★ **트리거 정책**(사용자 확정 08-11): 기본 배포 = `v*` 태그 push **자동 공개** · Homebrew = 릴리스 직후 자동(`TAP_TOKEN` 유무만) · **winget·choco만 저장소 변수로 잠금**(`WINGET_PUBLISH`/`CHOCO_PUSH` + 시크릿 — `nexa-memkeeper`(Windows 전용) 검증 패턴 차용 · 중앙 검수라 되돌리기 어려워서 · mac/Linux 경로에는 영향 없음). 치환은 `render-manifests.sh` **단일 지점**. **★ 08-11 첫 배포 실행 — v0.1.0 → v0.1.1 공개**(5타깃 자산 · Homebrew 탭 반영). 배포 중 실패 4건 수정(bash 안 PowerShell 변수 문법 · 한글 조사가 변수명에 붙어 `set -u` 사망 · `brew audit` 경로 인자 폐지 · winget 폴더 깊이 추측). ★ **macOS 격리 실측** — 서명 없는 앱 + quarantine = **SIGKILL + 앱 삭제**, 애드혹 서명도 못 넘는다 → Cask `postflight` 제거 + 애드혹 서명. 포장 = 플랫폼 관례(Win zip·mac/Linux tar.gz). 잔여 = ⓐ **서명·공증**(mac notarize·win 인증서 — 별도 결정 · **격리 제거가 임시방편임을 잊지 말 것**) ⓑ **데이터 경로 폴백**(FR-P-3 — 포터블/설치본 동작 차이는 **설정 영속 M3-15와 같은 슬라이스**, 지금은 저장하는 것이 없어 채널이 설치 방식만 다르다) ⓒ winget-pkgs PR·choco push 실제 제출 ⓓ Linux arm64) |
| **M5-4a** | 🔴 **코드 서명·공증**(macOS notarize + Windows 인증서) — ★ **현재 Cask `postflight`의 격리 표식 제거는 임시방편이다**. 실측(08-11): 서명 없는 앱 + quarantine = **SIGKILL + 앱 삭제** · 애드혹 서명으로도 못 넘는다 → 공증만이 정답. 인증서 구매·팀 등록이 선행(비용 결정 필요) | P1 | 중 | M5-4 | 🔴 **사용자 결정 대기**(Apple Developer $99/년 · Win 코드서명 인증서) |
| **M5-4b** | **winget·Chocolatey 실제 게시** — 매니페스트 생성·검증은 통과, **게시만 변수로 잠겨 있다**. ⓐ ~~Windows 실기 확인 선행~~ **✅ 실기 완료(08-11 · 이 PC)** — SHA-256 3/3 일치 · 무인 설치(`/S`)·업그레이드(0.1.1→0.1.2)·완전 제거 통과 · winget validate 통과 · 매니페스트 해시 실측 일치. ~~단 🔴 M5-4d(더블클릭 결함) 선행~~ **✅ 해소**(v0.1.3에 실려 배포 실물로 재검증) ⓑ 시크릿(`WINGET_TOKEN`·`CHOCO_API_KEY`) + 변수(`WINGET_PUBLISH`·`CHOCO_PUSH`) 설정 ⓒ `publish-windows-packages` 실행 ⓓ 검수 통과 확인(둘 다 중앙 저장소라 즉시 노출 아님) | P1 | 소 | M5-4d | ⏸ **사용자 보류**(08-11 결정 · v0.1.4까지 잠금 유지) — **선행 조건은 전부 충족**. 남은 것 = 변수·시크릿(사용자 소유)을 켜고 게시 잡 실행뿐 |
| **M5-4d** | 🔴 **Windows 더블클릭 무반응 — macOS 번들 버그의 미수정 쌍둥이**(실기 발견 08-11). 포터블 exe = 콘솔 서브시스템 + 인자 없음 → 스캐폴드 출력 후 종료 = **더블클릭 시 콘솔 번쩍**. NSIS **시작 메뉴 바로가기·완료 실행도 인자 없이** 호출 → 설치본 정식 실행 경로도 동일하게 깨짐. `launched_from_app_bundle()`([main.rs:116](../crates/nexa-beep/src/main.rs))이 `.app/Contents/MacOS/` **전용**이라 Windows 대응 없음. 처방 = ⓐ NSIS 바로가기·FINISHPAGE에 `--window --live`(설치본만) ⓑ **Explorer 실행 감지(부모 콘솔 없음·`GetConsoleProcessList==1`) → 창 모드**(포터블+설치본 · macOS와 대칭 · 권장) → v0.1.3 | **P0** | 소 | — | ✅ (08-11 — **ⓑ 채택**: `plat::launch::from_gui_shell()` GetConsoleProcessList 휴리스틱(반환 1 = 탐색기 새 콘솔) → 창 모드 · `hide_gui_console()`(FreeConsole)로 잔존 콘솔 제거 · macOS 번들 판정과 대칭 · NSIS 바로가기·완료에 `--window --live` 두 번째 방어선. 실측: 터미널 무인자=CLI 보존 · Start-Process=GUI 전환·콘솔 없음 · 431 green. **v0.1.3에 실려 공개 완료(08-11) — 배포 실물로 재검증 통과**. ★ 08-13 후속: **무인자 기본 = 창+라이브로 승격**(사용자 확정 — 어디서 부르든 무인자면 창 · `from_gui_shell` 휴리스틱은 **콘솔 분리에만** 잔존 · 사용법은 `--help` 신설·`--version` 명시 분리) |
| **M5-4c** | **Linux 배포 확장** — arm64(`aarch64-unknown-linux-gnu` · X11/Wayland 크로스 링크 부담) · `.rpm`·AppImage·Flatpak 수요 확인 후 결정 | P3 | 중 | M5-4 | ☐ |
| M5-5 | **24h 상주 누수 실측**(E-9 · NFR-B-6) · 100대 규모(E-8) | P0 | 중 | M5-1 | ☐ |
| M5-6 | 문서 마감 — 사용 안내 · **알려진 한계**(TOFU가 못 막는 것·기기 이전 시 기록 미이전 V-7·저장 위협 모델 H-5) | P0 | 소 | — | ☐ |

---

## §8. v2 · 포스트 v1 (보류)

| ID | 항목 | 우선 | 규모 | 의존 | 상태 |
|---|---|---|---|---|---|
| X-1 | **`nexa-beepd`** — 릴레이 서버(별도 저장소, DR-9) | P2 | 대 | D-9 | ⏸ |
| X-2 | 릴레이 모드 클라이언트(`RelayTransport`, DR-8 ②) — 오프라인 큐 서버 보관 | P2 | 대 | X-1 | ⏸ |
| X-6 | **다중 기기 신원 구현**(DR-20 · [20](20-adr-0007-multi-device-identity.md)) — 기기 목록 서명·검증 · 페어링 SAS · 폐기/롤백 방지 · sender copy · 기기 관리 화면 · **주 기기 + 복구 시드** — FR-D-14·FR-M-8·FR-S-28~40 | P1 | 대 | D-20 · D-21 | ⏸ |
| X-7 | **E2E 백업·기기 이전**([17 §8](17-adr-0005-history-at-rest.md)) — 새 기기가 **과거 기록**을 보게 하는 별개 설계(X-6로 안 풀림) | P2 | 대 | X-6 | ⏸ |
| X-8 | S5 서브넷 스캔 폴백 · 서브넷 너머 발견(FR-D-10) — 사용자 동의 UX | P2 | 중 | M1-8 | ⏸ |
| X-9 | QR·초대 링크 노드 등록([19 §8](19-adr-0006-manual-endpoint.md)) | P2 | 중 | M5-3b | ⏸ |
| X-10 | 진짜 그룹 채팅(FR-G-5) · 화면 캡처·주석 전송(FR-M-7) · 접근성(FR-U-8, R-9) · 전송 재개(FR-X-7) · 자동 업데이트(FR-P-6) · mDNS 광고 · 모바일 | P2 | 대 | v1 | ⏸ |

- [ ] **X-IME-CJK**: 일/중 입력 실측 — 대화(IME on) 조합 전 첫 키 유출 여부 · 목록 일/중 매칭 UX(인라인 검색 필드) 검토 → [27 §7-1](27-typeahead-hangul-composition.md)

---

## §9. 🔐 보안 상시 관찰 (SEC) — **끝나지 않는 항목**

> **다른 섹션과 성격이 다르다.** 마일스톤이 끝나면 닫히는 일이 아니라 **제품이 사는 동안 계속 돌아야 하는 일**이다.
> 기준 = [29 와이어 보안 점검 기준](29-wire-security-audit.md) W-1~W-12 · 회귀 = [18 §2](18-build-and-test.md) · 리스크 SSOT = [05 §4](05-requirements.md).

### 9-1. 점검 트리거 — **달력이 아니라 사건에 건다**

주기적 점검은 잊힌다. **무엇을 건드리면 무엇을 다시 보는지**로 묶는다.

| ID | 트리거(이걸 하면) | 반드시 다시 하는 것 | 근거 |
|---|---|---|---|
| **SEC-T1** | **발견 와이어 포맷을 바꿨다** | **W-1~W-7 재실행** — 인코더 골든 + **수동 캡처로 ASCII 눈으로 확인**(M1-12) | 평문 구간이라 한 필드가 곧 유출 |
| **SEC-T2** | **브로드캐스트에 필드를 추가하려 한다** | **허용 목록 심사**(FR-S-49) — *"이게 평문으로 LAN 전체에 방송돼도 되는가"* 를 먼저 답한다. 기본 답은 **아니오** | R-19가 이 심사 부재로 생겼다 |
| **SEC-T3** | **새 기능·ADR을 설계한다** | [10 §0](10-decision-record.md) 질문 — ***"이 계층의 봉투는 무엇이고 내용은 무엇인가"***. 답이 안 나오면 그 계층은 과하게 보고 있다 | 관통 원리 |
| **SEC-T4** | **외부로 나가는 경로를 추가한다** | **FR-S-20 예외 심사** — 지금 예외는 수신자 릴레이 **1건뿐**. 두 번째를 만들려면 ADR이 필요하다 | 인터넷 경로는 되돌리기 어렵다 |
| **SEC-T5** | **외부 crate를 추가한다** | 라이선스 퍼미시브 확인(DR-12) + **서브트리 전수** + 크기 실측 → [10 §3 원장](10-decision-record.md) 등재 | 공급망 |
| **SEC-T6** | **신뢰·신원 판정 코드를 고쳤다** | TOFU 전이(`Unverified`→`Pinned`→`FingerprintVerified`) **육안 재확인** + 신뢰 불상속 테스트 | 배지가 유일한 사용자 신호 |
| **SEC-T7** | **파일 수신 경로를 고쳤다** | [18 §2](18-build-and-test.md) **안전 회귀 전항목**(격리물 실행 불가·복원 후 표식 유지·Zip Slip·압축폭탄·traversal·심링크) | fail-closed 붕괴는 조용하다 |
| **SEC-T8** | **릴리스 직전** | **W-1~W-12 전체** + 안전 회귀 전항목 + at-rest 문자열 스캔 + 아래 9-2 트래커 **전 행 재평가** | 출시 게이트 |
| **SEC-T9** | **마일스톤 종료 시** | 9-2 트래커에서 **상태가 3주 이상 안 바뀐 행**을 다시 판단(방치 탐지) | 리스크는 잊혀서 커진다 |

### 9-2. 열린 보안 항목 트래커 — **닫힐 때까지 남긴다**

| 리스크 | 무엇이 위험한가 | 담당 | 상태 |
|---|---|---|:--:|
| ~~R-19~~ | **표시 이름 기본값이 실명을 평문 방송**(실측 확인) | **M1-10** · Q-29-1 | ✅ **해소(08-11)** — 정제 호스트명+지문 폴백 · 실측 `beep-{지문}` · 실명은 옵트인+고지 |
| **R-18** | **고정 공개키 = 영구 재식별자**(실측 확인) · 완화가 TOFU와 충돌 | Q-29-2 | 🔴 **판단 대기**(감수+고지 유력) |
| ~~R-17~~ | **TOFU 핀이 재시작하면 사라진다** → 매번 "최초 접촉" = MITM 창 상시 개방 | **M2-5a** · D-18 §3 | ✅ **해소(08-11)** — 신원 키+핀 세그먼트 영속(암호화·fail-closed) · 재오픈 `Known(Pinned)` 테스트 고정 |
| **R-1** | 최초 접촉 MITM — **원천 해소 불가** | M3-6(SAS UX) | 🚧 **완화 · 영구 관찰** |
| **R-12** | 키 파일 복제 — U-P1·U-P2 구현됨, **실증 미완**(재현 환경 필요) | 실증 시나리오 | 🚧 **구현 · 미실증** |
| **R-16** | 종료 경로 — 핵심 완료, **Windows 콘솔 핸들러·zeroize** 잔여 | M1-8x · M2-5 | 🚧 |
| **R-5** | 이미지 파서 취약점 = RCE | M4-5(`imgdec` 분리) | ☐ |
| **R-4** | 무인증 발신 → 스팸 폭탄 | M5-3 | ☐ |
| **R-13** | 수신자 릴레이 남용 — "내가 켠 규칙이 나를 때린다" | M3-10 | ☐ |
| **R-10 · R-11** | `UserId` 단일 실패점 · 팬아웃 중복 전송 | X-6 | ⏸ v2 |
| **W-8** | **세션 평문 부재를 실소켓으로 증명 못 했다**(단위 tap만) | **M4-3b** | ⏸ 실기 |
| **W-11** | `--chat-*` 도구가 **본문을 stdout에 찍는다** | Q-29-4 | ⚠️ 검증 도구 한정 |
| **W-12** | at-rest 평문 부재 미검증 | M2-5b | ☐ |

### 9-3. 상시 과제

| ID | 항목 | 상태 |
|---|---|:--:|
| **SEC-1** | **와이어 평문 자동 회귀를 CI에 상시**(FR-S-51) — 이게 있어야 SEC-T1이 사람 기억에 의존하지 않는다 | 🚧 (08-09 — **안전 송수신 4항목은 CI 상시화 완료**([safety_regression](../crates/nbeep-safe/tests/safety_regression.rs) · 이름 있는 단계 · 변이 검사로 유효성 확인). 잔여 = **와이어 평문 회귀**(W-1~W-7) 자동화) |
| **SEC-2** | **수동 캡처 스크립트를 저장소에** 두고 포맷 변경 커밋에서 재실행 | ☐ **M1-12** |
| **SEC-3** | 릴리스 노트에 **"막지 못하는 것"** 을 항상 싣는다([29 §5](29-wire-security-audit.md) — 존재·연결 상대·길이·타이밍) | ☐ M5-6 |
| **SEC-4** | 리스크 등록부([05 §4](05-requirements.md))와 이 트래커의 **불일치 점검** — 새 리스크가 여기 안 올라오면 관찰에서 빠진다 | 상시 |

---
