#!/usr/bin/env bash
# s4-scenario.sh — ADR-0015 S4 **따라잡기** 실기(같은 PC · 신원 3개 · docs/46 §5-4 · docs/48 §3-6 · 10-10)
#   ./tools/s4-scenario.sh [출력 폴더]      # 기본 = $BEEP_MULTI/s4-out (BEEP_MULTI 기본 ~/.nexa-beep-multi)
#
# 무엇을 재는가 — "꺼져 있던 기기가 돌아오면 형제에게서 놓친 대화를 받는가":
#   X1·X2 = 같은 사용자(핸들 s4test-x · 같은 암호 = 형제) · Y1 = 남.
#   ① Y1·X1만 켠 채 Y1 → X1 `y1-hello` · X1 → Y1 `x1-reply`(X1은 대화 창을 열었다 = 읽음).
#   ② X2(새 신원)를 켠다 — X1과 XXpsk3 형제 성립 → 디제스트 → X2가 Y1 스레드 2줄을 끌어온다 ·
#      X1이 읽었으므로 읽음 동기가 뒤따라 X2의 안읽음 0. X2는 Y1과 **세션을 맺지 않는다**(핀 없음).
#   ③ X2의 핀(trust.seg)을 지우고 켠다 — **핀 없는 상대**의 기록이 매핑 표(history/{short}.id)로 복원되는가
#      (②에서 Y1은 X-사용자 기기 목록 갱신을 보고 X2에도 세션을 맺어 핀이 생긴다 — 10-10 실측 ·
#       그래서 핀을 지워야 이 경로가 실제로 밟힌다 · 테스트 전용 조작).
# 판정 = 각 신원의 stderr(`NEXA_USER_TRACE=1` · `NEXA_SCRIPT dump`)를 grep — 추정 금지 · 실측 필수.
# ⚠ 신원 폴더(s4-x1 · s4-x2 · s4-y1)는 **매번 새로 만든다**(옛 핀·기록이 판정을 흐린다).
set -uo pipefail
cd "$(dirname "$0")/.."
M=${BEEP_MULTI:-$HOME/.nexa-beep-multi}
OUT=${1:-$M/s4-out}; mkdir -p "$OUT"; OUT=$(cd "$OUT" && pwd)
EXE=""; case "${OSTYPE:-}" in msys* | cygwin*) EXE=".exe" ;; esac
[ -x "target/release/nexa-beep$EXE" ] || { echo "❌ target/release/nexa-beep 없음 — cargo build --release -p nexa-beep -p nbeep-imgdec"; exit 1; }
PASS="s4pass-aaaa-bbbb"

seed() { # 폴더 · 표시 이름 · 핸들
  rm -rf "$M/$1"; mkdir -p "$M/$1/data"
  cp -f "target/release/nexa-beep$EXE" "target/release/nbeep-imgdec$EXE" "$M/$1/"
  printf '_schema=1\nprofile.display_name=%s\nuser.enabled=on\nuser.handle=%s\nuser.passphrase=%s\nuser.verified=on\nchat.window_mode=single\napp.autostart=off\n' \
    "$2" "$3" "$PASS" > "$M/$1/data/settings.cfg"
}
run() { # 폴더 · 스크립트 · 로그
  ( cd "$M/$1" && NEXA_USER_TRACE=1 NEXA_SCRIPT="$2" "./nexa-beep$EXE" --window --live 2> "$3" >/dev/null ) &
}

echo "▶ 신원 준비(새로) — $M/{s4-x1,s4-x2,s4-y1} · 출력 $OUT"
seed s4-x1 s4x1 s4test-x
seed s4-x2 s4x2 s4test-x
seed s4-y1 s4y1 s4test-y

