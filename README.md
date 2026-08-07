# Nexa Beep — 제로 컨피그 로컬 네트워크 메신저

> Zero-configuration LAN messenger — run it, and everyone else running it on your network is just there.

**Nexa Beep**는 **설정이 없는** 로컬 네트워크 메신저입니다.
서버 주소도, 계정도, 상대 등록도 필요 없습니다. 실행하면 같은 네트워크에서 실행 중인
사용자가 **자동으로 목록에 나타나고**, 고르면 **바로 대화**가 시작됩니다.
발견된 사용자는 **그룹으로 묶어** 그룹 전체에 한 번에 보낼 수 있습니다.

## 설계 원칙

1. **실행 = 참여 (Zero-config)** — 첫 실행부터 대화까지 설정 화면이 없다.
2. **자동 발견 (Auto-discovery)** — 사전 등록 없이 서로를 스스로 찾는다. 멀티캐스트가 막힌 사내망까지 **다단 폴백**으로.
3. **직접 대화 (Serverless-first)** — 기본은 P2P 직접 연결. 서버 경유는 **선택**이지 전제가 아니다.
4. **종단 암호화 (E2E)** — 경로가 무엇이든 내용은 두 단말만 읽는다. 릴레이 서버도 예외 없다.
5. **받은 파일은 실행되지 않는다** — Text·Image·File을 주고받되, 수신 파일은 **격리 → 검사 → 승인**을 거쳐야 실체가 된다. 앱은 어떤 경우에도 받은 파일을 실행하지 않는다.
6. **가볍다 (Budget-first)** — 런타임 의존 없는 네이티브. 최소 크기·최소 메모리·장기 실행 누수 0.
7. **어디서나 같은 화면** — Windows(x64·ARM64)·macOS·Linux에서 **모든 렌더링을 직접 그려** 동일한 UI.

## 동작 모드

| 모드 | 설명 | 상태 |
|---|---|---|
| **로컬 직접**(기본) | LAN 자동 발견 + P2P 직접 연결. 서버 없음 | v1 범위 |
| **릴레이 경유**(선택) | 특정 서버를 추가하면 IP·로컬에 매이지 않고 일반 메신저처럼 통신. 서버는 봉투만 보고 내용은 못 읽는다 | 설계에 반영 · 구현은 v1 이후 (서버는 별도 프로젝트 `nexa-beepd`) |

## 현재 상태

- 단계: **M-1 설계** — [경쟁 프로그램 37종 조사 완료](docs/03-competitive-landscape.md). 기술 스택·프로토콜 ADR 진행 예정.
- 상세 → [docs/STATUS.md](docs/STATUS.md)

## 문서 — 📖 [문서 홈](docs/README.md)에서 시작

바로가기: [현황 STATUS](docs/STATUS.md) · [진행 DEVLOG](docs/DEVLOG.md) · [기능·마일스톤](docs/MILESTONES.md) · [경쟁 조사 03](docs/03-competitive-landscape.md) · [결정 기록 10](docs/10-decision-record.md) · [이식 메모리](CLAUDE.md)

## 프로젝트 정보 / 라이선스

| 항목 | 내용 |
| --- | --- |
| 조직 | **SosomLab** — <https://sosomlab.com> |
| 개발자 | Sangyong Bae — kiros33@gmail.com |
| 관련 저장소 | [`nexa-dir2`](https://github.com/SosomLab/nexa-dir2)(문서·컨트롤 표준 차용) · `nexa-beepd`(릴레이 서버 — 예정) |

**PolyForm Noncommercial 1.0.0** ([LICENSE.md](LICENSE.md) · 한글 [LICENSE.ko.md](LICENSE.ko.md)) — 개인·비상업 무료, 상업 사용은 유료 라이선스(문의 kiros33@sosomlab.com).
