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
| `macos-arm64` | `.dmg` | `.tar.gz` |
| `macos-x64` | `.dmg` | `.tar.gz` |
| `linux-x64` | `.deb` | `.tar.gz` |

**압축 형식은 플랫폼 관례를 따른다**(사용자 확정 08-11). Windows는 zip이고,
`setup.exe`에는 **zip 사본을 하나 더** 올린다 — 실행 파일 확장자를 막는 브라우저·사내
프록시 때문이다. macOS/Linux 포터블은 `tar.gz`로, **실행 권한이 보존된다**(풀고 나서
`chmod`할 필요가 없다). `.dmg`/`.deb`은 이미 배포 형식이라 덧씌우지 않는다.

무결성 확인용 `SHA256SUMS.txt`도 함께 올린다 — 서명이 없는 배포에서 사용자가 가진
유일한 검증 수단이다.

## ⚠️ macOS 격리(quarantine) — 실측으로 확인한 것

서명·공증이 없는 앱은 격리 표식이 붙어 있으면 **실행 즉시 SIGKILL(exit 137)** 되고,
macOS가 `/Applications`에서 앱을 치워 버린다. 08-11 실측(macOS 15 · Intel):

| 상태 | 결과 |
| --- | --- |
| 서명 없음 + 격리 | `exit 137`(강제 종료 · 앱 삭제됨) |
| **애드혹 서명** + 격리 | `exit 137` — **서명은 격리를 넘지 못한다** |
| 격리 제거(서명 유무 무관) | 정상 실행 |

→ ① `.app`에 **애드혹 서명**을 붙인다(Apple Silicon은 서명 없는 번들을 아예 실행하지
않는다) ② **Cask `postflight`에서 격리 표식을 뗀다**(그것 외에 방법이 없다 — 공증에는
Apple Developer 인증서가 필요하다). 무엇을 왜 하는지는 cask `caveats`와 릴리스 노트에
그대로 밝힌다. **사용자가 모르는 채로 보안 검사를 끄지는 않는다.**

`.dmg`를 직접 받은 경우에는 사용자가 직접 떼야 한다:

```bash
xattr -dr com.apple.quarantine "/Applications/Nexa Beep.app"
```

## 트리거 정책 (사용자 확정 08-11)

**기본 배포는 자동, 패키지 매니저 제출은 채널마다 다르다.**

| 무엇 | 언제 | 잠금 |
| --- | --- | --- |
| GitHub Release(설치본·포터블·체크섬) | `v*` 태그 push → **자동 공개** | 없음 |
| Homebrew 탭(macOS/Linux) | 릴리스 직후 **자동** | `TAP_TOKEN` 시크릿 유무 |
| winget · Chocolatey(Windows) | 릴리스 직후 실행되나 **기본 꺼짐** | **저장소 변수** + 시크릿 |

```bash
git tag v0.1.0 && git push origin v0.1.0     # 이게 전부
```

태그 없이 산출물만 확인하려면 Actions → **release** → *Run workflow*(그 경로로 만들면
**초안**이다). 제출만 다시 하려면 **homebrew** 또는 **publish-windows-packages**
워크플로를 태그를 주고 실행한다.

### Windows 채널만 변수로 잠근 이유

winget과 Chocolatey는 **중앙 저장소 검수**를 거치고, 한번 올라가면 되돌리기 어렵다.
그래서 `nexa-memkeeper`(Windows 전용 프로젝트)에서 검증된 **변수+시크릿 이중 게이트**를
그대로 차용했다 — 기본이 꺼짐이라 준비 전에는 아무것도 나가지 않고, 준비되면 변수
하나만 켜면 된다(코드 무변경).

| 채널 | 변수(Settings ▸ Variables) | 시크릿 |
| --- | --- | --- |
| winget | `WINGET_PUBLISH=true` | `WINGET_TOKEN` |
| Chocolatey | `CHOCO_PUSH=true` | `CHOCO_API_KEY` |

**Homebrew에는 이 스위치가 없다.** 검수가 없는 내 탭이고 되돌리기도 커밋 하나여서,
릴리스와 함께 따라가는 편이 사용자에게 일관된다. `TAP_TOKEN`이 없으면 파일만 만들어
아티팩트로 올리고 **왜 안 나갔는지 말한다**(조용히 성공한 척하지 않는다).

변수가 꺼져 있어도 매니페스트는 **항상 만들어** 아티팩트·릴리스 자산으로 올린다 —
손으로 제출할 수 있게.

## Homebrew (macOS · Linux)

탭 저장소 <https://github.com/kiros33/homebrew-tap>에 두 정의를 넣는다.

| 채널 | 이름 | 설치 |
| --- | --- | --- |
| 설치본(.app) | Cask `nexa-beep` | `brew install --cask kiros33/tap/nexa-beep` |
| 포터블(실행 파일) | Formula `nexa-beep-portable` | `brew install kiros33/tap/nexa-beep-portable` |

이름을 다르게 둔 이유는, 같은 탭에 같은 이름의 cask와 formula가 있으면
`brew install nexa-beep`이 무엇을 뜻하는지 모호해지기 때문이다.

★ **Homebrew Cask는 설치 시 quarantine 속성을 뗀다** — 서명하지 않은 이 앱도
Gatekeeper 경고 없이 실행된다. macOS 사용자에게 권할 경로다.

> 참고 프로젝트(`sosomlab-tauri-test1`)에는 *"새 버전마다 cask의 version/sha256를
> 손으로 갱신해야 한다"*가 마찰점으로 적혀 있었다. 여기서는 `render-manifests.sh`가
> 실제 산출물에서 해시를 계산하므로 그 손작업이 없다.

## winget · Chocolatey

매니페스트 **틀**은 이 디렉터리에 있고(`winget/`, `choco/`, `homebrew/`), 체크섬 자리는
자리표시자다. **`render-manifests.sh`가 유일한 치환 지점**이며(릴리스 경로와 제출 경로가
각자 치환하면 언젠가 갈라진다), 실제 산출물의 SHA256으로 채워
`...package-manifests.zip`으로 릴리스에 첨부한다 — 손으로 적으면 언젠가 틀리고,
체크섬이 틀린 매니페스트는 **사용자 기기에서 설치 실패**로 나타난다. 워크플로는
치환되지 않은 자리표시자가 하나라도 남으면 거기서 멈춘다.

패키지는 채널별로 **따로** 낸다(패키지 관리자 관례).

| | winget | choco |
| --- | --- | --- |
| 설치본 | `SosomLab.NexaBeep` | `nexa-beep` |
| 포터블 | `SosomLab.NexaBeep.Portable` | `nexa-beep-portable` |

변수를 켜면 워크플로가 제출까지 한다 — winget은 `wingetcreate submit`으로 PR,
choco는 `choco pack` 후 `choco push`. 손으로 하려면 아티팩트를 받아서:

- **winget** — `winget validate <폴더>`로 검증 후
  [microsoft/winget-pkgs](https://github.com/microsoft/winget-pkgs)에 같은 경로로 PR.
- **choco** — `choco push <nupkg> --source https://push.chocolatey.org/ --api-key <키>`.

둘 다 **검수를 거쳐야 노출된다** — push했다고 바로 설치되지 않는다.

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
