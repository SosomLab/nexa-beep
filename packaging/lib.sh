#!/usr/bin/env bash
# packaging/lib.sh — 패키징 스크립트 공용 조각(source 해서 쓴다 · 실행 파일 아님).
#
# 이식 원천: ../nexa-sql/packaging/lib.sh (10-09 · 사용자 "nexa-sql을 참고해서 리눅스 배포 진행").
# 원칙: **스크립트 = 로컬에서도 CI에서도 같은 명령**. release.yml이 포장 셸 조각을 따로 들고 있으면
#   언젠가 갈라지고 그 지점이 곧 버그가 된다 — 08-29 `.desktop` 런처 결함(packaging/ 수정이 CI 인라인에
#   반영되지 않아 v0.2.2~0.2.9 Linux 전부 데모를 띄움)이 정확히 그 유형이었다.
#
# 제공:
#   ROOT          저장소 루트            OUT_ROOT = $ROOT/target/packaging (git 무시 · /target/)
#   VERSION       Cargo.toml [workspace.package] version (NEXA_VERSION으로 덮어쓰기 — release.yml이 태그 버전을 넘긴다)
#   APP_NAME      "Nexa Beep"            BUNDLE_ID = com.sosomlab.nexa-beep
#   step / note / die / have / sha256_of / cargo_bin / stage_docs <dir> / glibc_floor <bin...> / slug_of <target>

set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
OUT_ROOT="$ROOT/target/packaging"
BRANDING="$ROOT/packaging/branding"
APP_NAME="Nexa Beep"
BUNDLE_ID="com.sosomlab.nexa-beep"
PUBLISHER="SosomLab"
MAINTAINER="Sangyong Bae <kiros33@gmail.com>"
HOMEPAGE="https://github.com/SosomLab/nexa-beep"

# 버전 = 워크스페이스 단일 원천. 태그 빌드는 release.yml이 NEXA_VERSION으로 넘긴다
# (태그 ≠ Cargo 버전은 brew formula의 test가 잡는다 — 18 §5).
VERSION="${NEXA_VERSION:-$(grep -m1 '^version = ' "$ROOT/Cargo.toml" | sed -E 's/version = "(.*)"/\1/')}"

step() { printf '\n── %s ──\n' "$*"; }
note() { printf '  · %s\n' "$*"; }
die()  { printf '  ✗ %s\n' "$*" >&2; exit 1; }
have() { command -v "$1" >/dev/null 2>&1; }

# sha256 — OS마다 도구 이름이 다르다(리눅스 sha256sum · macOS shasum).
sha256_of() {
    if have sha256sum; then sha256sum "$1" | cut -d' ' -f1; else shasum -a 256 "$1" | cut -d' ' -f1; fi
}

# rustup 툴체인을 우선한다 — PATH에 배포판/Homebrew cargo가 앞서면 크로스 타깃 std를 못 찾는다(nexa-sql 09-16 실기).
cargo_bin() {
    if have rustup; then
        local rb; rb="$(dirname "$(rustup which cargo 2>/dev/null || echo "$HOME/.cargo/bin/cargo")")"
        [ -x "$rb/cargo" ] && { echo "$rb/cargo"; return; }
    fi
    command -v cargo
}

# 릴리스 자산 슬러그(release.yml 매트릭스와 같은 철자 — 자산 이름 = nexa-beep-<ver>-<slug>.*).
slug_of() {
    case "$1" in
        x86_64-unknown-linux-*)  echo linux-x64 ;;
        aarch64-unknown-linux-*) echo linux-arm64 ;;
        *) die "슬러그 미정의 타깃: $1" ;;
    esac
}

# 세 OS가 똑같이 담는 문서: 라이선스 2종 · README.
stage_docs() {
    local dst="$1"
    mkdir -p "$dst"
    install -m 0644 "$ROOT/LICENSE.md"    "$dst/LICENSE.md"
    install -m 0644 "$ROOT/LICENSE.ko.md" "$dst/LICENSE.ko.md"
    install -m 0644 "$ROOT/README.md"     "$dst/README.md"
}

# ★ glibc 하한 = **선언이 아니라 실측**. 바이너리의 심볼 버전 요구 중 최댓값을 읽는다.
#   왜: v0.2.17까지 control이 `libc6 (>= 2.31)`을 선언했지만 ubuntu-latest(24.04) 빌드는 GLIBC_2.39를
#   요구했다(10-09 공개 .deb objdump 실측) → Ubuntu 22.04·Debian 12에서 설치는 되고 실행만 실패.
#   "불변식은 검증이 붙어야 지켜진다"(09-17 NFR-B-5 교훈)의 Linux 판.
glibc_floor() {
    have objdump || die "objdump 없음(binutils)"
    local max=""
    for b in "$@"; do
        local v
        v="$(objdump -T "$b" | grep -oE 'GLIBC_[0-9]+\.[0-9]+' | sed 's/GLIBC_//' | sort -t. -k1,1n -k2,2n -u | tail -1)"
        [ -n "$v" ] || die "GLIBC 심볼 버전을 못 읽음: $b"
        if [ -z "$max" ] || [ "$(printf '%s\n%s\n' "$max" "$v" | sort -t. -k1,1n -k2,2n | tail -1)" != "$max" ]; then max="$v"; fi
    done
    echo "$max"
}
