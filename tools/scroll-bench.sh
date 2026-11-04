#!/usr/bin/env bash
# scroll-bench.sh — 2000줄 기록 스크롤 실측(PERF-3 · 11-04 · 3-OS · 헤드리스): 줄바꿈 캐시(48d3392)가 **기록 길이와 무관**한지
#   (30줄만 실측 · 51 §7 #2) 2000줄로 본다. 신원 2개(sb-x · sb-y)를 새로 만들어 X가 Y와 대화 창을 열고 `send_n=<N>`(한 틱에 N줄)
#   → 휠 20회(80ms) → dump → quit → **X 재기동**(기록 N줄 복원 · `[startup] loop` = 복원 비용 포함) → 휠 20회 → quit.
#   집계 = 구간별 `[paint]` avg/max(send · wheel · 재기동 wheel) · `send_n` 소요 · `[startup]` · 판정 = 휠 paint 평균 ≤16ms(05 B-12).
#   ⚠ 헤드리스는 직접 그리기 경로라 51 §3 실화면 기준선과 **비교하지 않는다**(회귀 유무만 · 같은 조건 반복).
#
# 사용:  tools/scroll-bench.sh [출력 폴더] [바이너리 폴더] [줄 수]   # 기본 = target/scroll-<일시> · target/debug · 2000
set -uo pipefail
. "$(dirname "${BASH_SOURCE[0]}")/lib/xvfb.sh"
cd "$(dirname "${BASH_SOURCE[0]}")/.."
OUT=${1:-target/scroll-$(date +%m%d-%H%M%S)}; mkdir -p "$OUT"; OUT=$(cd "$OUT" && pwd)
BINDIR=${2:-target/debug}
N=${3:-2000}
BUDGET_MS=16
M=${BEEP_MULTI:-$HOME/.nexa-beep-multi}
export NEXA_NO_AUTOSTART=1 NEXA_HEADLESS=1 NEXA_PAINT_TRACE=1
trap 'BEEP_MULTI="$M" ./tools/autostart-clean.sh -q' EXIT
EXE=""; case "${OSTYPE:-}" in msys* | cygwin*) EXE=".exe" ;; esac
[ -x "$BINDIR/nexa-beep$EXE" ] || { echo "❌ $BINDIR/nexa-beep 없음 — cargo build -p nexa-beep -p nbeep-imgdec"; exit 1; }

seed() { rm -rf "$M/$1"; mkdir -p "$M/$1/data"; cp -f "$BINDIR/nexa-beep$EXE" "$BINDIR/nbeep-imgdec$EXE" "$M/$1/"
  printf '_schema=1\nprofile.display_name=%s\nchat.window_mode=single\napp.autostart=off\n' "$2" > "$M/$1/data/settings.cfg"; }
run() { ( cd "$M/$1" && NEXA_SCRIPT="$2" "./nexa-beep$EXE" --window --live 2> "$3" >/dev/null ) & }
# 휠 20회(80ms · 위 10 → 아래 10 · 120 = 한 노치) — 먼저 커서를 창 가운데(대화 본문)로 옮긴다(헤드리스 = OS 포커스 없음 →
#   script_target = 메인 창 · 단일 창 모드라 본문이 메인에 있다 · 커서 (0,0)은 툴바).
wheels() { local t=$1 s=";$1:move=240,380"; t=$((t+100)); for i in $(seq 1 10); do s="$s;$t:wheel=120"; t=$((t+80)); done; for i in $(seq 1 10); do s="$s;$t:wheel=-120"; t=$((t+80)); done; echo "$s"; }

echo "▶ 신원 준비(새로) — $M/{sb-x,sb-y} · N=$N · 출력 $OUT"
seed sb-x sbx; seed sb-y sby
echo "▶ ① X: 10s·14s activate → 18s send_n=$N → 30s 휠 20 → 36s dump · 38s quit / Y: 40s quit"
run sb-x "10000:activate=sby;14000:activate=sby;18000:send_n=$N:perf$(wheels 30000);36000:dump;38000:quit" "$OUT/x.log"
run sb-y "40000:quit" "$OUT/y.log"
wait
echo "▶ ② X·Y 재기동 — X: 기록 $N줄 복원 · 10s·14s activate(연결·창) · 18s 휠 20 · 24s dump · 26s quit / Y: 28s quit"
# Y도 다시 띄운다(activate는 발견된 상대만 — ①의 Y는 끝났다).
run sb-x "10000:activate=sby;14000:activate=sby$(wheels 18000);24000:dump;26000:quit" "$OUT/x-b.log"
run sb-y "28000:quit" "$OUT/y-b.log"
wait

echo; echo "═══ 집계 ═══"
summ() { # 로그 · 라벨 — [script] 표식으로 구간을 갈라 [paint] 평균/최대
  awk -v lab="$2" '
  /^\[script\] /{ s=$3; if(s~/^activate/)ph="activate"; else if(s~/^send_n/)ph="send_n"; else if(s~/^wheel/)ph="wheel"; else if(s~/^move/)ph="move"; else if(s=="dump")ph="dump"; next }
  /^\[paint\] / && ph=="wheel" { split($4,a,"="); p=a[2]+0; n++; ps+=p; if(p>pm)pm=p }
  END{ if(n>0) printf "%s wheel paint n=%d avg=%.2fms max=%.2fms\n", lab, n, ps/n, pm; else printf "%s wheel paint n=0\n", lab }' "$1"; }
summ "$OUT/x.log" "① 발신 직후"
summ "$OUT/x-b.log" "② 재기동(복원 $N줄)"
grep -h '\[script\] send_n' "$OUT/x.log" | sed 's/^/  /'
grep -h '\[startup\]' "$OUT/x.log" "$OUT/x-b.log" | sed 's/^/  /'
LINES=$(grep -c '\[dump\] thread' "$OUT/x-b.log" 2>/dev/null || echo 0)
echo "  재기동 dump thread 줄 = $LINES (기대 ≥ $N)"
a1=$(summ "$OUT/x.log" a | sed -n 's/.*avg=\([0-9.]*\)ms.*/\1/p'); a2=$(summ "$OUT/x-b.log" b | sed -n 's/.*avg=\([0-9.]*\)ms.*/\1/p')
ok=1
for v in "$a1" "$a2"; do
  if [ -z "$v" ] || ! awk -v v="$v" -v b="$BUDGET_MS" 'BEGIN{exit !(v<=b)}'; then ok=0; fi
done
[ "$LINES" -ge "$N" ] || ok=0
{ echo "lines=$N"; echo "wheel_avg_ms_after_send=$a1"; echo "wheel_avg_ms_after_restart=$a2"; echo "restored_thread_lines=$LINES"; echo "budget_ms=$BUDGET_MS"; } > "$OUT/summary.txt"
[ "$ok" = 1 ] && echo "PASS scroll-bench (휠 paint 평균 ≤ ${BUDGET_MS}ms · 복원 ≥ $N줄 · $OUT)" || { echo "FAIL scroll-bench — $OUT"; exit 1; }
