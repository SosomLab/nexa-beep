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

## 현재 상태 (2026-08-13 · **v0.1.5 공개**)

**설계는 거의 끝났고(문서 31종·ADR 11종), 코드는 M1~M4가 병렬로 진행 중이며, 배포는 먼저 완주했다.**

| 항목 | 상태 |
| --- | --- |
| **되는 것** | 실행하면 같은 LAN의 상대가 자동으로 뜨고(UDP 발견) → 고르면 **Noise_XX 암호화 세션** → 대화 · **파일 송수신**(무해화 게이트 + 수신측 격리 성공까지 확인하는 종단 ack) · 프로필 교환 · 설정/신원/신뢰 핀 영속 |
| **배포** | **v0.1.5** — Windows(x64·ARM64) · macOS(x64·ARM64) · Linux(x64) **5타깃 × 2채널**(설치본·포터블) · Homebrew 탭 · 태그 push = 자동 공개 → [Releases](https://github.com/SosomLab/nexa-beep/releases) |
| **테스트** | 워크스페이스 **485 green** · clippy 경고 0 · CI 4잡(lint / test 3-OS / cross-build / 예산 게이트) |
| **예산** | 산출물 **0.76MB**/게이트 10MB ✅ · 유휴 RSS — Windows 관측 **17.1MB**(≤30MB 안쪽) · **mac 85.7MB 재실측 잔여** |
| **다음 관문** | 🔴 사용자 확정 대기 — 메시지 등급·알림(ADR-0010) · v1.0 기능 커트라인 · 기록 저장 암호화 잔여(ADR-0005 §4~) |

- 예산 목표: 유휴 RSS **≤30MB** · 산출물 **≤10MB**/타깃 · **런타임 의존 0** · 24시간 상주 누수 0 ([05 NFR-B](docs/05-requirements.md)).
- 한 장 현황 → [docs/STATUS.md](docs/STATUS.md) · 기능별 → [docs/MILESTONES.md](docs/MILESTONES.md)

## 설치

모든 채널은 태그 릴리스에서 나온다. 자세한 안내는 **[위키 Install](https://github.com/SosomLab/nexa-beep/wiki/Install)**.

```sh
# macOS — Homebrew
brew tap kiros33/tap && brew install --cask nexa-beep

# Linux — deb 또는 포터블
sudo dpkg -i nexa-beep-*-linux-x64.deb
```

Windows는 설치본(`*-setup.exe` — 사용자 단위·무권한) 또는 포터블 zip을 풀고 `nexa-beep.exe`를 더블클릭한다.
릴리스마다 `SHA256SUMS.txt`가 함께 올라간다.

> ⚠️ 아직 **코드 서명·공증 전**이다. macOS Cask는 설치 시 격리 표식을 제거하며(임시방편), Windows는 SmartScreen 경고가 날 수 있다.

## 문서 — 📖 [문서 홈](docs/README.md)에서 시작

바로가기: [현황 STATUS](docs/STATUS.md) · [진행 DEVLOG](docs/DEVLOG.md) · [기능·마일스톤](docs/MILESTONES.md) · [종단 동작 설명서 30](docs/30-end-to-end-walkthrough.md) · [경쟁 조사 03](docs/03-competitive-landscape.md) · [결정 기록 10](docs/10-decision-record.md) · [이식 메모리](CLAUDE.md)

요약·안내는 **[위키](https://github.com/SosomLab/nexa-beep/wiki)**(Install · Features · Architecture · Security · Development · Release Notes).
단일 진실 원천(SSOT)은 언제나 저장소 `docs/`다.

## 프로젝트 정보 / 라이선스

| 항목 | 내용 |
| --- | --- |
| 조직 | **SosomLab** — <https://sosomlab.com> |
| 개발자 | Sangyong Bae — kiros33@gmail.com |
| 관련 저장소 | [`nexa-dir2`](https://github.com/SosomLab/nexa-dir2)(문서·컨트롤 표준 차용) · `nexa-beepd`(릴레이 서버 — 예정) |

**PolyForm Noncommercial 1.0.0** ([LICENSE.md](LICENSE.md) · 한글 [LICENSE.ko.md](LICENSE.ko.md)) — 개인·비상업 무료, 상업 사용은 유료 라이선스(문의 kiros33@sosomlab.com).
