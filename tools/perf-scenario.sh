#!/usr/bin/env bash
# perf-scenario.sh — docs/51 §2 성능 시나리오 재현(3-OS · PERF-1). A가 B와 **분리 대화 창**을 열고
#   30줄 전송(120ms) → 휠 20회(80ms) → 이동 10회 → 우클릭 → 메뉴 안 이동 10회 → dump → quit.
#   페인트 트레이스(`NEXA_PAINT_TRACE`)·창 트레이스는 <out>/A.log · B.log, 기동 7초 뒤 메모리는 <out>/mem-idle.txt.
#   10-10 mac 실측은 세션 scratchpad에 있었다 — Windows·Linux(PERF-1)에서 같은 절차를 쓰려고 도구로 올린다.
#
# 사용:  tools/perf-scenario.sh [출력 폴더]      # 기본 = ./target/perf-<일시>
# 전제:  릴리스 산출물(target/release/nexa-beep + nbeep-imgdec) · 3신원 폴더 $HOME/.nexa-beep-multi/{A,B}(relaunch.sh가 만든다)
#        실행 중인 nexa-beep은 미리 종료한다(발견·포트가 겹치면 수치가 흐려진다).
# ⚠ 스크립트 단계는 **이벤트 루프 틱(≈200ms)으로 묶여** 실행된다(10-10 Win 실측 — 80ms 간격 휠 20회가 9프레임).
#   그래서 "휠 N회 → 프레임 수"는 입력 밀림의 근거가 아니다. 근거는 paint·present ms 자체(docs/51 §3).
set -uo pipefail
cd "$(dirname "${BASH_SOURCE[0]}")/.."
OUT=${1:-target/perf-$(date +%m%d-%H%M%S)}; mkdir -p "$OUT"; OUT=$(cd "$OUT" && pwd)
M=${BEEP_MULTI:-$HOME/.nexa-beep-multi}
EXE=""; case "${OSTYPE:-}" in msys* | cygwin*) EXE=".exe" ;; esac
[ -x "target/release/nexa-beep$EXE" ] || { echo "릴리스 산출물이 없다 — cargo build --release -p nexa-beep -p nbeep-imgdec"; exit 1; }
for d in A B; do mkdir -p "$M/$d"; cp -f "target/release/nexa-beep$EXE" "target/release/nbeep-imgdec$EXE" "$M/$d/"; done
BNAME=$("$M/B/nexa-beep$EXE" --whoami 2>/dev/null | awk -F'= ' '/^name/{print $2; exit}')
[ -n "$BNAME" ] || { echo "B 신원 이름을 못 읽었다($M/B)"; exit 1; }
# 시각표(ms) — activate 2회(1회차 = 연결만 · 2회차 = 창 열기)
S="9000:activate=$BNAME;13000:activate=$BNAME"; t=16000
for i in $(seq 1 30); do S="$S;$t:send=perf-line-$i-가나다라마바사아자차카타파하-the-quick-brown-fox"; t=$((t+120)); done
t=$((t+1500))
for _ in $(seq 1 10); do S="$S;$t:wheel=120"; t=$((t+80)); done
for _ in $(seq 1 10); do S="$S;$t:wheel=-120"; t=$((t+80)); done
t=$((t+500))
for i in $(seq 0 9); do S="$S;$t:move=$((300+i*20)),$((200+i*10))"; t=$((t+80)); done
t=$((t+300)); S="$S;$t:rclick=450,260"; t=$((t+400))
for i in $(seq 0 9); do S="$S;$t:move=$((470+i*2)),$((275+i*4))"; t=$((t+80)); done
t=$((t+800)); S="$S;$t:dump"; t=$((t+1500)); S="$S;$t:quit"
echo "$S" > "$OUT/script-A.txt"
( cd "$M/B" && NEXA_WIN_TRACE=1 NEXA_SCRIPT="$((t+3000)):quit" "./nexa-beep$EXE" --window --live 2> "$OUT/B.log" ) &
sleep 2
( cd "$M/A" && NEXA_PAINT_TRACE=1 NEXA_WIN_TRACE=1 NEXA_SCRIPT="$S" "./nexa-beep$EXE" --window --live --separate-windows 2> "$OUT/A.log" ) &
sleep 7
case "${OSTYPE:-}" in
  msys* | cygwin*) powershell.exe -NoProfile -Command "Get-Process nexa-beep | Select-Object Id,Path,WorkingSet64,PrivateMemorySize64,HandleCount | Format-Table -AutoSize | Out-String; (Get-Counter '\Process(nexa-beep)\Working Set - Private').CounterSamples | ForEach-Object { 'ws-private=' + [math]::Round(\$_.CookedValue/1MB,1) + 'MB' }" | tr -d '\r' > "$OUT/mem-idle.txt" ;;
  darwin*) for p in $(pgrep -f "$M/[AB]/nexa-beep"); do footprint "$p" 2>/dev/null | grep -m1 -i "phys_footprint"; done > "$OUT/mem-idle.txt" ;;
  *) for p in $(pgrep -f "$M/[AB]/nexa-beep"); do grep -E "RssAnon|VmRSS" "/proc/$p/status"; done > "$OUT/mem-idle.txt" ;;
esac
wait
# 집계 — 단계별 paint/present 평균·최대(A.log의 [script] 표식으로 구간을 가른다)
awk '
/^\[script\] /{ s=$3; if(s~/^activate/)ph="activate"; else if(s~/^send/)ph="send30"; else if(s~/^wheel/)ph="wheel"; else if(s~/^rclick/)ph="rclick+menu"; else if(s~/^move/&&ph!="rclick+menu")ph="move"; else if(s=="dump")ph="dump"; next }
/^\[paint\] /{ role=$2; sz=$3; split($4,a,"="); split($5,b,"="); p=a[2]+0; q=b[2]+0; k=ph" "role" "sz; n[k]++; ps[k]+=p; qs[k]+=q; if(p>pm[k])pm[k]=p; if(q>qm[k])qm[k]=q }
END{ for(k in n) printf "%-40s n=%3d paint avg=%5.2f max=%5.2f  present avg=%5.2f max=%5.2f\n", k, n[k], ps[k]/n[k], pm[k], qs[k]/n[k], qm[k] }' "$OUT/A.log" | sort | tee "$OUT/summary.txt"
echo "threads=$(grep -c 'dump\] thread' "$OUT/A.log") · 출력 = $OUT"
