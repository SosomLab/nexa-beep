#!/usr/bin/env bash
# load-bench.sh — 합성 부하 데모(INS-4 · 11-04 · 3-OS · 헤드리스 · 49 C-2 피어 500 목록 스크롤 · D-2 활성 메모리 NFR-B-2):
#   InMemory 데모(`--window` · 에코 봇 7)에 `NEXA_DEMO_PEERS=500` 가짜 피어를 더해 목록 500행 → 분리 창 모드로 대화 창 5개 열고
#   (activate 2회 = 연결·창) 한 창에 20줄 → 메인(목록) 휠 20회 → 창 수 dump → 활성 메모리 샘플(inspect-sample · 49 D-1 정의).
#   판정 = 창 ≥ 6(메인+5) · 목록 휠 paint 평균 ≤ 16ms(C-2 60fps) · private ≤ 80MB(D-2 · 피어 ≤ 100일 때만 판정 · 그 이상은 참고값).
#   썸네일 50은 잔여(이미지 재료 필요). verify.sh --perf = 피어 100(게이트) · C-2용 500은 손으로 `tools/load-bench.sh "" target/release 500`.
#
# 사용:  tools/load-bench.sh [출력 폴더] [바이너리 폴더] [피어 수]   # 기본 = target/load-<일시> · target/debug · 500
set -uo pipefail
. "$(dirname "${BASH_SOURCE[0]}")/lib/xvfb.sh"
cd "$(dirname "${BASH_SOURCE[0]}")/.."
OUT=${1:-target/load-$(date +%m%d-%H%M%S)}; mkdir -p "$OUT"; OUT=$(cd "$OUT" && pwd)
BINDIR=${2:-target/debug}
PEERS=${3:-500}
MEM_MB=80; PAINT_MS=16
export NEXA_NO_AUTOSTART=1 NEXA_HEADLESS=1 NEXA_PAINT_TRACE=1 NEXA_DEMO_PEERS="$PEERS"
EXE=""; case "${OSTYPE:-}" in msys* | cygwin*) EXE=".exe" ;; esac
[ -x "$BINDIR/nexa-beep$EXE" ] || { echo "❌ $BINDIR/nexa-beep 없음 — cargo build -p nexa-beep -p nbeep-imgdec"; exit 1; }
INST="$OUT/inst"; mkdir -p "$INST/data"; cp -f "$BINDIR/nexa-beep$EXE" "$BINDIR/nbeep-imgdec$EXE" "$INST/"
printf '_schema=1\nprofile.display_name=loadbench\nchat.window_mode=separate\napp.autostart=off\n' > "$INST/data/settings.cfg"
OUTN="$OUT"; case "${OSTYPE:-}" in msys* | cygwin*) OUTN=$(cygpath -m "$OUT") ;; esac
S="4000:activate=bob-linux;6000:activate=bob-linux;6500:activate=bora-win;7000:activate=bora-win;7500:activate=bill-mac;8000:activate=bill-mac;8500:activate=김철수;9000:activate=김철수;9500:activate=김영희;10000:activate=김영희;11000:send_n=20:load;12000:windows.dump=$OUTN/wins.txt;13000:move=200,300"
t=13500; for i in $(seq 1 10); do S="$S;$t:wheel=-120"; t=$((t+80)); done; for i in $(seq 1 10); do S="$S;$t:wheel=120"; t=$((t+80)); done
S="$S;17000:dump;30000:quit"
echo "▶ 피어 $PEERS · 대화 창 5 · 20줄 · 목록 휠 20 · 출력 $OUT"
( cd "$INST" && NEXA_SCRIPT="$S" "./nexa-beep$EXE" --window --separate-windows 2> "$OUT/app.log" >/dev/null ) &
sleep 21
bash tools/inspect-sample.sh --match "$(basename "$OUT")" --interval 1 --count 2 --out "$OUT/mem.csv" >/dev/null 2>&1
wait
wins=$(head -1 "$OUT/wins.txt" 2>/dev/null | sed -n 's/^wins=\([0-9]*\).*/\1/p'); wins=${wins:-0}
rows=$(grep -c '\[dump\] peer' "$OUT/app.log" 2>/dev/null || echo 0)
paint=$(awk '/^\[script\] /{ s=$3; ph=(s ~ /^wheel/) ? "wheel" : (s ~ /^move/ ? "move" : "other"); next }
  /^\[paint\] Main/ && ph=="wheel" { split($4,a,"="); n++; ps+=a[2]+0; if (a[2]+0>pm) pm=a[2]+0 }
  END { if (n>0) printf "%.2f %.2f %d", ps/n, pm, n; else print "- - 0" }' "$OUT/app.log")
set -- $paint; pavg=$1; pmax=$2; pn=$3
mem=$(tail -1 "$OUT/mem.csv" 2>/dev/null | awk -F',' 'NF>=8{print $3}')
grep -h '^\[demo\]' "$OUT/app.log" | sed 's/^/  /'
echo "  창 수 wins=$wins (기대 ≥ 6) · 목록 휠 paint n=$pn avg=${pavg}ms max=${pmax}ms (≤ ${PAINT_MS}) · private=${mem:-?}MB (≤ ${MEM_MB}) · dump 피어 행=$rows"
{ echo "peers=$PEERS"; echo "wins=$wins"; echo "list_wheel_paint_avg_ms=$pavg"; echo "list_wheel_paint_max_ms=$pmax"; echo "private_mb=${mem:-}"; } > "$OUT/summary.txt"
ok=1
[ "$wins" -ge 6 ] || ok=0
# 피어 0이면 목록이 스크롤되지 않아 휠 paint 표본이 없다(n=0) — 휠 판정은 생략하고 창 수·메모리만 본다(11-06).
if [ "$PEERS" -gt 0 ]; then
  [ "$pavg" != "-" ] && awk -v v="$pavg" -v b="$PAINT_MS" 'BEGIN{exit !(v<=b)}' || ok=0
else
  echo "  (피어 0 — 목록 휠 표본 없음 · 휠 판정 생략)"
fi
# D-2 게이트의 조건은 피어 100(NFR-B-2) — 그보다 많으면 메모리는 참고값(11-04 릴리스 실측: 0 → 15.3 · 100 → 26.1 · 500 → 90.1MB =
# 피어당 ≈0.15MB · 500은 게이트 밖 = 피어당 상주 비용 추적 과제).
if [ "$PEERS" -le 100 ]; then
  [ -n "${mem:-}" ] && awk -v v="$mem" -v b="$MEM_MB" 'BEGIN{exit !(v<=b)}' || ok=0
else
  echo "  (피어 $PEERS > 100 — private은 참고값 · D-2 판정은 피어 100 실행에서)"
fi
[ "$ok" = 1 ] && echo "PASS load-bench ($OUT)" || { echo "FAIL load-bench — $OUT"; exit 1; }
