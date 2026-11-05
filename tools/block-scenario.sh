#!/usr/bin/env bash
# block-scenario.sh — 수신 제한(M5-3 BL-5 · docs/38) 자동 실기(11-05 · 3-OS · 헤드리스 기본): 신원 2개(bl-x · bl-y)를 새로 만들어
#   ① 미확인 발신자 격리(M5-3 2차 ⓒ · X `block.first_contact=on`으로 기동): Y 첫 메시지 → X `held_first n=1` · 스레드 0줄 · unread 0 →
#   `first.accept` → 표시 → Y 둘째 메시지는 카드 없이 바로 표시. ★ X는 10s에 Y 대화창을 **연다**(activate) — 창을 열기만 한 것은
#   "대화한 적"이 아니다(지문 안내 줄 = `is_local_notice` 제외 · `e10c402` · 11-05 첫 실행에서 이 경로로 격리가 풀렸던 회귀 가드). ② 그 뒤 X가 Y를 차단(block.peer) → X 목록에서 Y 소멸(그림자 · §6) · Y가 보낸 2줄이 X에 닿지 않음 → 해제(block.unblock) → Y 행 복귀 ·
#   재연결 · 메시지 도달 → 이름 축(block.name) → 소멸 → block.clear → 복귀 → IP 축(block.ip) 등재 → block.clear → 차단 0건.
#   판정 = 각 dump 구간(P·Q·R = 첫 접촉 · A~H = 차단)(`[script] <ms>ms dump` 표식 사이)의 `[dump] block <peer|ip|name> <값> auto= until=` 줄 · `[dump] peer` 행 · `[dump] thread` 줄.
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

seed() { # 폴더 · 표시 이름 · [추가 설정 줄]
  rm -rf "$M/$1"; mkdir -p "$M/$1/data"
  cp -f "$BINDIR/nexa-beep$EXE" "$BINDIR/nbeep-imgdec$EXE" "$M/$1/"
  printf '_schema=1\nprofile.display_name=%s\nchat.window_mode=single\napp.autostart=off\n' "$2" > "$M/$1/data/settings.cfg"
  [ -n "${3:-}" ] && printf '%s\n' "$3" >> "$M/$1/data/settings.cfg"
}
run() { # 폴더 · 스크립트 · 로그
  ( cd "$M/$1" && NEXA_SCRIPT="$2" "./nexa-beep$EXE" --window --live 2> "$3" >/dev/null ) &
}
short() { "$M/$1/nexa-beep$EXE" --whoami 2>/dev/null | awk -F'= ' '/^fingerprint/{print substr($2,1,8); exit}'; }

echo "▶ 신원 준비(새로) — $M/{bl-x,bl-y} · 출력 $OUT"
# 첫 접촉 승인제는 스크립트 동사가 없어 설정 파일로 켠다 · `_ack=on` = 사용자 의사 표식(없으면 부팅 1회 이관이 on을 결함 저장분으로 보고 off로 되돌린다)
seed bl-x blx $'block.first_contact=on\nblock.first_contact_ack=on'; seed bl-y bly

# 시간표(ms) — dump 구간 이름: P 17s(첫 메시지 대기) · Q 20s(수락 뒤) · R 24s(둘째 메시지) ·
#   A 26s(차단 전) · B 32s(peer 차단 직후) · C 42s(차단 중 Y 발신 뒤) · D 60s(해제 뒤 도달) ·
#   E 68s(이름 축) · F 78s(clear 뒤 복귀) · G 82s(IP 등재) · H 86s(clear 뒤 0건).
echo "▶ X: 10s Y 창 열기(activate — 열어도 격리 유지) · 17s dump P · 18s first.accept · 20s dump Q · 24s dump R · 26s dump A · 28s block.peer=bly · 32s dump B · 42s dump C"
echo "     44s block.unblock · 60s dump D · 62s block.name=bly · 68s dump E · 70s block.clear · 78s dump F · 80s block.ip=127.0.0.1 · 82s dump G"
echo "     84s block.clear · 86s dump H · 88s quit"
echo "▶ Y: 12s 연결 · 15s send 첫 접촉 · 21s send 두번째 · 34s send blocked-1 · 35s send blocked-2 · 52s 재연결 · 54s send 해제 뒤 도달 · 88s quit"
run bl-x "10000:activate=bly;17000:dump;18000:first.accept;20000:dump;24000:dump;26000:dump;28000:block.peer=bly;32000:dump;42000:dump;44000:block.unblock=;60000:dump;62000:block.name=bly;68000:dump;70000:block.clear;78000:dump;80000:block.ip=127.0.0.1;82000:dump;84000:block.clear;86000:dump;88000:quit" "$OUT/x.log"
run bl-y "12000:activate=blx;15000:send=첫 접촉;21000:send=두번째;34000:send=blocked-1;35000:send=blocked-2;52000:activate=blx;54000:send=해제 뒤 도달;88000:quit" "$OUT/y.log"
wait
XS=$(short bl-x); YS=$(short bl-y) # 신원은 첫 기동 때 생긴다 — 그 뒤에 읽어야 지문이 있다
echo "   X=$XS Y=$YS"

