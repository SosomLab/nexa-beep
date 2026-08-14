#!/usr/bin/env bash
# mkicons.sh — SVG 아이콘 원본을 96x96 1채널 알파 마스크로 굽는다 (개발 도구 · 배포물 아님).
#
# 왜 사전 래스터인가: 본체는 런타임 SVG 파서를 링크하지 않는다(자산 규약 08-09 ·
# ToolIcon::Mask = "모양만" · 색은 테마 기준색 틴트). 그래서 모양을 빌드 전에 구워 둔다.
#
# 입력  : assets/icons-src/<name>.svg          (커밋된 원본 — 출처·라이선스는 docs/10 §4)
# 출력  : crates/nbeep-ui/assets/icon-<name>-96.alpha   (96*96 = 9216 바이트)
# 사용법: tools/mkicons.sh [name ...]          (인자 없으면 icons-src 전체)
#
# 필요 도구: rsvg-convert(librsvg) · magick(ImageMagick)
#   macOS  : brew install librsvg imagemagick
#   Ubuntu : apt install librsvg2-bin imagemagick
#
# 굽고 나면 crates/nbeep-ui/src/lib.rs 의 icons 모듈에 상수를 추가해야 실제로 쓰인다.

set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
SRC_DIR="${ROOT}/assets/icons-src"
OUT_DIR="${ROOT}/crates/nbeep-ui/assets"
SIZE=96

for tool in rsvg-convert magick; do
  if ! command -v "${tool}" >/dev/null 2>&1; then
    echo "error: ${tool} 이(가) 없다. 위 주석의 설치 명령을 참고할 것." >&2
    exit 1
  fi
done

if [ "$#" -gt 0 ]; then
  names=("$@")
else
  names=()
  for f in "${SRC_DIR}"/*.svg; do
    names+=("$(basename "${f}" .svg)")
  done
fi

tmp="$(mktemp -d)"
trap 'rm -rf "${tmp}"' EXIT

for name in "${names[@]}"; do
  svg="${SRC_DIR}/${name}.svg"
  if [ ! -f "${svg}" ]; then
    echo "error: 원본이 없다 — ${svg}" >&2
    exit 1
  fi

  # stroke="currentColor" 는 래스터라이저가 해석하지 못한다 — 검정으로 고정한다.
  # 알파만 뽑아 쓰므로 색 자체는 의미가 없다(모양만 남는다).
  sed 's/currentColor/#000000/g' "${svg}" > "${tmp}/${name}.svg"

  rsvg-convert -w "${SIZE}" -h "${SIZE}" -b none "${tmp}/${name}.svg" -o "${tmp}/${name}.png"
  magick "${tmp}/${name}.png" -alpha extract -depth 8 "gray:${OUT_DIR}/icon-${name}-${SIZE}.alpha"

  out="${OUT_DIR}/icon-${name}-${SIZE}.alpha"
  bytes="$(wc -c < "${out}" | tr -d ' ')"
  want="$((SIZE * SIZE))"
  if [ "${bytes}" != "${want}" ]; then
    echo "error: ${out} 크기가 ${bytes} — ${want} 여야 한다" >&2
    exit 1
  fi
  echo "ok  ${out}  (${bytes} bytes)"
done
