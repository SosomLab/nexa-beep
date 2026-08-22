# nexa-beepd 단발 테스트 배포 — GCP Compute Engine VM 생성→빌드→상주→핀 출력까지 한 번에.
# (Cloud Run은 부적합: 원시 TCP·UDP 미지원 + 다중 인스턴스가 메모리 상태(RID 명부)를 가른다)
#
# 전제: gcloud CLI 설치·인증(`winget install Google.CloudSDK` → `gcloud init`) · 저장소 루트에서 실행.
# 비용: e2-small ≈ $0.02/h + 외부 IPv4 ≈ $0.005/h — 실측 세션 몇 시간 = 수백 원 미만.
#       ★ 상시 유지 목적이 아니다 — 끝나면 -Teardown 으로 지운다(서버는 저장 0이라 잃는 것 없음).
#
# 사용:
#   .\tools\beepd-cloud\deploy-gcp.ps1                 # 생성 + 배포 + 기동
#   .\tools\beepd-cloud\deploy-gcp.ps1 -Teardown       # VM·방화벽 삭제(과금 중단)
param(
    [string]$Project = '',
    [string]$Zone = 'asia-northeast3-a',   # 서울 — 지연 실측이 현실적. 프리 티어 VM을 원하면 us-west1-b
    [string]$MachineType = 'e2-small',     # 기본 경로(정적 바이너리)는 e2-micro도 충분 — 소스 폴백 대비 여유
    [string]$Name = 'beepd-test',
    [switch]$Teardown,
    [switch]$FromSource                    # 로컬 musl 교차 빌드 대신 종전 VM 소스 빌드 경로
)
$ErrorActionPreference = 'Stop'

if (-not (Get-Command gcloud -ErrorAction SilentlyContinue)) {
    Write-Host '[실패] gcloud CLI가 없다 — winget install Google.CloudSDK 후 gcloud init'
    exit 1
}
if (-not $Project) { $Project = (gcloud config get-value project 2>$null) }
if (-not $Project) {
    Write-Host '[실패] GCP 프로젝트 미지정 — -Project <id> 또는 gcloud config set project <id>'
    exit 1
}
$common = @('--project', $Project, '--zone', $Zone)

if ($Teardown) {
    Write-Host "[철거] VM·방화벽 삭제 — 과금 중단($Name @ $Zone)"
    gcloud compute instances delete $Name @common --quiet
    gcloud compute firewall-rules delete beepd-test-47300 --project $Project --quiet
    Write-Host '[철거] 완료 — 로컬 data/server.pin의 이 서버 항목은 재배포 시 키가 바뀌므로 지워 둘 것'
    exit 0
}

# ① 산출물 준비 — ★기본 = **musl 정적 바이너리 로컬 교차 빌드**(08-22 전환): beepd는
#    외부 의존 0(순수 Rust)이라 Windows/mac 어디서든 rust-lld로 링크된다(C 툴체인 불요).
#    VM에는 651KB 바이너리 하나만 올라가고 빌드 도구가 아예 필요 없다.
#    -FromSource = 종전 소스 tarball 경로(교차 빌드가 안 되는 환경 폴백).
$binPath = 'target/x86_64-unknown-linux-musl/release/nexa-beepd'
if (-not $FromSource) {
    Write-Host "[1/5] musl 정적 교차 빌드 (HEAD: $(git rev-parse --short HEAD))"
    rustup target add x86_64-unknown-linux-musl 2>$null | Out-Null
    # 링커(rust-lld)는 저장소 .cargo/config.toml이 정한다 — 여기서 중복 지정하지 않는다.
    cargo build --release -p nexa-beepd --target x86_64-unknown-linux-musl
    if ($LASTEXITCODE -ne 0) {
        Write-Host '[실패] musl 교차 빌드 — -FromSource 로 VM 소스 빌드 경로를 쓸 수 있다'
        exit 1
    }
} else {
    # 소스 스냅샷 — 현재 브랜치 HEAD(커밋된 것만 · Cargo.lock 포함 = 재현 빌드)
    Write-Host "[1/5] 소스 tarball (HEAD: $(git rev-parse --short HEAD))"
    git archive HEAD -o beepd-src.tar.gz
}

# ② 방화벽 — TCP 47300(제어·중계) + UDP 47300(관측·홀펀칭). UDP를 빼먹으면
#    펀칭이 조용히 죽고 릴레이 폴백만 도니, 반드시 둘 다 연다.
Write-Host '[2/5] 방화벽 규칙(tcp/udp 47300)'
$fw = gcloud compute firewall-rules list --project $Project --filter 'name=beepd-test-47300' --format 'value(name)' 2>$null
if (-not $fw) {
    gcloud compute firewall-rules create beepd-test-47300 --project $Project `
        --allow 'tcp:47300,udp:47300' --target-tags beepd --direction INGRESS
}

# ③ VM 생성(이미 있으면 재사용)
Write-Host "[3/5] VM $Name ($MachineType @ $Zone)"
$exists = gcloud compute instances list --project $Project --filter "name=$Name" --format 'value(name)' 2>$null
if (-not $exists) {
    gcloud compute instances create $Name @common `
        --machine-type $MachineType --image-family debian-12 --image-project debian-cloud `
        --tags beepd
    Start-Sleep 15   # SSH 키 전파 대기
}

# ④ 업로드 + 셋업 — 바이너리 경로 = 수 초 · 소스 폴백 = 수 분
if (-not $FromSource) {
    Write-Host '[4/5] 업로드 + VM 셋업(정적 바이너리 설치·systemd 상주 — 수 초)'
    gcloud compute scp $binPath tools/beepd-cloud/vm-setup.sh "${Name}:/tmp/" @common
} else {
    Write-Host '[4/5] 업로드 + VM 셋업(rust 설치·빌드·systemd 상주 — 수 분)'
    gcloud compute scp beepd-src.tar.gz tools/beepd-cloud/vm-setup.sh "${Name}:/tmp/" @common
}
# CR 제거 후 실행 — Windows 체크아웃이 CRLF로 바꿔 놨어도 bash가 죽지 않게
# (.gitattributes *.sh eol=lf가 1차 방어 · 이 줄은 과거 체크아웃 대비 2차 방어).
gcloud compute ssh $Name @common --command 'sudo bash -c "tr -d \"\r\" < /tmp/vm-setup.sh > /tmp/setup.lf.sh && bash /tmp/setup.lf.sh"'

# ⑤ 결과 안내
$ip = gcloud compute instances describe $Name @common --format 'value(networkInterfaces[0].accessConfigs[0].natIP)'
Write-Host '──────────────────────────────────────────────'
Write-Host "[5/5] 배포 완료 — 서버: ${ip}:47300 (위 로그의 '서버 신원(핀)' = 클라 대조 값)"
Write-Host ''
Write-Host '실측(26 §3-7 절차 · 주소만 공인 IP):'
Write-Host "  수신: nexa-beep --chat-live 이름 --server ${ip}:47300        (시작 출력의 [신원] 64hex = 내 지문)"
Write-Host "  발신: nexa-beep --chat-connect-via <상대지문> --server ${ip}:47300   (★다른 망 = 폰 테더링이면 실 NAT 실측)"
Write-Host '  관전: 성립 1줄이 "UDP 직결(홀펀칭)"인가 "릴레이 경유"인가 · 서버 로그(journalctl -u beepd)는 봉투만인가'
Write-Host ''
Write-Host "철거(과금 중단): .\tools\beepd-cloud\deploy-gcp.ps1 -Teardown -Project $Project -Zone $Zone"
