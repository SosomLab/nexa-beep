# 41. beepd 설치·운영 가이드 — 릴레이 서버를 세우고 상주시키는 법

> **성격**: 운영 문서(실측 기반 — 2026-08-22 OCI 무료 티어 VM 첫 상시 배치에서 검증).
> 서버가 "무엇을 하는가/못 보는가"는 [32 ADR-0013](32-adr-0013-server-modes.md),
> 클라이언트 접속·검증 절차는 [26 §3-7](26-run-and-manual-test.md), 단발 실측 킷은
> [tools/beepd-cloud](../tools/beepd-cloud/README.md).
> ⚠️ **실서버의 주소·핀은 공개 문서에 적지 않는다**(스팸 표면) — 이 문서는 절차만.

## 0. 한 장 요약

```bash
# Linux x64 기준 — 릴리스 자산은 musl 정적이라 어느 배포판이든 이 네 줄이 전부다
curl -fsSLO https://github.com/SosomLab/nexa-beep/releases/download/beepd-v0.2.3/nexa-beepd-0.2.3-linux-x64.tar.gz
tar -xzf nexa-beepd-0.2.3-linux-x64.tar.gz
sudo install -m 0755 nexa-beepd-0.2.3-linux-x64/nexa-beepd /opt/beepd/nexa-beepd
sudo /opt/beepd/nexa-beepd --port 47300      # 첫 실행 = 키 생성 + "서버 신원(핀)" 출력
```

