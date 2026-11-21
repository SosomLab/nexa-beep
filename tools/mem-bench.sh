#!/usr/bin/env bash
# mem-bench.sh — 회수 시험·누수 기울기(PERF-5 · 11-04 → 11-21 2회 순환 개정 · 3-OS · 헤드리스 · sql 71 C-2·E · 49 D-5).
#   신원 2개(mb-x 발신 · mb-y 수신 `xfer.approval=auto`)를 새로 만들어 상호 왕래(양쪽 send) 뒤
#   ① 기준(25s) → [순환 a] 올림 = X send_n 20 + 설정·로그·격리함 창 + `sendfile` <MiB> → Y 자동 수락·격리 → 놓음 = `close=` 창 전부 +
#   `chat.close` → 유휴 60s(15s 샘플) → ④a → [순환 b] 같은 올림(파일 이름만 다름 — 이어받기로 접히지 않게) → 놓음 → 유휴 → ④b.
#   측정 = 49 D-1 정의(Windows `Working Set - Private` · Linux RssAnon · macOS `vmmap --summary`의 Physical footprint · 0.1MB 분해능) · X·Y 양쪽.
#   ★ 판정(11-21 개정) = **④b − ④a ≤ 2MB**(05 NFR-B-9 · 51 §7 #10) ∧ 마지막 유휴 기울기 ≤ 0.5MB(#11 → 0).
#     첫 부하로 차는 유한 캐시(로그 링 · 글리프 · 창을 처음 연 몫)와 할당자가 쥔 페이지는 순환 a에서 다 차므로, 순환 b의 증가만 누수로 본다
#     (11-21 mac vmmap: ④−① +5.0/+3.4MB = 전부 malloc 힙 · 기울기 0 · Debug·릴리스 같은 값). ④a − ①은 보고 + 안전 상한 16MB(넘으면 ✗ = 캐시 상한 파손 신호).
#   수신은 스트리밍(M4-12 · 11-07)이라 Y 피크가 파일 크기만큼 오르지 않는다.
#
# 사용:  tools/mem-bench.sh [출력 폴더] [바이너리 폴더] [MiB]   # 기본 = target/mem-<일시> · target/debug · Debug 64 / release 256(규격 1GiB는 인자)
set -uo pipefail
. "$(dirname "${BASH_SOURCE[0]}")/lib/xvfb.sh"
cd "$(dirname "${BASH_SOURCE[0]}")/.."
OUT=${1:-target/mem-$(date +%m%d-%H%M%S)}; mkdir -p "$OUT"; OUT=$(cd "$OUT" && pwd)
BINDIR=${2:-target/debug}
# 크기 기본 = 바이너리 폴더로 고른다(11-21 mac 실측 전송 = Debug 2.15MB/s · 릴리스 33MB/s — 순환 하나가 80s 창 안에 끝나게).
case "$BINDIR" in *release*) MIB=${3:-256} ;; *) MIB=${3:-64} ;; esac
LIMIT_MB=2    # ④b − ④a
CACHE_MB=16   # ④a − ① 안전 상한
M=${BEEP_MULTI:-$HOME/.nexa-beep-multi}
export NEXA_NO_AUTOSTART=1 NEXA_HEADLESS=1 NEXA_DEVLOG=all
trap 'BEEP_MULTI="$M" ./tools/autostart-clean.sh -q' EXIT
EXE=""; case "${OSTYPE:-}" in msys* | cygwin*) EXE=".exe" ;; esac
[ -x "$BINDIR/nexa-beep$EXE" ] || { echo "❌ $BINDIR/nexa-beep 없음 — cargo build -p nexa-beep -p nbeep-imgdec"; exit 1; }

