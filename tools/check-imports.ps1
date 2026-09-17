<#
.SYNOPSIS
  NFR-B-5 "외부 런타임 의존 0" 실측 게이트 — Windows 산출물의 임포트 테이블을
  화이트리스트와 대조한다.

.DESCRIPTION
  [docs/05 NFR-B-5]는 "OS 인박스 라이브러리만 · 임포트/링크 테이블을 화이트리스트로
  CI 검사"라고 적어 두고도 **검사가 구현된 적이 없었다**. 그 사이 MSVC 기본값(동적
  CRT)으로 링크된 산출물이 `vcruntime140.dll`(VC++ 재배포 · 인박스 아님)에 의존한
  채 릴리스 15번을 지나갔고, 깨끗한 Windows에서 실행되지 않았다(winget 자동 검증
  `STATUS_DLL_NOT_FOUND` 0xC0000135 · 09-17). 그래서 이 스크립트가 있다.

  PE 헤더를 직접 읽는다 — dumpbin·VS 툴체인·외부 모듈에 의존하지 않는다
  (러너 PATH에 없을 수 있고, 게이트가 환경을 타면 게이트가 아니다).
  일반 임포트(디렉터리 1)와 지연 로드 임포트(디렉터리 13)를 모두 본다.
  `LoadLibraryW` 동적 로드(AMSI 등)는 임포트 테이블에 없으므로 여기서 안 잡힌다 —
  그건 의도된 범위다(있으면 실행이 실패하는 의존만 여기서 막는다).

.PARAMETER Path
  검사할 실행 파일(여러 개 가능).

.PARAMETER List
  대조하지 않고 임포트 목록만 출력한다(화이트리스트 갱신용).

.EXAMPLE
  pwsh tools/check-imports.ps1 target/release/nexa-beep.exe target/release/nbeep-imgdec.exe
#>
[CmdletBinding()]
param(
  [Parameter(Mandatory = $true, Position = 0, ValueFromRemainingArguments = $true)][string[]]$Path,
  [switch]$List
)

$ErrorActionPreference = 'Stop'

# ── 화이트리스트 ────────────────────────────────────────────────────────────
# 원칙 = **Windows에 기본 탑재된 것만**. 재배포 패키지가 필요한 것(vcruntime·msvcp·
# msvcr·concrt·vcomp)은 여기 절대 들어오지 않는다.
# 새 DLL을 쓰기 시작하면 이 게이트가 **일부러** 실패한다 — 인박스인지 사람이 한 번
# 확인하고 이 목록에 올리라는 뜻이다(그게 NFR-B-5가 요구하는 "검토"다).
$Whitelist = @(
  'advapi32.dll', 'bcrypt.dll', 'bcryptprimitives.dll', 'comdlg32.dll', 'crypt32.dll',
  'dbghelp.dll', 'dwmapi.dll', 'gdi32.dll', 'imm32.dll', 'iphlpapi.dll', 'kernel32.dll',
  'ntdll.dll', 'ole32.dll', 'oleaut32.dll', 'powrprof.dll', 'propsys.dll', 'rpcrt4.dll',
  'shcore.dll', 'shell32.dll', 'shlwapi.dll', 'user32.dll', 'userenv.dll', 'uxtheme.dll',
  'version.dll', 'winmm.dll', 'ws2_32.dll'
)
# 패턴 허용: API set(`api-ms-win-*`)과 `ext-ms-win-*`은 OS가 제공하는 가상 DLL이다.
# UCRT(`api-ms-win-crt-*`)도 Windows 10+ 인박스라 여기에 포함된다.
$WhitelistPatterns = @('^api-ms-win-', '^ext-ms-win-')
# 이름을 알아보고 사유를 짚어 주는 것들(실패 메시지 품질용).
$KnownBad = @{
  'vcruntime140.dll'   = 'VC++ 2015+ 재배포 — `-C target-feature=+crt-static` 누락'
  'vcruntime140_1.dll' = 'VC++ 2015+ 재배포 — `-C target-feature=+crt-static` 누락'
  'msvcp140.dll'       = 'VC++ 2015+ 재배포(C++ 표준 라이브러리)'
  'concrt140.dll'      = 'VC++ 2015+ 재배포'
  'vcomp140.dll'       = 'VC++ 2015+ 재배포(OpenMP)'
}

