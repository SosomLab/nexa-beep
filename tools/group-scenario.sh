#!/usr/bin/env bash
# group-scenario.sh — 공유 그룹 자동 실기(11-04 · 3-OS · 헤드리스 기본 · REPLY-1 ① + GINV-1 S1): 신원 3개(gp-x 소유자 · gp-y · gp-z)를
#   새로 만들어 X가 방을 만들고(group.create) Y·Z가 수락(group.accept) → X 발신(group.send) → Y 답장(group.reply · 방 kind 10)
#   → 전원 dump(그룹 메타 `[dump] group …` · 방 스레드 `[dump] gthread …`) → X 재기동 dump(구성원 상태 메모·방 기록 영속).
#   ② 확장(11-04 · GINV-1 S2~S4 전 회귀망): X가 W를 추가(group.add) → W 거절(group.decline → 소유자 명부 제거·재배포) →
#   X가 Z 제외(group.remove → Z 방 닫힘 · Y 명부 갱신) → 최종 명부 = X·Y(X 재기동 dump로 판정).
#
# 사용:  tools/group-scenario.sh [출력 폴더] [바이너리 폴더]   # 기본 = $BEEP_MULTI/group-out · target/debug
# 판정 = 각 신원의 stderr grep(추정 금지 · 실측 필수).  ⚠ 신원 폴더는 매번 새로 만든다.
set -uo pipefail
. "$(dirname "${BASH_SOURCE[0]}")/lib/xvfb.sh"
cd "$(dirname "${BASH_SOURCE[0]}")/.."
M=${BEEP_MULTI:-$HOME/.nexa-beep-multi}
OUT=${1:-$M/group-out}; mkdir -p "$OUT"; OUT=$(cd "$OUT" && pwd)
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
  ( cd "$M/$1" && NEXA_USER_TRACE=1 NEXA_SCRIPT="$2" "./nexa-beep$EXE" --window --live 2> "$3" >/dev/null ) &
}
short() { "$M/$1/nexa-beep$EXE" --whoami 2>/dev/null | awk -F'= ' '/^fingerprint/{print substr($2,1,8); exit}'; }

echo "▶ 신원 준비(새로) — $M/{gp-x,gp-y,gp-z} · 출력 $OUT"
seed gp-x gpx; seed gp-y gpy; seed gp-z gpz; seed gp-w gpw

echo "▶ ① X: 12s·16s 연결 · 22s create / Y·Z: 30s accept / X: 38s send / Y: 46s reply / X: 56s dump · 58s add gpw / W: 64s decline / X: 68s remove gpz / 74s dump · 78s quit"
run gp-x "12000:activate=gpy;16000:activate=gpz;22000:group.create=실기방:gpy,gpz;38000:group.send=방 원본 글;56000:dump;58000:group.add=gpw;68000:group.remove=gpz;74000:dump;78000:quit" "$OUT/x.log"
run gp-y "30000:group.accept;46000:group.reply=방 답장이야;74000:dump;78000:quit" "$OUT/y.log"
run gp-z "30000:group.accept;56000:dump;74000:dump;78000:quit" "$OUT/z.log"
run gp-w "64000:group.decline;74000:dump;78000:quit" "$OUT/w.log"
wait
XS=$(short gp-x); YS=$(short gp-y); ZS=$(short gp-z); WS=$(short gp-w) # 신원은 첫 기동 때 생긴다 — 그 뒤에 읽어야 지문이 있다

echo "▶ ② X 재기동(구성원 상태 메모·방 기록 영속) — 6s dump · 8s quit"
run gp-x "6000:dump;8000:quit" "$OUT/x-b.log"
wait

echo; echo "═══ 판정 ═══"
ok=0; bad=0
chk() { local n; n=$(grep -c -E "$3" "$2" 2>/dev/null || true); n=${n:-0}
  if [ "$n" -ge 1 ]; then printf '  ✓ %s (%s건)\n' "$1" "$n"; ok=$((ok+1)); else printf '  ✗ %s (0건)\n' "$1"; bad=$((bad+1)); fi; }
chk_not() { local n; n=$(grep -c -E "$3" "$2" 2>/dev/null || true); n=${n:-0}
  if [ "$n" -eq 0 ]; then printf '  ✓ %s (0건 = 기대)\n' "$1"; ok=$((ok+1)); else printf '  ✗ %s (%s건 · 기대 0)\n' "$1" "$n"; bad=$((bad+1)); fi; }
