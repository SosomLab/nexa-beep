---
name: install-local
description: 로컬에서 릴리스로 컴파일한 산출물을 설치된 자리(.deb /usr/bin · brew cask .app · NSIS %LOCALAPPDATA%)에 덮어쓰고 설치본처럼 실행한다 — 릴리스를 받지 않고 설치본을 갱신. "설치본에 반영", "빌드해서 설치 자리에 넣어줘", "새 버전 설치한 것처럼 테스트", "배포 없이 설치본 갱신", "로컬 빌드로 업데이트" 같은 요청에 쓴다.
---

# 설치 자리 덮어쓰기 실기 — 빌드 → 중지 → 덮어쓰기 → 설치본 실행 (SSOT = [docs/18 §0-2](../../../docs/18-build-and-test.md))

```bash
./tools/install-local.sh             # ① 종료 ② 릴리스 빌드 ③ 설치 자리 덮어쓰기 ④ 설치본 실행 ⑤ md5·프로세스·--whoami 확인
./tools/install-local.sh --debug     # 디버그 프로필(진단)
./tools/install-local.sh --no-build  # 직전 산출물 그대로
./tools/install-local.sh --no-run    # 복사까지만
./tools/install-local.sh --assets    # (Linux) .desktop·아이콘까지
NEXA_INSTALL_DIR=/opt/x ./tools/install-local.sh   # 비표준 자리(Linux)
```

```powershell
pwsh tools/install-local.ps1 [-Debug] [-NoBuild] [-NoRun]   # Windows 네이티브 — Git Bash 불요(sh를 부르면 여기로 위임)
```

| OS | 설치 자리 | 탐지·권한 | 기동 |
|---|---|---|---|
| Linux(.deb) | `/usr/bin/{nexa-beep,nbeep-imgdec}` | `NEXA_INSTALL_DIR` → `command -v` → `/usr/bin` · root → **sudo 1회**(TTY 없으면 pkexec GUI 창) | `gio launch <설치 .desktop 경로>` · 로그 `target/installed-nexa-beep.log` |
| macOS(brew cask) | `/Applications/Nexa Beep.app/Contents/MacOS/…` | 사용자 소유 · ad-hoc 재서명+quarantine 제거 | `open -a "Nexa Beep"` |
| Windows(NSIS) | `%LOCALAPPDATA%\Programs\NexaBeep\…exe` | `$env:NEXA_INSTALL_DIR` → HKCU `InstallDir` → 기본 · 사용자 소유 | `Start-Process`(무인자) |

## 언제 이것, 언제 relaunch

- **relaunch** = 개발 빌드 3신원(`target/release` + `~/.nexa-beep-multi/{A,B}`) — 그룹·발견 실기.
- **install-local** = **설치본 하나**를 새 산출물로 — 설치 경로·런처(.desktop/.app/시작 메뉴)·
  **자동 실행 등록·트레이·재부팅·부팅 스윕(0600)** 같이 "설치 자리에서만 재현되는" 실기.
- 둘은 신원이 다르다(설치본 = `~/.config/nexa-beep`·`~/Library/Application Support`·`%APPDATA%` /
  개발 빌드 = exe 옆 `data/`). 자동 실행 슬롯도 exe 경로로 갈린다(08-29). ⑤의 `--whoami` 출력으로 어느 신원이 떴는지 확인한다.

## 주의

- Linux `/usr/bin`은 `sudo`가 비밀번호를 물으므로 **TTY 없는 Claude 세션에서는 pkexec 창이 뜨거나 멈춘다** — 명령을 사용자에게
  건네고 결과(`md5`·`--version`·`/proc/<pid>/exe`)로 확인한다. mac/Win은 세션에서 완주 가능(mac 09-05 실측 43s).
- 버전 문자열·패키지 관리자 등록은 그대로다(실기 전용 · 배포 대체 아님). 다음 정식 설치가 덮어쓴다.
- 판정은 md5 대조(③)와 프로세스 수·`--whoami`(⑤)로 한다 — "복사했다"가 "바뀌었다"는 아니다. 개발 트리를 설치 자리로 오인하면 스크립트가 중단한다.
