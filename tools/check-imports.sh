#!/usr/bin/env bash
# check-imports.sh — NFR-B-5 "외부 런타임 의존 0" 실측 게이트 · mac/Linux 판(INS-1 · 11-04 · 49 A-3 · Windows 판 = check-imports.ps1).
#   macOS: `otool -L` — 허용 = OS 인박스뿐(`/usr/lib/*` dyld 공유 캐시 · `/System/Library/*` 프레임워크). Homebrew(`/opt/homebrew` ·
#          `/usr/local`)·`@rpath`·상대 경로 dylib = 실패(깨끗한 Mac에서 안 뜬다).
#   Linux: `ldd` — 허용 = glibc 핵심(libc·libm·libpthread·libdl·librt·ld-linux·linux-vdso)·libgcc_s만. musl 정적("not a dynamic
#          executable") = 통과. 그 밖(libX11·libxkbcommon·libssl…)은 실패 — 우리는 X11을 x11rb 순수 Rust로, Wayland·xkb는 dlopen으로
#          써서 링크 테이블에 없어야 한다(있으면 의존이 새로 들어온 것 · 사람 검토 뒤 여기 등재).
#
# 사용:  tools/check-imports.sh <bin>…        # 하나라도 위반 = 종료 1
#        tools/check-imports.sh --list <bin>  # 대조 없이 목록만(화이트리스트 갱신용)
set -uo pipefail
LIST=0; [ "${1:-}" = "--list" ] && { LIST=1; shift; }
[ $# -ge 1 ] || { echo "사용: $0 [--list] <bin>…" >&2; exit 2; }
os=$(uname -s)
bad=0
for bin in "$@"; do
  [ -f "$bin" ] || { echo "❌ $bin 없음"; bad=1; continue; }
  echo "▶ $bin"
  case "$os" in
    Darwin)
      deps=$(otool -L "$bin" 2>/dev/null | tail -n +2 | awk '{print $1}')
      while IFS= read -r d; do
        [ -n "$d" ] || continue
        if [ "$LIST" = 1 ]; then echo "  $d"; continue; fi
        case "$d" in
          /usr/lib/*|/System/Library/*) echo "  ✓ $d" ;;
          *) echo "  ✗ $d  — OS 인박스 아님(Homebrew·@rpath·번들 밖 dylib)"; bad=1 ;;
        esac
      done <<< "$deps"
      ;;
    Linux)
      out=$(ldd "$bin" 2>&1 || true)
      if printf '%s' "$out" | grep -qi 'not a dynamic executable\|statically linked'; then
        echo "  ✓ 정적(musl) — 동적 의존 0"; continue
      fi
      deps=$(printf '%s\n' "$out" | awk '{print $1}')
      while IFS= read -r d; do
        [ -n "$d" ] || continue
        if [ "$LIST" = 1 ]; then echo "  $d"; continue; fi
        case "$d" in
          linux-vdso.so.*|libc.so.6|libm.so.6|libpthread.so.0|libdl.so.2|librt.so.1|libgcc_s.so.1|/lib*/ld-linux-*|/lib/ld-*|ld-linux-*)
            echo "  ✓ $d" ;;
          *) echo "  ✗ $d  — glibc 핵심·libgcc_s 밖(새 런타임 의존 · 사람 검토 뒤 등재)"; bad=1 ;;
        esac
      done <<< "$deps"
      ;;
    *) echo "  (이 OS는 Windows 판 tools/check-imports.ps1을 쓴다: $os)"; continue ;;
  esac
done
if [ "$LIST" = 1 ]; then exit 0; fi
[ "$bad" = 0 ] && echo "PASS 임포트 화이트리스트(NFR-B-5 · $os)" || { echo "FAIL 임포트 화이트리스트(NFR-B-5 · $os) — 새 의존은 인박스인지 확인 뒤 화이트리스트에"; exit 1; }
