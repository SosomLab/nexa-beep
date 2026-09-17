$ErrorActionPreference = 'Stop'

# Server (portable convention) — unpack the zip into the package folder and
# create shims only. There is no arm64 asset, so the same x64 zip is given for
# both slots (choco requires both).
$toolsDir = Split-Path -Parent $MyInvocation.MyCommand.Definition

Install-ChocolateyZipPackage `
  -PackageName 'nexa-beepd' `
  -Url         'https://github.com/SosomLab/nexa-beep/releases/download/beepd-v@VERSION@/nexa-beepd-@VERSION@-windows-x64.zip' `
  -Checksum    '@SHA_BEEPD_WIN_X64@' -ChecksumType 'sha256' `
  -Url64bit    'https://github.com/SosomLab/nexa-beep/releases/download/beepd-v@VERSION@/nexa-beepd-@VERSION@-windows-x64.zip' `
  -Checksum64  '@SHA_BEEPD_WIN_X64@' -ChecksumType64 'sha256' `
  -UnzipLocation $toolsDir

Write-Host 'The first run creates the server identity key and prints the server fingerprint (pin):'
Write-Host '  nexa-beepd --port 47300 --key "C:\ProgramData\beepd\beepd.key" --verbose'
Write-Host 'Running as a service (Task Scheduler) and firewall (TCP+UDP 47300): docs/41-beepd-ops-guide.md'
