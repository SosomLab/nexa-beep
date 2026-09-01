$ErrorActionPreference = 'Stop'

# 포터블 — zip을 패키지 폴더에 풀고 shim만 만든다(DR-4: 설치 흔적 없음).
# 아키텍처별 zip을 각각 주면 choco가 OS에 맞는 것을 고른다.
$toolsDir = Split-Path -Parent $MyInvocation.MyCommand.Definition

Install-ChocolateyZipPackage `
  -PackageName 'nexa-beep-portable' `
  -Url         'https://github.com/SosomLab/nexa-beep/releases/download/v@VERSION@/nexa-beep-@VERSION@-windows-x64-portable.zip' `
  -Checksum    '@SHA_WIN_X64_PORTABLE@' -ChecksumType 'sha256' `
  -Url64bit    'https://github.com/SosomLab/nexa-beep/releases/download/v@VERSION@/nexa-beep-@VERSION@-windows-x64-portable.zip' `
  -Checksum64  '@SHA_WIN_X64_PORTABLE@' -ChecksumType64 'sha256' `
  -UnzipLocation $toolsDir

# nbeep-imgdec(이미지 격리 디코드 · M4-5)는 본체가 형제 경로에서 부르는 보조 실행
# 파일이다 — 사용자 명령이 아니므로 shim을 만들지 않는다(.ignore).
Get-ChildItem $toolsDir -Recurse -Filter 'nbeep-imgdec.exe' | ForEach-Object {
  New-Item -ItemType File -Path "$($_.FullName).ignore" -Force | Out-Null
}
