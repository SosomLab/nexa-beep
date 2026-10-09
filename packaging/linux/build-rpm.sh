#!/usr/bin/env bash
# build-rpm.sh — Linux 설치본 `.rpm`(rpmbuild + nexa-beep.spec · deb와 **같은 FHS 스테이징**을 포장).
#
#   packaging/linux/build-deb.sh [--skip-build] && packaging/linux/build-rpm.sh [--target <triple>]
#
# 스테이징 = build-deb.sh가 만든 target/packaging/linux/deb-root/usr (없으면 build-deb.sh를 먼저 돌린다 ·
#   같은 바이너리 보증). deb/rpm이 다른 빌드를 담는 일이 구조적으로 없다.
# 산출: target/packaging/linux/nexa-beep-<ver>-<slug>.rpm (자산 이름 규약 = 다른 타깃과 동일 · rpm 내부 이름은 spec 규약)
# 이식 원천: ../nexa-sql/packaging/linux/build-rpm.sh (10-09).
# 검증(설치 없이): rpm -qpi / -qpl / -K. 설치 스모크는 rpm 계열 러너가 없어 CI에서도 목록·무결성 검사만
#   (ubuntu 러너에 rpm 설치 불가 — Fedora/RHEL 실기는 사람이 한다 · docs/18 §5).
source "$(dirname "${BASH_SOURCE[0]}")/../lib.sh"

have rpmbuild || die "rpmbuild 없음(apt-get install rpm / dnf install rpm-build)"
TARGET="${NEXA_LINUX_TARGET:-x86_64-unknown-linux-gnu}"
PASS=()
while [ $# -gt 0 ]; do
    case "$1" in
        --target) shift; TARGET="$1" ;;
        --skip-build) PASS+=("$1") ;;
        *) die "알 수 없는 인자: $1" ;;
    esac
    shift
done
case "$TARGET" in aarch64-*) RARCH=aarch64 ;; *) RARCH=x86_64 ;; esac
SLUG="$(slug_of "$TARGET")"

OUT="$OUT_ROOT/linux"; STAGE="$OUT/deb-root/usr"
if [ ! -x "$STAGE/bin/nexa-beep" ]; then
    note "스테이징 없음 → build-deb.sh 실행"
    "$ROOT/packaging/linux/build-deb.sh" --target "$TARGET" ${PASS[@]+"${PASS[@]}"} >/dev/null
fi
[ -x "$STAGE/bin/nexa-beep" ] || die "스테이징 없음: $STAGE"

TOP="$OUT/rpmbuild"; rm -rf "$TOP"; mkdir -p "$TOP"/{BUILD,RPMS,SOURCES,SPECS,SRPMS}
# RPM Version 필드는 '-'를 못 쓴다 — 사전 릴리스 접미사(0.0.0-dev.abc)는 '~'로(정렬상 앞선다 · rpm 규약).
RPM_VER="${VERSION//-/\~}"

step "rpmbuild ($RARCH · $RPM_VER)"
rpmbuild -bb "$ROOT/packaging/linux/nexa-beep.spec" \
    --define "_topdir $TOP" \
    --define "_version $RPM_VER" \
    --define "_stagedir $STAGE" \
    --target "$RARCH" >/dev/null
BUILT="$(find "$TOP/RPMS" -name '*.rpm' | head -1)"
[ -n "$BUILT" ] || die "rpm 산출물 없음"
FINAL="$OUT/nexa-beep-${VERSION}-${SLUG}.rpm"
mv "$BUILT" "$FINAL"
note "$(basename "$FINAL") $(du -h "$FINAL" | cut -f1) · sha256 $(sha256_of "$FINAL")"

step "검증(설치 없이)"
rpm -qpi "$FINAL" 2>/dev/null | grep -E '^(Name|Version|Release|Architecture|License)' | sed 's/^/    /'
rpm -qpl "$FINAL" 2>/dev/null | grep -E 'bin/(nexa-beep|nbeep-imgdec)$|nexa-beep.desktop$|256x256/apps/nexa-beep.png$' | sed 's/^/    /'
rpm -K --nosignature "$FINAL" | sed 's/^/    /'
echo "RPM=$FINAL"
