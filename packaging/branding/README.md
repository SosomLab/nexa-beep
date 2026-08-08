# Nexa Beep 브랜딩 자산

앱 아이콘의 **SSOT**. SosomLab "Nexa" 계열(풀블리드 라운드 스퀘어)을 계승하되, Nexa Beep의
정체성인 **제로 컨피그 LAN 메신저**에 맞춰 **말풍선(대화) + 비콘/브로드캐스트 파동(“beep”=신호)**
모티프로 그렸다. nexa-dir2의 `>_`(폴더+터미널) 자리에 **대화+발견 파동**을 둔 것이 계열 내 차별점.

## 파일

| 파일 | 크기 | 용도 |
| --- | --- | --- |
| `icon.svg` | 벡터 | **원본 SSOT** — 아이콘 변경은 여기부터 |
| `nexa-beep-1024.png` | 1024 | 스토어·고해상도(macOS `.app`/`.icns` 원본) |
| `nexa-beep-256.png` | 256 | 콘솔 로고·일반 배포 |
| `nexa-beep-64.png` | 64 | 작은 아이콘 |
| `nexa-beep.ico` | 256+64 | Windows 아이콘(멀티 프레임) |

## 색·모티프

- 배경: accent 그라디언트 `#4A97FF → #2C6BE6`(앱 `theme.accent` 계열).
- 전경: 흰 말풍선(대화) + accent 비콘 점 + 상승 파동 2겹(발견/브로드캐스트 = “beep”).
- 모서리 반경 232/1024(≈ macOS 스퀘어클 느낌) · 배경 밖은 투명.

## 재생성(SVG → PNG/ICO)

`icon.svg`를 고친 뒤 아래로 재추출한다(도구: `rsvg-convert`, `imagemagick`).

```bash
cd packaging/branding
rsvg-convert -w 1024 -h 1024 icon.svg -o nexa-beep-1024.png
rsvg-convert -w 256  -h 256  icon.svg -o nexa-beep-256.png
rsvg-convert -w 64   -h 64   icon.svg -o nexa-beep-64.png
magick nexa-beep-256.png nexa-beep-64.png nexa-beep.ico
```

## 공통 문구

| 항목 | 값 |
| --- | --- |
| App name | `Nexa Beep` |
| Publisher / 개발자 | `SosomLab` (Sangyong Bae · kiros33@gmail.com) |
| Repository | `git@github.com:SosomLab/nexa-beep.git` |
| 한 줄 소개 | 제로 컨피그 로컬 네트워크 메신저 — 실행하면 같은 LAN의 사용자가 자동으로 뜨고 즉시 대화 |

> 참고: 계열 원본은 nexa-dir2 `packaging/branding/`(같은 추출 흐름). 런타임 창 아이콘 연결(winit
> `set_window_icon`)은 PNG 디코더가 붙는 시점에 `IconImage::from_rgba` 경로로 후속 가능.
