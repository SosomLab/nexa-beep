#!/usr/bin/env bash
# boot-trace.sh — 부팅 트레이스 5점 확인(INS-3 · 11-04 · 3-OS · 헤드리스): 새 신원으로 1회 기동해 `[startup] … settings=`와
#   `NEXA_BOOT_TRACE=<파일>` 줄(settings_ms · loop_ms · window_ms · first_paint_ms · first_peer_ms)이 나오는지 본다.
#   첫 피어는 같은 망에 다른 인스턴스가 있을 때만(없으면 줄이 없는 것이 정상).
#
# 사용:  tools/boot-trace.sh [출력 폴더] [바이너리 폴더]   # 기본 = target/boot-<일시> · target/debug
set -uo pipefail
. "$(dirname "${BASH_SOURCE[0]}")/lib/xvfb.sh"
cd "$(dirname "${BASH_SOURCE[0]}")/.."
OUT=${1:-target/boot-$(date +%m%d-%H%M%S)}; mkdir -p "$OUT"; OUT=$(cd "$OUT" && pwd)
BINDIR=${2:-target/debug}
export NEXA_NO_AUTOSTART=1 NEXA_HEADLESS=1 NEXA_PAINT_TRACE=1
EXE=""; case "${OSTYPE:-}" in msys* | cygwin*) EXE=".exe" ;; esac
[ -x "$BINDIR/nexa-beep$EXE" ] || { echo "❌ $BINDIR/nexa-beep 없음 — cargo build -p nexa-beep -p nbeep-imgdec"; exit 1; }
INST="$OUT/inst"; mkdir -p "$INST/data"; cp -f "$BINDIR/nexa-beep$EXE" "$BINDIR/nbeep-imgdec$EXE" "$INST/"
printf '_schema=1\nprofile.display_name=boottrace\nchat.window_mode=single\napp.autostart=off\n' > "$INST/data/settings.cfg"
TRACE="$OUT/boot-trace.txt"; TRACEN="$TRACE"; case "${OSTYPE:-}" in msys* | cygwin*) TRACEN=$(cygpath -m "$TRACE") ;; esac
( cd "$INST" && NEXA_BOOT_TRACE="$TRACEN" NEXA_SCRIPT="8000:quit" "./nexa-beep$EXE" --window --live 2> "$OUT/app.log" >/dev/null ); RC=$?
echo "▶ stderr:"; grep '\[startup\]' "$OUT/app.log" | sed 's/^/  /'
echo "▶ $TRACE:"; sed 's/^/  /' "$TRACE" 2>/dev/null
ok=1
[ "$RC" = 0 ] || { echo "❌ 종료 코드 $RC"; ok=0; }
grep -q '^settings_ms=[0-9]* loop_ms=[0-9]* window_ms=[0-9]* first_paint_ms=[0-9]* budget_ms=' "$TRACE" 2>/dev/null || { echo "❌ 트레이스 파일에 5점 줄 없음"; ok=0; }
grep -q '\[startup\] loop=.* settings=[0-9]*ms' "$OUT/app.log" || { echo "❌ stderr [startup]에 settings= 없음"; ok=0; }
[ "$ok" = 1 ] && echo "PASS boot-trace ($OUT)" || { echo "FAIL boot-trace ($OUT)"; exit 1; }
