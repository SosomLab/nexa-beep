#!/usr/bin/env bash
# mem-bench.sh — 회수 4점 시험·누수 기울기(PERF-5 · 11-04 · 3-OS · 헤드리스 · sql 71 C-2·E · 49 D-5): 기준 → 올림 → 놓음 → 유휴.
#   신원 2개(mb-x 발신 · mb-y 수신 `xfer.approval=auto`)를 새로 만들어 상호 왕래(양쪽 send) 뒤
#   ① 기준(25s) → 올림 = X send_n 20 + 설정·로그·격리함 창 + `sendfile` <MiB>(2000줄 유지분은 설계상 보존(DR-26)이라 scroll-bench 몫) → Y 자동 수락·격리(수신 완료 대기)
#   → ② 올림 뒤(Y 로그 xfer_done 폴링) → 놓음 = `close=` 창 전부 + `chat.close` → ③ → 유휴 60s(15s 샘플 4) → ④.
#   측정 = 49 D-1 정의(Windows `Working Set - Private` · Linux RssAnon · macOS phys_footprint) · X·Y 양쪽.
#   판정 = ④−① ≤ 2MB(05 NFR-B-9 · 51 §7 #10) · 뒤 절반 기울기(마지막 3 샘플 증가) ≤ 0.5MB(#11 → 0).
#   ⚠ 수신은 메모리 조립 + 전량 봉인(M4-12 미구현)이라 ②의 Y 피크가 파일 크기만큼 오르는 것은 설계상 정상 — 보는 것은 ④ 복귀.
#
# 사용:  tools/mem-bench.sh [출력 폴더] [바이너리 폴더] [MiB]   # 기본 = target/mem-<일시> · target/debug · 256(규격 1GiB는 인자)
set -uo pipefail
. "$(dirname "${BASH_SOURCE[0]}")/lib/xvfb.sh"
cd "$(dirname "${BASH_SOURCE[0]}")/.."
OUT=${1:-target/mem-$(date +%m%d-%H%M%S)}; mkdir -p "$OUT"; OUT=$(cd "$OUT" && pwd)
BINDIR=${2:-target/debug}
MIB=${3:-256} # 규격은 1GiB(49 D-5) — Debug 로컬 ≈3.9MB/s라 기본은 256MiB(≈5분) · 릴리스·1GiB는 인자로
LIMIT_MB=2
M=${BEEP_MULTI:-$HOME/.nexa-beep-multi}
export NEXA_NO_AUTOSTART=1 NEXA_HEADLESS=1 NEXA_DEVLOG=all
trap 'BEEP_MULTI="$M" ./tools/autostart-clean.sh -q' EXIT
EXE=""; case "${OSTYPE:-}" in msys* | cygwin*) EXE=".exe" ;; esac
[ -x "$BINDIR/nexa-beep$EXE" ] || { echo "❌ $BINDIR/nexa-beep 없음 — cargo build -p nexa-beep -p nbeep-imgdec"; exit 1; }

seed() { # 폴더 · 이름 · [추가 설정]
  rm -rf "$M/$1"; mkdir -p "$M/$1/data"; cp -f "$BINDIR/nexa-beep$EXE" "$BINDIR/nbeep-imgdec$EXE" "$M/$1/"
  printf '_schema=1\nprofile.display_name=%s\nchat.window_mode=single\napp.autostart=off\nui.language=ko\n%s' "$2" "${3:-}" > "$M/$1/data/settings.cfg"; }
