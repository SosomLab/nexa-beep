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

# 1) 저장소 루트에서 — VM 생성→소스 업로드→빌드→상주→핀 출력까지 한 번에
.\tools\beepd-cloud\deploy-gcp.ps1           # 기본: 서울 e2-small(세션 몇 시간 = 수백 원 미만)

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

- [`deploy-gcp.ps1`](deploy-gcp.ps1) — PC 쪽: 방화벽(tcp/udp 47300)·VM·업로드·철거(`-Teardown`)
- [`vm-setup.sh`](vm-setup.sh) — VM 쪽: 스왑(1GB VM)·rustup 최소·`-p nexa-beepd` 빌드·systemd 상주

> 소스 빌드인 이유: 이 저장소의 리눅스 산출물은 CI(release-server.yml · `beepd-v*` 태그)가
> 굽는데, 검증 전 브랜치는 push 전이라 **VM에서 소스 빌드**가 가장 곧은 길이다(tarball =
> `git archive HEAD` — 커밋된 것만 · Cargo.lock 포함 재현 빌드). 병합·태그 후에는 릴리스
> 자산을 받아 올리는 쪽이 빠르다.