상주는 [§4 systemd](#4-시스템-등록-systemd-상주), 방화벽은 [§5](#5-방화벽--이중이다),
1GB급 무료 VM이면 [§7 저메모리 체크리스트](#7-저메모리-vm-체크리스트-1gb-무료-티어-실측)가 **필수**다.

## 1. 배포 형태 — 무엇을 받는가

- **릴리스 채널**: 클라이언트(`v*`)와 **완전 분리**된 `beepd-v*` 태그
  ([release-server.yml](../.github/workflows/release-server.yml) — W-4).
  → https://github.com/SosomLab/nexa-beep/releases (릴리스명 `nexa-beepd N.N.N`)
- **자산 4종 + `SHA256SUMS.txt`**:

| 자산 | 대상 | 비고 |
|---|---|---|
| `nexa-beepd-*-linux-x64.tar.gz` | Linux x86_64 | ★ **musl 정적**(static-pie · 실측 651KB) — glibc·배포판 무관, scratch 컨테이너 가능 |
| `nexa-beepd-*-linux-arm64.tar.gz` | Linux aarch64 | ★ musl 정적 — OCI Ampere A1·라즈베리파이류 |
| `nexa-beepd-*-windows-x64.zip` | Windows | 사내 PC를 릴레이로 쓰는 시나리오 |
| `nexa-beepd-*-macos-arm64.tar.gz` | macOS | 〃 |

- 아카이브 내용물 = `nexa-beepd`(단일 실행 파일) + `README.md` + `LICENSE.md`.
  **외부 의존 0** — 설치할 런타임·라이브러리가 없다(DR-5의 서버판).
- 무결성 검증: `sha256sum -c` 로 `SHA256SUMS.txt`와 대조(항목만 골라 확인해도 된다).

## 2. 설치 경로 규약

| 경로 | 내용 | 이유 |
|---|---|---|
| `/opt/beepd/nexa-beepd` | 실행 파일 | 단일 파일 앱의 관례 위치(패키지 매니저 밖) |
| `/opt/beepd/beepd.key` | **서버 신원 키**(첫 실행 시 자동 생성 · 68B `NBK1`) | 키 = 서버의 정체([§6](#6-서버-신원-키--핀의-원천)) — 실행 파일과 같은 홈에 |
| `/opt/beepd` (홈) | 상주 계정 `beepd`의 홈 = WorkingDirectory | 저장할 데이터가 **없으므로**(파이프 · S-3) 이게 전부다 |

```bash
sudo useradd -r -d /opt/beepd -s /sbin/nologin beepd   # 비루트 상주 계정(로그인 불가)
sudo mkdir -p /opt/beepd
sudo install -m 0755 nexa-beepd "/opt/beepd/nexa-beepd"
sudo chown -R beepd:beepd /opt/beepd
```

- **비루트가 기본**이다 — 47300은 비특권 포트라 root가 필요 없고, 서버는 파일을
  읽고 쓸 일이 키 하나뿐이다(T0 원칙의 서버판).
- 업데이트 = 새 자산으로 실행 파일만 교체(`install` 덮어쓰기) 후 재시작.
  **`beepd.key`는 절대 지우지 않는다**([§6]).

## 3. 실행 — 옵션과 첫 기동

```bash
/opt/beepd/nexa-beepd --port 47300 --key /opt/beepd/beepd.key --verbose
```

| 옵션 | 기본 | 뜻 |
|---|---|---|
| `--port <N>` | 47300 | **TCP 제어/중계 + UDP 관측이 같은 번호**를 쓴다(클라 규약) |
| `--bind <IP>` | 0.0.0.0 | 바인드 주소 |
| `--key <경로>` | `beepd.key`(작업 디렉터리) | 서버 신원 키 — 없으면 생성 |
| `--rate <값>` | `1m` | **연결당** 릴레이 예산(B/s) — `auto`/`100k`/`1m`/…/`0`(무제한) |
| `--verbose` | off | 봉투 수준 로그(conn#·RID 앞 4B·ch#·바이트 — 이름·내용·키 없음) |

첫 기동 출력에서 **`서버 신원(핀)` 64hex를 반드시 적어 둔다** — 클라이언트가 첫
접속에서 TOFU로 핀하는 값이고, 관리자가 배포하는 "우리 서버의 정체"다.

```
nexa-beepd v0.2.3 가동
  TCP 제어/중계 : 0.0.0.0:47300
  UDP 관측      : 47300
  서버 신원(핀) : 5c5e…(64hex — 이 값을 사용자에게 배포)
  릴레이 예산   : 1048576 B/s/연결
```

- 미지 인자 = 안내 후 종료(조용한 오동작 금지 — 클라이언트와 같은 규약).
- 서버는 **아무것도 저장하지 않는다** — 재시작해도 잃는 것은 진행 중이던 채널뿐이고,
  클라이언트가 백오프로 재접속한다.

## 4. 시스템 등록 — systemd 상주

`/etc/systemd/system/beepd.service` (08-22 OCI 실전 유닛 그대로):

```ini
[Unit]
Description=nexa-beepd relay server
After=network-online.target
Wants=network-online.target

[Service]
User=beepd
WorkingDirectory=/opt/beepd
ExecStart=/opt/beepd/nexa-beepd --port 47300 --key /opt/beepd/beepd.key --verbose
Restart=on-failure
MemoryMax=128M

[Install]
WantedBy=multi-user.target
```

```bash
sudo systemctl daemon-reload
sudo systemctl enable --now beepd        # 등록 + 즉시 기동(재부팅 자동 복귀)
systemctl is-active beepd                # active 확인
sudo journalctl -u beepd -f              # 로그(핀 값은 여기 첫 블록에)
```

- **`MemoryMax=128M`** — 서버 실측 상주가 수 MB라 여유가 크지만, 저메모리 VM에서
  서버가 폭주해도 **시스템 전체를 굶기지 못하게** 상한을 박는다([§7]의 교훈).
- `Restart=on-failure` + "저장 0" 조합이라 재시작이 언제나 안전하다.
- **Windows**: 서비스 등록 없이 작업 스케줄러(로그온 시 실행)나 수동 실행으로 충분
  (사내 PC 릴레이 시나리오). **macOS**: launchd plist(`KeepAlive`) — 둘 다 옵션·기본
  경로는 수동 실행이다(서버의 주 무대는 Linux — [32 §12-6]).

## 5. 방화벽 — **이중**이다

같은 포트 번호로 **TCP와 UDP 둘 다**, 그리고 **두 겹 모두** 열어야 한다.
UDP를 빼먹으면 관측·홀펀칭이 조용히 죽고 릴레이 폴백만 돈다(성능 저하로만 보여 원인을 놓친다).

| 겹 | 어디서 | 예 |
|---|---|---|
| ① 클라우드 방화벽 | OCI 보안 목록 / GCP 방화벽 규칙 / AWS SG | TCP 47300 + **UDP 47300** ingress |
| ② OS 방화벽 | Oracle Linux/RHEL = firewalld · Ubuntu = ufw(기본 무장 해제) | 아래 명령 |

```bash
# Oracle Linux / RHEL 계열 (firewalld) — 08-22 OCI 실측
sudo firewall-cmd --permanent --add-port=47300/tcp --add-port=47300/udp
sudo firewall-cmd --reload && sudo firewall-cmd --list-ports

# Ubuntu (ufw를 쓰는 경우)
sudo ufw allow 47300/tcp && sudo ufw allow 47300/udp
```

> ⚠️ **클라우드 보안 목록만 열고 끝났다고 믿지 말 것** — Oracle Linux 이미지는
> firewalld가 기본 가동이라 ①만 열면 패킷이 OS에서 죽는다(08-22 실측 경로).

## 6. 서버 신원 키 — 핀의 원천

- `beepd.key` = 서버의 **정체 그 자체**. 클라이언트는 첫 접속에서 이 키를 핀(TOFU)
  하고, 이후 **키가 다르면 접속을 중단하고 시끄럽게 경고**한다(DR-28 — 조용한 재핀 없음).
- 따라서:
  - **백업 필수** — VM을 지우고 다시 만들 계획이라면 `beepd.key`를 먼저 사본해 두고,
    새 VM의 `/opt/beepd/beepd.key`로 복원한다. 그래야 사용자들의 핀이 그대로 산다.
  - 키를 **잃으면** = 새 키 생성 = 전 사용자에게 핀 불일치 경고 → 각자
    `data/server.pin`에서 옛 줄을 지우고 재핀해야 한다(사람의 결정 — 자동화 없음).
  - 키가 **유출되면** = 서버 사칭 가능(단, 대화 내용은 종단 E2E라 여전히 못 본다 —
    S-3) → 의도적으로 키를 교체하고 사용자들에게 새 핀을 공지한다.
- 파일 권한: `chown beepd:beepd` + 기본 0600(서버가 생성 시). 백업본도 600으로.

## 7. 저메모리 VM 체크리스트 (1GB 무료 티어 — 실측)

beepd 자체는 수 MB지만, **1GB급 무료 VM은 기본 이미지 상태가 이미 OOM 경계선**이다.
08-22 OCI E2.1.Micro(Oracle Linux)에서 SSH 행까지 간 실사례에서 나온 처방:

| # | 조치 | 근거(실측) |
|---|---|---|
| 1 | `sudo systemctl mask dnf-makecache.timer` | dnf 메타캐시가 **회당 ~350MB** — 반복 OOM의 주범(콘솔 히스토리에 dnf 사살 기록 다수) |
| 2 | 무거운 클라우드 에이전트 비활성 — OCI는 **인스턴스 API로**(Cloud Guard WLP·Vulnerability Scanning·OS Mgmt Hub → DISABLED · Run Command/Monitoring은 유지) | oci-wlp 등이 수십 MB 상주 + 주기 스캔 스파이크. API 방식은 SSH가 죽어도 적용된다 |
| 3 | **kdump crashkernel 제거** — `grubby --update-kernel=ALL --remove-args=crashkernel` + `systemctl disable --now kdump` + 재부팅 | ★ crashkernel이 **1GB의 절반(512MB)을 예약** — RAM 총량 498MB로 관측 → 제거 후 945MB 복구 |
| 4 | 스왑 확보(+2G) — `fallocate -l 2G /swapfile2 && chmod 600 … && mkswap … && swapon …` + fstab | 기본 498MB 스왑은 스파이크 한 번에 소진됐다 |
| 5 | 유닛에 `MemoryMax=128M` | 서버가 원인이 되는 역방향도 차단([§4]) |

- 진단 요령: SSH가 행이면 밖에서 **클라우드 콘솔 히스토리**를 캡처해 커널 OOM
  로그를 읽는다(OCI: `oci compute console-history capture` → `get-content`).
  행 상태의 SOFTRESET(ACPI)은 무시될 수 있다 — 유예 후 강제되며, 급하면 RESET.
- 더 나은 그릇: OCI **Ampere A1**(4 OCPU/24GB · Always Free)이면 이 절 전체가
  불필요하다 — `linux-arm64` musl 자산이 그대로 얹힌다.

## 8. 클라이언트 연결 — 사용자에게 배포할 안내

관리자가 배포할 값은 **주소:포트**와 (대조용) **서버 핀 64hex** 두 가지다.

- **GUI**(v0.2.3+): 설정 › 서버 → 모드 `managed` · 주소 · 포트 → 저장하면 2초 안에
  자동 접속. 상태바 "서버 키를 핀했습니다"의 값이 배포된 핀과 같은지 확인.
- **CLI**: `nexa-beep --chat-live 이름 --server <주소>:47300` (수신 대기 + 내 지문 출력) ·
  `nexa-beep --chat-connect-via <상대지문64> --server <주소>:47300` (발신).
- 상대 지정은 **키 지문**으로 한다(서버에 명부가 없다 — 회전 RID·봉투 원리):
  내 지문 = `nexa-beep --whoami`의 `full =` 줄. GUI는 `⌘/Ctrl+K` 모달에 지문 입력.
- 인터넷 경유 채널은 **지문 대조(`/verify`) 전 파일 차단**이 정상이다(FR-S-24).
- 검증 시나리오·체크리스트: [26 §3-7](26-run-and-manual-test.md) ·
  [tools/beepd-cloud README](../tools/beepd-cloud/README.md)의 6항.

## 9. 운영 잡무

```bash
systemctl is-active beepd && sudo journalctl -u beepd --since -1h   # 상태·최근 로그
sudo systemctl restart beepd                                        # 재시작(저장 0 — 안전)
# 업데이트: 새 자산 받기 → 검증 → 교체 → 재시작 (키는 그대로)
sudo install -m 0755 nexa-beepd /opt/beepd/nexa-beepd && sudo systemctl restart beepd
# 철거: systemctl disable --now beepd → /opt/beepd 삭제 (beepd.key 백업 먼저!)
```

- 로그는 **봉투만** 담는다(`--verbose`여도 conn#·RID 앞 4B·ch#·바이트 수) —
  로그 유출이 대화 유출이 되지 않는다(S-3). 그래도 로그 보존 정책은 짧게.
- `--rate`는 연결당 상한이다 — 공개망에 열어 두는 서버라면 기본 1 MiB/s 유지 권장
  (무단 사용자가 있어도 파이프 비용이 상한된다). 신뢰 사용자만이라면 `0`도 무방.
