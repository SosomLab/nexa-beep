# install-local.ps1 — Windows 설치본 자리 덮어쓰기 실기(09-05 · tools/install-local.sh의 Windows 네이티브 판):
#   ① 종료 → ② 릴리스 빌드 → ③ NSIS 설치 폴더(%LOCALAPPDATA%\Programs\NexaBeep)에 exe 2개 덮어쓰기 → ④ 설치본 실행 → ⑤ 확인.
#   Git Bash/MSYS 없이 pwsh만으로 돈다(종전 install-local.sh의 Windows 분기는 여기로 위임한다).
#
# 사용:  pwsh tools/install-local.ps1             # ①~⑤ 전부(릴리스 프로필 — 배포본과 같은 최적화)
#        pwsh tools/install-local.ps1 -Debug      # 디버그 프로필(패닉 위치 등 진단)
#        pwsh tools/install-local.ps1 -NoBuild    # 직전 산출물 그대로
#        pwsh tools/install-local.ps1 -NoRun      # 복사까지만
#        $env:NEXA_INSTALL_DIR = 'D:\Apps\NexaBeep'  # 비표준 설치 자리 강제(기본 = 레지스트리 InstallDir → %LOCALAPPDATA%\Programs\NexaBeep)
# 전제:  설치본이 한 번은 설치돼 있어야 한다(폴더·바로가기·언인스톨러·자동 실행 슬롯은 NSIS 몫 — 여기서는 exe만 바꾼다).
# 데이터: 설치본(%APPDATA% 또는 exe 옆 data\)은 개발 인스턴스(target\*\data)와 **다른** 이력·설정·신원이다 — `--whoami`로 확인.
# ⚠ 버전 문자열·winget/choco 등록은 그대로다 — 실기 전용이지 배포 대체가 아니다. 다음 정식 설치가 덮어쓴다.

param([switch]$Debug, [switch]$NoBuild, [switch]$NoRun)

$ErrorActionPreference = "Stop"
$root = Split-Path -Parent $PSScriptRoot
$profile = if ($Debug) { "debug" } else { "release" }
function Say($m) { Write-Host ""; Write-Host "▶ $m" -ForegroundColor White }

# ── 설치 자리 — 환경변수 → NSIS가 남긴 HKCU InstallDir → 기본 경로(installer.nsi InstallDir와 동일해야 한다)
$dst = $env:NEXA_INSTALL_DIR
if (-not $dst) {
    $dst = (Get-ItemProperty -Path "HKCU:\Software\SosomLab\NexaBeep" -Name InstallDir -ErrorAction SilentlyContinue).InstallDir
}
if (-not $dst) { $dst = Join-Path $env:LOCALAPPDATA "Programs\NexaBeep" }
if (-not (Test-Path (Join-Path $dst "nexa-beep.exe"))) {
    throw "설치본이 없다: $dst\nexa-beep.exe — 먼저 NSIS 설치본을 한 번 설치한다(또는 `$env:NEXA_INSTALL_DIR 지정)"
}
# 개발 트리를 설치 자리로 오인하지 않는다(target\ 을 자기 자신에 덮어쓰는 사고 방지).
if ((Resolve-Path $dst).Path.StartsWith((Join-Path $root "target"), [System.StringComparison]::OrdinalIgnoreCase)) {
    throw "설치 자리가 개발 트리다($dst) — 배포본을 설치한 뒤 다시 실행"
}
Write-Host "설치 자리 = $dst (win · $profile)"

# ── ① 종료 — 개발 인스턴스·imgdec 워커까지(exe 잠금 해제 · 자동 실행 슬롯 줄다리기 회피)
Say "① 실행 중인 nexa-beep 종료"
$running = Get-Process nexa-beep, nbeep-imgdec -ErrorAction SilentlyContinue
if ($running) {
    $running | Stop-Process -Force
    Start-Sleep -Milliseconds 800
    Write-Host ("   종료: " + (($running | ForEach-Object { "$($_.ProcessName)#$($_.Id)" }) -join " "))
} else { Write-Host "   실행 중인 프로세스 없음" }