# ★ 절대 경로로 띄운다(11-12 Linux 실측) — `./nexa-beep`로 띄우면 명령줄에 신원 폴더가 없어 mem_mb의 `pgrep -f "$M/<폴더>/nexa-beep"`가
#   못 찾고 전 샘플 "-" → "샘플 없음" FAIL(perf-scenario는 6055fd5에서 PID 직접으로 고친 같은 함정). 작업 폴더는 그대로 신원 폴더.
run() { ( cd "$M/$1" && NEXA_SCRIPT="$2" "$M/$1/nexa-beep$EXE" --window --live 2> "$3" >/dev/null ) & }
# 사설 메모리(MB · 49 D-1) — 프로세스는 실행 파일 경로(신원 폴더)로 고른다.
mem_mb() { # 폴더 이름
  case "${OSTYPE:-}" in
    # 인스턴스 이름(nexa-beep · nexa-beep#1 …)은 PID와 무관하므로 'ID Process' 카운터로 PID → 인스턴스를 찾는다(11-04 실측: 이름만 보면 두 인스턴스가 같은 값).
    #   ⚠ 이 PC는 인스턴스 이름이 전부 "nexa-beep"(접미 #1 없음)이라 이름으로는 못 가른다 → 'ID Process'와 'Working Set - Private'를
    #   **한 호출**로 받아 같은 순서(인스턴스별 1:1)의 색인으로 PID에 맞춘다(11-04 실측).
    msys* | cygwin*) powershell.exe -NoProfile -Command "\$p = Get-Process nexa-beep -ErrorAction SilentlyContinue | Where-Object { \$_.Path -like '*\\$1\\*' } | Select-Object -First 1; if (\$p) { \$cs = (Get-Counter -Counter @('\Process(nexa-beep*)\ID Process', '\Process(nexa-beep*)\Working Set - Private') -ErrorAction SilentlyContinue).CounterSamples; \$ids = @(\$cs | Where-Object { \$_.Path -like '*ID Process' }); \$wss = @(\$cs | Where-Object { \$_.Path -like '*Working Set - Private' }); \$v = \$null; for (\$i = 0; \$i -lt \$ids.Count; \$i++) { if ([int]\$ids[\$i].CookedValue -eq \$p.Id -and \$i -lt \$wss.Count) { \$v = \$wss[\$i].CookedValue } }; if (\$v -ne \$null) { [math]::Round(\$v/1MB, 2) } else { [math]::Round(\$p.PrivateMemorySize64/1MB, 2) } } else { '-' }" | tr -d '\r' ;;
    darwin*) p=$(pgrep -f "$M/$1/nexa-beep" | head -1); [ -n "$p" ] && footprint "$p" 2>/dev/null | awk '/phys_footprint/{gsub(/[^0-9.]/,"",$NF); print $NF; exit}' || echo "-" ;;
    *) p=$(pgrep -f "$M/$1/nexa-beep" | head -1); [ -n "$p" ] && awk '/RssAnon/{printf "%.2f\n", $2/1024}' "/proc/$p/status" || echo "-" ;;
  esac
}

echo "▶ 신원 준비(새로) — $M/{mb-x,mb-y} · 파일 ${MIB}MiB · 출력 $OUT"
# 속도·크기 상한 해제(기본 = 100KiB/s · 100MiB — 1GiB가 몇 시간 걸린다 · 11-04 실측) · Y = 자동 수락.
RATE=$'xfer.send_rate=1g\nxfer.recv_rate=1g\nxfer.send_max_mb=unlimited\nxfer.recv_max_mb=unlimited\n'
seed mb-x mbx "$RATE"; seed mb-y mby "${RATE}xfer.approval=auto"$'\n'
BIG="$M/mb-x/big-${MIB}.bin"; dd if=/dev/zero of="$BIG" bs=1048576 count="$MIB" status=none 2>/dev/null || head -c $((MIB*1048576)) /dev/zero > "$BIG"
BIGN="$BIG"; case "${OSTYPE:-}" in msys* | cygwin*) BIGN=$(cygpath -m "$BIG") ;; esac
# 시각표(ms · ui.language=ko라 창 제목은 한글): 왕래(X 18s send · Y 20s send = mutual) → ① 25s → 올림 26~32s → 수신 완료
#   대기(Y 로그 `xfer_done` 폴링 · 최대 200s) → ② → 놓음 210s(창 닫기·대화 닫기) → ③ 220s → 유휴 → ④ 280s → quit 290s.
X="10000:activate=mby;14000:activate=mby;18000:send=hello;26000:send_n=20:mem;28000:open=settings;29000:open=log;30000:open=quarantine;32000:sendfile=$BIGN;210000:close=설정;210200:close=로그;210400:close=격리;210600:chat.close;290000:quit"
Y="16000:activate=mbx;20000:send=hi;210600:chat.close;290000:quit"
T0=$(date +%s)
run mb-x "$X" "$OUT/x.log"; run mb-y "$Y" "$OUT/y.log"
at() { local now; now=$(( $(date +%s) - T0 )); [ "$now" -lt "$1" ] && sleep $(( $1 - now )); }
samp() { # 라벨
  local x y t; t=$(( $(date +%s) - T0 )); x=$(mem_mb mb-x); y=$(mem_mb mb-y); printf '%-14s t=%3ss  X=%7s MB  Y=%7s MB\n' "$1" "$t" "$x" "$y"; printf '%s|%s|%s|%s\n' "$1" "$t" "$x" "$y" >> "$OUT/samples.txt"; }