chk_count() { local n; n=$(grep -c -E "$3" "$2" 2>/dev/null || true); n=${n:-0}
  if [ "$n" -eq "$4" ]; then printf '  ✓ %s (%s건 = 기대)\n' "$1" "$n"; ok=$((ok+1)); else printf '  ✗ %s (%s건 · 기대 %s)\n' "$1" "$n" "$4"; bad=$((bad+1)); fi; }
chk "X: 방 생성(소유자 = X · 구성원 3)"                      "$OUT/x.log"   "\[dump\] group .* name=실기방 mine=Owner owner=${XS} members=.*=.*,.*=.*,.*="
chk "X: Y·Z 수락 메모 = joined(GINV-1 S1)"                   "$OUT/x.log"   "\[dump\] group .* name=실기방 .* members=.*joined.*joined"
chk "Y: 수락 뒤 mine=Joined"                                  "$OUT/y.log"   "\[dump\] group .* name=실기방 mine=Joined owner=${XS}"
chk "Z: 수락 뒤 mine=Joined"                                  "$OUT/z.log"   "\[dump\] group .* name=실기방 mine=Joined owner=${XS}"
chk "Y: X 발신 수신(방 스레드 · origin=X)"                    "$OUT/y.log"   "\[dump\] gthread .* mine=false .* origin=${XS} seq=[1-9][0-9]* text=방 원본 글"
chk "Y: 답장 줄 = 참조(X#seq)+발췌"                          "$OUT/y.log"   "\[dump\] gthread .* mine=true .* text=방 답장이야 reply=${XS}#[1-9][0-9]*:방 원본 글"
chk "X: Y 답장 수신(kind 10 · 참조 동반 · from=gpy)"           "$OUT/x.log"   "\[dump\] gthread .* mine=false from=gpy origin=${YS} .* text=방 답장이야 reply=${XS}#[1-9][0-9]*:방 원본 글"
chk "Z: Y 답장 팬아웃 수신(참조 동반)"                         "$OUT/z.log"   "\[dump\] gthread .* mine=false .* text=방 답장이야 reply=${XS}#[1-9][0-9]*:방 원본 글"
# ② 확장: 추가 → 거절(명부 제거·재배포) → 제외(방 닫힘) · 최종 명부 = X·Y.
chk "X: group.add 실행(58s)"                                   "$OUT/x.log"   "\[script\] 58[0-9]{3}ms group.add=gpw"
chk_not "X 74s dump: W 없음(거절 = 명부 제거)"                   "$OUT/x.log"   "\[dump\] group .* name=실기방 .* members=.*${WS}="
chk_not "W: 거절 뒤 방 없음"                                     "$OUT/w.log"   "\[dump\] group .* name=실기방"
chk_count "Z: 제외 뒤 방 닫힘(56s dump엔 있고 74s dump엔 없음 = 1건)" "$OUT/z.log" "\[dump\] group .* name=실기방" 1
chk "Y: 명부 갱신 = 둘(X·Y)"                                    "$OUT/y.log"   "\[dump\] group .* name=실기방 mine=Joined owner=${XS} members=[0-9a-f]{8}=[a-z]+,[0-9a-f]{8}=[a-z]+$"
chk_not "Y 명부: Z 없음"                                        "$OUT/y.log"   "\[dump\] group .* name=실기방 .* members=.*${ZS}="
chk "X 재기동: 최종 명부 = X·Y(joined 보존 · groups.seg v4)"      "$OUT/x-b.log" "\[dump\] group .* name=실기방 mine=Owner owner=${XS} members=[0-9a-f]{8}=(joined|me),[0-9a-f]{8}=(joined|me)$"
chk "X 재기동: 방 기록 2줄 복원(원본·답장 참조)"                "$OUT/x-b.log" "\[dump\] gthread .* text=(방 원본 글|방 답장이야 reply=${XS}#[1-9][0-9]*:방 원본 글)"
if grep -q "group.reply: 열쇠 있는 줄이 없다" "$OUT/y.log"; then printf '  ✗ Y: 원본 미수신 상태에서 reply\n'; bad=$((bad+1)); fi
echo "결과: ✓ $ok · ✗ $bad   (로그 = $OUT/{x,y,z,w,x-b}.log)"
[ "$bad" = 0 ]
