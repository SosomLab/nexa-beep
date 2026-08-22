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
| **릴레이 경유**(선택) | 특정 서버를 추가하면 IP·로컬에 매이지 않고 일반 메신저처럼 통신. 서버는 봉투만 보고 내용은 못 읽는다 | **개발 중** — 서버 MVP `nexa-beepd`(같은 저장소 · `beepd-v*` 별도 배포) + UDP 홀펀칭→릴레이 폴백 사다리 + CLI 접속까지 구현 · GUI 배선·실 NAT 실증 잔여 |

## 현재 상태 (2026-08-22 · **v0.2.3 공개 — 서버 접속(GUI Managed) 개통 · 릴레이 서버 첫 릴리스**)

**설계는 사실상 끝났고**(문서 41종 · ADR 14종 = ✅13/📐1), **코드는 M1~M5가 병렬로 진행 중**이며,
**배포는 3채널(Releases · Homebrew · winget/Chocolatey)까지 완주**했다.

| 항목 | 상태 |
| --- | --- |
| **되는 것** | 실행하면 같은 LAN의 상대가 자동으로 뜨고(UDP 발견 S1~S4) → 고르면 **Noise_XX 암호화 세션** → 대화 · **파일 송수신**(무해화 4단계 게이트 + 종단 ack · 10GB급 스트리밍 · **끊겨도 이어받기** · 풍선 안 ⏸▶✕ · 요청 단위 승인) · **공유 그룹 채팅**(한 방 대화·초대 수락제·기록 영속·파일 팬아웃) · 프로필 교환(아바타·변경 전파) · 알림·트레이 상주 · **시작 시 자동 실행**(기본 on·옵트아웃) · **CLI 단말**(`--chat-live` — 발견 조회·능동 연결·수신 전용 채널) · 설정/신원/신뢰 핀/대화 기록 **전부 암호화 영속** |
| **배포** | **v0.2.3** — Windows(x64·ARM64) · macOS(x64·ARM64) · Linux(x64) **5타깃 × 2채널**(설치본·포터블 · 자산 14종) · **Homebrew** 자동 추종 · winget·Chocolatey는 **첫 게시 검수 대기**(통과 전 새 버전 제출 보류) · 서버 `nexa-beepd`는 **`beepd-v*` 별도 릴리스**(Linux = musl 정적) · 태그 push = 자동 → [Releases](https://github.com/SosomLab/nexa-beep/releases) |
| **테스트** | 워크스페이스 **777 green** · clippy 경고 0 · CI 4잡(lint / test 3-OS / cross-build / 예산 게이트) |
| **예산** | 산출물 **약 1.7MB**/게이트 10MB ✅ · 유휴 실점유 **Windows 17.1MB · macOS 18~20MB**(`phys_footprint`) — **3-OS 전부 ≤30MB ✅**(R-8 해소 · mac `ps` RSS는 reclaimable 허수라 판정에 쓰지 않는다) |
| **보안** | 저장 축 **평문 0 실측 감사**(기록·격리물·PII·프로필 캐시 전부 `NBSE` 봉인) · **크립토 셰레딩**(대화 삭제 = 데이터 키 폐기) · **AMSI 실물 검사**(Windows) · **경로 2축 정책**(신원 신뢰는 키에 · 경로 등급은 통로에 — 인터넷 경유는 대조 전 파일 차단) |
| **실증** | **2-PC 실기**(Win↔Mac — 메시지·파일·프로필 왕복 · 교차 대용량 전송·이어받기) · **3-신원 그룹 실기** · **10분 자동 네트워크 계측**(과다 송수신 0 — [39 점검 기록부](docs/39-netmon-records.md)) |
| **서버 축**(진행 중) | **UDP + 릴레이 서버 개통**(08-21~22) — 자작 경량 ARQ `UdpLink` · 릴레이 서버 MVP `nexa-beepd`(blind 파이프 · 외부 의존 0 · 0.35MB) · 랑데부→**홀펀칭**→릴레이 폴백 사다리 · CLI `--server` 접속(서버 핀 TOFU) — 종단 Noise는 코드 무변경 관통 · **잔여 = GUI 배선 · 실 NAT 홀펀칭 실증** |

- 예산 목표: 유휴 RSS **≤30MB** · 산출물 **≤10MB**/타깃 · **런타임 의존 0** · 24시간 상주 누수 0 ([05 NFR-B](docs/05-requirements.md)).
- 한 장 현황 → [docs/STATUS.md](docs/STATUS.md) · 기능별 → [docs/MILESTONES.md](docs/MILESTONES.md) · **못 막는 것** → [docs/40 알려진 한계](docs/40-known-limitations.md)

## 설치

모든 채널은 태그 릴리스에서 나온다. 자세한 안내는 **[위키 Install](https://github.com/SosomLab/nexa-beep/wiki/Install)**.

```sh
# macOS — Homebrew (권장: Cask가 격리 표식을 떼 준다)
brew install --cask kiros33/tap/nexa-beep

# Windows — winget 또는 Chocolatey (첫 게시 · 중앙 검수 통과 후 동작)
winget install SosomLab.NexaBeep
choco  install nexa-beep

# Linux — deb 또는 포터블
sudo dpkg -i nexa-beep-*-linux-x64.deb
```

패키지 관리자를 쓰지 않으면 설치본(`*-setup.exe` — 사용자 단위·무권한) 또는 포터블 zip을 풀고
`nexa-beep.exe`를 더블클릭한다. 릴리스마다 `SHA256SUMS.txt`가 함께 올라간다.

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
| 관련 저장소 | [`nexa-dir2`](https://github.com/SosomLab/nexa-dir2)(문서·컨트롤 표준 차용) · `nexa-beepd`(릴레이 서버 — [crates/nexa-beepd](crates/nexa-beepd) · 같은 저장소 `beepd-v*` 별도 배포) |

**PolyForm Noncommercial 1.0.0** ([LICENSE.md](LICENSE.md) · 한글 [LICENSE.ko.md](LICENSE.ko.md)) — 개인·비상업 무료, 상업 사용은 유료 라이선스(문의 kiros33@sosomlab.com).
