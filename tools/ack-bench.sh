#!/usr/bin/env bash
# ack-bench.sh — 메시지 왕복(ack) 지연 실측(INS-6 · 11-04 · 3-OS · 헤드리스 · 49 F-1 · NFR-B-9 "LAN 1:1 ≤50ms P95"):
#   신원 2개(ab-x · ab-y)를 새로 만들어 TOFU 성립 뒤 X가 100ms 간격으로 N줄 보내고, Y의 Delivered ack(설정 `chat.send_delivered=on` ·
#   핀 고정 상대에게 자동)가 X에 돌아오는 왕복 ms를 앱 계측(`NEXA_ACK_TRACE=1` → 종료 때 `[ack] n= p50= p95= max=`)으로 모은다.
#   49 F-1 원안은 CLI `--chat-live` 양단이었으나 제품 경로(GUI 세션 액터·Noise·mux)를 그대로 재는 쪽이 정확해 GUI 헤드리스로 잰다.
#   ⚠ 로컬 루프백 수치는 LAN 하한이다(실 LAN은 + 왕복 지연 · 51 §3 기준선과 같은 주의).
#
# 사용:  tools/ack-bench.sh [출력 폴더] [바이너리 폴더] [횟수]   # 기본 = target/ack-<일시> · target/debug · 100
set -uo pipefail
. "$(dirname "${BASH_SOURCE[0]}")/lib/xvfb.sh"
cd "$(dirname "${BASH_SOURCE[0]}")/.."
OUT=${1:-target/ack-$(date +%m%d-%H%M%S)}; mkdir -p "$OUT"; OUT=$(cd "$OUT" && pwd)
BINDIR=${2:-target/debug}
N=${3:-100}
P95_MS=50
M=${BEEP_MULTI:-$HOME/.nexa-beep-multi}
export NEXA_NO_AUTOSTART=1 NEXA_HEADLESS=1 NEXA_ACK_TRACE=1
trap 'BEEP_MULTI="$M" ./tools/autostart-clean.sh -q' EXIT
EXE=""; case "${OSTYPE:-}" in msys* | cygwin*) EXE=".exe" ;; esac
[ -x "$BINDIR/nexa-beep$EXE" ] || { echo "❌ $BINDIR/nexa-beep 없음 — cargo build -p nexa-beep -p nbeep-imgdec"; exit 1; }
seed() { rm -rf "$M/$1"; mkdir -p "$M/$1/data"; cp -f "$BINDIR/nexa-beep$EXE" "$BINDIR/nbeep-imgdec$EXE" "$M/$1/"
  printf '_schema=1\nprofile.display_name=%s\nchat.window_mode=single\napp.autostart=off\nchat.send_delivered=on\n' "$2" > "$M/$1/data/settings.cfg"; }
run() { ( cd "$M/$1" && NEXA_SCRIPT="$2" "./nexa-beep$EXE" --window --live 2> "$3" >/dev/null ) & }

echo "▶ 신원 준비(새로) — $M/{ab-x,ab-y} · N=$N · 출력 $OUT"
seed ab-x abx; seed ab-y aby
S="10000:activate=aby;14000:activate=aby"; t=20000
for i in $(seq 1 "$N"); do S="$S;$t:send=ack-$i"; t=$((t+100)); done
END=$((t+6000))
echo "▶ X: 10s·14s activate → 20s부터 ${N}줄(100ms) → $((END/1000))s quit / Y: 16s activate · $((END/1000+2))s quit"
run ab-x "$S;$END:quit" "$OUT/x.log"
run ab-y "16000:activate=abx;$((END+2000)):quit" "$OUT/y.log"
wait
line=$(grep -h '^\[ack\]' "$OUT/x.log" | tail -1)
echo "  $line"
p95=$(printf '%s' "$line" | sed -n 's/.* p95=\([0-9]*\)ms.*/\1/p'); n=$(printf '%s' "$line" | sed -n 's/.*n=\([0-9]*\) .*/\1/p')
{ echo "line=$line"; echo "n=$n"; echo "p95_ms=$p95"; echo "gate_p95_ms=$P95_MS"; } > "$OUT/summary.txt"
if [ -n "$p95" ] && [ "${n:-0}" -ge $((N * 9 / 10)) ] && [ "$p95" -le "$P95_MS" ]; then
  echo "PASS ack-bench (n=$n ≥ 90% · p95 ${p95}ms ≤ ${P95_MS}ms · $OUT)"
else
  echo "FAIL ack-bench — [ack] 줄 없음 또는 n<90% 또는 p95>${P95_MS}ms ($OUT)"; exit 1
fi
