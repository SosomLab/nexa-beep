#!/usr/bin/env bash
# 기준 재기동 — 종료 → 재빌드 → 3신원 기동 → 발견 확인 (한 번에)
#
# 왜 스크립트인가: 손으로 하면 매번 한 단계씩 빠졌다(빌드를 잊거나, 옛 프로세스가
# 남아 포트를 물고 있거나, imgdec 복사를 빠뜨려 아바타만 죽거나).
#
# ★ 기본은 **신원 보존**이다. 폴더를 지우면 핀·그룹·설정이 통째로 사라져
#   그룹 테스트를 처음부터 다시 해야 한다. 새 신원이 필요하면 --fresh.
#
# 사용:
#   tools/relaunch.sh            # 종료 → 빌드 → 3신원(기존 신원 유지)
#   tools/relaunch.sh --fresh    # A·B 신원을 새로 만든다(핀·그룹 초기화)
#   tools/relaunch.sh --gate     # 빌드 대신 전체 게이트(fmt·clippy·test·rustdoc)
#   tools/relaunch.sh --no-build # 빌드 건너뛰고 재기동만
#
# 참고: docs/33 §2(신원 3개) · docs/26 §3-4(원리 — data/는 실행 파일을 따라간다)
set -uo pipefail

cd "$(dirname "$0")/.."
ROOT=$(pwd)
MULTI=${BEEP_MULTI:-/tmp/beep-multi}
FRESH=0 BUILD=1 GATE=0
for a in "$@"; do
  case "$a" in
    --fresh) FRESH=1 ;;
    --no-build) BUILD=0 ;;
    --gate) GATE=1 ;;
    -h | --help) sed -n '2,20p' "$0"; exit 0 ;;
    *) echo "알 수 없는 인자: $a (--help)"; exit 2 ;;
  esac
done

say() { printf '\n\033[1m▶ %s\033[0m\n' "$1"; }

# ── ① 종료 ────────────────────────────────────────────────────────────────
say "① 실행 중인 nexa-beep 종료"
BEFORE=$(pgrep -f "nexa-beep" 2>/dev/null | wc -l | tr -d ' ')
pkill -f "nexa-beep" 2>/dev/null
sleep 2
# 2차 — 안 죽은 것(끊긴 세션의 프로브 잔재 등)은 강제 종료한다. 하나라도 남으면
# 포트·발견이 겹쳐 다음 단계 판정이 흐려진다(08-14: 고아 프로브 1개가 가드를 막았다).
if [ -n "$(pgrep -f 'nexa-beep' 2>/dev/null)" ]; then
  pkill -9 -f "nexa-beep" 2>/dev/null
  sleep 1
fi
LEFT=$(pgrep -f "nexa-beep" 2>/dev/null | wc -l | tr -d ' ')
echo "   종료 전 ${BEFORE}개 → 남은 ${LEFT}개"
[ "$LEFT" != "0" ] && { echo "   ⚠️ 강제 종료 후에도 남았다 — 확인 후 다시"; pgrep -fl "nexa-beep"; exit 1; }

# ── ② 빌드 ────────────────────────────────────────────────────────────────
if [ "$GATE" = "1" ]; then
  say "② 전체 게이트 (fmt · clippy · test · rustdoc)"
  cargo fmt --all --check || { echo "   ❌ fmt"; exit 1; }
  cargo clippy --workspace --all-targets -q 2>&1 | grep -E "^(warning|error)" && { echo "   ❌ clippy"; exit 1; }
  cargo test --workspace -q 2>&1 | grep -E "^test result" |
    awk -F'[ ;]' '{p+=$4;f+=$6} END{printf "   테스트 %d 통과 / %d 실패\n",p,f; if(f>0) exit 1}' || exit 1
  cargo doc --workspace --no-deps -q 2>&1 | grep -E "^(error|warning)" && { echo "   ❌ rustdoc"; exit 1; }
  echo "   게이트 통과 ✅"
elif [ "$BUILD" = "1" ]; then
  say "② 릴리스 빌드 (nexa-beep + nbeep-imgdec)"
  cargo build --release -p nexa-beep -p nbeep-imgdec 2>&1 | tail -1
else
  say "② 빌드 건너뜀 (--no-build)"
fi
[ -x target/release/nexa-beep ] || { echo "   ❌ 빌드 산출물이 없다"; exit 1; }

# ── ③ 3신원 준비 ──────────────────────────────────────────────────────────
# 신원은 '실행 파일 옆 data/'에 붙는다(포터블 규칙 DR-4) — 폴더가 곧 신원이다.
say "③ 신원 3개 준비 ($([ "$FRESH" = 1 ] && echo '새 신원' || echo '기존 신원 유지'))"
[ "$FRESH" = "1" ] && rm -rf "$MULTI"
for d in A B; do
  mkdir -p "$MULTI/$d"
  cp -f target/release/nexa-beep target/release/nbeep-imgdec "$MULTI/$d/"  # imgdec 동거 필수
done
echo "   기본 = $ROOT/target/release  ·  A·B = $MULTI/{A,B}"

# ── ④ 기동 ────────────────────────────────────────────────────────────────
# ★ nohup + disown 으로 **셸에서 떼어낸다**. 그냥 `&`로 띄우면 이 스크립트를 부른
#   셸의 프로세스 그룹에 남아, 그 셸이 종료·중단될 때 창이 같이 죽는다(08-14 실측 —
#   크래시 로그 없이 조용히 사라져 원인 찾는 데 시간을 썼다).
say "④ 기동 (셸에서 분리)"
nohup ./target/release/nexa-beep --window --live > /tmp/beep-base.log 2>&1 &
disown
nohup "$MULTI/A/nexa-beep" --window --live > "$MULTI/A/out.log" 2>&1 &
disown
nohup "$MULTI/B/nexa-beep" --window --live > "$MULTI/B/out.log" 2>&1 &
disown
sleep 6
RUNNING=$(pgrep -f "nexa-beep --window" 2>/dev/null | wc -l | tr -d ' ')
echo "   창 ${RUNNING}개"

# ── ⑤ 발견 확인 ───────────────────────────────────────────────────────────
# 셋이 서로 보이는지까지 봐야 "기동됐다"고 말할 수 있다(창만 떠도 발견이 막힐 수 있다).
say "⑤ 발견 확인 (프로브 8초)"
SEEN=$(./target/release/nexa-beep --discover-probe 8 2>&1 |
  grep -oE "peer=[0-9a-f]+ .*name=[^ ]*" | sort -u)
echo "$SEEN" | sed 's/^/   /'
N=$(echo "$SEEN" | grep -c "peer=" || true)
if [ "$RUNNING" = "3" ] && [ "$N" -ge 3 ]; then
  printf '\n\033[32m✅ 3신원 기동·상호 발견 완료\033[0m — 정리: pkill -f nexa-beep\n'
else
  printf '\n\033[33m⚠️ 창 %s개 · 발견 %s명\033[0m — 로그: /tmp/beep-base.log · %s/{A,B}/out.log\n' "$RUNNING" "$N" "$MULTI"
  exit 1
fi
