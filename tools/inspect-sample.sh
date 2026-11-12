#!/usr/bin/env bash
# inspect-sample.sh — 3-OS 상주 샘플러(INS-5 · 11-04 · 49 §5 D-1·D-3·D-4·D-6 · NFR-B-1·6·7): 실행 중인 nexa-beep 한 프로세스의
#   사설 메모리·커밋·핸들(fd)·스레드·누적 CPU ms·소켓 수를 N초 간격으로 CSV에 적고, 끝에 Δ(마지막−처음)·최대·뒤 절반 기울기를 요약한다.
#   측정 정의 = 49 D-1(Windows `Working Set - Private` · Linux RssAnon · macOS phys_footprint). 읽기 전용(프로세스를 건드리지 않는다).
#
# 사용:  tools/inspect-sample.sh [--match <경로 조각>] [--interval 초] [--duration 초 | --count N] [--out csv] [--gate]
#   --match     프로세스 선택 = 실행 파일 경로 조각(기본 nexa-beep · 설치본 "NexaBeep" · 시험 "mb-x" 등) — 여러 개면 첫 것
#   --interval  샘플 간격(기본 5s) · --duration 총 시간(기본 60s · 24h = 86400) · --count 샘플 수(지정하면 duration 무시)
#   --out       CSV 경로(기본 target/inspect-<일시>.csv)
#   --gate      요약으로 판정(종료 1): Δ사설메모리 ≤ 2MB(NFR-B-1 24h 누수) · Δ핸들 ≤ 0(+5 여유 · NFR-B-6) · 뒤 절반 기울기 ≤ 0.5MB
# 예:    tools/inspect-sample.sh --match NexaBeep --interval 60 --duration 86400 --gate   # 설치본 24h 상주(M5-5)
set -uo pipefail
cd "$(dirname "${BASH_SOURCE[0]}")/.."
MATCH="nexa-beep"; INTERVAL=5; DURATION=60; COUNT=0; OUT=""; GATE=0
while [ $# -gt 0 ]; do
  case "$1" in
    --match) MATCH="$2"; shift ;;
    --interval) INTERVAL="$2"; shift ;;
    --duration) DURATION="$2"; shift ;;
    --count) COUNT="$2"; shift ;;
    --out) OUT="$2"; shift ;;
    --gate) GATE=1 ;;
    *) echo "unknown arg: $1" >&2; exit 2 ;;
  esac
  shift
done
[ "$COUNT" -gt 0 ] || COUNT=$(( DURATION / INTERVAL )); [ "$COUNT" -ge 1 ] || COUNT=1
[ -n "$OUT" ] || { mkdir -p target; OUT="target/inspect-$(date +%m%d-%H%M%S).csv"; }
os=$(uname -s)

# 대상 PID(mac·Linux) — `pgrep -f`는 명령줄 전체를 보므로 `--match <값>`을 인자로 받은 **이 스크립트의 bash**도 걸린다(11-12 Linux 실측:
#   load-bench가 앱 대신 bash를 재서 private=0.04MB · threads=1로 헛통과). 셸은 건너뛴다.
find_pid() {
  local p c
  for p in $(pgrep -f "$MATCH"); do
    c=$(ps -o comm= -p "$p" 2>/dev/null); c=${c##*/}
    case "$c" in bash | sh | zsh | dash | "") continue ;; esac
    echo "$p"; return 0
  done
}

