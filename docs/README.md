# 📖 Nexa Beep — 문서 홈

> **처음 보는 사람을 위한 길잡이.** 아래 **추천 읽기 순서**를 따라가면 _시발점 → 주요 목표 → 진행 경과 → 상세_ 순으로 이해할 수 있다.
> 프로젝트 한 줄 소개는 루트 [README](../README.md).
>
> **한 장 현황이 급하면 →** [STATUS](STATUS.md) · **최근 무슨 일 →** [DEVLOG](DEVLOG.md) · **기능 현황 →** [MILESTONES](MILESTONES.md).

---

## 🧭 추천 읽기 순서

1. **왜 만드나** — [00 비전](00-vision.md) : 문제 정의 · 경쟁 좌표 · 성공 기준
2. **경쟁 지형** — [03 경쟁 프로그램 조사](03-competitive-landscape.md) : 국내·국외 유사 프로그램 기능표·장단점
3. **안전 설계** — [04 안전 송수신](04-safe-transfer.md) : 페이로드 3종 + 수신 파일 무해화
4. **무엇을 만드나** — [05 요구사항](05-requirements.md) : FR/NFR(예산 수치)/제약/리스크
5. **핵심 결정** — [10 결정 기록](10-decision-record.md) : DR-1~14 + ADR 색인
6. **어떻게 짓나** — [01 아키텍처](01-architecture.md) → [02 로드맵](02-roadmap.md)
7. **지금 상태** — [STATUS](STATUS.md) → [MILESTONES](MILESTONES.md)
8. **진행 경과** — [DEVLOG](DEVLOG.md)(시간 역순) → 관심 날짜 [journal/](journal/)

## ① 시발점 · 정체성

| 문서 | 내용 |
|---|---|
| [00 비전](00-vision.md) | ★ 문제 정의 · 경쟁 좌표 · 성공 기준 S-1~8 · 정직하게 어려운 것 V-1~6 |
| [03 경쟁 프로그램 조사](03-competitive-landscape.md) | ★ 국내·국외 유사 프로그램 37종 · 기능 매트릭스 · 장단점 |
| [04 안전 송수신 설계](04-safe-transfer.md) | ★ Text/Image/File 페이로드 · **수신 파일 무해화 4단계 게이트**(MotW·Gatekeeper·AMSI·CDR) |
| [05 요구사항](05-requirements.md) | ★ FR 57건 · **NFR-B 예산 게이트(수치 확정)** · 제약 C-1~6 · 리스크 R-1~9 |
| [10 결정 기록](10-decision-record.md) | ★ DR-1~14 · ADR 색인 · 의존성/차용자산 원장 |
| [CLAUDE.md](../CLAUDE.md) | 이식용 프로젝트 메모리 |
| [LICENSE](../LICENSE.md) · [한글](../LICENSE.ko.md) | PolyForm Noncommercial 1.0.0 — 의존성 퍼미시브 온리(DR-12) |

## ② 주요 목표 · 설계

