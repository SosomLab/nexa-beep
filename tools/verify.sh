#!/usr/bin/env bash
# verify.sh — 자동 실기 **한 명령**(사용자 11-04 "스크립트 실행은 자동화되도록 규칙 보완" · docs/18 §3·§5 0-a):
#   시나리오 스크립트를 하나씩 손으로 고르지 않는다. 빌드 → 헤드리스 스모크 → 답장 → S4 따라잡기(→ 성능 → 구버전 상대)
#   → 임시 산출물 정리를 순서대로 돌리고 요약 표 한 장과 종료 코드(실패 ≥1 = 1)를 낸다. 창은 사용자 화면에 띄우지 않는다
#   (NEXA_TEST_HEADLESS=1 · 구버전 상대만 예외 — 옵트인).
#
#   ./tools/verify.sh            # 기본: 빌드(debug+release) · smoke · reply · s4 · tmp-clean   ≈ 5~8분
#   ./tools/verify.sh --quick    # 빌드(debug) · smoke · reply                                   ≈ 2분
#   ./tools/verify.sh --perf     # 기본 + perf-scenario(헤드리스 수치 — 51 §3 기준선과 비교하지 않는다)
#   ./tools/verify.sh --oldpeer  # 기본 + oldpeer-scenario(⚠ 구버전 창이 화면에 ≈45s · gh 필요)
#   ./tools/verify.sh --no-build # 빌드 생략(이미 최신일 때)
# 결과 = target/verify-<일시>/{smoke,reply,s4,perf,oldpeer}.log · 마지막 줄 "verify: N통과 M실패".
# 권한: `.claude/settings.json`의 allow에 `bash tools/*`·`./tools/*`가 있어 세션이 묻지 않고 돌린다(새 스크립트도 같은 자리).
set -uo pipefail
cd "$(dirname "${BASH_SOURCE[0]}")/.."
export NEXA_TEST_HEADLESS=1 NEXA_NO_AUTOSTART=1
QUICK=0; PERF=0; OLDPEER=0; BUILD=1
for a in "$@"; do
  case "$a" in
    --quick) QUICK=1 ;;
    --perf) PERF=1 ;;
    --oldpeer) OLDPEER=1 ;;
    --no-build) BUILD=0 ;;
    *) echo "unknown arg: $a" >&2; exit 2 ;;
  esac
done
OUT="target/verify-$(date +%m%d-%H%M%S)"; mkdir -p "$OUT"
pass=0; fail=0; rows=()
step() { # 이름 · 로그 · 명령…
  local name="$1" log="$2"; shift 2
  local t0=$SECONDS
  printf '\n\033[1m▶ %s\033[0m\n' "$name"
  if "$@" > "$OUT/$log" 2>&1; then
    printf '  ✓ %s (%ss)\n' "$name" "$((SECONDS - t0))"; pass=$((pass + 1)); rows+=("✓ $name $((SECONDS - t0))s")
  else
    printf '  ✗ %s (%ss) — %s\n' "$name" "$((SECONDS - t0))" "$OUT/$log"; tail -8 "$OUT/$log" | sed 's/^/    /'
    fail=$((fail + 1)); rows+=("✗ $name $((SECONDS - t0))s → $OUT/$log")
  fi
}
if [ "$BUILD" = 1 ]; then
  step "build debug" build-debug.log cargo build -p nexa-beep -p nbeep-imgdec
  [ "$QUICK" = 1 ] || step "build release" build-release.log cargo build --release -p nexa-beep -p nbeep-imgdec
fi
step "headless-smoke" smoke.log bash tools/headless-smoke.sh "$OUT/smoke"
step "reply-scenario" reply.log bash tools/reply-scenario.sh
if [ "$QUICK" = 0 ]; then
  step "s4-scenario" s4.log bash tools/s4-scenario.sh
  [ "$PERF" = 1 ] && step "perf-scenario" perf.log bash tools/perf-scenario.sh "$OUT/perf"
  [ "$OLDPEER" = 1 ] && step "oldpeer-scenario" oldpeer.log bash tools/oldpeer-scenario.sh
fi
step "tmp-clean" tmp-clean.log bash tools/tmp-clean.sh
echo; echo "═══ verify 요약 ($OUT) ═══"
printf '  %s\n' "${rows[@]}"
echo "verify: ${pass}통과 ${fail}실패"
[ "$fail" = 0 ]
