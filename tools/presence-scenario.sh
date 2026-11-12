#!/usr/bin/env bash
# presence-scenario.sh — 내 상태(오프라인 모드 · PRES-1 · `ec2d8ae` · 동사 `presence=<online|offline>` `db46b2f`) 같은 PC 자동 축(11-12 ·
#   헤드리스 기본): 신원 2개(pr-x · pr-y)를 새로 만들어 X가 온라인 → 오프라인 → 온라인으로 바꾸는 동안 양쪽 dump로 판정한다.
#   ① A(온라인): 양쪽이 서로 live=true · X 첫 줄이 Y에 도달 ② X `presence=offline` → B: Y가 보는 X 행 live=false(대화가 있어 행은 남는다 ·
#   GOODBYE) · X가 보는 Y 행 live=false(발견 표 비움) ③ 오프라인 중 X 발신 → C: X 스레드에는 있고 Y에는 닿지 않음(대기 큐)
#   ④ X `presence=online` → D: 양쪽 live=true 복귀 · 대기분이 Y에 도달. 절차 원문 = docs/26 §3-4 ① 7.
#   2-PC 축(다른 기기 목록에서 사라짐 · 서버 재접속)은 사람 실기로 남는다.
#
# 사용:  tools/presence-scenario.sh [출력 폴더] [바이너리 폴더]   # 기본 = $BEEP_MULTI/presence-out · target/debug
# 전제:  빌드 산출물(nexa-beep + nbeep-imgdec). 판정 = stderr grep(추정 금지 · 실측 필수). ⚠ 신원 폴더는 매번 새로 만든다.
set -uo pipefail
cd "$(dirname "${BASH_SOURCE[0]}")/.."
export NEXA_TEST_HEADLESS=${NEXA_TEST_HEADLESS:-1} # 사용자 화면에 창을 띄우지 않는다(NEXA_TEST_HEADLESS=0 = 실제 화면)
. "$(dirname "${BASH_SOURCE[0]}")/lib/xvfb.sh"
M=${BEEP_MULTI:-$HOME/.nexa-beep-multi}
OUT=${1:-$M/presence-out}; mkdir -p "$OUT"; OUT=$(cd "$OUT" && pwd)
BINDIR=${2:-target/debug}
export NEXA_NO_AUTOSTART=1
trap 'BEEP_MULTI="$M" ./tools/autostart-clean.sh -q' EXIT
EXE=""; case "${OSTYPE:-}" in msys* | cygwin*) EXE=".exe" ;; esac
[ -x "$BINDIR/nexa-beep$EXE" ] || { echo "❌ $BINDIR/nexa-beep 없음 — cargo build -p nexa-beep -p nbeep-imgdec"; exit 1; }

seed() { # 폴더 · 표시 이름
  rm -rf "$M/$1"; mkdir -p "$M/$1/data"
  cp -f "$BINDIR/nexa-beep$EXE" "$BINDIR/nbeep-imgdec$EXE" "$M/$1/"
  printf '_schema=1\nprofile.display_name=%s\nchat.window_mode=single\napp.autostart=off\n' "$2" > "$M/$1/data/settings.cfg"
}
run() { # 폴더 · 스크립트 · 로그 — 앱 층 판정 로그(presence 줄)를 stderr로
  ( cd "$M/$1" && NEXA_DEVLOG="${NEXA_DEVLOG:-app}" NEXA_SCRIPT="$2" "./nexa-beep$EXE" --window --live 2> "$3" >/dev/null ) &
}
short() { "$M/$1/nexa-beep$EXE" --whoami 2>/dev/null | awk -F'= ' '/^fingerprint/{print substr($2,1,8); exit}'; }

echo "▶ 신원 준비(새로) — $M/{pr-x,pr-y} · 출력 $OUT"
seed pr-x prx; seed pr-y pry
# 시간표(양쪽 dump 시각 동일): A 20s(온라인) · B 28s(X 오프라인 뒤) · C 36s(오프라인 중 발신 뒤) · D 54s(온라인 복귀 뒤).
echo "▶ X: 10s Y 창 · 14s send 온라인 첫 줄 · 20s dump A · 22s presence=offline · 28s dump B · 30s send 오프라인 중 · 36s dump C · 38s presence=online · 54s dump D · 56s quit"
echo "▶ Y: 12s X 창 · 16s send 답 · 20s dump A · 28s dump B · 36s dump C · 54s dump D · 56s quit"
run pr-x "10000:activate=pry;14000:send=온라인 첫 줄;20000:dump;22000:presence=offline;28000:dump;30000:send=오프라인 중;36000:dump;38000:presence=online;54000:dump;56000:quit" "$OUT/x.log"
run pr-y "12000:activate=prx;16000:send=답;20000:dump;28000:dump;36000:dump;54000:dump;56000:quit" "$OUT/y.log"
wait
XS=$(short pr-x); YS=$(short pr-y)
echo "   X=$XS Y=$YS"