echo "▶ ① Y1·X1 기동 — Y1: 12s 연결·16s 창·20s 송신 / X1: 24s 창·28s 답장 / 70s dump · 74s quit"
run s4-y1 "12000:activate=s4x1;16000:activate=s4x1;20000:send=y1-hello;70000:dump;74000:quit" "$OUT/y1.log"
run s4-x1 "24000:activate=s4y1;28000:send=x1-reply;70000:dump;74000:quit" "$OUT/x1.log"
sleep 36
echo "▶ ② X2 기동(t=36s) — 형제 성립·따라잡기 뒤 30s dump · 32s quit (Y1과는 세션 없음)"
run s4-x2 "30000:dump;32000:quit" "$OUT/x2-a.log"
wait
echo "▶ ③ X2 핀 삭제 후 재기동 — 8s dump · 10s quit (핀 없는 기록의 재시작 매핑)"
rm -f "$M/s4-x2/data/trust.seg" "$M/s4-x2/data/trust.seg.locked"
run s4-x2 "8000:dump;10000:quit" "$OUT/x2-b.log"
wait

echo
echo "═══ 판정 ═══"
ok=0; bad=0
chk() { # 설명 · 로그 · 정규식 · 기대 개수(기본 ≥1)
  local n; n=$(grep -c -E "$3" "$2" 2>/dev/null || true); n=${n:-0}
  if { [ -z "${4:-}" ] && [ "$n" -ge 1 ]; } || { [ -n "${4:-}" ] && [ "$n" -eq "$4" ]; }; then
    printf '  ✓ %s (%s건)\n' "$1" "$n"; ok=$((ok+1))
  else
    printf '  ✗ %s (%s건 · 기대 %s)\n' "$1" "$n" "${4:-≥1}"; bad=$((bad+1))
  fi
}
chk "Y1→X1 수신(X1 스레드에 y1-hello)"            "$OUT/x1.log"   'thread .* mine=false .* text=y1-hello'
chk "X1→Y1 답장(Y1 스레드에 x1-reply)"            "$OUT/y1.log"   'thread .* mine=false .* text=x1-reply'
chk "X1: X2에 디제스트 송신(entries=2)"            "$OUT/x1.log"   'sync digest to .* entries=2'
chk "X2: 디제스트 수신 → 청구 2건"                 "$OUT/x2-a.log" 'sync digest from .* pulls=2'
chk "X1: 라인 응답 2회(origin Y1 · origin X1)"     "$OUT/x1.log"   'sync lines to .* n=1' 2
chk "X2: 라인 수신 2회(added=1)"                   "$OUT/x2-a.log" 'sync lines from .* added=1' 2
chk "X2: 읽음 동기(unread 1->0)"                  "$OUT/x2-a.log" 'sync read from .* unread 1->0'
chk "X2 dump: y1-hello(상대 줄 · origin=Y1)"      "$OUT/x2-a.log" 'thread .* mine=false from=s4y1 origin=[0-9a-f]{8} seq=[1-9][0-9]* text=y1-hello'
chk "X2 dump: x1-reply(내 줄 · origin=X1)"        "$OUT/x2-a.log" 'thread .* mine=true .* origin=[0-9a-f]{8} seq=[1-9][0-9]* text=x1-reply'
chk "X2 dump: Y1 행 안읽음 0"                      "$OUT/x2-a.log" 'peer .* name=s4y1 .* unread=0'
chk "X2: 라인은 형제(X1)에게서 왔다(Y1 직접 수신 아님)" "$OUT/x2-a.log" 'sync lines from .* -> thread' 2
chk "X2 재기동 dump: 기록 2줄 복원(핀 없는 상대)"   "$OUT/x2-b.log" 'thread .* text=(y1-hello|x1-reply)' 2
chk "X2 재기동 dump: Y1 행 = 이름 힌트(s4y1) · 세션 없음" "$OUT/x2-b.log" 'peer .* name=s4y1 .* live=false'
echo
echo "결과: ✓ $ok · ✗ $bad   (로그 = $OUT/{y1,x1,x2-a,x2-b}.log)"
[ "$bad" = 0 ]
