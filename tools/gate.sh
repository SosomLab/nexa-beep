#!/usr/bin/env bash
# gate.sh — 로컬 게이트 한 번에(docs/18 §1 · CI 잡 하나하나에 대응 행이 있어야 한다는 10-09 교훈).
#   ./tools/gate.sh            # fmt --check · clippy(-D warnings) · rustdoc(-D warnings) · test · win x64·mac arm64 check
#   ./tools/gate.sh --fast     # 테스트·크로스 check 생략(문서/주석만 고쳤을 때)
# 실패하면 그 단계에서 멈춘다(종료 코드 ≠ 0). 10-09 v0.3.0·v0.3.1 CI가 rustdoc 한글 대괄호 링크로 두 번 죽은 뒤 신설.
set -euo pipefail
cd "$(dirname "${BASH_SOURCE[0]}")/.."
F=nbeep-core/testkit,nbeep-net/testkit,nbeep-crypto/testkit
FAST=0; [ "${1:-}" = "--fast" ] && FAST=1
step() { printf '\n\033[1m▶ %s\033[0m\n' "$1"; }
step "fmt --check";   cargo fmt --all -- --check
step "clippy";        cargo clippy --workspace --all-targets --features "$F" -- -D warnings
step "rustdoc 링크";  RUSTDOCFLAGS="-D warnings" cargo doc --workspace --no-deps --features "$F" >/dev/null
if [ "$FAST" = 0 ]; then
  step "test";        cargo test --workspace --features "$F" 2>&1 | grep -E "^test result|FAILED|panicked" | awk '{p+=$4; f+=$6; i+=$8} /FAILED|panicked/{print; bad=1} END{print "합계 passed="p" failed="f" ignored="i; exit bad}'
  for t in x86_64-pc-windows-msvc aarch64-apple-darwin; do
    step "check --target $t"; cargo check --workspace --all-targets --target "$t" --features "$F" >/dev/null
  done
fi
printf '\n\033[32m✅ 게이트 통과\033[0m\n'
