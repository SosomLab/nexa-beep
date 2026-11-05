#!/usr/bin/env bash
# block-scenario.sh — 수신 제한(M5-3 BL-5 · docs/38) 자동 실기(11-05 · 3-OS · 헤드리스 기본): 신원 2개(bl-x · bl-y)를 새로 만들어
#   X가 Y를 차단(block.peer) → X 목록에서 Y 소멸(그림자 · §6) · Y가 보낸 2줄이 X에 닿지 않음 → 해제(block.unblock) → Y 행 복귀 ·
#   재연결 · 메시지 도달 → 이름 축(block.name) → 소멸 → block.clear → 복귀 → IP 축(block.ip) 등재 → block.clear → 차단 0건.
#   판정 = 각 dump 구간(`[script] <ms>ms dump` 표식 사이)의 `[dump] block <peer|ip|name> <값> auto= until=` 줄 · `[dump] peer` 행 · `[dump] thread` 줄.
#
#   ⚠ IP 축: 루프백 2신원은 같은 PC라 Y의 주소 = X 자신의 주소다. X가 그 주소를 막으면 같은 PC의 다른 인스턴스(사용자 설치본 포함)까지 걸리고,
#   127.0.0.1은 발견 경로가 아니라 집행 효과를 판정할 수 없다 → 여기서는 **IP 등재·해제(목록 왕복)만** 본다. "IP 추가 → 상대 연결 실패"는
#   2-PC 실기(BL-5 사람 몫)로 남긴다. 집행 판정은 이름 축(block.name — 발견 Appeared 게이트 · 같은 그림자 경로)으로 대신한다.
#
# 사용:  tools/block-scenario.sh [출력 폴더] [바이너리 폴더]   # 기본 = $BEEP_MULTI/block-out · target/debug
# 전제:  빌드 산출물(nexa-beep + nbeep-imgdec). 판정 = stderr grep(추정 금지 · 실측 필수). ⚠ 신원 폴더는 매번 새로 만든다.
set -uo pipefail
cd "$(dirname "${BASH_SOURCE[0]}")/.."
export NEXA_TEST_HEADLESS=${NEXA_TEST_HEADLESS:-1} # 사용자 화면에 창을 띄우지 않는다(NEXA_TEST_HEADLESS=0 = 실제 화면)
. "$(dirname "${BASH_SOURCE[0]}")/lib/xvfb.sh"
M=${BEEP_MULTI:-$HOME/.nexa-beep-multi}
OUT=${1:-$M/block-out}; mkdir -p "$OUT"; OUT=$(cd "$OUT" && pwd)
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
run() { # 폴더 · 스크립트 · 로그
  ( cd "$M/$1" && NEXA_SCRIPT="$2" "./nexa-beep$EXE" --window --live 2> "$3" >/dev/null ) &
}
short() { "$M/$1/nexa-beep$EXE" --whoami 2>/dev/null | awk -F'= ' '/^fingerprint/{print substr($2,1,8); exit}'; }

echo "▶ 신원 준비(새로) — $M/{bl-x,bl-y} · 출력 $OUT"
seed bl-x blx; seed bl-y bly

# 시간표(ms) — dump 구간 이름: A 14s(차단 전) · B 20s(peer 차단 직후) · C 30s(차단 중 Y 발신 뒤) · D 48s(해제 뒤 도달) ·
#   E 56s(이름 축) · F 66s(clear 뒤 복귀) · G 70s(IP 등재) · H 74s(clear 뒤 0건).
echo "▶ X: 10s 연결 · 14s dump A · 16s block.peer=bly · 20s dump B · 30s dump C · 32s block.unblock · 48s dump D · 50s block.name=bly · 56s dump E"
echo "     58s block.clear · 66s dump F · 68s block.ip=127.0.0.1 · 70s dump G · 72s block.clear · 74s dump H · 76s quit"
echo "▶ Y: 12s 연결 · 22s send blocked-1 · 23s send blocked-2 · 40s 재연결 · 42s send 해제 뒤 도달 · 76s quit"
run bl-x "10000:activate=bly;14000:dump;16000:block.peer=bly;20000:dump;30000:dump;32000:block.unblock=;48000:dump;50000:block.name=bly;56000:dump;58000:block.clear;66000:dump;68000:block.ip=127.0.0.1;70000:dump;72000:block.clear;74000:dump;76000:quit" "$OUT/x.log"
run bl-y "12000:activate=blx;22000:send=blocked-1;23000:send=blocked-2;40000:activate=blx;42000:send=해제 뒤 도달;76000:quit" "$OUT/y.log"
wait
XS=$(short bl-x); YS=$(short bl-y) # 신원은 첫 기동 때 생긴다 — 그 뒤에 읽어야 지문이 있다
echo "   X=$XS Y=$YS"