# dump 구간 분리 — n번째 `[script] …ms dump` 뒤의 `[dump]` 줄만 $OUT/<x|y>-<A..D>.txt로.
for w in x y; do
  awk -v out="$OUT" -v w="$w" 'BEGIN{split("A B C D",L," ")} /^\[script\] [0-9]+ms dump$/{n++; f=out "/" w "-" L[n] ".txt"; printf "" > f; next} /^\[dump\]/{ if(n>0) print >> f }' "$OUT/$w.log"
  for s in A B C D; do [ -f "$OUT/$w-$s.txt" ] || : > "$OUT/$w-$s.txt"; done
done

echo; echo "═══ 판정 ═══"
ok=0; bad=0
cnt() { local n; n=$(grep -c -E "$2" "$1" 2>/dev/null || true); echo "${n:-0}"; }
chk() { local n; n=$(cnt "$2" "$3"); if [ "$n" -ge 1 ]; then printf '  ✓ %s (%s건)\n' "$1" "$n"; ok=$((ok+1)); else printf '  ✗ %s (0건)\n' "$1"; bad=$((bad+1)); fi; }
chk_not() { local n; n=$(cnt "$2" "$3"); if [ "$n" -eq 0 ]; then printf '  ✓ %s (0건 = 기대)\n' "$1"; ok=$((ok+1)); else printf '  ✗ %s (%s건 · 기대 0)\n' "$1" "$n"; bad=$((bad+1)); fi; }
ROW_X="^\[dump\] peer ${XS} name=" # Y가 보는 X 행 — ★ 이름은 보지 않는다: 오프라인 행은 이름이 `beep-<지문>`으로 내려간다(11-12 실측 · DOC-45 관찰 후속과 같은 현상)
ROW_Y="^\[dump\] peer ${YS} name=" # X가 보는 Y 행
chk     "A 온라인: Y가 보는 X live=true"                     "$OUT/y-A.txt" "${ROW_X}.*live=true"
chk     "A 온라인: X가 보는 Y live=true"                     "$OUT/x-A.txt" "${ROW_Y}.*live=true"
chk     "A 온라인: X 첫 줄이 Y에 도달"                       "$OUT/y-A.txt" "^\[dump\] thread ${XS} mine=false .* text=온라인 첫 줄$"
chk     "A 온라인: Y가 보는 X 이름 = prx"                    "$OUT/y-A.txt" "${ROW_X}prx "
chk     "X: presence=offline 실행"                           "$OUT/x.log"   "^\[script\] [0-9]+ms presence=offline$"
chk     "X: 오프라인 판정 로그(offline=true)"                "$OUT/x.log"   "app/decide presence .*offline=true"
chk     "B 오프라인: Y가 보는 X 행 유지(대화 있음) · live=false" "$OUT/y-B.txt" "${ROW_X}.*live=false"
chk_not "B 오프라인: Y가 보는 X live=true 아님"              "$OUT/y-B.txt" "${ROW_X}.*live=true"
chk_not "B 오프라인: X가 보는 Y live=true 아님(발견 표 비움)" "$OUT/x-B.txt" "${ROW_Y}.*live=true"
chk     "C 오프라인 중 발신: X 스레드에 있음(내 줄)"         "$OUT/x-C.txt" "^\[dump\] thread ${YS} mine=true .* text=오프라인 중$"
chk_not "C 오프라인 중 발신: Y에 닿지 않음(대기)"            "$OUT/y-C.txt" "text=오프라인 중$"
chk     "X: presence=online 실행"                            "$OUT/x.log"   "^\[script\] [0-9]+ms presence=online$"
chk     "X: 온라인 판정 로그(offline=false)"                 "$OUT/x.log"   "app/decide presence .*offline=false"
chk     "D 복귀: Y가 보는 X live=true"                       "$OUT/y-D.txt" "${ROW_X}.*live=true"
chk     "D 복귀: X가 보는 Y live=true"                       "$OUT/x-D.txt" "${ROW_Y}.*live=true"
chk     "D 복귀: 대기분이 Y에 도달"                          "$OUT/y-D.txt" "^\[dump\] thread ${XS} mine=false .* text=오프라인 중$"
echo "결과: ✓ $ok · ✗ $bad   (로그 = $OUT/{x,y}.log · 구간 = $OUT/{x,y}-{A,B,C,D}.txt)"
[ "$bad" = 0 ]
