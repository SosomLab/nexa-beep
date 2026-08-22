# beepd-cloud — 릴레이 서버 단발 클라우드 실측 킷

> **목적**: 공인 IP 뒤의 `nexa-beepd`로 **실 NAT 홀펀칭·릴레이 폴백을 실측**한다
> (X-UDP-c 잔여 실기 — 루프백/LAN에선 검증 불가능한 유일한 축).
> **상시 운영용이 아니다** — GCP 외부 IPv4가 유료(≈$3.6/월)라 세션 끝나면 지운다.
> 상시 무료가 필요해지면 Oracle Cloud Always Free(공인 IPv4 무료 · linux-arm64 자산 그대로)로.

## 왜 VM인가 (Cloud Run 불가)

Cloud Run은 HTTP(S)/WebSocket/gRPC 전용이라 **원시 TCP 프레이밍이 안 통과**하고
**UDP가 아예 없다**(관측·홀펀칭 사망). 또 다중 인스턴스가 메모리 상태(RID 명부·채널)를
가른다. 서버가 0.35MB·의존 0·저장 0이라 최소 VM이 정확히 맞는 그릇이다.

## 절차 (요약 — 상세는 스크립트 주석)

```powershell
# 0) 1회: gcloud 설치·인증
winget install Google.CloudSDK
gcloud init                                  # 로그인 + 프로젝트 선택

# 1) 저장소 루트에서 — musl 정적 교차 빌드→VM 생성→바이너리 업로드→상주→핀 출력까지 한 번에
.\tools\beepd-cloud\deploy-gcp.ps1           # 기본: 서울 e2-small(세션 몇 시간 = 수백 원 미만)
#   (교차 빌드가 안 되는 환경이면 -FromSource — 종전 VM 소스 빌드 폴백)

# 2) 실측 (docs/26 §3-7 그대로 · 주소만 공인 IP)
nexa-beep --chat-live 이름 --server <IP>:47300
nexa-beep --chat-connect-via <상대지문> --server <IP>:47300   # ★ 다른 망(폰 테더링)에서

# 3) 철거 — 과금 중단(서버는 저장 0이라 잃는 것 없음)
.\tools\beepd-cloud\deploy-gcp.ps1 -Teardown
```

## 실측 체크리스트 (기록은 journal에 — 추정 금지·실측 필수)

| # | 확인 | 기대 |
|---|---|---|
| 1 | 같은 NAT 안 2클라(집 PC 둘) | 헤어핀 지원 공유기 = UDP 직결 · 미지원 = **릴레이 폴백으로 대화 성립**(둘 다 정상) |
| 2 | **다른 NAT**(집 ↔ 폰 테더링) | 일반 공유기(Full/Restricted/Port-restricted) = "경로: UDP 직결(홀펀칭)" 기대 |
| 3 | Symmetric·CGNAT(통신사 테더링이 흔히 이 유형) | 펀치 실패 → **"릴레이 경유" 자동 폴백** · 대화·파일·수신확인 정상 |
| 4 | 서버 `journalctl -u beepd` | **봉투만**(conn#·rid 앞 4B·ch# — 이름·내용·키 없음) |
| 5 | 핀: 철거 후 재배포 → 재접속 | 새 서버 키 = **접속 중단 + 경로 안내**(시끄럽게 — DR-28) → `data/server.pin` 해당 줄 삭제 후 재핀 |
| 6 | 원격 경로 파일 게이트 | CLI 인터넷 경유 = 파일 양방향 차단 유지(메시지는 허용 — M5-3b 곱 판정) |

## 파일

- [`deploy-gcp.ps1`](deploy-gcp.ps1) — PC 쪽: musl 정적 교차 빌드·방화벽(tcp/udp 47300)·VM·업로드·철거(`-Teardown`)
- [`vm-setup.sh`](vm-setup.sh) — VM 쪽: 바이너리 설치·systemd 상주(`-FromSource` 폴백 = 스왑·rustup·소스 빌드)

> ★ **기본 = musl 정적 바이너리**(08-22 전환): beepd는 외부 의존 0(순수 Rust)이라
> Windows/mac 어디서든 `rustup target add x86_64-unknown-linux-musl` + `rust-lld`로
> C 툴체인 없이 교차 빌드된다(실측 651KB · static-pie). VM에는 빌드 도구가 아예
> 필요 없고 glibc 버전과도 무관하다. release-server.yml의 Linux 자산도 같은 musl
> 정적이라, 태그(`beepd-v*`) 이후에는 릴리스 자산을 받아 올려도 같은 물건이다.
> `-FromSource`는 교차 빌드가 막힌 환경의 폴백(종전 경로 · VM에서 rustup+빌드).
