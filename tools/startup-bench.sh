#!/usr/bin/env bash
# startup-bench.sh — 기동 시간 콜드/웜 N회 중앙값(PERF-4 · 11-04 · 3-OS · 헤드리스): `[startup] loop= window= first_paint=`
#   트레이스(`NEXA_PAINT_TRACE=1` · ef47b3a)를 릴리스 산출물로 N회 모아 중앙값을 낸다. 판정 = docs/05 NFR-B-11 · docs/49 INS-3
#   (first_paint ≤ 500ms). 창은 사용자 화면에 띄우지 않는다(NEXA_HEADLESS=1 — 숨은 창은 틱(≈200ms)마다 직접 그리므로
#   first_paint에 틱 1개 분량(≤200ms)이 얹힌다 · 실제 화면 수치는 이보다 작다).
#
#   콜드 = 매번 **새 신원 폴더**(신원 키 생성 · 저장소 생성 · 설정 기본값) · 웜 = 같은 폴더 재기동(신원·저장소 복원).
#   ⚠ OS 파일 캐시의 콜드(재부팅 직후)는 강제하지 않는다 — 여기 "콜드"는 앱 관점(첫 실행)이다.
#
# 사용:  tools/startup-bench.sh [출력 폴더] [바이너리 폴더] [횟수]   # 기본 = target/startup-<일시> · target/release · 5
# 결과:  <out>/cold-N.log · warm-N.log · summary.txt(값·중앙값) · 종료 코드 = 두 중앙값 모두 ≤ 500ms면 0.
set -uo pipefail
. "$(dirname "${BASH_SOURCE[0]}")/lib/xvfb.sh"
cd "$(dirname "${BASH_SOURCE[0]}")/.."
OUT=${1:-target/startup-$(date +%m%d-%H%M%S)}; mkdir -p "$OUT"; OUT=$(cd "$OUT" && pwd)
BINDIR=${2:-target/release}
N=${3:-5}
BUDGET=500
export NEXA_NO_AUTOSTART=1 NEXA_HEADLESS=1 NEXA_PAINT_TRACE=1
EXE=""; case "${OSTYPE:-}" in msys* | cygwin*) EXE=".exe" ;; esac
[ -x "$BINDIR/nexa-beep$EXE" ] || { echo "❌ $BINDIR/nexa-beep 없음 — cargo build --release -p nexa-beep -p nbeep-imgdec"; exit 1; }
INST="$OUT/inst"

fresh() { rm -rf "$INST"; mkdir -p "$INST/data"; cp -f "$BINDIR/nexa-beep$EXE" "$BINDIR/nbeep-imgdec$EXE" "$INST/"
  printf '_schema=1\nprofile.display_name=bench\nchat.window_mode=single\napp.autostart=off\n' > "$INST/data/settings.cfg"; }
one() { # 로그 → first_paint ms(없으면 빈 줄)
  ( cd "$INST" && NEXA_SCRIPT="2500:quit" "./nexa-beep$EXE" --window --live 2> "$1" >/dev/null )
  sed -n 's/^\[startup\] loop=\([0-9]*\)ms window=\([0-9]*\)ms first_paint=\([0-9]*\)ms.*$/\1 \2 \3/p' "$1" | head -1
}
median() { # 숫자 목록(공백) → 중앙값
  printf '%s\n' "$@" | sort -n | awk '{a[NR]=$1} END{ if (NR==0) {print "-"; exit} m=int((NR+1)/2); if (NR%2) print a[m]; else print int((a[m]+a[m+1])/2) }'
}

cold=(); warm=(); cold_rows=(); warm_rows=()
echo "▶ 콜드 ${N}회(새 신원 폴더) · 바이너리 $BINDIR"
for i in $(seq 1 "$N"); do
  fresh; r=$(one "$OUT/cold-$i.log")
  [ -n "$r" ] || { echo "  ✗ cold-$i: [startup] 줄 없음 — $OUT/cold-$i.log"; continue; }
  set -- $r; cold+=("$3"); cold_rows+=("cold-$i loop=$1 window=$2 first_paint=$3")
  printf '  cold-%s loop=%sms window=%sms first_paint=%sms\n' "$i" "$1" "$2" "$3"
done
echo "▶ 웜 ${N}회(마지막 콜드 폴더 재기동)"
for i in $(seq 1 "$N"); do
  r=$(one "$OUT/warm-$i.log")
  [ -n "$r" ] || { echo "  ✗ warm-$i: [startup] 줄 없음 — $OUT/warm-$i.log"; continue; }
  set -- $r; warm+=("$3"); warm_rows+=("warm-$i loop=$1 window=$2 first_paint=$3")
  printf '  warm-%s loop=%sms window=%sms first_paint=%sms\n' "$i" "$1" "$2" "$3"
done
cm=$(median "${cold[@]}"); wm=$(median "${warm[@]}")
{
  printf '%s\n' "${cold_rows[@]}" "${warm_rows[@]}"
  echo "cold_median_first_paint_ms=$cm"
  echo "warm_median_first_paint_ms=$wm"
  echo "budget_ms=$BUDGET"
} > "$OUT/summary.txt"
echo; echo "═══ 판정(first_paint 중앙값 · 예산 ${BUDGET}ms · NFR-B-11) ═══"
ok=1
for pair in "콜드:$cm" "웜:$wm"; do
  v=${pair#*:}; k=${pair%%:*}
  if [ "$v" != "-" ] && [ "${#cold[@]}" -ge 1 ] && [ "$v" -le "$BUDGET" ]; then printf '  ✓ %s 중앙값 %sms\n' "$k" "$v"
  else printf '  ✗ %s 중앙값 %s\n' "$k" "$v"; ok=0; fi
done
echo "요약 = $OUT/summary.txt"
[ "$ok" = 1 ]