: > "$OUT/samples.txt"
at 25; samp "① 기준"
# 수신 완료 대기 — Y 상세 로그의 xfer_done(격리 완료) · 상한 200s(Debug 로컬 실측 ≈3.9MB/s → 256MiB ≈ 70s + 해시·봉인).
while ! grep -q 'xfer_done' "$OUT/y.log" 2>/dev/null && [ $(( $(date +%s) - T0 )) -lt 200 ]; do sleep 2; done
grep -q 'xfer_done' "$OUT/y.log" 2>/dev/null && echo "  수신 완료 t=$(( $(date +%s) - T0 ))s" || echo "  ⚠ 200s 안에 xfer_done 없음(②는 미완 상태)"
samp "② 올림 뒤"
at 220; samp "③ 놓음 뒤"
at 235; samp "유휴 235"
at 250; samp "유휴 250"
at 265; samp "유휴 265"
at 280; samp "④ 유휴 280"
wait
rm -f "$BIG"

echo; echo "═══ 판정(④−① ≤ ${LIMIT_MB}MB · 뒤 절반 기울기 ≤ 0.5MB) ═══"
grep -h 'xfer_done\|xfer_send_done\|offer_judge' "$OUT/x.log" "$OUT/y.log" | sed 's/.*\[dev\] /  /' | cut -c1-150 | head -4
ok=1
for who in X Y; do
  col=$([ "$who" = X ] && echo 3 || echo 4)
  b=$(sed -n 1p "$OUT/samples.txt" | awk -F'|' -v c="$col" '{print $c}'); e=$(sed -n 7p "$OUT/samples.txt" | awk -F'|' -v c="$col" '{print $c}')
  m5=$(sed -n 5p "$OUT/samples.txt" | awk -F'|' -v c="$col" '{print $c}')
  if [ "$b" = "-" ] || [ "$e" = "-" ]; then echo "  ✗ $who: 샘플 없음"; ok=0; continue; fi
  # 기울기는 ① 기준선 아래 샘플을 기준선으로 올려 계산한다(11-07) — Windows가 유휴 중 Working Set을 걷어냈다가
  # 다시 들이면(0.95 → 3.43MB 실측) "기준 아래에서 회복"이 증가처럼 세졌다. 누수 판정 의도(④−① · 기준 위로의 증가)는 그대로.
  d=$(awk -v a="$b" -v z="$e" 'BEGIN{printf "%.2f", z-a}'); sl=$(awk -v a="$m5" -v z="$e" -v b="$b" 'BEGIN{if(a<b)a=b; if(z<b)z=b; printf "%.2f", z-a}')
  if awk -v d="$d" -v l="$LIMIT_MB" 'BEGIN{exit !(d<=l)}' && awk -v s="$sl" 'BEGIN{exit !(s<=0.5)}'; then printf '  ✓ %s: ④−① = %s MB · 250→280s 기울기 %s MB\n' "$who" "$d" "$sl"
  else printf '  ✗ %s: ④−① = %s MB · 250→280s 기울기 %s MB\n' "$who" "$d" "$sl"; ok=0; fi
  echo "${who}_delta_mb=$d" >> "$OUT/summary.txt"; echo "${who}_tail_slope_mb=$sl" >> "$OUT/summary.txt"
done
echo "mib=$MIB" >> "$OUT/summary.txt"; echo "limit_mb=$LIMIT_MB" >> "$OUT/summary.txt"
[ "$ok" = 1 ] && echo "PASS mem-bench ($OUT)" || { echo "FAIL mem-bench — $OUT"; exit 1; }
