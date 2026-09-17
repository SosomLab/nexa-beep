$ErrorActionPreference = 'Stop'

# The portable package lives entirely in the package folder — choco removing the
# folder is all it takes, and the shims go with it.
# NOTE: anything the user created next to the executable (DR-4 portable state)
# is left untouched.
Write-Host 'Nexa Beep (Portable) uninstall - removing the package folder only.'
