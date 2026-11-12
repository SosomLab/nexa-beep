#!/usr/bin/env bash
# flood-scenario.sh — 발견 광고 폭주 **감쇠**(M5-3 2차 ⓑ · docs/38 §5 · `dbba0a8`) 자동 실기(11-12 · 헤드리스 기본): 신원 2개(fl-x · fl-y)를
#   새로 만들어 ① 평시(주입 전) = 감쇠 로그 0줄(오탐 없음 — 정상 상대는 경로 수만큼 겹쳐도 `DISC_FLOOD_PPS`=40 아래) ② Y의 진짜 Announce
#   한 장을 받아 같은 바이트를 200pps × 6초 재전송(`tools/lib/disc-flood.py` · 발견 패킷은 서명 없는 힌트라 재전송 = 같은 상대의 폭주)
#   → X에 `disc/decide discovery … flood_pps=40 … throttled` 로그 ③ 감쇠 ≠ 차단: 주입 뒤에도 Y 행 유지 · 차단 목록 0건
#   ④ 반영 스로틀: 주입 6초 동안 감쇠 로그 줄 수 ≤ 6000ms/500ms(+여유) — 로그는 "반영을 통과시킨 관측"마다 1줄이라 줄 수가 곧 반영 횟수다.
#   기대값 출처 = crates/nexa-beep/src/app.rs `DISC_FLOOD_PPS`(40) · `DISC_THROTTLE_MS`(500) · `disc_throttled`.
#
#   ★ 통과 횟수의 뜻: `disc_throttled`의 시각은 패킷 도착이 아니라 `poll_discovery`가 큐를 비우는 **처리 시각**이다 — 유휴 틱(≈1Hz)에 몰아
#   비우면 한 번 비울 때 1건만 통과하므로 6초 ≈ 5~6회(500ms 기준 12회가 아니다 · 개발 세션 확인 11-12). 그래서 상한(500ms에 최대 1회)과
#   하한 ≥ 3(생존 판정이 굶지 않는다)을 같이 본다.
#
#   ⚠⚠ **실제 LAN으로 나간다**(TTL 1) — `verify.sh` 기본·`--perf`에는 없고 `--flood`로만 돈다. 격리망·단독 링크에서만(남의 설치본 · 보안 장비 R-22).
#   ⚠ 주입은 발견 멀티캐스트 그룹으로 나간다(TTL 1 · 6초 · 1,200장) — 같은 링크의 다른 Nexa Beep도 "fly" 광고를 그만큼 받는다
#   (받는 쪽은 같은 감쇠로 흡수 · 차단 없음). 유니캐스트는 같은 PC의 47100 소켓 중 하나에만 닿아 대상 인스턴스를 고를 수 없다.
#
# 사용:  tools/flood-scenario.sh [출력 폴더] [바이너리 폴더]   # 기본 = $BEEP_MULTI/flood-out · target/debug
# 전제:  빌드 산출물(nexa-beep + nbeep-imgdec) · python3(없으면 건너뜀 = 종료 0 + 사유). 판정 = stderr grep(추정 금지 · 실측 필수).
set -uo pipefail
cd "$(dirname "${BASH_SOURCE[0]}")/.."
export NEXA_TEST_HEADLESS=${NEXA_TEST_HEADLESS:-1} # 사용자 화면에 창을 띄우지 않는다(NEXA_TEST_HEADLESS=0 = 실제 화면)
. "$(dirname "${BASH_SOURCE[0]}")/lib/xvfb.sh"
M=${BEEP_MULTI:-$HOME/.nexa-beep-multi}
OUT=${1:-$M/flood-out}; mkdir -p "$OUT"; OUT=$(cd "$OUT" && pwd)
BINDIR=${2:-target/debug}
PPS=200; SECS=6
export NEXA_NO_AUTOSTART=1
trap 'BEEP_MULTI="$M" ./tools/autostart-clean.sh -q' EXIT
EXE=""; case "${OSTYPE:-}" in msys* | cygwin*) EXE=".exe" ;; esac
[ -x "$BINDIR/nexa-beep$EXE" ] || { echo "❌ $BINDIR/nexa-beep 없음 — cargo build -p nexa-beep -p nbeep-imgdec"; exit 1; }
PY=""; for c in python3 python; do command -v "$c" >/dev/null 2>&1 && "$c" -c 'import sys; sys.exit(0 if sys.version_info[0] == 3 else 1)' 2>/dev/null && { PY=$c; break; }; done
[ -n "$PY" ] || { echo "⚠ 건너뜀 — python3 없음(주입기 tools/lib/disc-flood.py)"; exit 0; }

seed() { # 폴더 · 표시 이름
  rm -rf "$M/$1"; mkdir -p "$M/$1/data"
  cp -f "$BINDIR/nexa-beep$EXE" "$BINDIR/nbeep-imgdec$EXE" "$M/$1/"
  printf '_schema=1\nprofile.display_name=%s\nchat.window_mode=single\napp.autostart=off\n' "$2" > "$M/$1/data/settings.cfg"
}
run() { # 폴더 · 스크립트 · 로그 — X는 발견 층 판정 로그를 stderr로(NEXA_DEVLOG=disc = decide+detail)
  ( cd "$M/$1" && NEXA_DEVLOG=disc NEXA_SCRIPT="$2" "./nexa-beep$EXE" --window --live 2> "$3" >/dev/null ) &
}
short() { "$M/$1/nexa-beep$EXE" --whoami 2>/dev/null | awk -F'= ' '/^fingerprint/{print substr($2,1,8); exit}'; }

