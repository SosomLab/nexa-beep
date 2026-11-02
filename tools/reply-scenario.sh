#!/usr/bin/env bash
# reply-scenario.sh — 답장 기능 와이어·기록 자동 실기(11-02 · 3-OS · 헤드리스 기본): 신원 2개(rp-x · rp-y)를 새로 만들어
#   X가 Y에게 "원본 글"을 보내고 Y가 `reply=` 동사로 마지막 줄에 답장 → 양쪽 dump의 `reply=<원본 short>#<seq>:<발췌>` 꼬리로
#   판정 · Y 재기동 dump = 기록 tag 7 복원 · X 재기동 dump = 받은 답장 복원.
#
# 사용:  tools/reply-scenario.sh [출력 폴더] [바이너리 폴더]   # 기본 = $BEEP_MULTI/reply-out · target/debug
# 전제:  빌드 산출물(nexa-beep + nbeep-imgdec) · 실행 중인 nexa-beep은 미리 종료(발견·포트가 겹친다).
set -uo pipefail
cd "$(dirname "${BASH_SOURCE[0]}")/.."
export NEXA_TEST_HEADLESS=${NEXA_TEST_HEADLESS:-1} # 사용자 화면에 창을 띄우지 않는다(NEXA_TEST_HEADLESS=0 = 실제 화면)
. "$(dirname "${BASH_SOURCE[0]}")/lib/xvfb.sh"
M=${BEEP_MULTI:-$HOME/.nexa-beep-multi}
OUT=${1:-$M/reply-out}; mkdir -p "$OUT"; OUT=$(cd "$OUT" && pwd)
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

echo "▶ 신원 준비(새로) — $M/{rp-x,rp-y} · 출력 $OUT"
seed rp-x rpx; seed rp-y rpy
echo "▶ ① X·Y 기동 — X: 10s 연결·14s 창·18s send 원본 / Y: 22s 창·26s reply 답장 / 34s dump · 38s quit"
# X 21s = **내 글(원본)에 답장**(사용자 11-02 "자기 글에 대해서도 답장") — 그 시점 X의 마지막 열쇠 줄 = 내 원본.
run rp-x "10000:activate=rpy;14000:activate=rpy;18000:send=원본 글 첫 줄;21000:reply=내 글에 이어서;34000:dump;38000:quit" "$OUT/x.log"
run rp-y "22000:activate=rpx;26000:reply=답장이야;34000:dump;38000:quit" "$OUT/y.log"
wait
XS=$(short rp-x); YS=$(short rp-y) # 신원은 첫 기동 때 생긴다 — 그 뒤에 읽어야 지문이 있다
echo "   X=$XS Y=$YS"
echo "▶ ② 재기동(기록 tag 7 복원) — 6s dump · 8s quit"
run rp-x "6000:dump;8000:quit" "$OUT/x-b.log"
run rp-y "6000:dump;8000:quit" "$OUT/y-b.log"
wait

echo; echo "═══ 판정 ═══"
ok=0; bad=0
chk() { local n; n=$(grep -c -E "$3" "$2" 2>/dev/null || true); n=${n:-0}
  if [ "$n" -ge 1 ]; then printf '  ✓ %s (%s건)\n' "$1" "$n"; ok=$((ok+1)); else printf '  ✗ %s (0건)\n' "$1"; bad=$((bad+1)); fi; }
chk "X→Y 원본 도착(Y 스레드 mine=false)"                "$OUT/y.log"   "thread .* mine=false .* text=원본 글 첫 줄"
chk "Y 답장 줄 = 참조 (X#seq) + 발췌(마지막 열쇠 줄 = X의 자기 답장)" "$OUT/y.log"   "thread .* mine=true .* text=답장이야 reply=${XS}#[1-9][0-9]*:내 글에 이어서"
chk "X 수신 답장 줄 = 참조 (X#seq) + 발췌(와이어 kind 2)" "$OUT/x.log"   "thread .* mine=false .* text=답장이야 reply=${XS}#[1-9][0-9]*:내 글에 이어서"
chk "Y 재기동: 답장 참조 복원(기록 tag 7)"               "$OUT/y-b.log" "thread .* text=답장이야 reply=${XS}#[1-9][0-9]*:내 글에 이어서"
chk "X 재기동: 받은 답장 참조 복원(기록 tag 7)"          "$OUT/x-b.log" "thread .* text=답장이야 reply=${XS}#[1-9][0-9]*:내 글에 이어서"
chk "Y: reply 동사가 열쇠 줄을 찾았다(경고 없음)"        "$OUT/y.log"   "^\[script\] 26[0-9]{3}ms reply=답장이야"
chk "X 자기 글 답장: X 스레드 mine=true + 참조 (X#seq)"  "$OUT/x.log"   "thread .* mine=true .* text=내 글에 이어서 reply=${XS}#[1-9][0-9]*:원본 글 첫 줄"
chk "X 자기 글 답장: Y 수신 줄에도 참조·발췌"           "$OUT/y.log"   "thread .* mine=false .* text=내 글에 이어서 reply=${XS}#[1-9][0-9]*:원본 글 첫 줄"
if grep -q "reply: 열쇠 있는 줄이 없다" "$OUT/y.log"; then printf '  ✗ Y: 원본 미수신 상태에서 reply\n'; bad=$((bad+1)); fi
echo; echo "결과: ✓ $ok · ✗ $bad   (로그 = $OUT/{x,y,x-b,y-b}.log)"
[ "$bad" = 0 ]