# dump 구간 분리 — n번째 `[script] …ms dump` 뒤의 `[dump]` 줄만 $OUT/x-<A..H>.txt로.
awk -v out="$OUT" 'BEGIN{split("P Q R A B C D E F G H",L," ")} /^\[script\] [0-9]+ms dump$/{n++; f=out "/x-" L[n] ".txt"; printf "" > f; next} /^\[dump\]/{ if(n>0) print >> f }' "$OUT/x.log"
for s in P Q R A B C D E F G H; do [ -f "$OUT/x-$s.txt" ] || : > "$OUT/x-$s.txt"; done

echo; echo "═══ 판정 ═══"
ok=0; bad=0
cnt() { local n; n=$(grep -c -E "$2" "$1" 2>/dev/null || true); echo "${n:-0}"; }
chk() { local n; n=$(cnt "$2" "$3"); if [ "$n" -ge 1 ]; then printf '  ✓ %s (%s건)\n' "$1" "$n"; ok=$((ok+1)); else printf '  ✗ %s (0건)\n' "$1"; bad=$((bad+1)); fi; }
chk_not() { local n; n=$(cnt "$2" "$3"); if [ "$n" -eq 0 ]; then printf '  ✓ %s (0건 = 기대)\n' "$1"; ok=$((ok+1)); else printf '  ✗ %s (%s건 · 기대 0)\n' "$1" "$n"; bad=$((bad+1)); fi; }
chk_count() { local n; n=$(cnt "$2" "$3"); if [ "$n" -eq "$4" ]; then printf '  ✓ %s (%s건 = 기대)\n' "$1" "$n"; ok=$((ok+1)); else printf '  ✗ %s (%s건 · 기대 %s)\n' "$1" "$n" "$4"; bad=$((bad+1)); fi; }
PEER_Y="^\[dump\] peer ${YS} name=bly "
chk       "X: Y 창 열기(activate) 실행"                        "$OUT/x.log"   "^\[script\] [0-9]+ms activate=bly$"
chk_count "P 첫 메시지: held_first ${YS} n=1(붙듦 · 창을 열었어도)"             "$OUT/x-P.txt" "^\[dump\] held_first ${YS} n=1$" 1
chk_not   "P 첫 메시지: 스레드에 없음(표시·기록 전)"            "$OUT/x-P.txt" "^\[dump\] thread .* text=첫 접촉$"
chk       "P 첫 메시지: Y 행 unread=0(알림·배지 없음)"          "$OUT/x-P.txt" "${PEER_Y}.*unread=0$"
chk       "X: first.accept 1건"                                 "$OUT/x.log"   "^\[script\] first.accept: 1$"
chk       "Q 수락 뒤: 첫 메시지 표시"                           "$OUT/x-Q.txt" "^\[dump\] thread ${YS} mine=false .* text=첫 접촉$"
chk_not   "Q 수락 뒤: held_first 0건"                           "$OUT/x-Q.txt" "^\[dump\] held_first "
chk       "R 둘째 메시지: 카드 없이 바로 표시"                  "$OUT/x-R.txt" "^\[dump\] thread ${YS} mine=false .* text=두번째$"
chk_not   "R 둘째 메시지: held_first 0건"                       "$OUT/x-R.txt" "^\[dump\] held_first "
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
echo "결과: ✓ $ok · ✗ $bad   (로그 = $OUT/{x,y}.log · 구간 = $OUT/x-{P,Q,R,A..H}.txt)"
[ "$bad" = 0 ]
