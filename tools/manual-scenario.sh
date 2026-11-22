#!/usr/bin/env bash
# manual-scenario.sh — 수동 주소 추가 경로 단독 실기(11-21 · `c0db154` 후속 · TODO MAN-B · 헤드리스 기본 · mac/Linux):
#   신원 2개(mn-x · mn-y)를 새로 만들고 **둘 다 발견 광고를 끈다**(`net.discovery_advertise=off`) — 서로의 광고가 없어 자동 발견·자동 다이얼이 0이어야
#   "수동 경로 단독"이 성립한다. Y를 먼저 띄워 **실제 청취 포트를 lsof로 읽고**(설정값을 믿지 않는다) X가 동사 `add=127.0.0.1:<포트>`로 붙는다.
#   판정 = ① add 전 X dump에 피어 행 0(전제) · Y 로그에 자동 다이얼 없음 ② add 뒤 X dump에 피어 행 1개 live=true · manual_add_failed 0건
#   ③ Y dump에 피어 행 1개 · X가 보낸 한 줄이 Y 스레드에 도달. (광고가 없어 이름은 안 건너온다 — 행 이름 = `beep-<지문>`.)
#   ② 지문 경로(`add=<64hex>` · MAN-B · 11-22) = 로컬 nexa-beepd + B 정지(STOP) → 핸드셰이크 무응답 실패 통지·미기억(아래 ② 블록 · MAN_FP=0 = 생략).
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
# ═══ ② 지문 경로(MAN-B · 11-22) — "통로는 열렸는데 핸드셰이크 무응답" ═══
#   로컬 nexa-beepd(임시 포트 · 127.0.0.1 · 키는 출력 폴더 · 운영 서버 접속 없음)에 A·B가 붙는다(둘 다 광고 off · 서버 공개 off —
#   명부로 서로 보이면 "기억되지 않음" 판정이 흐려진다). B를 kill -STOP(이 시나리오가 띄운 자식만) → A가 `add=<B 지문>` →
#   기대 = A devlog `conn/decide manual_add_failed`(why 포함)가 add 뒤 ≈10s(OUTBOUND_HANDSHAKE_TIMEOUT) · A 목록에 B 행 없음.
#   서버가 먼저 끊어 다른 why로 빨리 끝나면 그 why와 시각을 그대로 적는다(판정은 why가 아니라 "실패 통지 1건 ∧ 기억 안 됨").
#   MAN_FP=0 = 이 단계 생략. 서버 바이너리 = $BINDIR/nexa-beepd(verify build debug가 같이 만든다).
if [ "${MAN_FP:-1}" = 1 ]; then
  echo; echo "▶ ② 지문 경로 — 로컬 nexa-beepd · B 정지 · A add=<B 지문>"
  BD="$BINDIR/nexa-beepd$EXE"
  if [ ! -x "$BD" ]; then echo "  ✗ $BD 없음 — cargo build -p nexa-beepd"; bad=$((bad+1)); else
    SP=""; for c in 47391 47392 47393 47394 47395; do lsof -nP -iTCP:"$c" -sTCP:LISTEN >/dev/null 2>&1 || { SP=$c; break; }; done
    [ -n "$SP" ] || { echo "  ✗ 빈 서버 포트 없음(47391~47395)"; bad=$((bad+1)); }
  fi
  if [ -x "$BD" ] && [ -n "$SP" ]; then
    "$BD" --port "$SP" --bind 127.0.0.1 --key "$OUT/beepd.key" --verbose > "$OUT/beepd.log" 2>&1 &
    BDP=$!
    sleep 1
    seedfp() { # 폴더 · 이름 — 서버 Managed · 검증 표식(주소만 대조 · 핀은 server.pin 없음 = 첫 접속 TOFU) · 공개 off
      seed "$1" "$2"
      printf 'net.server.mode=managed\nnet.server.address=127.0.0.1\nnet.server.port=%s\nnet.server.announce=off\nserver.verified=127.0.0.1:%s 00 0\n' "$SP" "$SP" >> "$M/$1/data/settings.cfg"
    }
    seedfp mf-a mfa; seedfp mf-b mfb
    # B: 5s dump(서버 붙음 확인) · 60s quit(정지가 풀린 뒤 스스로 끝난다).
    run mf-b "5000:dump;60000:quit" "$OUT/fb.log"
    sleep 7
    # 지문은 B가 한 번 떠 신원 키를 만든 뒤에 읽는다(첫 기동 전 --whoami = 빈 값 · 11-22 1차 실측).
    FPB=$(cd "$M/mf-b" && ./nexa-beep$EXE --whoami 2>/dev/null | awk '/^full/{print $3}')
    echo "  서버 127.0.0.1:$SP (pid $BDP) · B 지문 ${FPB:0:8}…"
    [ ${#FPB} = 64 ] || echo "  ! B 지문을 못 읽음"
    pb=$(pgrep -f "$M/mf-b/nexa-beep" | head -1)
    [ -n "$pb" ] && kill -STOP "$pb"
    echo "  B pid=${pb:-없음} 정지(STOP) · A: 5s dump(서버) · 7s add · 30s dump · 32s quit"
    T0=$(date +%s)
    run mf-a "5000:dump;7000:add=$FPB;30000:dump;32000:quit" "$OUT/fa.log"
    tadd=""; tfail=""
    for _ in $(seq 1 175); do # ≤35s
      now=$(( $(date +%s) - T0 ))
      [ -z "$tadd" ] && grep -q '\[script\] [0-9]*ms add=' "$OUT/fa.log" 2>/dev/null && tadd=$now
      [ -z "$tfail" ] && grep -q 'manual_add_failed' "$OUT/fa.log" 2>/dev/null && tfail=$now
      pgrep -f "$M/mf-a/nexa-beep" >/dev/null || break
      sleep 0.2
    done
    [ -n "$pb" ] && kill -CONT "$pb"
    wait "$!" 2>/dev/null # A
    for _ in $(seq 1 150); do pgrep -f "$M/mf-b/nexa-beep" >/dev/null || break; sleep 0.2; done # B 60s quit
    pkill -f "$M/mf-b/nexa-beep" 2>/dev/null
    kill "$BDP" 2>/dev/null; wait "$BDP" 2>/dev/null
    seg "$OUT/fa.log" 1 > "$OUT/fa-pre.txt"; seg "$OUT/fa.log" 2 > "$OUT/fa-end.txt"
    why=$(grep -m1 'manual_add_failed' "$OUT/fa.log" | grep -oE 'why=[^|]*' | sed 's/ *$//')
    lat="-"; [ -n "$tadd" ] && [ -n "$tfail" ] && lat=$((tfail - tadd))
    echo "  add 관측 t=${tadd:-?}s · 실패 통지 t=${tfail:-?}s → add 뒤 ≈${lat}s(1s 분해능) · ${why:-why 없음}"
    { echo "server_port=$SP"; echo "add_to_fail_s=$lat"; echo "${why:-why=}"; } > "$OUT/fp-summary.txt"
    echo; echo "═══ 판정 ② ═══"
    chk "전제: A 5s dump 서버 live=true"               "$OUT/fa-pre.txt" '\[dump\] server live=true' 1
    chk "전제: B 5s dump 서버 live=true(정지 전)"       "$OUT/fb.log"     '\[dump\] server live=true' 1
    chk "A: add=<지문> 실행"                            "$OUT/fa.log"     '\[script\] [0-9]+ms add=[0-9a-f]{64}'
    chk "A: manual_add_failed 1건"                      "$OUT/fa.log"     'manual_add_failed' 1
    chk "A 30s dump: B 행 없음(기억 안 됨)"             "$OUT/fa-end.txt" '\[dump\] peer ' 0
    if [ "$lat" != "-" ] && [ "$lat" -ge 8 ] && [ "$lat" -le 14 ]; then printf '  ✓ 실패까지 ≈%ss(핸드셰이크 타임아웃 10s ±)\n' "$lat"; ok=$((ok+1))
    else printf '  ✗ 실패까지 %ss(기대 8~14 · why로 원인 확인)\n' "$lat"; bad=$((bad+1)); fi
  fi
fi
echo
echo "결과: ✓ $ok · ✗ $bad   (로그 = $OUT/{x,y}.log · 구간 = $OUT/x-{pre,post,end}.txt · 포트 = $OUT/port.txt)"
[ "$bad" = 0 ]
