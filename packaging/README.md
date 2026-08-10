# packaging — 배포 산출물 만들기

> 워크플로 = [`.github/workflows/release.yml`](../.github/workflows/release.yml).
> 빌드·테스트 절차 SSOT는 [docs/18](../docs/18-build-and-test.md), 채널 결정은 **DR-4**,
> 타깃 결정은 **DR-3**([docs/10](../docs/10-decision-record.md)).

## 채널과 타깃

**설치본 + 포터블 2채널**(DR-4)을 **5개 타깃**에 낸다(DR-3).

| 타깃 | 설치본 | 포터블 |
| --- | --- | --- |
| `windows-x64` | NSIS `.exe` (+`.zip`) | `.zip` |
| `windows-arm64` | NSIS `.exe` (+`.zip`) | `.zip` |
| `macos-arm64` | `.dmg` (+`.zip`) | `.zip` |
| `macos-x64` | `.dmg` (+`.zip`) | `.zip` |
| `linux-x64` | `.deb` (+`.zip`) | `.zip` |

설치본을 **원본과 zip 둘 다** 올리는 이유는 실행 파일 확장자를 막는 브라우저·사내
프록시 때문이다(사용자 요청 08-11). 무결성 확인용 `SHA256SUMS.txt`도 함께 올린다 —
서명이 없는 배포에서 사용자가 가진 유일한 검증 수단이다.

## 첫 배포는 수기다 (사용자 확정 08-11)

`release.yml`의 **태그 push 트리거는 주석 처리되어 있다.** 태그를 밀어도 아무 일도
일어나지 않는다. 릴리스는 다음 절차로만 만들어진다.

1. GitHub → Actions → **release** → *Run workflow*
2. `tag` 입력(예: `v0.1.0`) · `publish`는 **끈 채로 둔다**
3. 산출물이 **초안(draft) 릴리스**로 올라온다 → 내려받아 실기 확인
4. 문제 없으면 GitHub에서 **Publish release**

자동 배포로 바꾸려면 `release.yml` 상단의 `push: tags` 블록 주석만 풀면 된다.

## winget · Chocolatey

매니페스트 **틀**은 이 디렉터리에 있고(`winget/`, `choco/`), 체크섬 자리는
`@SHA_...@` 자리표시자다. 릴리스 워크플로가 **실제 산출물의 SHA256으로 채워서**
`...package-manifests.zip`으로 릴리스에 첨부한다 — 손으로 적으면 언젠가 틀리고,
체크섬이 틀린 매니페스트는 **사용자 기기에서 설치 실패**로 나타난다. 워크플로는
치환되지 않은 자리표시자가 하나라도 남으면 거기서 멈춘다.

패키지는 채널별로 **따로** 낸다(패키지 관리자 관례).

| | winget | choco |
| --- | --- | --- |
| 설치본 | `SosomLab.NexaBeep` | `nexa-beep` |
| 포터블 | `SosomLab.NexaBeep.Portable` | `nexa-beep-portable` |

**제출도 수기다.**

- **winget** — 생성된 `winget/manifests/s/SosomLab/...` 트리를
  [microsoft/winget-pkgs](https://github.com/microsoft/winget-pkgs) 같은 경로에 두고 PR.
  로컬 검증: `winget validate <매니페스트 폴더>` · 설치 시험: `winget install --manifest <폴더>`
- **choco** — `cd choco/nexa-beep && choco pack` 후 `choco push --api-key <키>`.
  포터블도 같은 방식. (API 키는 저장소에 두지 않는다.)

## 설치 위치와 권한

Windows 설치본은 **사용자 단위**(`%LOCALAPPDATA%\Programs\NexaBeep` · HKCU)로 넣는다.
관리자 권한을 요구하지 않는다 — T0(무권한)에서 전 기능이 돌아야 한다는 원칙(DR-14)을
설치 단계에도 적용한 것이고, 덕분에 winget/choco의 무인 설치가 권한 상승 없이 통과한다.

## 아직 아닌 것

- **서명하지 않는다**(v1) — 인증서가 없다. macOS Gatekeeper·Windows SmartScreen이
  경고하며, 릴리스 노트에 우회 방법을 숨기지 않고 적는다. 서명은 배포 신뢰의 문제라
  별도 결정으로 다뤄야 한다.
- **포터블과 설치본의 동작 차이가 아직 없다** — 저장하는 것이 없기 때문이다
  (설정 영속 M3-15가 D-25 확정 대기). 채널만 먼저 갈라 두었고, "영속물을 실행 파일
  옆에"(DR-4 · FR-P-3)는 설정 영속과 같은 슬라이스에서 들어간다. 그때까지 두 채널은
  **설치 방식만** 다르다.
- **Linux는 x86_64만** — DR-3이 요구하는 것은 Linux이고 아키텍처는 명시가 없다.
  arm64는 크로스 링크(X11/Wayland) 부담이 있어 수요 확인 후 붙인다.
- `nbeep-imgdec`는 넣지 않는다 — 본체가 아직 호출하지 않는다(M4-5 잔여).
  쓰지 않는 바이너리를 배포에 넣으면 공격면만 늘어난다.
