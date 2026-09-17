$ErrorActionPreference = 'Stop'

# Portable — unpack the zip into the package folder and create shims only
# (DR-4: leaves no install footprint).
# Giving a zip per architecture lets choco pick the one matching the OS.
$toolsDir = Split-Path -Parent $MyInvocation.MyCommand.Definition

Install-ChocolateyZipPackage `
  -PackageName 'nexa-beep-portable' `
  -Url         'https://github.com/SosomLab/nexa-beep/releases/download/v@VERSION@/nexa-beep-@VERSION@-windows-x64-portable.zip' `
  -Checksum    '@SHA_WIN_X64_PORTABLE@' -ChecksumType 'sha256' `
  -Url64bit    'https://github.com/SosomLab/nexa-beep/releases/download/v@VERSION@/nexa-beep-@VERSION@-windows-x64-portable.zip' `
  -Checksum64  '@SHA_WIN_X64_PORTABLE@' -ChecksumType64 'sha256' `
  -UnzipLocation $toolsDir

# nbeep-imgdec (sandboxed image decoding, M4-5) is a helper executable the main
# program invokes from its sibling path — it is not a user command, so no shim.
Get-ChildItem $toolsDir -Recurse -Filter 'nbeep-imgdec.exe' | ForEach-Object {
  New-Item -ItemType File -Path "$($_.FullName).ignore" -Force | Out-Null
}
