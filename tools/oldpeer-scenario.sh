#!/usr/bin/env bash
# oldpeer-scenario.sh — 구버전 상대 호환 자동 실기(REPLY-1 ⑤ · 11-04): 신버전 X(target/debug) ↔ **실물 구버전 Y**(GitHub Release
#   포터블 · 기본 v0.6.1 = 답장 kind 2를 모르는 마지막 판). Y가 X에게 원본을 보내고 X가 `reply=`로 답장 → Y는 평문
#   (`┃ 이름: 발췌` + 본문)으로 받아야 하고 X 로그에 `reply_fallback=1`이 남아야 한다(53a1a68). 새 와이어 kind를 더할 때마다
#   같은 틀로 "구버전이 조용히 버리지 않는가"를 본다.
#
# 사용:  tools/oldpeer-scenario.sh [구버전 태그=v0.6.1] [출력 폴더]
#   구버전 바이너리는 target/oldpeer/<태그>/ 에 `gh release download`로 받아 둔다(한 번 · 신뢰 = 우리 Release 자산).
# ⚠ 구버전(≤0.6.1)은 NEXA_HEADLESS를 모른다 → Y 창이 **실제 화면에 잠깐(≈45s) 뜬다**. 사용자 화면을 쓰는 중이면 돌리지 않는다.
#   신버전 X는 헤드리스. Windows(Git Bash)·mac·Linux 공통(자산 이름 = OS별 포터블).
set -uo pipefail
cd "$(dirname "${BASH_SOURCE[0]}")/.."
TAG=${1:-v0.6.1}
M=${BEEP_MULTI:-$HOME/.nexa-beep-multi}
OUT=${2:-$M/oldpeer-out}; rm -rf "$OUT"; mkdir -p "$OUT"; OUT=$(cd "$OUT" && pwd)
export NEXA_NO_AUTOSTART=1
trap 'BEEP_MULTI="$M" ./tools/autostart-clean.sh -q' EXIT
EXE=""; ASSET=""
case "$(uname -s)" in
  MINGW*|MSYS*|CYGWIN*) EXE=".exe"; ASSET="nexa-beep-${TAG#v}-windows-x64-portable.zip" ;;
  Darwin) ASSET="nexa-beep-${TAG#v}-macos-$(uname -m | sed 's/arm64/aarch64/')-portable.tar.gz" ;;
  Linux)  ASSET="nexa-beep-${TAG#v}-linux-x64-portable.tar.gz" ;;
esac
[ -x "target/debug/nexa-beep$EXE" ] || { echo "❌ target/debug/nexa-beep 없음 — cargo build -p nexa-beep -p nbeep-imgdec"; exit 1; }
OLD="target/oldpeer/$TAG"
if [ ! -x "$OLD/nexa-beep$EXE" ]; then
  mkdir -p "$OLD"
  echo "▶ 구버전 $TAG 포터블 받기 → $OLD ($ASSET)"
  gh release download "$TAG" -p "$ASSET" -D "$OLD" --clobber || { echo "❌ gh release download 실패(자산 이름 확인: gh release view $TAG)"; exit 1; }
  case "$ASSET" in
    *.zip) (cd "$OLD" && unzip -q -o "$ASSET") ;;
    *.tar.gz) (cd "$OLD" && tar xzf "$ASSET") ;;
  esac
  # 포터블 묶음 안의 실행 파일을 폴더 최상위로(묶음 구조가 바뀌어도 한 번 찾으면 된다).
  f=$(find "$OLD" -name "nexa-beep$EXE" -type f | head -1); [ -n "$f" ] && [ "$f" != "$OLD/nexa-beep$EXE" ] && cp -f "$f" "$OLD/"
  g=$(find "$OLD" -name "nbeep-imgdec$EXE" -type f | head -1); [ -n "$g" ] && [ "$g" != "$OLD/nbeep-imgdec$EXE" ] && cp -f "$g" "$OLD/"
fi
[ -x "$OLD/nexa-beep$EXE" ] || { echo "❌ $OLD/nexa-beep 없음"; exit 1; }
echo "▶ 구버전 = $("$OLD/nexa-beep$EXE" --version 2>/dev/null | head -1) · 신버전 = $(target/debug/nexa-beep$EXE --version 2>/dev/null | head -1)"

seed() { # 폴더 · 표시 이름 · 바이너리 폴더
  rm -rf "$M/$1"; mkdir -p "$M/$1/data"
  cp -f "$3/nexa-beep$EXE" "$M/$1/"; [ -f "$3/nbeep-imgdec$EXE" ] && cp -f "$3/nbeep-imgdec$EXE" "$M/$1/"
  printf '_schema=1\nprofile.display_name=%s\nchat.window_mode=single\napp.autostart=off\nlog.dev_mode=on\nlog.dev_layers=chat\n' "$2" > "$M/$1/data/settings.cfg"
}
run() { # 폴더 · 스크립트 · 로그 · 헤드리스(1/0)
  ( cd "$M/$1" && NEXA_HEADLESS="$4" NEXA_DEVLOG=chat NEXA_SCRIPT="$2" "./nexa-beep$EXE" --window --live 2> "$3" >/dev/null ) &
}
echo "▶ 신원 준비(새로) — $M/{op-x(신),op-y(구)} · 출력 $OUT"
seed op-x opx target/debug
seed op-y opy "$OLD"
echo "▶ X(신·헤드리스) 10s 연결·14s 창 / Y(구·화면) 18s 창·22s send 원본 / X 30s reply / 40s dump · 44s quit"
run op-x "10000:activate=opy;14000:activate=opy;30000:reply=폴백 답장;40000:dump;44000:quit" "$OUT/x.log" 1
run op-y "18000:activate=opx;22000:send=원본 글 하나;40000:dump;44000:quit" "$OUT/y.log" 0
wait

echo; echo "═══ 판정 ═══"
ok=0; bad=0
chk() { local n; n=$(grep -c -E "$3" "$2" 2>/dev/null || true); n=${n:-0}
  if [ "$n" -ge 1 ]; then printf '  ✓ %s (%s건)\n' "$1" "$n"; ok=$((ok+1)); else printf '  ✗ %s (0건)\n' "$1"; bad=$((bad+1)); fi; }
chk "Y(구)→X 원본 도착"                                   "$OUT/x.log" 'thread .* mine=false .* text=원본 글 하나'
chk "X 내 풍선 = 답장 그대로(참조+발췌)"                   "$OUT/x.log" 'thread .* mine=true .* text=폴백 답장 reply=[0-9a-f]{8}#[1-9][0-9]*:원본 글 하나'
chk "X 로그: 기기별 평문 폴백 1건(reply_fallback=1)"        "$OUT/x.log" 'send_text .* reply_fallback=1'
chk "Y(구) 수신 = 평문 인용 한 줄(┃ opy: 원본 글 하나)"     "$OUT/y.log" 'thread .* mine=false .* text=┃ opy: 원본 글 하나'
chk "Y(구) 수신 본문(폴백 답장)이 뒤따른다"                 "$OUT/y.log" '^폴백 답장$'
echo "결과: ✓ $ok · ✗ $bad   (로그 = $OUT/{x,y}.log)"
[ "$bad" = 0 ]
