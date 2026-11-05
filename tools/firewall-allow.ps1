<#
.SYNOPSIS
  Windows 방화벽 "네트워크 액세스 허용" 창 억제 — 개발·시험 경로의 nexa-beep.exe에 인바운드 허용 규칙을 한 번에 넣는다(11-04 · 사용자 요청).

.DESCRIPTION
  Windows Defender 방화벽은 **실행 파일 경로** 단위로 묻는다. 시험 스크립트(tools/*.sh)는 바이너리를
  `$HOME\.nexa-beep-multi\<신원>\nexa-beep.exe`로 복사해 띄우므로 신원 폴더마다 처음 한 번씩 창이 뜬다
  (재빌드로 내용이 바뀌어도 경로가 같으면 다시 묻지 않는다 · 새 폴더가 생기면 또 뜬다).
  이 스크립트는 아래 경로 전부에 규칙을 만든다(멱등 — 같은 이름 규칙은 지우고 다시 만든다):
    ① target\debug · target\release 의 nexa-beep.exe
    ② $HOME\.nexa-beep-multi\* 에 이미 있는 신원 폴더
    ③ tools\*.sh 가 `seed <이름>`으로 만드는 신원 이름(아직 폴더가 없어도 경로 규칙은 미리 넣을 수 있다)
    ④ 설치본 %LOCALAPPDATA%\Programs\NexaBeep\nexa-beep.exe (있을 때)
  규칙 그룹 = "Nexa Beep dev" · 인바운드 · 허용 · 모든 프로필 · 모든 프로토콜.
  ⚠ 관리자 권한 필요 — 일반 창에서 실행하면 UAC로 자기 자신을 승격해 다시 연다.
  ⚠ 세션(개발·협업)이 돌리지 않는다 — 시스템 설정 변경이라 **사용자가 1회** 실행한다.

.PARAMETER Remove
  "Nexa Beep dev" 그룹의 규칙을 전부 지운다.

.PARAMETER Extra
  추가 신원 이름(예: -Extra tr-x,tr-y). $HOME\.nexa-beep-multi\<이름>\nexa-beep.exe 규칙을 더 넣는다.

.EXAMPLE
  powershell -ExecutionPolicy Bypass -File tools\firewall-allow.ps1
  powershell -ExecutionPolicy Bypass -File tools\firewall-allow.ps1 -Remove
#>
[CmdletBinding()]
param(
    [switch]$Remove,
    [string[]]$Extra = @()
)

$ErrorActionPreference = 'Stop'
$Group = 'Nexa Beep dev'

# ── 승격 ───────────────────────────────────────────────────────────────────
$isAdmin = ([Security.Principal.WindowsPrincipal][Security.Principal.WindowsIdentity]::GetCurrent()
    ).IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)
if (-not $isAdmin) {
    Write-Host '관리자 권한이 필요합니다 — UAC 창에서 승인해 주세요.'
    $args = @('-NoProfile', '-ExecutionPolicy', 'Bypass', '-File', "`"$PSCommandPath`"")
    if ($Remove) { $args += '-Remove' }
    if ($Extra.Count -gt 0) { $args += @('-Extra', ($Extra -join ',')) }
    $exe = if ($PSVersionTable.PSEdition -eq 'Core') { 'pwsh' } else { 'powershell' }
    Start-Process -FilePath $exe -ArgumentList $args -Verb RunAs -Wait
    exit
}

# ── 지우기 ─────────────────────────────────────────────────────────────────
if ($Remove) {
    $old = @(Get-NetFirewallRule -Group $Group -ErrorAction SilentlyContinue)
    $old | Remove-NetFirewallRule
    Write-Host "지움: $($old.Count)건 (그룹 '$Group')"
    exit
}

# ── 경로 수집 ───────────────────────────────────────────────────────────────
$repo = Split-Path -Parent $PSScriptRoot
$multi = if ($env:BEEP_MULTI) { $env:BEEP_MULTI } else { Join-Path $HOME '.nexa-beep-multi' }
$paths = [System.Collections.Generic.List[string]]::new()

foreach ($p in @('debug', 'release')) {
    $paths.Add((Join-Path $repo "target\$p\nexa-beep.exe"))
}
# 이미 있는 신원 폴더
if (Test-Path $multi) {
    Get-ChildItem -Path $multi -Directory | ForEach-Object {
        $paths.Add((Join-Path $_.FullName 'nexa-beep.exe'))
    }
}
# tools/*.sh 의 seed <이름> — 아직 없는 폴더도 미리
Get-ChildItem -Path (Join-Path $repo 'tools') -Filter '*.sh' | ForEach-Object {
    Select-String -Path $_.FullName -Pattern 'seed ([A-Za-z0-9_-]+) ' -AllMatches | ForEach-Object {
        $_.Matches | ForEach-Object { $paths.Add((Join-Path $multi "$($_.Groups[1].Value)\nexa-beep.exe")) }
    }
}
foreach ($n in $Extra) { if ($n) { $paths.Add((Join-Path $multi "$n\nexa-beep.exe")) } }
# 설치본(NSIS · %LOCALAPPDATA%\Programs\NexaBeep)
$inst = Join-Path $env:LOCALAPPDATA 'Programs\NexaBeep\nexa-beep.exe'
if (Test-Path $inst) { $paths.Add($inst) }

$paths = $paths | Sort-Object -Unique

# ── 규칙 생성(멱등) ─────────────────────────────────────────────────────────
$made = 0
foreach ($path in $paths) {
    $name = "Nexa Beep dev — $path"
    Get-NetFirewallRule -DisplayName $name -ErrorAction SilentlyContinue | Remove-NetFirewallRule
    New-NetFirewallRule -DisplayName $name -Group $Group -Direction Inbound -Action Allow `
        -Program $path -Profile Any -Protocol Any -Enabled True | Out-Null
    $made++
    Write-Host "허용: $path"
}
# 알림 자체는 그대로 둔다(규칙 없는 새 경로는 여전히 묻는다 — 새 신원 폴더가 생기면 다시 실행).
Write-Host "완료: ${made}건 (그룹 '$Group'). 되돌리기 = -Remove"
