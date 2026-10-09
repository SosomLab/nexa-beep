#!/usr/bin/env bash
# build-deb.sh — Linux 설치본 `.deb`(dpkg-deb · FHS 레이아웃 · DR-4 설치본 채널).
#
#   packaging/linux/build-deb.sh [--skip-build] [--target x86_64-unknown-linux-gnu]
#   NEXA_BIN_DIR=target/release packaging/linux/build-deb.sh --skip-build   # 이미 빌드한 산출물로 포장만
#
# 레이아웃: /usr/bin/{nexa-beep,nbeep-imgdec} · /usr/share/applications/nexa-beep.desktop ·
#           /usr/share/icons/hicolor/<N>x<N>/apps/nexa-beep.png(16…512) + scalable/apps/nexa-beep.svg ·
#           /usr/share/doc/nexa-beep/{LICENSE.md,LICENSE.ko.md,README.md,copyright}
# 산출:    target/packaging/linux/nexa-beep-<ver>-<slug>.deb  (자산 이름 규약 = release.yml 다른 타깃과 동일)
# 이식 원천: ../nexa-sql/packaging/linux/build-deb.sh (10-09) — 종전에는 이 내용이 release.yml 인라인에만 있었다.
# 검증(설치 없이): dpkg-deb --info/--contents · desktop-file-validate · glibc 하한 실측 출력.
#   설치 스모크는 CI(sudo dpkg -i → nexa-beep --version → dpkg -r → 잔여 0 · release.yml).
source "$(dirname "${BASH_SOURCE[0]}")/../lib.sh"

SKIP_BUILD=0
TARGET="${NEXA_LINUX_TARGET:-x86_64-unknown-linux-gnu}"
while [ $# -gt 0 ]; do
    case "$1" in
        --skip-build) SKIP_BUILD=1 ;;
        --target) shift; TARGET="$1" ;;
        *) die "알 수 없는 인자: $1" ;;
    esac
    shift
done
case "$TARGET" in aarch64-*) ARCH=arm64 ;; *) ARCH=amd64 ;; esac
SLUG="$(slug_of "$TARGET")"
have dpkg-deb || die "dpkg-deb 없음(apt-get install dpkg-dev)"

OUT="$OUT_ROOT/linux"; PKG="$OUT/deb-root"; rm -rf "$PKG"
FINAL="$OUT/nexa-beep-${VERSION}-${SLUG}.deb"
BIN="${NEXA_BIN_DIR:-$ROOT/target/$TARGET/release}"

step "빌드 (release · $TARGET)"
if [ "$SKIP_BUILD" = 0 ]; then
    # imgdec(이미지 격리 디코드 · M4-5)은 본체의 형제 실행 파일 — 없으면 아바타가 이니셜 폴백으로만 뜬다.
    "$(cargo_bin)" build --release -p nexa-beep -p nbeep-imgdec --target "$TARGET"
else
    note "빌드 생략 · $BIN"
fi
[ -x "$BIN/nexa-beep" ] && [ -x "$BIN/nbeep-imgdec" ] || die "산출물 없음: $BIN/{nexa-beep,nbeep-imgdec}"

step "스테이징(FHS) → $PKG"
mkdir -p "$PKG/DEBIAN" "$PKG/usr/bin" "$PKG/usr/share/applications" "$PKG/usr/share/doc/nexa-beep"
install -m 0755 "$BIN/nexa-beep"    "$PKG/usr/bin/nexa-beep"
install -m 0755 "$BIN/nbeep-imgdec" "$PKG/usr/bin/nbeep-imgdec"
install -m 0644 "$ROOT/packaging/linux/nexa-beep.desktop" "$PKG/usr/share/applications/nexa-beep.desktop"
# hicolor 아이콘 — 크기별 PNG(packaging/branding/png · 원본 icon.svg에서 구움) + 벡터.
for n in 16 24 32 48 64 128 256 512; do
    d="$PKG/usr/share/icons/hicolor/${n}x${n}/apps"; mkdir -p "$d"
    install -m 0644 "$BRANDING/png/nexa-beep-$n.png" "$d/nexa-beep.png"
done
mkdir -p "$PKG/usr/share/icons/hicolor/scalable/apps"
install -m 0644 "$BRANDING/icon.svg" "$PKG/usr/share/icons/hicolor/scalable/apps/nexa-beep.svg"
stage_docs "$PKG/usr/share/doc/nexa-beep"
# Debian 정책: /usr/share/doc/<pkg>/copyright
{ echo "Format: https://www.debian.org/doc/packaging-manuals/copyright-format/1.0/"
  echo "Upstream-Name: nexa-beep"; echo "Source: $HOMEPAGE"; echo
  echo "Files: *"; echo "Copyright: 2026 $PUBLISHER"; echo "License: PolyForm-Noncommercial-1.0.0"
  echo " See LICENSE.md in this directory."; } > "$PKG/usr/share/doc/nexa-beep/copyright"

# ★ 의존 하한 = 바이너리 실측(lib.sh glibc_floor). 선언값을 손으로 적지 않는다.
GLIBC="$(glibc_floor "$PKG/usr/bin/nexa-beep" "$PKG/usr/bin/nbeep-imgdec")"
size_kb=$(du -sk "$PKG/usr" | cut -f1)
sed -e "s/@VERSION@/$VERSION/g" -e "s/@ARCH@/$ARCH/g" -e "s/@SIZE@/$size_kb/g" -e "s/@GLIBC@/$GLIBC/g" \
    "$ROOT/packaging/linux/control" > "$PKG/DEBIAN/control"
grep -qE '@[A-Z]+@' "$PKG/DEBIAN/control" && die "control에 치환되지 않은 자리표시자가 남았다"
# 설치/제거 후 아이콘·데스크톱 캐시 갱신(도구 없으면 무시 — 실패해도 설치는 막지 않는다).
cat > "$PKG/DEBIAN/postinst" <<'EOF'
#!/bin/sh
set -e
command -v gtk-update-icon-cache >/dev/null 2>&1 && gtk-update-icon-cache -q -t -f /usr/share/icons/hicolor 2>/dev/null || true
command -v update-desktop-database >/dev/null 2>&1 && update-desktop-database -q /usr/share/applications 2>/dev/null || true
exit 0
EOF
cp "$PKG/DEBIAN/postinst" "$PKG/DEBIAN/postrm"; chmod 0755 "$PKG/DEBIAN/postinst" "$PKG/DEBIAN/postrm"
note "glibc 하한(실측) = $GLIBC · Installed-Size = ${size_kb}KiB"

step "dpkg-deb --build"
mkdir -p "$OUT"; rm -f "$FINAL"
dpkg-deb --build --root-owner-group "$PKG" "$FINAL" >/dev/null
note "$(basename "$FINAL") $(du -h "$FINAL" | cut -f1) · sha256 $(sha256_of "$FINAL")"

step "검증(설치 없이)"
dpkg-deb --info "$FINAL" | grep -E '^ (Package|Version|Architecture|Depends|Installed-Size):' | sed 's/^/   /'
dpkg-deb --contents "$FINAL" | grep -E 'usr/bin/(nexa-beep|nbeep-imgdec)$|nexa-beep.desktop$|256x256/apps/nexa-beep.png$|scalable/apps/nexa-beep.svg$' | awk '{print "    " $NF}'
if have desktop-file-validate; then
    desktop-file-validate "$PKG/usr/share/applications/nexa-beep.desktop" && note "desktop-file-validate ✓"
else
    note "desktop-file-validate 없음(desktop-file-utils) — 건너뜀"
fi
echo "DEB=$FINAL"