# dump 구간 분리 — n번째 `[script] …ms dump` 뒤의 `[dump]` 줄만 $OUT/x-<A..H>.txt로.
awk -v out="$OUT" 'BEGIN{split("A B C D E F G H",L," ")} /^\[script\] [0-9]+ms dump$/{n++; f=out "/x-" L[n] ".txt"; printf "" > f; next} /^\[dump\]/{ if(n>0) print >> f }' "$OUT/x.log"
for s in A B C D E F G H; do [ -f "$OUT/x-$s.txt" ] || : > "$OUT/x-$s.txt"; done

echo; echo "═══ 판정 ═══"
ok=0; bad=0
cnt() { local n; n=$(grep -c -E "$2" "$1" 2>/dev/null || true); echo "${n:-0}"; }
chk() { local n; n=$(cnt "$2" "$3"); if [ "$n" -ge 1 ]; then printf '  ✓ %s (%s건)\n' "$1" "$n"; ok=$((ok+1)); else printf '  ✗ %s (0건)\n' "$1"; bad=$((bad+1)); fi; }
chk_not() { local n; n=$(cnt "$2" "$3"); if [ "$n" -eq 0 ]; then printf '  ✓ %s (0건 = 기대)\n' "$1"; ok=$((ok+1)); else printf '  ✗ %s (%s건 · 기대 0)\n' "$1" "$n"; bad=$((bad+1)); fi; }
chk_count() { local n; n=$(cnt "$2" "$3"); if [ "$n" -eq "$4" ]; then printf '  ✓ %s (%s건 = 기대)\n' "$1" "$n"; ok=$((ok+1)); else printf '  ✗ %s (%s건 · 기대 %s)\n' "$1" "$n" "$4"; bad=$((bad+1)); fi; }
PEER_Y="^\[dump\] peer ${YS} name=bly "
chk       "A 차단 전: Y 행 있음"                               "$OUT/x-A.txt" "$PEER_Y"
chk_count "A 차단 전: 차단 0건"                                 "$OUT/x-A.txt" "^\[dump\] block " 0
chk       "X: block.peer 실행(${YS} forever)"                  "$OUT/x.log"   "^\[script\] block.peer: ${YS} forever"
chk_count "B: block peer 1줄(영구 · 수동 = auto=0 until=0)"     "$OUT/x-B.txt" "^\[dump\] block peer .* auto=0 until=0$" 1
chk_not   "B: Y 행 소멸(그림자 · 배지 아님)"                    "$OUT/x-B.txt" "$PEER_Y"
chk_not   "C: 차단 중 Y 발신 미도달(blocked-1·2)"               "$OUT/x-C.txt" "^\[dump\] thread .* text=blocked-[12]$"
chk_not   "C: Y 행 여전히 없음"                                 "$OUT/x-C.txt" "$PEER_Y"
chk       "X: block.unblock 1건"                                "$OUT/x.log"   "^\[script\] block.unblock: 1$"
chk_count "D: 해제 뒤 차단 0건"                                 "$OUT/x-D.txt" "^\[dump\] block " 0
chk       "D: Y 행 복귀(발견 재등장)"                           "$OUT/x-D.txt" "$PEER_Y"
chk       "D: 해제 뒤 Y 메시지 도달(재연결)"                    "$OUT/x-D.txt" "^\[dump\] thread ${YS} mine=false .* text=해제 뒤 도달$"
chk_not   "D: 차단 중 보낸 줄은 여전히 없음(되살아나지 않음)"   "$OUT/x-D.txt" "^\[dump\] thread .* text=blocked-[12]$"
chk_count "E: block name 1줄(bly · 영구)"                       "$OUT/x-E.txt" "^\[dump\] block name bly auto=0 until=0$" 1
chk_not   "E: 이름 축으로 Y 행 소멸"                            "$OUT/x-E.txt" "$PEER_Y"
chk       "X: block.clear(이름 1건)"                            "$OUT/x.log"   "^\[script\] block.clear: 1$"
chk_count "F: clear 뒤 차단 0건"                                "$OUT/x-F.txt" "^\[dump\] block " 0
chk       "F: Y 행 복귀"                                        "$OUT/x-F.txt" "$PEER_Y"
chk_count "G: block ip 1줄(127.0.0.1 · 기본 24h = until>0)"     "$OUT/x-G.txt" "^\[dump\] block ip 127\.0\.0\.1(/32)? auto=0 until=[1-9][0-9]*$" 1
chk_count "H: 최종 clear 뒤 차단 0건"                           "$OUT/x-H.txt" "^\[dump\] block " 0
echo "결과: ✓ $ok · ✗ $bad   (로그 = $OUT/{x,y}.log · 구간 = $OUT/x-{A..H}.txt)"
[ "$bad" = 0 ]
