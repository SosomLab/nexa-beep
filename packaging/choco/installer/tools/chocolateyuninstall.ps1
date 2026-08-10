$ErrorActionPreference = 'Stop'

# 설치본이 남긴 제거 정보를 그대로 쓴다(HKCU — 사용자 단위 설치라 여기 있다).
$key = 'HKCU:\Software\Microsoft\Windows\CurrentVersion\Uninstall\NexaBeep'
if (Test-Path $key) {
  $uninst = (Get-ItemProperty $key).UninstallString -replace '"', ''
  if ($uninst -and (Test-Path $uninst)) {
    Uninstall-ChocolateyPackage -PackageName 'nexa-beep' -FileType 'exe' `
      -SilentArgs '/S' -File $uninst -ValidExitCodes @(0)
  }
} else {
  Write-Host 'Nexa Beep 설치 정보를 찾지 못했습니다 — 이미 제거된 것으로 봅니다.'
}
