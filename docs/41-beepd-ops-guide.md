# 41. beepd 설치·운영 가이드 — 릴레이 서버를 세우고 상주시키는 법

> **성격**: 운영 문서(실측 기반 — 2026-08-22 OCI 무료 티어 VM 첫 상시 배치에서 검증).
> **범위**: **Linux · Windows · macOS 3-OS**. 주 무대는 Linux(클라우드 상주)지만 사내 PC를
> 릴레이로 쓰는 경우가 있어 OS별 경로·상주 등록·방화벽·키 권한을 각각 적는다.
> Linux는 OCI 실배치 실측, macOS는 이 저장소 맥에서 기동 실측(08-22), Windows는 절차 기준.
> 서버가 "무엇을 하는가/못 보는가"는 [32 ADR-0013](32-adr-0013-server-modes.md),
> 클라이언트 접속·검증 절차는 [26 §3-7](26-run-and-manual-test.md), 단발 실측 킷은
> [tools/beepd-cloud](../tools/beepd-cloud/README.md).
> ⚠️ **사설 서버의 주소·핀은 공개 문서에 적지 않는다**(스팸 표면) — 이 문서는 절차만.
> 예외 = **공식 기본 서버**([§8-1](#8-1-공식-기본-서버) — 앱 기본값에 실리는 순간 공개
> 정보이고, 핀 공개는 밴드 밖 검증 값 배포가 목적이다).

## 0. 한 장 요약

**Linux 상주가 기본**이고(클라우드), Windows·macOS는 사내 PC를 릴레이로 쓸 때다 —
경로는 [§2-1](#2-1-windows--macos-경로), 상주 등록은 [§4-2 Windows](#4-2-windows--작업-스케줄러)·
[§4-3 macOS](#4-3-macos--launchd-상주), 한눈 대응표는 [§9-1](#9-1-os별-대응표--같은-일을-어디서-하나).

```bash
# Linux x64 기준 — 릴리스 자산은 musl 정적이라 어느 배포판이든 이 네 줄이 전부다
curl -fsSLO https://github.com/SosomLab/nexa-beep/releases/download/beepd-v0.2.3/nexa-beepd-0.2.3-linux-x64.tar.gz
tar -xzf nexa-beepd-0.2.3-linux-x64.tar.gz
sudo install -m 0755 nexa-beepd-0.2.3-linux-x64/nexa-beepd /opt/beepd/nexa-beepd
sudo /opt/beepd/nexa-beepd --port 47300      # 첫 실행 = 키 생성 + "서버 신원(핀)" 출력
```

상주는 [§4-1 systemd](#4-1-linux--systemd), 방화벽은 [§5](#5-방화벽--이중이다),
1GB급 무료 VM이면 [§7 저메모리 체크리스트](#7-저메모리-vm-체크리스트-1gb-무료-티어--실측)가 **필수**다.

## 1. 배포 형태 — 무엇을 받는가

- **릴리스 채널**: 클라이언트(`v*`)와 **완전 분리**된 `beepd-v*` 태그
  ([release-server.yml](../.github/workflows/release-server.yml) — W-4).
  → https://github.com/SosomLab/nexa-beep/releases (릴리스명 `nexa-beepd N.N.N`)
- **패키지 매니저**(beepd-v0.2.5부터 — [publish-beepd-packages.yml](../.github/workflows/publish-beepd-packages.yml) ·
  클라 채널과 스위치까지 분리):

  ```bash
  brew install kiros33/tap/nexa-beepd     # macOS(Apple Silicon)·Linux — 탭이라 즉시
  winget install SosomLab.NexaBeepd       # Windows — 중앙 검수 통과 후
  choco  install nexa-beepd               # Windows — 중앙 검수 통과 후
  ```

  brew는 내 탭이라 릴리스와 함께 자동 갱신되고, winget/choco는 첫 제출 검수를
  거쳐야 노출된다. 어느 채널이든 실체는 같은 릴리스 자산(sha256 자동 대조).
- **자산 4종 + `SHA256SUMS.txt`**:

| 자산 | 대상 | 비고 |
|---|---|---|
| `nexa-beepd-*-linux-x64.tar.gz` | Linux x86_64 | ★ **musl 정적**(static-pie · 실측 651KB) — glibc·배포판 무관, scratch 컨테이너 가능 |
| `nexa-beepd-*-linux-arm64.tar.gz` | Linux aarch64 | ★ musl 정적 — OCI Ampere A1·라즈베리파이류 |
| `nexa-beepd-*-windows-x64.zip` | Windows x64 | **콘솔 앱**(클라이언트와 달리 `windows_subsystem` 미지정 — 창이 아니라 콘솔에 뜬다) · 별도 런타임 설치 불요 |
| `nexa-beepd-*-macos-arm64.tar.gz` | macOS **Apple Silicon 전용** | ⚠️ **Intel Mac 자산은 없다** — arm64 바이너리는 Intel에서 실행되지 않는다(Rosetta는 반대 방향 변환). Intel은 [§4-3](#4-3-macos--launchd-상주)의 소스 빌드 |

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

### 2-1. Windows · macOS 경로

| OS | 실행 파일 | 키 | 왜 이 자리인가 |
|---|---|---|---|
| **Windows** | `C:\Program Files\beepd\nexa-beepd.exe` | `C:\ProgramData\beepd\beepd.key` | Program Files는 실행 계정이 쓸 수 없다 — **키는 반드시 쓰기 가능한 머신 공용 경로**로 뺀다(`--key`로 지정). 안 나누면 첫 기동이 키 생성에 실패한다 |
| **macOS** | `/usr/local/beepd/nexa-beepd` | `/usr/local/beepd/beepd.key` | `/usr/local`은 관리자 소유라 SIP 보호 밖이다 → Linux와 같은 "실행 파일 옆" 규약을 그대로 쓴다 |

```powershell
# Windows (관리자 PowerShell)
New-Item -ItemType Directory -Force "C:\Program Files\beepd", "C:\ProgramData\beepd" | Out-Null
Expand-Archive .\nexa-beepd-0.2.3-windows-x64.zip -DestinationPath $env:TEMP\beepd -Force
Copy-Item $env:TEMP\beepd\*\nexa-beepd.exe "C:\Program Files\beepd\"
```

```bash
# macOS (Apple Silicon — 릴리스 자산)
sudo mkdir -p /usr/local/beepd
tar -xzf nexa-beepd-0.2.3-macos-arm64.tar.gz
sudo install -m 0755 nexa-beepd-0.2.3-macos-arm64/nexa-beepd /usr/local/beepd/nexa-beepd
# ⚠️ 브라우저로 받았다면 격리 표식을 뗀다(무서명 배포 — curl로 받으면 붙지 않는다)
sudo xattr -dr com.apple.quarantine /usr/local/beepd/nexa-beepd
```

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

### 3-1. OS별 첫 기동

```powershell
# Windows (관리자 PowerShell) — 콘솔에 그대로 뜬다. Ctrl+C로 중단
& "C:\Program Files\beepd\nexa-beepd.exe" --port 47300 --key "C:\ProgramData\beepd\beepd.key" --verbose
```

```bash
# macOS — 실측 08-22(이 저장소 맥에서 기동 확인)
sudo /usr/local/beepd/nexa-beepd --port 47300 --key /usr/local/beepd/beepd.key --verbose
```

**macOS 실측(08-22 · 네이티브 빌드)** — 기동 즉시 `TCP *:47300 (LISTEN)` + `UDP *:47300`
둘 다 잡히고, 키가 없으면 생성 후 핀을 출력한다. **상주 RSS 0.9MB**(`ps -o rss=`) ·
바이너리 508KB. 서버가 가볍다는 주장은 Linux만의 이야기가 아니다.

> ⚠️ **`--bind` 기본값 `0.0.0.0`은 IPv4만 듣는다**(3-OS 공통 · 실측에서 `lsof`가 IPv4
> 소켓만 보여 준다). IPv6로도 받으려면 **`--bind ::`** 로 띄운다 — IPv6 전용 VM
> (예: 외부 IPv4가 없는 GCP 인스턴스)에서는 이걸 빠뜨리면 아무도 못 붙는다.

## 4. 시스템 등록 — OS별 상주

서버의 주 무대는 Linux지만([32 §12-6]), 사내 PC를 릴레이로 쓰는 시나리오가 있어 3-OS 모두 적는다.

### 4-1. Linux — systemd

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
### 4-2. Windows — 작업 스케줄러

⚠️ **`sc.exe create`로 직접 서비스 등록하면 안 된다.** `nexa-beepd`는 **평범한 콘솔
앱**이라 서비스 제어 관리자(SCM)에 응답하지 않는다 — 등록해도 시작 시
**오류 1053**(서비스가 제때 응답하지 않았습니다)로 죽는다. 서비스로 만들려면 별도
래퍼(NSSM·WinSW 등 외부 도구)가 필요하고, 그건 이 문서의 범위 밖이다.

**추가 설치물 없이 되는 방법 = 작업 스케줄러**(시스템 시작 시 · 로그온 불요):

```powershell
# 관리자 PowerShell — 부팅 시 자동 시작 + 창 없이 상주
$exe = "C:\Program Files\beepd\nexa-beepd.exe"
$arg = '--port 47300 --key "C:\ProgramData\beepd\beepd.key" --verbose'
$act = New-ScheduledTaskAction -Execute $exe -Argument $arg
$trg = New-ScheduledTaskTrigger -AtStartup
$prn = New-ScheduledTaskPrincipal -UserId "SYSTEM" -LogonType ServiceAccount -RunLevel Highest
$set = New-ScheduledTaskSettingsSet -AllowStartIfOnBatteries -DontStopIfGoingOnBatteries `
       -RestartCount 3 -RestartInterval (New-TimeSpan -Minutes 1) -ExecutionTimeLimit 0
Register-ScheduledTask -TaskName beepd -Action $act -Trigger $trg -Principal $prn -Settings $set

Start-ScheduledTask -TaskName beepd
Get-ScheduledTask -TaskName beepd | Get-ScheduledTaskInfo    # LastTaskResult 0 = 정상
```

- `-UserId "SYSTEM"`이면 **로그온하지 않아도** 뜨고 콘솔 창이 보이지 않는다.
  일반 계정으로 돌리려면 47300이 비특권 포트라 권한 승격이 필요 없다(Linux의 비루트 원칙과 같다).
- ★ **첫 핀 값을 봐야 한다** — SYSTEM으로 돌리면 콘솔 출력이 어디에도 남지 않는다.
  **최초 1회는 수동으로 포그라운드 실행**([§3-1](#3-1-os별-첫-기동))해 핀을 받아 적고,
  그다음에 작업을 등록한다. 또는 `--verbose` 출력을 파일로 돌린다
  (`cmd /c ""C:\Program Files\beepd\nexa-beepd.exe" … >> C:\ProgramData\beepd\beepd.log 2>&1"`).
- 제거 = `Unregister-ScheduledTask -TaskName beepd -Confirm:$false`.

### 4-3. macOS — launchd 상주

`/Library/LaunchDaemons/com.sosomlab.beepd.plist`(**LaunchDaemon** = 부팅 시 · 로그온 불요.
LaunchAgent는 로그인 세션에 묶이므로 서버에는 쓰지 않는다):

```xml
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0"><dict>
  <key>Label</key>            <string>com.sosomlab.beepd</string>
  <key>ProgramArguments</key> <array>
    <string>/usr/local/beepd/nexa-beepd</string>
    <string>--port</string><string>47300</string>
    <string>--key</string> <string>/usr/local/beepd/beepd.key</string>
    <string>--verbose</string>
  </array>
  <key>RunAtLoad</key>        <true/>
  <key>KeepAlive</key>        <true/>
  <key>WorkingDirectory</key> <string>/usr/local/beepd</string>
  <key>StandardOutPath</key>  <string>/usr/local/beepd/beepd.log</string>
  <key>StandardErrorPath</key><string>/usr/local/beepd/beepd.log</string>
</dict></plist>
```

```bash
sudo chown root:wheel /Library/LaunchDaemons/com.sosomlab.beepd.plist
sudo chmod 0644      /Library/LaunchDaemons/com.sosomlab.beepd.plist
sudo launchctl bootstrap system /Library/LaunchDaemons/com.sosomlab.beepd.plist   # 등록+기동
sudo launchctl print system/com.sosomlab.beepd | head -20                          # 상태
sudo launchctl bootout  system/com.sosomlab.beepd                                  # 해제
```

- **핀 값은 `beepd.log` 첫 블록**에 있다(`StandardOutPath` — systemd의 journalctl 자리).
- `KeepAlive`가 systemd의 `Restart=on-failure`에 해당한다. 서버는 저장이 없어 재시작이 언제나 안전하다.
- **Intel Mac**은 릴리스 자산이 없다 → 소스에서 빌드한다(외부 의존 0이라 툴체인만 있으면 된다):
  ```bash
  cargo build --release -p nexa-beepd   # → target/release/nexa-beepd (실측 508KB · macOS x86_64)
  ```
- `launchctl load -w`는 구식 문법이다(동작은 하지만 `bootstrap`/`bootout`을 쓴다).
- 무서명 배포라 **브라우저로 받은 파일은 격리 표식** 때문에 실행이 막힌다 →
  [§2-1](#2-1-windows--macos-경로)의 `xattr -dr com.apple.quarantine`.

## 5. 방화벽 — **이중**이다

같은 포트 번호로 **TCP와 UDP 둘 다**, 그리고 **두 겹 모두** 열어야 한다.
UDP를 빼먹으면 관측·홀펀칭이 조용히 죽고 릴레이 폴백만 돈다(성능 저하로만 보여 원인을 놓친다).

| 겹 | 어디서 | 예 |
|---|---|---|
| ① 클라우드 방화벽 | OCI 보안 목록 / GCP 방화벽 규칙 / AWS SG | TCP 47300 + **UDP 47300** ingress |
| ② OS 방화벽 | Linux: RHEL 계열 = firewalld · Ubuntu = ufw(기본 무장 해제) · **Windows: Defender 방화벽**(기본 차단) · **macOS: 응용 프로그램 방화벽**(기본 off인 경우가 많다) | 아래 명령 |

```bash
# Oracle Linux / RHEL 계열 (firewalld) — 08-22 OCI 실측
sudo firewall-cmd --permanent --add-port=47300/tcp --add-port=47300/udp
sudo firewall-cmd --reload && sudo firewall-cmd --list-ports

# Ubuntu (ufw를 쓰는 경우)
sudo ufw allow 47300/tcp && sudo ufw allow 47300/udp
```

```powershell
# Windows (관리자 PowerShell) — TCP·UDP 두 줄 다 필요하다
New-NetFirewallRule -DisplayName "beepd 47300 TCP" -Direction Inbound -Protocol TCP -LocalPort 47300 -Action Allow
New-NetFirewallRule -DisplayName "beepd 47300 UDP" -Direction Inbound -Protocol UDP -LocalPort 47300 -Action Allow
Get-NetFirewallRule -DisplayName "beepd*" | Format-Table DisplayName, Enabled, Direction
```

```bash
# macOS — 응용 프로그램 방화벽은 포트가 아니라 **앱 단위**다
sudo /usr/libexec/ApplicationFirewall/socketfilterfw --getglobalstate      # 꺼져 있으면 할 일 없음
sudo /usr/libexec/ApplicationFirewall/socketfilterfw --add /usr/local/beepd/nexa-beepd
sudo /usr/libexec/ApplicationFirewall/socketfilterfw --unblockapp /usr/local/beepd/nexa-beepd
```

> **macOS 주의** — 켜져 있으면 무서명 바이너리가 첫 인바운드에서 **"수신 연결을 허용하시겠습니까"
> 대화상자**를 띄운다. 헤드리스·원격 관리 중이면 그 창을 아무도 못 눌러 조용히 막힌 것처럼 보인다.
> 위 `--add`/`--unblockapp`를 **미리** 넣어 둔다. 포트 단위로 통제하려면 ALF가 아니라 `pf`를 쓴다.

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
- 파일 권한(OS별):

| OS | 키 경로 | 권한 조치 |
|---|---|---|
| Linux | `/opt/beepd/beepd.key` | `chown beepd:beepd` + 기본 0600(서버가 생성 시). 백업본도 600 |
| Windows | `C:\ProgramData\beepd\beepd.key` | 상속 끊고 실행 계정만 남긴다 — `icacls "C:\ProgramData\beepd\beepd.key" /inheritance:r /grant:r "SYSTEM:(R,W)" "Administrators:(R,W)"` (기본 ProgramData ACL은 **인증된 사용자 전원 읽기**라 그대로 두면 안 된다) |
| macOS | `/usr/local/beepd/beepd.key` | `sudo chown root:wheel … && sudo chmod 600 …`(LaunchDaemon이 root로 돌 때) |

## 7. 저메모리 VM 체크리스트 (1GB 무료 티어 — 실측)

> **이 절은 Linux 클라우드 VM 한정이다** — Windows·macOS 상주(사내 PC)에는 해당 없다.

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

> **이 값들이 어떻게 쓰여 두 사람이 만나는지**(회전 RID·랑데부·홀펀칭·릴레이 폴백·종단 Noise)는
> **[42 릴레이 랑데부 종단 설명서](42-relay-rendezvous-walkthrough.md)** — 장애 원인을 가릴 때
> [42 §11 실패 모드](42-relay-rendezvous-walkthrough.md#11-실패-모드와-진단)가 이 문서의 방화벽·키 절과 짝이다.

### 8-1. 공식 기본 서버

앱의 서버 주소 기본값(설정 › 서버 — 08-22 등록 · v0.2.4+):

| 항목 | 값 |
|---|---|
| 주소 | **`beepd.sosomlab.com:47300`** (TCP+UDP) |
| 서버 신원(핀) | `5c5ee9321439f0292f90f6d7b949e5be3e8dc94cfc6a142208e957bf83c0cb5e` |

- 첫 접속 때 상태바/CLI에 뜨는 핀이 **위 값과 다르면 접속하지 말 것**(사칭·중간자).
- 모드를 `managed`로 켠 뒤 **[Test]를 한 번 눌러** 접속을 승인한다(주소·포트 기본값이
  이미 이 값 — ★08-22 개정: 전환·값 변경은 **자동 접속하지 않는다**. Test 성공만이
  접속을 열고, 이후 부팅·재접속은 자동 유지된다. 실패 = 경고 노트+서버 미사용).
- 실측(08-22): Test 1회로 DNS 해석→접속→핀 일치→검증 마커 영속까지 완료.
  운영 실체는 OCI 무료 티어 VM([§7]) — 연결당 릴레이 예산 1 MiB/s.

- **GUI**(v0.2.3+): 설정 › 서버 → 모드 `managed` · 주소 · 포트 → **[Test]**(v0.2.6+ —
  그 전 판은 저장만으로 자동 접속). 상태바 "서버 키를 핀했습니다"의 값이 배포된
  핀과 같은지 확인. 접속 중에는 툴바 프로필 왼쪽에 waypoints 표시가 뜬다.
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
# ★ 교체 전 해시 대조(08-22 절차) — 같으면 "이미 최신"을 1초에 판정한다
sha256sum nexa-beepd /opt/beepd/nexa-beepd
sudo install -m 0755 nexa-beepd /opt/beepd/nexa-beepd && sudo systemctl restart beepd
# 철거: systemctl disable --now beepd → /opt/beepd 삭제 (beepd.key 백업 먼저!)
```

### 9-1. OS별 대응표 — 같은 일을 어디서 하나

| 하는 일 | Linux(systemd) | Windows(작업 스케줄러) | macOS(launchd) |
|---|---|---|---|
| 상태 | `systemctl is-active beepd` | `Get-ScheduledTaskInfo -TaskName beepd` | `sudo launchctl print system/com.sosomlab.beepd` |
| 시작·중지 | `systemctl start/stop beepd` | `Start-ScheduledTask` / `Stop-ScheduledTask -TaskName beepd` | `sudo launchctl kickstart -k system/com.sosomlab.beepd` / `bootout` |
| 로그(핀 포함) | `journalctl -u beepd` | 리다이렉트한 `C:\ProgramData\beepd\beepd.log` | `/usr/local/beepd/beepd.log`(`StandardOutPath`) |
| 자동 시작 등록/해제 | `systemctl enable/disable --now beepd` | `Register-` / `Unregister-ScheduledTask` | `launchctl bootstrap` / `bootout system` |
| 업데이트 | 파일 교체 → `systemctl restart` | **작업 중지 → exe 교체 → 시작**(실행 중이면 파일 잠김) | 파일 교체 → `launchctl kickstart -k` |
| 메모리 상한 | 유닛 `MemoryMax=128M` | (작업 스케줄러엔 없음 — 필요하면 작업 개체/WSRM) | (launchd엔 직접 대응 없음 — `ulimit`/래퍼) |

> ★ **Windows 업데이트 시 파일 잠김** — 실행 중인 exe는 덮어쓸 수 없다. `Stop-ScheduledTask`
> 먼저다(Linux/macOS는 실행 중 교체가 되지만, 재시작 전까지 옛 코드가 돈다는 점은 같다).

- 로그는 **봉투만** 담는다(`--verbose`여도 conn#·RID 앞 4B·ch#·바이트 수) —
  로그 유출이 대화 유출이 되지 않는다(S-3). 그래도 로그 보존 정책은 짧게.
- **Windows·macOS 상주는 로그 로테이션이 없다**(systemd journal과 달리 파일에 무한 append) —
  `--verbose`로 오래 돌릴 거면 주기적으로 잘라 낸다.
- `--rate`는 연결당 상한이다 — 공개망에 열어 두는 서버라면 기본 1 MiB/s 유지 권장
  (무단 사용자가 있어도 파이프 비용이 상한된다). 신뢰 사용자만이라면 `0`도 무방.