seed() { # 폴더 · 이름 · [추가 설정]
  rm -rf "$M/$1"; mkdir -p "$M/$1/data"; cp -f "$BINDIR/nexa-beep$EXE" "$BINDIR/nbeep-imgdec$EXE" "$M/$1/"; nb_warm "$M/$1/nexa-beep$EXE" "$M/$1/nbeep-imgdec$EXE"
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
    # mac = vmmap "Physical footprint:  33.8M"(K/M/G · 소수 1자리 = 0.1MB 분해능 · 11-21). `footprint`는 MB 정수라 2MB 판정에 거칠었다.
    #   vmmap은 대상을 잠깐 세우므로 샘플 시점에만 부른다(이 함수가 곧 샘플).
    darwin*) p=$(pgrep -f "$M/$1/nexa-beep" | head -1); [ -n "$p" ] && vmmap --summary "$p" 2>/dev/null | awk '/^Physical footprint:/{v=$NF; u=substr(v,length(v)); n=substr(v,1,length(v)-1); m=(u=="K")?n/1024:(u=="G")?n*1024:(u=="M")?n:v/1048576; printf "%.2f\n", m; exit}' || echo "-" ;;
    *) p=$(pgrep -f "$M/$1/nexa-beep" | head -1); [ -n "$p" ] && awk '/RssAnon/{printf "%.2f\n", $2/1024}' "/proc/$p/status" || echo "-" ;;
  esac
}

echo "▶ 신원 준비(새로) — $M/{mb-x,mb-y} · 파일 ${MIB}MiB × 2(순환 a·b) · 출력 $OUT"
# 속도·크기 상한 해제(기본 = 100KiB/s · 100MiB — 1GiB가 몇 시간 걸린다 · 11-04 실측) · Y = 자동 수락.
RATE=$'xfer.send_rate=1g\nxfer.recv_rate=1g\nxfer.send_max_mb=unlimited\nxfer.recv_max_mb=unlimited\n'
seed mb-x mbx "$RATE"; seed mb-y mby "${RATE}xfer.approval=auto"$'\n'
mkbig() { dd if=/dev/zero of="$1" bs=1048576 count="$MIB" status=none 2>/dev/null || head -c $((MIB*1048576)) /dev/zero > "$1"; }
winpath() { case "${OSTYPE:-}" in msys* | cygwin*) cygpath -m "$1" ;; *) printf '%s' "$1" ;; esac; }
BIGA="$M/mb-x/big-${MIB}-a.bin"; BIGB="$M/mb-x/big-${MIB}-b.bin"; mkbig "$BIGA"; mkbig "$BIGB"
# 시각표(s · ui.language=ko라 창 제목은 한글) — 순환마다 같은 모양: 올림(+0 send_n · +2~4 창 · +6 sendfile) → 수신 대기 → 놓음(+84) →
#   ③(+94) → 유휴 샘플(+109 · +124 · +139) → ④(+154). 순환 a = 26s 시작 · 순환 b = 186s 시작 · quit 350s.
cyc() { # 시작(s) · 파일 — X 쪽 NEXA_SCRIPT 조각
  local s=$1; printf '%d000:send_n=20:mem;%d000:open=settings;%d000:open=log;%d000:open=quarantine;%d000:sendfile=%s;%d000:close=설정;%d200:close=로그;%d400:close=격리;%d600:chat.close' \
    "$s" $((s+2)) $((s+3)) $((s+4)) $((s+6)) "$(winpath "$2")" $((s+84)) $((s+84)) $((s+84)) $((s+84)); }
CA=26; CB=186
X="10000:activate=mby;14000:activate=mby;18000:send=hello;$(cyc $CA "$BIGA");$(( (CB-2) ))000:activate=mby;$(cyc $CB "$BIGB");350000:quit"
Y="16000:activate=mbx;20000:send=hi;$((CA+84))600:chat.close;$(( (CB-2) ))000:activate=mbx;$((CB+84))600:chat.close;350000:quit"
T0=$(date +%s)
run mb-x "$X" "$OUT/x.log"; run mb-y "$Y" "$OUT/y.log"
at() { local now; now=$(( $(date +%s) - T0 )); [ "$now" -lt "$1" ] && sleep $(( $1 - now )); }
samp() { # 라벨
  local x y t; t=$(( $(date +%s) - T0 )); x=$(mem_mb mb-x); y=$(mem_mb mb-y); printf '%-14s t=%3ss  X=%7s MB  Y=%7s MB\n' "$1" "$t" "$x" "$y"; printf '%s|%s|%s|%s\n' "$1" "$t" "$x" "$y" >> "$OUT/samples.txt"; }