# ── ② 빌드
if (-not $NoBuild) {
    Say "② $profile 빌드 (nexa-beep + nbeep-imgdec)"
    Push-Location $root
    try {
        if ($Debug) { cargo build -p nexa-beep -p nbeep-imgdec } else { cargo build --release -p nexa-beep -p nbeep-imgdec }
        if ($LASTEXITCODE -ne 0) { throw "빌드 실패(exit $LASTEXITCODE)" }
    } finally { Pop-Location }
} else { Say "② 빌드 건너뜀 (-NoBuild)" }
$src = Join-Path $root "target\$profile"
foreach ($f in "nexa-beep.exe", "nbeep-imgdec.exe") {
    if (-not (Test-Path (Join-Path $src $f))) { throw "산출물이 없다: $src\$f" }
}

# ── ③ 덮어쓰기 — md5 대조로 "복사했다"가 아니라 "바뀌었다"를 확인한다
Say "③ 설치 자리 덮어쓰기 → $dst"
function Sum($p) { (Get-FileHash -Algorithm MD5 $p).Hash.Substring(0, 12).ToLower() }
$before = Sum (Join-Path $dst "nexa-beep.exe")
foreach ($f in "nexa-beep.exe", "nbeep-imgdec.exe") {
    Copy-Item (Join-Path $src $f) (Join-Path $dst $f) -Force
    Write-Host ("   복사: {0,-18} {1,12:N0} B" -f $f, (Get-Item (Join-Path $src $f)).Length)
}
$after = Sum (Join-Path $dst "nexa-beep.exe")
$mark = if ($before -eq $after) { "(동일 — 산출물이 바뀌지 않았다)" } else { "✓ 교체" }
Write-Host "   nexa-beep.exe: $before → $after $mark"
if ($after -ne (Sum (Join-Path $src "nexa-beep.exe"))) { throw "설치 자리와 산출물이 다르다" }
# 버전 = Cargo.toml(workspace.package) — 설치본 exe도 --version이 되지만(attach_parent_console) 콘솔 없는 자리에서는 파일이 안전하다.
$ver = (Select-String -Path (Join-Path $root "Cargo.toml") -Pattern '^version = "([^"]+)"' | Select-Object -First 1).Matches[0].Groups[1].Value
Write-Host "   버전: nexa-beep $ver ($profile)"

# ── ④ 설치본처럼 실행(시작 메뉴 바로가기와 같은 무인자 경로 = 창+실물)
if ($NoRun) { Say "④ 실행 생략 (-NoRun)"; Write-Host ""; Write-Host "완료 — 설치본($dst)이 현재 작업 트리 산출물로 바뀌었다."; exit 0 }
Say "④ 설치본 실행"
$exe = Join-Path $dst "nexa-beep.exe"
Start-Process -FilePath $exe -WorkingDirectory $dst
Start-Sleep -Seconds 3

# ── ⑤ 확인 — 프로세스 수 + 그 exe가 실제로 로드하는 신원·데이터 경로
Say "⑤ 확인"
$procs = @(Get-Process nexa-beep -ErrorAction SilentlyContinue)
Write-Host "   실행 중 프로세스 = $($procs.Count)개"
if ($procs.Count -eq 0) { throw "창이 뜨지 않았다 — 수동: `"$exe`"" }
$procs | Select-Object Id, Path, StartTime | Format-Table -AutoSize | Out-String | Write-Host
& $exe --whoami 2>$null | ForEach-Object { "   $_" } | Select-Object -First 6
Write-Host ""
Write-Host "완료 — 설치본($dst)이 현재 작업 트리 산출물로 바뀌었다. 정식 설치(NSIS/winget/choco)가 다시 덮어쓴다."
