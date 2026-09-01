$ErrorActionPreference = 'Stop'

# 서버(포터블 규약) — zip을 패키지 폴더에 풀고 shim만 만든다. arm64 자산이 없어
# x64 zip 하나를 양쪽 슬롯에 준다(choco 규약상 둘 다 필요).
$toolsDir = Split-Path -Parent $MyInvocation.MyCommand.Definition

Install-ChocolateyZipPackage `
  -PackageName 'nexa-beepd' `
  -Url         'https://github.com/SosomLab/nexa-beep/releases/download/beepd-v@VERSION@/nexa-beepd-@VERSION@-windows-x64.zip' `
  -Checksum    '@SHA_BEEPD_WIN_X64@' -ChecksumType 'sha256' `
  -Url64bit    'https://github.com/SosomLab/nexa-beep/releases/download/beepd-v@VERSION@/nexa-beepd-@VERSION@-windows-x64.zip' `
  -Checksum64  '@SHA_BEEPD_WIN_X64@' -ChecksumType64 'sha256' `
  -UnzipLocation $toolsDir

Write-Host '첫 실행이 서버 신원 키를 만들고 "서버 신원(핀)"을 출력합니다:'
Write-Host '  nexa-beepd --port 47300 --key "C:\ProgramData\beepd\beepd.key" --verbose'
Write-Host '상주(작업 스케줄러)·방화벽(TCP+UDP 47300): docs/41-beepd-ops-guide.md'
