$ErrorActionPreference = 'Stop'

# Reuse the uninstall information left by the installer (HKCU — this is a
# per-user install, so that is where it lives).
$key = 'HKCU:\Software\Microsoft\Windows\CurrentVersion\Uninstall\NexaBeep'
if (Test-Path $key) {
  $uninst = (Get-ItemProperty $key).UninstallString -replace '"', ''
  if ($uninst -and (Test-Path $uninst)) {
    Uninstall-ChocolateyPackage -PackageName 'nexa-beep' -FileType 'exe' `
      -SilentArgs '/S' -File $uninst -ValidExitCodes @(0)
  }
} else {
  Write-Host 'Nexa Beep install information not found - assuming it is already removed.'
}
