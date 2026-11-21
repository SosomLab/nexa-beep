#!/usr/bin/env bash
# manual-scenario.sh — 수동 주소 추가 경로 단독 실기(11-21 · `c0db154` 후속 · TODO MAN-B · 헤드리스 기본 · mac/Linux):
#   신원 2개(mn-x · mn-y)를 새로 만들고 **둘 다 발견 광고를 끈다**(`net.discovery_advertise=off`) — 서로의 광고가 없어 자동 발견·자동 다이얼이 0이어야
#   "수동 경로 단독"이 성립한다. Y를 먼저 띄워 **실제 청취 포트를 lsof로 읽고**(설정값을 믿지 않는다) X가 동사 `add=127.0.0.1:<포트>`로 붙는다.
#   판정 = ① add 전 X dump에 피어 행 0(전제) · Y 로그에 자동 다이얼 없음 ② add 뒤 X dump에 피어 행 1개 live=true · manual_add_failed 0건
#   ③ Y dump에 피어 행 1개 · X가 보낸 한 줄이 Y 스레드에 도달. (광고가 없어 이름은 안 건너온다 — 행 이름 = `beep-<지문>`.)
#   지문 경로(`add=<64hex>` · 서버 경유)는 이 시나리오 밖(서버가 필요하다).
#
# 사용:  tools/manual-scenario.sh [출력 폴더] [바이너리 폴더]   # 기본 = $BEEP_MULTI/manual-out · target/debug
# 전제:  빌드 산출물(nexa-beep + nbeep-imgdec) · lsof. 판정 = stderr grep(추정 금지 · 실측 필수). ⚠ 신원 폴더는 매번 새로 만든다.
set -uo pipefail
cd "$(dirname "${BASH_SOURCE[0]}")/.."
export NEXA_TEST_HEADLESS=${NEXA_TEST_HEADLESS:-1} # 사용자 화면에 창을 띄우지 않는다(NEXA_TEST_HEADLESS=0 = 실제 화면)
. "$(dirname "${BASH_SOURCE[0]}")/lib/xvfb.sh"
M=${BEEP_MULTI:-$HOME/.nexa-beep-multi}
OUT=${1:-$M/manual-out}; mkdir -p "$OUT"; OUT=$(cd "$OUT" && pwd)
BINDIR=${2:-target/debug}
export NEXA_NO_AUTOSTART=1
trap 'BEEP_MULTI="$M" ./tools/autostart-clean.sh -q' EXIT
EXE=""; case "${OSTYPE:-}" in msys* | cygwin*) EXE=".exe" ;; esac
[ -x "$BINDIR/nexa-beep$EXE" ] || { echo "❌ $BINDIR/nexa-beep 없음 — cargo build -p nexa-beep -p nbeep-imgdec"; exit 1; }
command -v lsof >/dev/null || { echo "❌ lsof 없음(청취 포트를 읽는 수단)"; exit 1; }

seed() { # 폴더 · 표시 이름
  rm -rf "$M/$1"; mkdir -p "$M/$1/data"
  cp -f "$BINDIR/nexa-beep$EXE" "$BINDIR/nbeep-imgdec$EXE" "$M/$1/"; nb_warm "$M/$1/nexa-beep$EXE" "$M/$1/nbeep-imgdec$EXE"
  printf '_schema=1\nprofile.display_name=%s\nchat.window_mode=single\napp.autostart=off\nui.language=ko\nnet.discovery_advertise=off\n' "$2" > "$M/$1/data/settings.cfg"
}
# 절대 경로로 띄운다 — 명령줄에 신원 폴더가 있어야 pgrep으로 Y를 고를 수 있다(mem-bench 11-12와 같은 이유).
run() { ( cd "$M/$1" && NEXA_DEVLOG="${NEXA_DEVLOG:-conn}" NEXA_SCRIPT="$2" "$M/$1/nexa-beep$EXE" --window --live 2> "$3" >/dev/null ) & }

