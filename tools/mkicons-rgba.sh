#!/usr/bin/env bash
# mkicons-rgba.sh — 컬러 SVG 아이콘을 96x96 RGBA 원시 바이트로 굽는다 (개발 도구 · 배포물 아님).
#
# mkicons.sh(1채널 알파 마스크 = 모양만·틴트)와 다른 축: **색이 아이콘의 정보**인 자산
# (M3-14b 신뢰 배지 — 상태를 색+모양 2중으로 나른다)은 RGBA 그대로 굽는다.
# 본체는 여전히 런타임 SVG 파서 0 — `IconImage::from_rgba`(brand-64.rgba 선례)로 읽는다.
#
# 입력  : assets/icons-src/<name>.svg              (커밋된 원본 — 출처·라이선스는 docs/10 §4)
# 출력  : crates/nbeep-ui/assets/<name>-96.rgba    (96*96*4 = 36,864 바이트)
# 사용법: tools/mkicons-rgba.sh <name> [name ...]
#
# 필요 도구: rsvg-convert(librsvg) · magick(ImageMagick) — mkicons.sh와 동일.

set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
SRC_DIR="${ROOT}/assets/icons-src"
OUT_DIR="${ROOT}/crates/nbeep-ui/assets"
SIZE=96

for tool in rsvg-convert magick; do
  if ! command -v "${tool}" >/dev/null 2>&1; then
    echo "error: ${tool} 이(가) 없다 — mkicons.sh 주석의 설치 명령 참고." >&2
    exit 1
  fi
done

if [ "$#" -eq 0 ]; then
  echo "사용법: tools/mkicons-rgba.sh <name> [name ...]  (assets/icons-src/<name>.svg)" >&2
  exit 1
fi

tmp="$(mktemp -d)"
trap 'rm -rf "${tmp}"' EXIT

for name in "$@"; do
  svg="${SRC_DIR}/${name}.svg"
  if [ ! -f "${svg}" ]; then
    echo "error: 원본이 없다 — ${svg}" >&2
    exit 1
  fi
  rsvg-convert -w "${SIZE}" -h "${SIZE}" -b none "${svg}" -o "${tmp}/${name}.png"
  # RGBA 원시 바이트 — 채널 순서 R,G,B,A(IconImage::from_rgba 계약).
  magick "${tmp}/${name}.png" -depth 8 "rgba:${OUT_DIR}/${name}-${SIZE}.rgba"
  out="${OUT_DIR}/${name}-${SIZE}.rgba"
  bytes="$(wc -c < "${out}" | tr -d ' ')"
  want="$((SIZE * SIZE * 4))"
  if [ "${bytes}" != "${want}" ]; then
    echo "error: ${out} 크기가 ${bytes} — ${want} 여야 한다" >&2
    exit 1
  fi
  echo "ok  ${out}  (${bytes} bytes)"
done
