#!/usr/bin/env bash
# xvfb.sh — Linux GUI 자동 시험을 **가상 디스플레이(Xvfb)** 안에서(사용자 11-02 "xvfb처럼 다른 디스플레이에서" · nexa-sql D-435 이식).
#   창이 사용자 화면에 뜨지 않고 포커스·클립보드·입력과 분리된다. 쓰는 법: 스크립트 서두 `cd …` 다음에
#   `. "$(dirname "${BASH_SOURCE[0]}")/lib/xvfb.sh"` 한 줄(tools/ 바로 아래 스크립트 기준).
#     Linux + NEXA_XVFB≠0 + xvfb-run 있음 + 아직 Xvfb 안이 아니면 → 같은 스크립트를 `xvfb-run -a -s "-screen 0 <화면>"` 아래에서
#     다시 실행하고 그 종료 코드로 끝난다(재진입 표식 NEXA_XVFB_ACTIVE=1 · 자식은 다시 감싸지 않는다).
#     NEXA_XVFB=0 = 실제 DISPLAY · xvfb-run 없음 = 경고 한 줄 뒤 실제 화면(`sudo apt install xvfb x11-utils`) ·
#     화면 = NEXA_XVFB_SCREEN(기본 1600x1000x24). macOS·Windows에서는 아무것도 하지 않는다.
#     Wayland 세션이면 WAYLAND_DISPLAY·WAYLAND_SOCKET을 지워 winit이 Xvfb(X11)에 붙게 한다(11-08 Ubuntu GNOME 실측 — 안 지우면 실제 컴포지터에 붙어 사용자 화면에 창이 뜬다).
#   ★ 사용자 실기 인스턴스(relaunch.sh)는 이 파일을 포함하지 않는다 — 실기 창은 실제 화면에 떠야 한다.
if [ "$(uname -s)" = Linux ] && [ "${NEXA_XVFB:-1}" != 0 ] && [ -z "${NEXA_XVFB_ACTIVE:-}" ]; then
  if command -v xvfb-run >/dev/null 2>&1; then
    export NEXA_XVFB_ACTIVE=1
    echo "[xvfb] re-exec under Xvfb (${NEXA_XVFB_SCREEN:-1600x1000x24}) — NEXA_XVFB=0 uses the real display" >&2
    exec env -u WAYLAND_DISPLAY -u WAYLAND_SOCKET xvfb-run -a -s "-screen 0 ${NEXA_XVFB_SCREEN:-1600x1000x24}" bash "$0" "$@"
  else
    echo "[xvfb] xvfb-run not found — running on the real display (sudo apt install xvfb x11-utils)" >&2
  fi
fi
# 헤드리스 공통 진입점 — NEXA_TEST_HEADLESS=1이면 앱 자체 헤드리스(lib/headless.sh · 가드 있음).
. "${BASH_SOURCE[0]%/*}/headless.sh"
