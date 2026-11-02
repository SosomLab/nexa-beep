#!/usr/bin/env bash
# headless.sh — 앱 자체 헤드리스 백엔드(NEXA_HEADLESS=1 · 11-02 · nexa-sql D-436 이식) — 3-OS 공통 · xvfb.sh 끝에서 포함된다.
#   NEXA_TEST_HEADLESS=1 이면 NEXA_HEADLESS=1을 내보낸다 = 창이 화면에 전혀 뜨지 않고(OS 창 0) 포커스·입력·클립보드와 분리.
#   헤드리스에서는 OS가 창을 못 보므로(xwininfo · EnumWindows · CGWindowList = 0) 창 수 판정 = NEXA_SCRIPT `windows.dump=<파일>`
#   (첫 줄 wins=N · 줄마다 제목\t가로x세로\tvisible|hidden) · 캡처 = `capture=<png>`(메인) · `capture=<png>@<창 제목 일부>`(보조 창).
#   기본 = 꺼짐(종전 = Linux Xvfb · Windows/mac 실제 화면). 사용자 실기 인스턴스 스크립트는 이 파일을 포함하지 않는다.
if [ "${NEXA_TEST_HEADLESS:-0}" != 0 ]; then
  export NEXA_HEADLESS=1
  [ -n "${NEXA_HEADLESS_LIB_SAID:-}" ] ||
  echo "[headless] NEXA_HEADLESS=1 — windows stay hidden; window count = windows.dump · capture = capture:<png>[@<title>]" >&2
  export NEXA_HEADLESS_LIB_SAID=1
fi
hl_on() { [ "${NEXA_HEADLESS:-0}" != 0 ]; }
# windows.dump 파일의 wins=N 값.
hl_wins_read() { grep -o 'wins=[0-9]*' "$1" 2>/dev/null | head -1 | cut -d= -f2; }