| 문서 | 내용 |
|---|---|
| [01 아키텍처](01-architecture.md) | ★ 9크레이트 구조 · 단방향 의존 · 스레딩 · 데이터 흐름 · 영속성 · 테스트 전략 |
| [02 로드맵](02-roadmap.md) | ★ M0~M5 + 포스트 · 마일스톤별 게이트 · **리스크 소멸 시점** |
| [06 네트워크 스택 L1~L4](06-network-stack.md) | ★ 계층별 저수준 설계 · 다단 발견 사다리(S1~S6) · **능력 등급 T0~T2** · 실측 항목 E-1~E-9 |
| [07 ADR-0001 스택](07-adr-0001-stack.md) | ✅ **Accepted** — Rust · 자체 CPU 래스터라이저 · 플랫폼 계층 P2 · Wayland+X11 · 시스템 폰트 · **SP-1 스파이크** |
| [08 ADR-0002 디스커버리·전송·암호](08-adr-0002-discovery-transport.md) | ✅ **Accepted** — 자체 컴팩트 UDP · TCP 세션 · **Noise_XX + TOFU** · 발견은 "미검증 힌트" |
| [09 ADR-0003 전송 추상화](09-adr-0003-transport-abstraction.md) | 📐 **Proposed** — 4계층 경계 · `PeerId`/`Locator`/`Link`/`Session` · 인메모리 전송으로 네트워크 없는 테스트 |
| [11 ADR-0004 수신 무해화](11-adr-0004-quarantine.md) | 📐 **Proposed** — `.beepq` 레이아웃 · 위험 등급 4단계 확장자표 · 상태 기계(fail-closed) · MotW/quarantine API |
| [12 차용 자산 실측 평가](12-asset-reuse.md) | ★ **`ctl` 재사용 불가 판정**(HWND 모델) · 실제 자산은 `nexa-gui` 인프라 1,187 LOC · R-3 재평가 |
| [13 코드 설계 표준](13-code-design-standards.md) | ★★ 모듈화 · 포트&어댑터 · **`ActionKind` 단일 통로** · **인터셉터** · **사용 계측 레저** · 코드 리뷰 체크리스트 |
| [MILESTONES](MILESTONES.md) | ★ 기능·마일스톤 현황(✅/🚧/📐/☐) |

### 📌 문서 번호 배정 계획 (번호는 부여 후 **불변** — [16 §2-8](16-doc-git-conventions.md))

| 번호 | 문서 | 상태 |
|---|---|---|
| **00 · 01 · 02** | 비전 · 아키텍처 · 로드맵 | ✅ |
| **03 · 04** | 경쟁 조사 · 안전 송수신 | ✅ |
| **05** | 요구사항 | ✅ |
| **06** | 네트워크 스택 L1~L4 | ✅ |
| **07** | **ADR-0001 스택** | ✅ Accepted |
| **08** | **ADR-0002 디스커버리/전송/암호** | ✅ Accepted |
| **09** | **ADR-0003 전송 추상화** | 📐 Proposed |
| **10** | 결정 기록(DR·ADR 색인) | ✅ |
| **11** | **ADR-0004 수신 무해화** | 📐 Proposed |
| **12** | 차용 자산 실측 평가 | ✅ |
| **13** | 코드 설계 표준 | ✅ |
| **15 · 16 · 18** | 개발 방법론 · 문서/git 규약 · 빌드&테스트 | ✅ |

## ③ 진행 경과 · 할 일

| 문서 | 내용 |
|---|---|
| [STATUS](STATUS.md) | ★ 현재 상태 한 장 |
| [DEVLOG](DEVLOG.md) | ★ 진행 시간순(최신 위) · [journal/](journal/) 일자 상세 |
| [BRANCHES](BRANCHES.md) | 브랜치 생성/작업/병합/삭제 이력 |
| [TODO](TODO.md) | ★ 목표 순차 백로그 |

## ④ 개발 · 기여

| 문서 | 내용 |
|---|---|
| [18 빌드 & 테스트](18-build-and-test.md) | ★ 빌드·테스트·네트워크 검증 절차(SSOT) |
| [15 개발 방법론](15-dev-methodology.md) | 수직 슬라이스·스파이크·커밋 규약 |
| [16 문서·커밋/푸시 규약](16-doc-git-conventions.md) | ★ 문서 4층 체계·커밋/브랜치/푸시 규칙 — `nexa-dir2` 차용 |

---

> 문서 규약: 진행 기록은 **일자 단위** — 상세 `journal/YYYY-MM-DD.md`(시간 역순), 요약 [DEVLOG](DEVLOG.md)·기능 [MILESTONES](MILESTONES.md). 결정은 [10](10-decision-record.md), 빌드/테스트 SSOT는 [18](18-build-and-test.md). **문서 번호(NN-)는 불변** — 재번호 금지, 신규는 뒤에 append.