# 한 샘플 → "pid,private_mb,commit_mb,handles,threads,cpu_ms,sockets" (프로세스 없으면 빈 줄).
sample() {
  case "$os" in
    MINGW*|MSYS*|CYGWIN*)
      powershell.exe -NoProfile -Command "\$p = Get-Process nexa-beep -ErrorAction SilentlyContinue | Where-Object { \$_.Path -like '*$MATCH*' } | Select-Object -First 1; if (-not \$p) { exit 0 }; \$cs = (Get-Counter -Counter @('\Process(nexa-beep*)\ID Process', '\Process(nexa-beep*)\Working Set - Private') -ErrorAction SilentlyContinue).CounterSamples; \$ids = @(\$cs | Where-Object { \$_.Path -like '*ID Process' }); \$wss = @(\$cs | Where-Object { \$_.Path -like '*Working Set - Private' }); \$v = \$null; for (\$i = 0; \$i -lt \$ids.Count; \$i++) { if ([int]\$ids[\$i].CookedValue -eq \$p.Id -and \$i -lt \$wss.Count) { \$v = \$wss[\$i].CookedValue } }; if (\$v -eq \$null) { \$v = \$p.PrivateMemorySize64 }; \$tcp = @(Get-NetTCPConnection -OwningProcess \$p.Id -ErrorAction SilentlyContinue).Count; \$udp = @(Get-NetUDPEndpoint -OwningProcess \$p.Id -ErrorAction SilentlyContinue).Count; '{0},{1},{2},{3},{4},{5},{6}' -f \$p.Id, [math]::Round(\$v/1MB,2), [math]::Round(\$p.PrivateMemorySize64/1MB,2), \$p.HandleCount, \$p.Threads.Count, [int]\$p.TotalProcessorTime.TotalMilliseconds, (\$tcp + \$udp)" | tr -d '\r' ;;
    Linux)
      local pid; pid=$(find_pid); [ -n "$pid" ] || return 0
      local anon thr fds socks cpu
      anon=$(awk '/RssAnon/{printf "%.2f", $2/1024}' "/proc/$pid/status"); thr=$(awk '/Threads/{print $2}' "/proc/$pid/status")
      fds=$(ls "/proc/$pid/fd" 2>/dev/null | wc -l); socks=$(ls -l "/proc/$pid/fd" 2>/dev/null | grep -c 'socket:')
      cpu=$(awk '{printf "%d", ($14+$15)*10}' "/proc/$pid/stat")
      local vm; vm=$(awk '/VmRSS/{printf "%.2f", $2/1024}' "/proc/$pid/status")
      echo "$pid,$anon,$vm,$fds,$thr,$cpu,$socks" ;;
    Darwin)
      local pid; pid=$(find_pid); [ -n "$pid" ] || return 0
      local fp rss thr fds socks cpu
      fp=$(footprint "$pid" 2>/dev/null | awk '/phys_footprint/{gsub(/[^0-9.]/,"",$NF); print $NF; exit}'); [ -n "$fp" ] || fp=$(ps -o rss= -p "$pid" | awk '{printf "%.2f", $1/1024}')
      rss=$(ps -o rss= -p "$pid" | awk '{printf "%.2f", $1/1024}'); thr=$(ps -M -p "$pid" | tail -n +2 | wc -l | tr -d ' ')
      fds=$(lsof -p "$pid" 2>/dev/null | tail -n +2 | wc -l | tr -d ' '); socks=$(lsof -p "$pid" 2>/dev/null | grep -c 'IPv[46]')
      cpu=$(ps -o cputime= -p "$pid" | awk -F'[:.]' '{ if (NF>=3) print ($1*60+$2)*1000+$3*10; else print 0 }')
      echo "$pid,$fp,$rss,$fds,$thr,$cpu,$socks" ;;
    *) return 0 ;;
  esac
}

echo "t_s,pid,private_mb,commit_mb,handles,threads,cpu_ms,sockets" > "$OUT"
echo "▶ 샘플 $COUNT회 × ${INTERVAL}s · 대상 '*$MATCH*' · $OUT"
T0=$(date +%s); n=0
while [ "$n" -lt "$COUNT" ]; do
  s=$(sample); t=$(( $(date +%s) - T0 ))
  if [ -n "$s" ]; then echo "$t,$s" >> "$OUT"; printf '  t=%5ss  %s\n' "$t" "$s"; else echo "  t=${t}s  (프로세스 없음)"; fi
  n=$((n+1)); [ "$n" -lt "$COUNT" ] && sleep "$INTERVAL"
done

echo; echo "═══ 요약(Δ = 마지막−처음 · max · 뒤 절반 기울기) ═══"
awk -F',' 'NR>1 && NF>=8 { n++; t[n]=$1; m[n]=$3; h[n]=$5; th[n]=$6; c[n]=$7; s[n]=$8; if ($3>mm) mm=$3; if ($5>mh) mh=$5 }
  END {
    if (n < 2) { print "  샘플 2개 미만 — 요약 없음"; exit 0 }
    half = int(n/2)+1; dm = m[n]-m[1]; dh = h[n]-h[1]; ds = s[n]-s[1]; dt = t[n]-t[half]; slope = (dt>0) ? (m[n]-m[half]) : 0
    cpu_pct = (t[n]-t[1] > 0) ? (c[n]-c[1]) / ((t[n]-t[1])*10.0) : 0
    printf "  private_mb  첫 %.2f → 끝 %.2f  Δ %+.2f  max %.2f  뒤절반 %+.2f MB\n", m[1], m[n], dm, mm, slope
    printf "  handles     첫 %d → 끝 %d  Δ %+d  max %d\n", h[1], h[n], dh, mh
    printf "  threads     첫 %d → 끝 %d  ·  sockets 첫 %d → 끝 %d  Δ %+d\n", th[1], th[n], s[1], s[n], ds
    printf "  cpu         Δ %d ms / %d s  = %.3f %%(단일 코어)\n", c[n]-c[1], t[n]-t[1], cpu_pct
    printf "GATE %s %s %s\n", (dm<=2.0)?"ok":"mem", (dh<=5)?"ok":"handles", (slope<=0.5)?"ok":"slope"
  }' "$OUT" | tee "${OUT%.csv}.summary.txt"
if [ "$GATE" = 1 ]; then
  if grep -q '^GATE ok ok ok$' "${OUT%.csv}.summary.txt"; then echo "PASS inspect-sample(Δmem ≤ 2MB · Δhandles ≤ 5 · 기울기 ≤ 0.5MB)"; else echo "FAIL inspect-sample — $OUT"; exit 1; fi
fi
