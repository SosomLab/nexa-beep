<#
.SYNOPSIS
  실행 중인 nexa-beep 프로세스의 **순간 최대 메모리**를 짧은 간격으로 잰다(11-07 · M4-12 수신 피크 실측 · 협업 세션).

.DESCRIPTION
  `tools/mem-bench.sh`의 4점 샘플은 수신이 **끝난 뒤**를 재므로 수신 도중의 피크(파일 크기 배수)를 보지 못한다.
  이 스크립트는 실행 파일 경로에 `-Match` 문자열이 들어간 nexa-beep 프로세스 하나를 `-IntervalMs` 간격으로
  `-Seconds` 동안 폴링해 다음 셋의 최댓값을 남긴다:
    · PrivateMemorySize64(Private Bytes = 커밋 · 피크 판정의 기준)
    · WorkingSet64(그 순간의 상주)
    · PeakWorkingSet64(OS가 기억하는 상주 최대 — 폴링 사이의 순간도 잡는다)
  ⚠ 49 D-1 정의(`Working Set - Private` 카운터)와 지표가 다르다 — 회수 판정은 mem-bench, 순간 피크는 이 스크립트.
  읽기 전용 — 프로세스를 건드리지 않는다. 대상이 없으면 seen=False.

.EXAMPLE
  # mem-bench와 같이 — 수신(mb-y)·발신(mb-x)을 따로 잰다(mem-bench 시작 직후 백그라운드로)
  powershell -NoProfile -ExecutionPolicy Bypass -File tools\peak-poll.ps1 -Match mb-y -Seconds 120 -Out target\peak-y.txt
#>
param(
  [string]$Match = 'mb-y',
  [int]$Seconds = 120,
  [int]$IntervalMs = 250,
  [string]$Out = ''
)
$maxPriv = 0; $maxWs = 0; $maxPeakWs = 0; $seen = $false
$end = (Get-Date).AddSeconds($Seconds)
while ((Get-Date) -lt $end) {
  $p = Get-Process nexa-beep -ErrorAction SilentlyContinue |
    Where-Object { $_.Path -like "*\$Match\*" } | Select-Object -First 1
  if ($p) {
    $seen = $true
    $p.Refresh()
    if ($p.PrivateMemorySize64 -gt $maxPriv) { $maxPriv = $p.PrivateMemorySize64 }
    if ($p.WorkingSet64 -gt $maxWs) { $maxWs = $p.WorkingSet64 }
    if ($p.PeakWorkingSet64 -gt $maxPeakWs) { $maxPeakWs = $p.PeakWorkingSet64 }
  }
  Start-Sleep -Milliseconds $IntervalMs
}
$line = "match=$Match seen=$seen max_private_mb=$([math]::Round($maxPriv/1MB,1)) " +
        "max_ws_mb=$([math]::Round($maxWs/1MB,1)) peak_ws_mb=$([math]::Round($maxPeakWs/1MB,1))"
if ($Out) { $line | Out-File -FilePath $Out -Encoding utf8 }
$line
