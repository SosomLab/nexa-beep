#!/usr/bin/env bash
# beepd(릴레이 서버) 패키지 매니페스트 생성 — Homebrew · winget · Chocolatey
#
#   render-manifests-beepd.sh <VERSION> <자산디렉터리> <출력디렉터리>
#
# 클라이언트(render-manifests.sh)와 **별도 스크립트**다 — 자산 이름·태그(`beepd-v*`)·
# 패키지 식별자가 전부 다르고, 제품마다 "유일한 치환 지점"을 하나씩 갖는 편이
# 갈라짐을 막는다(같은 파일에서 분기하면 언젠가 서로를 밟는다).
# 체크섬은 자산에서 직접 계산한다(손으로 적은 해시는 언젠가 틀린다).
set -euo pipefail

VERSION="${1:?사용법: render-manifests-beepd.sh <VERSION> <자산디렉터리> <출력디렉터리>}"
ASSETS="${2:?자산 디렉터리}"
OUT="${3:?출력 디렉터리}"

here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

sha() {
  local f="$ASSETS/$1"
  [ -f "$f" ] || { echo "::error::자산 없음: $1" >&2; exit 1; }
  if command -v sha256sum >/dev/null 2>&1; then
    sha256sum "$f" | cut -d' ' -f1
  else
    shasum -a 256 "$f" | cut -d' ' -f1
  fi
}

V="$VERSION"
SHA_BEEPD_LINUX_X64=$(sha   "nexa-beepd-$V-linux-x64.tar.gz")
SHA_BEEPD_LINUX_ARM64=$(sha "nexa-beepd-$V-linux-arm64.tar.gz")
SHA_BEEPD_MAC_ARM64=$(sha   "nexa-beepd-$V-macos-arm64.tar.gz")
SHA_BEEPD_WIN_X64=$(sha     "nexa-beepd-$V-windows-x64.zip")
DATE=$(date -u +%Y-%m-%d)

fill() {
  sed -e "s/@VERSION@/$V/g" \
      -e "s/@DATE@/$DATE/g" \
      -e "s/@SHA_BEEPD_LINUX_X64@/$SHA_BEEPD_LINUX_X64/g" \
      -e "s/@SHA_BEEPD_LINUX_ARM64@/$SHA_BEEPD_LINUX_ARM64/g" \
      -e "s/@SHA_BEEPD_MAC_ARM64@/$SHA_BEEPD_MAC_ARM64/g" \
      -e "s/@SHA_BEEPD_WIN_X64@/$SHA_BEEPD_WIN_X64/g" \
      "$1" > "$2"
}

mkdir -p "$OUT"

# ── Homebrew(탭 배치 그대로: Formula/) ──
mkdir -p "$OUT/homebrew/Formula"
fill "$here/beepd/homebrew/nexa-beepd.rb" "$OUT/homebrew/Formula/nexa-beepd.rb"

# ── winget — microsoft/winget-pkgs 경로 규약(소문자 첫 글자·점을 디렉터리로) ──
dir="$OUT/winget/manifests/s/SosomLab/NexaBeepd/$V"
mkdir -p "$dir"
fill "$here/beepd/winget/version.yaml"   "$dir/SosomLab.NexaBeepd.yaml"
fill "$here/beepd/winget/locale.yaml"    "$dir/SosomLab.NexaBeepd.locale.ko-KR.yaml"
fill "$here/beepd/winget/installer.yaml" "$dir/SosomLab.NexaBeepd.installer.yaml"

# ── Chocolatey — 그대로 `choco pack` 가능한 형태 ──
mkdir -p "$OUT/choco/nexa-beepd/tools"
fill "$here/beepd/choco/nexa-beepd.nuspec"                 "$OUT/choco/nexa-beepd/nexa-beepd.nuspec"
fill "$here/beepd/choco/tools/chocolateyinstall.ps1"       "$OUT/choco/nexa-beepd/tools/chocolateyinstall.ps1"
fill "$here/beepd/choco/tools/chocolateyuninstall.ps1"     "$OUT/choco/nexa-beepd/tools/chocolateyuninstall.ps1"

echo "완료 — $OUT (homebrew/winget/choco)"
