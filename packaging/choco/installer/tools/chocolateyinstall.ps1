$ErrorActionPreference = 'Stop'

# 사용자 단위 NSIS 설치본(installer.nsi) — 권한 상승 없이 /S 무인 설치가 통과한다.
$packageArgs = @{
  packageName    = 'nexa-beep'
  fileType       = 'exe'
  url            = 'https://github.com/SosomLab/nexa-beep/releases/download/v@VERSION@/nexa-beep-@VERSION@-windows-x64-setup.exe'
  checksum       = '@SHA_WIN_X64_SETUP@'
  checksumType   = 'sha256'
  url64bit       = 'https://github.com/SosomLab/nexa-beep/releases/download/v@VERSION@/nexa-beep-@VERSION@-windows-x64-setup.exe'
  checksum64     = '@SHA_WIN_X64_SETUP@'
  checksumType64 = 'sha256'
  silentArgs     = '/S'
  validExitCodes = @(0)
}
Install-ChocolateyPackage @packageArgs
