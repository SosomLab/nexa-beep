$ErrorActionPreference = 'Stop'

# The server package lives entirely in the package folder — choco removing the
# folder is all it takes (shims included).
# NOTE: the server key the user created (beepd.key — a separate path is
# recommended) is left untouched.
Write-Host 'nexa-beepd uninstall - removing the package folder only (the server key is kept).'