ndone() { local n; n=$(grep -c 'xfer_done' "$OUT/y.log" 2>/dev/null); echo "${n:-0}"; } # grep -c는 0건이면 "0"과 rc 1 — `|| echo 0`을 붙이면 두 줄이 된다
waitdone() { # 순환 이름 · 기대 xfer_done 수 · 대기 끝(s)
  while [ "$(ndone)" -lt "$2" ] && [ $(( $(date +%s) - T0 )) -lt "$3" ]; do sleep 2; done
  [ "$(ndone)" -ge "$2" ] && echo "  순환 $1 수신 완료 t=$(( $(date +%s) - T0 ))s" || echo "  ⚠ 순환 $1: $3s까지 xfer_done 없음(②$1은 미완 상태)"; }
: > "$OUT/samples.txt"
at 25; samp "① 기준"
for c in a b; do
  s=$([ "$c" = a ] && echo $CA || echo $CB)
  waitdone "$c" "$([ "$c" = a ] && echo 1 || echo 2)" $((s+80)); samp "②$c 올림 뒤"
  at $((s+94)); samp "③$c 놓음 뒤"
  at $((s+109)); samp "유휴$c +15"
  at $((s+124)); samp "유휴$c +30"
  at $((s+139)); samp "유휴$c +45"
  at $((s+154)); samp "④$c 유휴 끝"
done
wait
rm -f "$BIGA" "$BIGB"

val() { awk -F'|' -v l="$1" -v c="$2" '$1==l{print $c; exit}' "$OUT/samples.txt"; } # 라벨 · 열(3=X 4=Y)
echo; echo "═══ 판정(④b−④a ≤ ${LIMIT_MB}MB · 마지막 유휴 기울기 ≤ 0.5MB · 참고 ④a−① ≤ ${CACHE_MB}MB) ═══"
grep -h 'xfer_done\|xfer_send_done' "$OUT/x.log" "$OUT/y.log" | sed 's/.*\[dev\] /  /' | cut -c1-150 | head -4
ok=1
for who in X Y; do
  col=$([ "$who" = X ] && echo 3 || echo 4)
  b=$(val "① 기준" "$col"); ea=$(val "④a 유휴 끝" "$col"); eb=$(val "④b 유휴 끝" "$col"); m=$(val "유휴b +30" "$col")
  bad=""; for v in "$b" "$ea" "$eb" "$m"; do { [ -z "$v" ] || [ "$v" = "-" ]; } && bad=1; done
  [ -n "$bad" ] && { echo "  ✗ $who: 샘플 없음"; ok=0; continue; } # 빈 값도 실패(11-21 mac 거짓 PASS)
  # 기울기는 ④a 아래 샘플을 ④a로 올려 계산한다(11-07 규칙의 순환판) — Windows가 유휴 중 Working Set을 걷어냈다가 다시 들이면 증가처럼 세졌다.
  d=$(awk -v a="$ea" -v z="$eb" 'BEGIN{printf "%.2f", z-a}'); c1=$(awk -v a="$b" -v z="$ea" 'BEGIN{printf "%.2f", z-a}')
  sl=$(awk -v a="$m" -v z="$eb" -v b="$ea" 'BEGIN{if(a<b)a=b; if(z<b)z=b; printf "%.2f", z-a}')
  if awk -v d="$d" -v l="$LIMIT_MB" 'BEGIN{exit !(d<=l)}' && awk -v s="$sl" 'BEGIN{exit !(s<=0.5)}' && awk -v c="$c1" -v l="$CACHE_MB" 'BEGIN{exit !(c<=l)}'; then mark="✓"; else mark="✗"; ok=0; fi
  printf '  %s %s: ④b−④a = %s MB · 기울기(유휴b +30 → ④b) %s MB · 참고 ④a−① = %s MB\n' "$mark" "$who" "$d" "$sl" "$c1"
  { echo "${who}_cycle_delta_mb=$d"; echo "${who}_tail_slope_mb=$sl"; echo "${who}_first_cycle_mb=$c1"; } >> "$OUT/summary.txt"
done
{ echo "mib=$MIB"; echo "limit_mb=$LIMIT_MB"; echo "cache_mb=$CACHE_MB"; } >> "$OUT/summary.txt"
[ "$ok" = 1 ] && echo "PASS mem-bench ($OUT)" || { echo "FAIL mem-bench — $OUT"; exit 1; }
