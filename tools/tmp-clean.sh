#!/usr/bin/env bash
# tmp-clean.sh — 개발·시험이 프로젝트 폴더 밖에 남긴 임시 산출물 정리(11-04 사용자 규칙 · docs/18 §3).
#
#   ./tools/tmp-clean.sh            # 기본: 1일 넘은 것만 지운다
#   ./tools/tmp-clean.sh --dry-run  # 지울 목록만 보인다
#   ./tools/tmp-clean.sh --age 0    # 지금 남아 있는 것 전부(실행 중인 시험이 없을 때만)
#
# 대상(접두 = 이 저장소 코드·도구가 만드는 이름만 · 남의 것은 건드리지 않는다):
#   ① OS 임시 폴더(TMPDIR/TEMP) — 단위 시험이 PID 접미로 만드는 `nbeep-*` `nb-*` `nexa-beep*` `nlt-e2e-*`
#      (정상 종료면 스스로 지우지만, 패닉·강제 종료가 남긴다) · relaunch `beep-base.log` · mktemp 빌드 로그 `tmp.*`는 제외
#   ② `target/perf-*`(perf-scenario 실측 폴더) · `target/tmp` — 저장소 안이지만 git 밖이라 쌓인다
#   ③ 하네스 스크래치패드 `…/Temp/claude/<이 저장소>/<세션>` — 14일 넘은 세션 폴더만(살아 있는 세션은 그보다 젊다)
#   ④ `$HOME/.nexa-beep-multi`의 **시험 신원·출력**(11-04 사용자 규칙 — 스크립트가 매번 새로 만든다): `gp-* rp-* sb-* mb-* op-* s4-*
#      tr-* idle-*` 폴더 · `*-out` 폴더 · 루트의 `*.log` — 실기 3신원 `A B C`(DR-4 durable · relaunch.sh가 관리)는 건드리지 않는다.
# 쓰지 않는 것: 실기 3신원 폴더 · 설치본 데이터.
set -euo pipefail
cd "$(dirname "${BASH_SOURCE[0]}")/.."

AGE_DAYS=1; DRY=0
while [ $# -gt 0 ]; do
  case "$1" in
    --dry-run) DRY=1 ;;
    --age) AGE_DAYS="$2"; shift ;;
    *) echo "unknown arg: $1" >&2; exit 2 ;;
  esac
  shift
done

# OS 임시 폴더 — Windows Git Bash는 TEMP(C:\Users\…\Temp)가 /c/… 로 들어온다.
TMP_ROOT="${TMPDIR:-${TEMP:-${TMP:-/tmp}}}"
TMP_ROOT="${TMP_ROOT%/}"
case "$(uname -s)" in
  MINGW*|MSYS*|CYGWIN*) TMP_ROOT="$(cygpath -u "$TMP_ROOT" 2>/dev/null || echo "$TMP_ROOT")" ;;
esac

removed=0; bytes=0
zap() { # $1 = 경로
  local p="$1" sz
  sz=$(du -sk "$p" 2>/dev/null | cut -f1 || echo 0)
  if [ "$DRY" = 1 ]; then
    printf '  (dry) %s  %sK\n' "$p" "$sz"
  else
    rm -rf -- "$p" 2>/dev/null && printf '  rm %s  %sK\n' "$p" "$sz" || printf '  (skip·busy) %s\n' "$p"
  fi
  removed=$((removed + 1)); bytes=$((bytes + sz))
}

# find의 -mtime은 "일 단위 24h 경과" — --age 0 = 전부.
age_expr=()
[ "$AGE_DAYS" -gt 0 ] && age_expr=(-mtime "+$((AGE_DAYS - 1))")

echo "▶ OS 임시 폴더: $TMP_ROOT (age ≥ ${AGE_DAYS}d)"
if [ -d "$TMP_ROOT" ]; then
  while IFS= read -r -d '' p; do zap "$p"; done < <(
    find "$TMP_ROOT" -maxdepth 1 -mindepth 1 \
      \( -name 'nbeep-*' -o -name 'nb-*' -o -name 'nexa-beep*' -o -name 'nlt-e2e-*' -o -name 'beep-base.log' -o -name 'beep-multi' \) \
      "${age_expr[@]}" -print0 2>/dev/null
  )
fi

echo "▶ target/ 실측 폴더"
while IFS= read -r -d '' p; do zap "$p"; done < <(
  find target -maxdepth 1 -mindepth 1 \( -name 'perf-*' -o -name 'tmp' -o -name 'rc-*' \) "${age_expr[@]}" -print0 2>/dev/null
)

# 하네스 스크래치패드 — 이 저장소 몫만 · 14일 이상(세션별 폴더).
SCRATCH="$TMP_ROOT/claude"
if [ -d "$SCRATCH" ]; then
  echo "▶ 하네스 스크래치패드(14d+): $SCRATCH"
  while IFS= read -r -d '' p; do zap "$p"; done < <(
    find "$SCRATCH" -maxdepth 2 -mindepth 2 -type d -path '*nexa-beep*' -mtime +13 -print0 2>/dev/null
  )
fi

# ④ 시험 신원·출력(11-04) — 실행 중인 시험이 있으면 그 폴더는 바쁜 상태라 건너뛴다(rm 실패 = skip).
MULTI="${BEEP_MULTI:-$HOME/.nexa-beep-multi}"
if [ -d "$MULTI" ]; then
  echo "▶ 시험 신원·출력(age ≥ ${AGE_DAYS}d): $MULTI (실기 A·B·C 제외)"
  while IFS= read -r -d '' p; do zap "$p"; done < <(
    find "$MULTI" -maxdepth 1 -mindepth 1 \
      \( -type d \( -name 'gp-*' -o -name 'rp-*' -o -name 'sb-*' -o -name 'mb-*' -o -name 'op-*' -o -name 's4-*' -o -name 'tr-*' -o -name 'idle-*' -o -name '*-out' \) \
         -o -type f -name '*.log' \) \
      "${age_expr[@]}" -print0 2>/dev/null
  )
fi

printf '정리 %s건 · 약 %sK\n' "$removed" "$bytes"
