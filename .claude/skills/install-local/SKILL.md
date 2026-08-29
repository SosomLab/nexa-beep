---
name: install-local
description: 릴리스 빌드 산출물을 로컬에 설치된 자리(.deb /usr/bin · brew cask .app · NSIS %LOCALAPPDATA%)에 덮어쓰고 설치본처럼 실행한다. "설치본에 반영", "빌드해서 설치 자리에 넣어줘", "새 버전 설치한 것처럼 테스트", "배포 없이 설치본 갱신" 같은 요청에 쓴다.
---

# 설치 자리 덮어쓰기 실기 — 빌드 → 중지 → 덮어쓰기 → 설치본 실행

```bash
./tools/install-local.sh             # ① 종료 ② 릴리스 빌드 ③ 설치 자리 덮어쓰기 ④ 설치본 실행 ⑤ 확인
./tools/install-local.sh --no-build  # 직전 산출물 그대로
./tools/install-local.sh --no-run    # 복사까지만
```

| OS | 설치 자리 | 권한 |
|---|---|---|
| Linux(.deb) | `/usr/bin/{nexa-beep,nbeep-imgdec}` | root 소유 → **sudo 1회**(비밀번호는 사용자 터미널에서) |
| macOS(brew cask) | `/Applications/Nexa Beep.app/Contents/MacOS/…` | 사용자 소유 · ad-hoc 재서명+quarantine 제거 포함 |
| Windows(NSIS) | `%LOCALAPPDATA%\Programs\NexaBeep\…exe` | 사용자 소유 |

## 언제 이것, 언제 relaunch

- **relaunch** = 개발 빌드 3신원(`target/release` + `~/.nexa-beep-multi/{A,B}`) — 그룹·발견 실기.
- **install-local** = **설치본 하나**를 새 산출물로 — 설치 경로·런처(.desktop/.app/시작 메뉴)·
  **자동 실행 등록·트레이·재부팅** 같이 "설치 자리에서만 재현되는" 실기.
- 둘은 신원이 다르다(설치본 = `~/.config/nexa-beep`·`~/Library/Application Support`·`%APPDATA%` /
  개발 빌드 = exe 옆 `data/`). 자동 실행 슬롯도 exe 경로로 갈린다(08-29).

## 주의

- Linux는 `sudo`가 비밀번호를 물으므로 **Claude 세션에서는 완주할 수 없다** — 명령을 사용자에게
  건네고 결과(`md5`·`--version`)로 확인한다. mac/Win은 세션에서 완주 가능.
- 버전 문자열·패키지 관리자 등록은 그대로다(실기 전용 · 배포 대체 아님). 다음 정식 설치가 덮어쓴다.
- 판정은 md5 대조(③)와 프로세스 수(⑤)로 한다 — "복사했다"가 "바뀌었다"는 아니다.