echo "▶ 신원 준비(새로) — $M/{fl-x,fl-y} · 출력 $OUT"
seed fl-x flx; seed fl-y fly
# 시간표: 14s dump A(평시) → 15s~ 주입(Announce 포착 뒤 ${SECS}s) → 26s dump B(주입 뒤) → 28s quit.
echo "▶ X: 14s dump A · 26s dump B · 28s quit / Y: 28s quit / 주입: 15s부터 fly Announce ${PPS}pps × ${SECS}s"
T0=$(date +%s)
run fl-x "14000:dump;26000:dump;28000:quit" "$OUT/x.log"
run fl-y "28000:quit" "$OUT/y.log"
now=$(( $(date +%s) - T0 )); [ "$now" -lt 15 ] && sleep $(( 15 - now ))
"$PY" tools/lib/disc-flood.py fly "$PPS" "$SECS" 8 > "$OUT/flood.txt" 2>&1; frc=$?
sed 's/^/   주입: /' "$OUT/flood.txt"
wait
XS=$(short fl-x); YS=$(short fl-y)
echo "   X=$XS Y=$YS"

# 구간 분리 — dump A 앞 = 평시 로그(pre.txt) · A·B 구간의 [dump] 줄.
awk -v out="$OUT" '/^\[script\] [0-9]+ms dump$/{n++; next} n==0{print > (out "/pre.txt")} /^\[dump\]/{ if(n==1) print > (out "/x-A.txt"); if(n==2) print > (out "/x-B.txt") }' "$OUT/x.log"
for f in pre.txt x-A.txt x-B.txt; do [ -f "$OUT/$f" ] || : > "$OUT/$f"; done

echo; echo "═══ 판정 ═══"
ok=0; bad=0
cnt() { local n; n=$(grep -c -E "$2" "$1" 2>/dev/null || true); echo "${n:-0}"; }
chk() { local n; n=$(cnt "$2" "$3"); if [ "$n" -ge 1 ]; then printf '  ✓ %s (%s건)\n' "$1" "$n"; ok=$((ok+1)); else printf '  ✗ %s (0건)\n' "$1"; bad=$((bad+1)); fi; }
chk_count() { local n; n=$(cnt "$2" "$3"); if [ "$n" -eq "$4" ]; then printf '  ✓ %s (%s건 = 기대)\n' "$1" "$n"; ok=$((ok+1)); else printf '  ✗ %s (%s건 · 기대 %s)\n' "$1" "$n" "$4"; bad=$((bad+1)); fi; }
PEER_Y="^\[dump\] peer ${YS} name=fly "
THR="flood_pps=40.*throttled"
MAXL=$(( SECS * 1000 / 500 + 3 )) # 반영 상한 = 주입 시간 / DISC_THROTTLE_MS(+경계 여유 3)
sent=$(sed -n 's/^sent=\([0-9]*\).*/\1/p' "$OUT/flood.txt"); sent=${sent:-0}
if [ "$frc" = 0 ] && [ "$sent" -ge $(( PPS * SECS * 8 / 10 )) ]; then printf '  ✓ 주입 %s장(기대 ≥ %s)\n' "$sent" $(( PPS * SECS * 8 / 10 )); ok=$((ok+1)); else printf '  ✗ 주입 실패(rc=%s · sent=%s)\n' "$frc" "$sent"; bad=$((bad+1)); fi
chk       "A 평시: Y 행 있음(발견)"                              "$OUT/x-A.txt" "$PEER_Y"
chk_count "평시(주입 전): 감쇠 로그 0줄(오탐 없음)"              "$OUT/pre.txt" "$THR" 0
chk       "주입 중: 감쇠 발동 로그(peer=${YS} · flood_pps=40)"    "$OUT/x.log"   "discovery .*peer=${YS}.*${THR}"
n=$(cnt "$OUT/x.log" "$THR")
if [ "$n" -le "$MAXL" ]; then printf '  ✓ 반영 스로틀: 감쇠 통과 %s회 ≤ %s(= %ss/500ms + 3)\n' "$n" "$MAXL" "$SECS"; ok=$((ok+1)); else printf '  ✗ 반영 스로틀: 감쇠 통과 %s회 > %s\n' "$n" "$MAXL"; bad=$((bad+1)); fi
if [ "$n" -ge 3 ]; then printf '  ✓ 생존 판정 유지: 감쇠 통과 %s회 ≥ 3(굶지 않음)\n' "$n"; ok=$((ok+1)); else printf '  ✗ 생존 판정 굶음: 감쇠 통과 %s회 < 3\n' "$n"; bad=$((bad+1)); fi
chk       "B 주입 뒤: Y 행 유지(감쇠 ≠ 소멸)"                    "$OUT/x-B.txt" "$PEER_Y"
chk_count "B 주입 뒤: 차단 0건(감쇠 ≠ 차단)"                     "$OUT/x-B.txt" "^\[dump\] block " 0
echo "결과: ✓ $ok · ✗ $bad   (로그 = $OUT/{x,y}.log · 주입 = $OUT/flood.txt · 구간 = $OUT/{pre,x-A,x-B}.txt)"
[ "$bad" = 0 ]
