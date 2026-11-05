#!/usr/bin/env bash
# verify.sh — 자동 실기 **한 명령**(사용자 11-04 "스크립트 실행은 자동화되도록 규칙 보완" · docs/18 §3·§5 0-a):
#   시나리오 스크립트를 하나씩 손으로 고르지 않는다. 빌드 → 헤드리스 스모크 → 답장 → 평문 스캔 → 와이어 스캔 → 그룹 → 수신 제한 → S4 따라잡기(→ 성능 → 구버전 상대)
#   → 임시 산출물 정리를 순서대로 돌리고 요약 표 한 장과 종료 코드(실패 ≥1 = 1)를 낸다. 창은 사용자 화면에 띄우지 않는다
#   (NEXA_TEST_HEADLESS=1 · 구버전 상대만 예외 — 옵트인).
#
#   ./tools/verify.sh            # 기본: 빌드(debug+release) · smoke · reply · group · block · s4 · tmp-clean   ≈ 7~10분
#   ./tools/verify.sh --quick    # 빌드(debug) · smoke · reply · scan-plaintext · wire-scan       ≈ 2.5분
#   ./tools/verify.sh --perf     # 기본 + perf-scenario(헤드리스 수치 — 51 §3 기준선과 비교하지 않는다) + startup-bench(PERF-4 콜드/웜 5회 중앙값) + scroll-bench(PERF-3 2000줄 휠 paint ms) + mem-bench(PERF-5 회수 4점 · 1GiB · ≈3분)
#   ./tools/verify.sh --oldpeer  # 기본 + oldpeer-scenario(⚠ 구버전 창이 화면에 ≈45s · gh 필요)
#   ./tools/verify.sh --no-build # 빌드 생략(이미 최신일 때)
# 권한: `.claude/settings.json` allow에 `tools/` 경로 접두가 등재돼 있어야 세션이 묻지 않고 돌린다 — 등재는 사용자가
#   `python tools/claude-permissions.py` 1회(세션은 자기 권한 파일을 못 고친다 · 새 스크립트는 그 RULES에도 등재).
# 결과 = target/verify-<일시>/{smoke,reply,scan,wire,group,block,s4,ack,perf,startup,scroll,mem,oldpeer}.log · 마지막 줄 "verify: N통과 M실패".
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
# INS-7(11-04 · 49 J-3·J-5): reply 시나리오가 남긴 신원의 데이터 폴더에 보낸 본문이 평문으로 없고 비밀 파일이 전부 봉인 봉투인지.
step "scan-plaintext" scan.log bash tools/scan-plaintext.sh "${BEEP_MULTI:-$HOME/.nexa-beep-multi}/rp-x/data" "원본 글 첫 줄" "답장이야" "내 글에 이어서"
# SEC-1(11-05 · docs/29 §4): 실소켓 와이어 평문 회귀 — 발견 패킷(그룹 조인 · 3-OS)은 패킷마다 W-1~W-7 자동 판정 · 세션 pcap(W-8)은 Linux+tcpdump+sudo일 때(CI ubuntu).
step "wire-scan" wire.log bash tools/wire-scan.sh "$OUT/wire"
if [ "$QUICK" = 0 ]; then
  step "group-scenario" group.log bash tools/group-scenario.sh
  step "block-scenario" block.log bash tools/block-scenario.sh # M5-3 BL-5(11-05 · docs/38 — 차단·그림자·해제 복귀 · ≈80s)
  step "s4-scenario" s4.log bash tools/s4-scenario.sh
  step "ack-bench" ack.log bash tools/ack-bench.sh "$OUT/ack"
  [ "$PERF" = 1 ] && step "perf-scenario" perf.log bash tools/perf-scenario.sh "$OUT/perf"
  [ "$PERF" = 1 ] && step "startup-bench" startup.log bash tools/startup-bench.sh "$OUT/startup"
  [ "$PERF" = 1 ] && step "scroll-bench" scroll.log bash tools/scroll-bench.sh "$OUT/scroll"
  [ "$PERF" = 1 ] && step "mem-bench" mem.log bash tools/mem-bench.sh "$OUT/mem"
  [ "$PERF" = 1 ] && step "load-bench(피어 100)" load.log bash tools/load-bench.sh "$OUT/load" target/debug 100
  [ "$OLDPEER" = 1 ] && step "oldpeer-scenario" oldpeer.log bash tools/oldpeer-scenario.sh
fi
step "tmp-clean" tmp-clean.log bash tools/tmp-clean.sh
echo; echo "═══ verify 요약 ($OUT) ═══"
printf '  %s\n' "${rows[@]}"
echo "verify: ${pass}통과 ${fail}실패"
[ "$fail" = 0 ]