echo "▶ 신원 준비(새로) — $M/{mn-x,mn-y} · 둘 다 광고 off · 출력 $OUT"
seed mn-x mnx; seed mn-y mny
# Y = 30s dump · 40s quit(원점 = Y 기동). X는 Y 포트를 읽은 뒤(≈2~5s) 기동하므로 X의 send(16s)는 Y 시계로 ≈20s 안팎 → Y 30s dump 전.
run mn-y "30000:dump;40000:quit" "$OUT/y.log"
P=""
for _ in $(seq 1 75); do
  py=$(pgrep -f "$M/mn-y/nexa-beep" | head -1)
  [ -n "$py" ] && P=$(lsof -nP -iTCP -sTCP:LISTEN -a -p "$py" 2>/dev/null | awk 'NR>1{n=$9; sub(/.*:/,"",n); print n; exit}')
  [ -n "$P" ] && break
  sleep 0.2
done
[ -n "$P" ] || { echo "❌ Y 청취 포트를 못 읽음(pid=${py:-없음})"; wait; exit 1; }
echo "▶ Y pid=$py 청취 TCP $P → X: 2s dump(전) · 4s add=127.0.0.1:$P · 12s dump · 14s activate · 16s send · 22s dump · 24s quit"
echo "$P" > "$OUT/port.txt"
run mn-x "2000:dump;4000:add=127.0.0.1:$P;12000:dump;14000:activate=mny;16000:send=수동 경로 첫 줄;22000:dump;24000:quit" "$OUT/x.log"
wait

ok=0; bad=0
chk() { # 설명 · 로그 · 정규식 · 기대 개수(기본 ≥1 · 0이면 없어야 함)
  # ⚠ `[dev]` 줄을 빼지 않는다 — 이 시나리오의 판정 원천(conn/decide manual_add_failed · start_connect)이 바로 dev 줄이다
  #   (빼면 "없음" 판정이 거짓으로 통과한다 · s4/presence의 chk와 다른 점).
  local n; n=$(grep -c -E "$3" "$2" 2>/dev/null || true); n=${n:-0}
  if { [ -z "${4:-}" ] && [ "$n" -ge 1 ]; } || { [ -n "${4:-}" ] && [ "$n" -eq "$4" ]; }; then printf '  ✓ %s (%s건)\n' "$1" "$n"; ok=$((ok+1))
  else printf '  ✗ %s (%s건 · 기대 %s)\n' "$1" "$n" "${4:-≥1}"; bad=$((bad+1)); fi
}
# dump 구간 = X 로그의 [script] … dump 줄 다음부터 다음 [script] 줄까지.
seg() { awk -v k="$2" '/\[script\] [0-9]+ms dump/{c++; on=(c==k); next} /\[script\]/{on=0} on' "$1"; }
seg "$OUT/x.log" 1 > "$OUT/x-pre.txt"; seg "$OUT/x.log" 2 > "$OUT/x-post.txt"; seg "$OUT/x.log" 3 > "$OUT/x-end.txt"
echo; echo "═══ 판정 ═══"
# 광고가 없으니 이름(profile.display_name)도 안 건너온다 — 행 이름은 지문 기본값 `beep-<지문>`(11-21 실측) → 이름이 아니라 "전에 없던 행"으로 본다.
chk "전제: add 전 X dump에 피어 행 없음"          "$OUT/x-pre.txt"  '\[dump\] peer ' 0
chk "전제: Y가 X로 자동 다이얼하지 않음"            "$OUT/y.log"      'conn/decide start_connect .* auto=true' 0
chk "X: add 실행"                                 "$OUT/x.log"      "\[script\] 4[0-9]{3}ms add=127\.0\.0\.1:$P"
chk "X: manual_add_failed 없음"                   "$OUT/x.log"      'manual_add_failed' 0
chk "X 12s dump: 피어 행 1개 live=true(= Y)"      "$OUT/x-post.txt" '\[dump\] peer [0-9a-f]{8} .*live=true' 1
chk "Y 30s dump: 피어 행 1개(= X)"                "$OUT/y.log"      '\[dump\] peer [0-9a-f]{8} ' 1
chk "Y: X가 보낸 한 줄 도달"                       "$OUT/y.log"      '\[dump\] thread .* mine=false .*text=수동 경로 첫 줄'
echo "결과: ✓ $ok · ✗ $bad   (로그 = $OUT/{x,y}.log · 구간 = $OUT/x-{pre,post,end}.txt · 포트 = $OUT/port.txt)"
[ "$bad" = 0 ]