function Get-PeImports {
  param([string]$File)

  $b = [System.IO.File]::ReadAllBytes($File)
  if ($b.Length -lt 0x40) { throw "$File : 너무 작다(PE 아님)" }
  if ($b[0] -ne 0x4D -or $b[1] -ne 0x5A) { throw "$File : MZ 서명 없음" }

  $peOff = [BitConverter]::ToInt32($b, 0x3C)
  if ($peOff -le 0 -or $peOff + 24 -gt $b.Length) { throw "$File : e_lfanew 범위 밖" }
  if ([BitConverter]::ToUInt32($b, $peOff) -ne 0x00004550) { throw "$File : PE 서명 없음" }

  $nSections   = [BitConverter]::ToUInt16($b, $peOff + 6)
  $optSize     = [BitConverter]::ToUInt16($b, $peOff + 20)
  $optOff      = $peOff + 24
  $magic       = [BitConverter]::ToUInt16($b, $optOff)
  # PE32+ = 0x20B(데이터 디렉터리가 +112) · PE32 = 0x10B(+96)
  $ddOff = if ($magic -eq 0x20B) { $optOff + 112 } elseif ($magic -eq 0x10B) { $optOff + 96 }
           else { throw "$File : 알 수 없는 optional header magic 0x$('{0:X}' -f $magic)" }

  $secOff = $optOff + $optSize
  $sections = @()
  for ($i = 0; $i -lt $nSections; $i++) {
    $s = $secOff + $i * 40
    if ($s + 40 -gt $b.Length) { throw "$File : 섹션 헤더 범위 밖" }
    $sections += [pscustomobject]@{
      VirtualSize    = [BitConverter]::ToUInt32($b, $s + 8)
      VirtualAddress = [BitConverter]::ToUInt32($b, $s + 12)
      SizeOfRawData  = [BitConverter]::ToUInt32($b, $s + 16)
      PointerToRaw   = [BitConverter]::ToUInt32($b, $s + 20)
    }
  }

  function Convert-RvaToOffset([uint32]$rva) {
    foreach ($s in $sections) {
      $span = [Math]::Max($s.VirtualSize, $s.SizeOfRawData)
      if ($rva -ge $s.VirtualAddress -and $rva -lt ($s.VirtualAddress + $span)) {
        return [int]($rva - $s.VirtualAddress + $s.PointerToRaw)
      }
    }
    return -1
  }

  function Read-AsciiZ([int]$off) {
    if ($off -lt 0 -or $off -ge $b.Length) { return $null }
    $end = $off
    while ($end -lt $b.Length -and $b[$end] -ne 0) { $end++ }
    return [Text.Encoding]::ASCII.GetString($b, $off, $end - $off)
  }

  $names = New-Object System.Collections.Generic.List[string]

  # 디렉터리 1 = 임포트 · 엔트리 20B · Name RVA는 +12 · 전부 0이면 끝
  $impRva = [BitConverter]::ToUInt32($b, $ddOff + 1 * 8)
  if ($impRva -ne 0) {
    $p = Convert-RvaToOffset $impRva
    while ($p -ge 0 -and $p + 20 -le $b.Length) {
      $nameRva = [BitConverter]::ToUInt32($b, $p + 12)
      $chunk = [BitConverter]::ToUInt32($b, $p) -bor $nameRva -bor [BitConverter]::ToUInt32($b, $p + 16)
      if ($chunk -eq 0) { break }
      $n = Read-AsciiZ (Convert-RvaToOffset $nameRva)
      if ($n) { $names.Add($n) }
      $p += 20
    }
  }

  # 디렉터리 13 = 지연 로드 임포트 · 엔트리 32B · Name RVA는 +4
  $delayRva = [BitConverter]::ToUInt32($b, $ddOff + 13 * 8)
  if ($delayRva -ne 0) {
    $p = Convert-RvaToOffset $delayRva
    while ($p -ge 0 -and $p + 32 -le $b.Length) {
      $nameRva = [BitConverter]::ToUInt32($b, $p + 4)
      if ($nameRva -eq 0) { break }
      $n = Read-AsciiZ (Convert-RvaToOffset $nameRva)
      if ($n) { $names.Add($n) }
      $p += 32
    }
  }

  return ($names | Sort-Object -Unique)
}

$failed = $false
foreach ($f in $Path) {
  if (-not (Test-Path $f)) { Write-Error "없는 파일: $f"; $failed = $true; continue }
  $full = (Resolve-Path $f).Path
  $imports = Get-PeImports $full
  $name = Split-Path $full -Leaf

  if ($List) {
    Write-Output "== $name"
    $imports | ForEach-Object { Write-Output "   $_" }
    continue
  }

  $bad = @()
  foreach ($imp in $imports) {
    $lc = $imp.ToLowerInvariant()
    if ($Whitelist -contains $lc) { continue }
    $ok = $false
    foreach ($pat in $WhitelistPatterns) { if ($lc -match $pat) { $ok = $true; break } }
    if (-not $ok) { $bad += $lc }
  }

  if ($bad.Count -eq 0) {
    Write-Output "OK   $name — 임포트 $($imports.Count)종 전부 인박스"
  }
  else {
    $failed = $true
    Write-Output "FAIL $name — 인박스가 아닌(또는 미검토) 임포트 $($bad.Count)종:"
    foreach ($x in $bad) {
      $why = if ($KnownBad.ContainsKey($x)) { $KnownBad[$x] } else { '화이트리스트 미등재 — 인박스인지 확인 후 tools/check-imports.ps1에 추가' }
      Write-Output "       - $x  ($why)"
    }
  }
}

if ($failed) {
  Write-Output ''
  Write-Output 'NFR-B-5(외부 런타임 의존 0) 위반. 배포하면 깨끗한 Windows에서 실행되지 않는다.'
  Write-Output '  · CRT면: .cargo/config.toml의 `+crt-static`이 RUSTFLAGS 환경변수에 덮여 무시됐는지 본다'
  Write-Output '    (cargo는 RUSTFLAGS가 있으면 config의 target.*.rustflags를 통째로 버린다).'
  exit 1
}
exit 0
