#!/usr/bin/env bash
# headless-smoke.sh — 헤드리스 기동 스모크(11-02 · 3-OS · CI ubuntu는 Xvfb 위에서): 앱을 **화면에 띄우지 않고** 기동해
#   메인 창 프레임을 PNG로 떨구고(capture) 창 목록(windows.dump)으로 "창 1 · hidden"을 판정한다. 디스커버리는 켜되(--live)
#   자동 실행 등록은 안 한다(NEXA_NO_AUTOSTART). 신원·데이터는 임시 폴더(실기 신원 무영향).
#
# 사용:  tools/headless-smoke.sh [출력 폴더] [바이너리]   # 기본 = target/headless-<일시> · target/debug/nexa-beep
# 판정:  cap.png 존재·비어 있지 않음(PNG 머리) · wins.txt 첫 줄 wins=1 · 그 창 hidden · 종료 코드 0.
set -uo pipefail
cd "$(dirname "${BASH_SOURCE[0]}")/.."
. "$(dirname "${BASH_SOURCE[0]}")/lib/xvfb.sh"
OUT=${1:-target/headless-$(date +%m%d-%H%M%S)}; mkdir -p "$OUT"; OUT=$(cd "$OUT" && pwd)
EXE=""; case "${OSTYPE:-}" in msys* | cygwin*) EXE=".exe" ;; esac
BIN=${2:-target/debug/nexa-beep$EXE}
[ -x "$BIN" ] || { echo "❌ $BIN 없음 — cargo build -p nexa-beep -p nbeep-imgdec"; exit 1; }
export NEXA_NO_AUTOSTART=1 NEXA_HEADLESS=1
INST="$OUT/inst"; mkdir -p "$INST"
cp -f "$BIN" "$INST/nexa-beep$EXE"
[ -x "$(dirname "$BIN")/nbeep-imgdec$EXE" ] && cp -f "$(dirname "$BIN")/nbeep-imgdec$EXE" "$INST/"
# ★ 앱에 주는 경로는 네이티브 형식(11-04 Windows 첫 실기: Git Bash의 `/d/...`를 앱이 "경로를 찾을 수 없습니다(os error 3)"로
#   거부해 capture·dump 둘 다 0건 = HL-1). bash 쪽 판정(-s · head)은 POSIX 경로 그대로.
OUTN="$OUT"; case "${OSTYPE:-}" in msys* | cygwin*) OUTN=$(cygpath -m "$OUT") ;; esac
# 기동 ≈1~2 s → 2.5 s 캡처 → 3.0 s 창 덤프 → 3.5 s 종료(헤드리스는 틱(≈200 ms)마다 전 창을 그린다).
S="2500:capture=$OUTN/cap.png;3000:windows.dump=$OUTN/wins.txt;3500:quit"
( cd "$INST" && NEXA_WIN_TRACE=1 NEXA_SCRIPT="$S" "./nexa-beep$EXE" --window --live 2> "$OUT/app.log" >/dev/null ); RC=$?
ok=1
[ "$RC" = 0 ] || { echo "❌ 종료 코드 $RC"; ok=0; }
if [ -s "$OUT/cap.png" ] && [ "$(head -c 4 "$OUT/cap.png" | od -An -c | tr -d ' ')" = "211PNG" ]; then
  echo "✓ capture → $OUT/cap.png ($(wc -c < "$OUT/cap.png") B)"
else
  echo "❌ cap.png 없음/손상"; ok=0
fi
W=$(hl_wins_read "$OUT/wins.txt")
if [ "$W" = 1 ] && grep -q $'\thidden$' "$OUT/wins.txt"; then
  echo "✓ windows.dump → wins=1 hidden"
else
  echo "❌ windows.dump 기대 wins=1 hidden — 실제:"; cat "$OUT/wins.txt" 2>/dev/null; ok=0
fi
grep -E '^\[(capture|script)\]' "$OUT/app.log" | sed 's/^/  /'
[ "$ok" = 1 ] && echo "PASS headless-smoke ($OUT)" || { echo "FAIL headless-smoke ($OUT) — app.log:"; tail -20 "$OUT/app.log"; exit 1; }
