#!/usr/bin/env bash
# 설치본 자리 덮어쓰기 실기 — 릴리스 빌드 → 실행 중지 → **설치된 파일 위에** 복사 → 설치본처럼 실행.
#
#   ./tools/install-local.sh              # ① 종료 ② 릴리스 빌드 ③ 설치 자리 덮어쓰기 ④ 설치본 실행 ⑤ 확인
#   ./tools/install-local.sh --no-build   # 빌드 건너뜀(직전 산출물 그대로)
#   ./tools/install-local.sh --no-run     # 복사까지만(실행은 사용자가)
#
# 왜: "고쳤는데 설치본에는 언제 들어가나"를 매번 릴리스(태그·CI·다운로드·재설치)로 풀면
# 실기 1회에 10분이 든다(08-29). 설치본 경로·자동 실행 등록·런처(.desktop/.app/시작 메뉴)는
# **설치 자리에서만** 재현되므로, 산출물을 그 자리에 얹어 "새 버전을 설치한 것처럼" 돌린다.
#
# OS별 설치 자리(= 각 포장의 SSOT와 동일해야 한다 — 바뀌면 여기도):
#   Linux   .deb            /usr/bin/{nexa-beep,nbeep-imgdec}                (root 소유 → sudo 1회)
#   macOS   brew cask .app  /Applications/Nexa Beep.app/Contents/MacOS/…     (사용자 소유 · sudo 불요)
#   Windows NSIS(HKCU)      %LOCALAPPDATA%\Programs\NexaBeep\…exe            (사용자 소유 · sudo 불요)
#
# ⚠ 버전 문자열은 Cargo.toml 그대로다 — 패키지 관리자(dpkg/brew/winget)의 등록 버전은 바뀌지 않고,
#   다음 정식 설치가 이 파일을 다시 덮어쓴다. 실기 전용이지 배포 대체가 아니다.
set -u
cd "$(dirname "$0")/.."
if ! command -v cargo >/dev/null 2>&1 && [ -x "$HOME/.cargo/bin/cargo" ]; then
  export PATH="$HOME/.cargo/bin:$PATH"
fi
BUILD=1 RUN=1
for a in "$@"; do
  case "$a" in
    --no-build) BUILD=0 ;;
    --no-run) RUN=0 ;;
    -h | --help) sed -n '2,20p' "$0"; exit 0 ;;
    *) echo "알 수 없는 인자: $a (--help)"; exit 2 ;;
  esac
done
say() { printf '\n\033[1m▶ %s\033[0m\n' "$1"; }

# ── OS 판별 · 설치 자리 ────────────────────────────────────────────────────
OS=linux EXE=""
case "${OSTYPE:-}" in
  darwin*) OS=mac ;;
  msys* | cygwin*) OS=win; EXE=".exe" ;;
esac
pw() { powershell.exe -NoProfile -Command "$1" 2>/dev/null | tr -d '\r'; }
case "$OS" in
  linux) DEST=/usr/bin ;;
  mac) DEST="/Applications/Nexa Beep.app/Contents/MacOS" ;;
  win)
    LAD=$(pw '$env:LOCALAPPDATA')
    DEST="$(cygpath -u "$LAD")/Programs/NexaBeep" ;;
esac
[ -d "$DEST" ] || { echo "❌ 설치 자리가 없다: $DEST — 먼저 정식 설치본을 한 번 설치한다(.deb / brew cask / NSIS)"; exit 1; }
[ -x "$DEST/nexa-beep$EXE" ] || { echo "❌ 설치본 실행 파일이 없다: $DEST/nexa-beep$EXE"; exit 1; }
echo "설치 자리 = $DEST ($OS)"

# ── ① 종료 ───────────────────────────────────────────────────────────────
say "① 실행 중인 nexa-beep 종료"
APP_RE='nexa-beep(\.exe)?( |$)'
if [ "$OS" = win ]; then
  pw "Get-Process nexa-beep -ErrorAction SilentlyContinue | Stop-Process -Force" >/dev/null
else
  pkill -f "$APP_RE" 2>/dev/null
fi
sleep 2
if [ "$OS" != win ] && pgrep -f "$APP_RE" >/dev/null 2>&1; then pkill -9 -f "$APP_RE"; sleep 1; fi
echo "   종료 완료"

# ── ② 빌드 ───────────────────────────────────────────────────────────────
if [ "$BUILD" = 1 ]; then
  say "② 릴리스 빌드 (nexa-beep + nbeep-imgdec)"
  LOG=$(mktemp)
  if ! cargo build --release -p nexa-beep -p nbeep-imgdec > "$LOG" 2>&1; then
    echo "   ❌ 빌드 실패 — 마지막 20줄:"; tail -20 "$LOG" | sed 's/^/   /'; rm -f "$LOG"; exit 1
  fi
  tail -1 "$LOG"; rm -f "$LOG"
else
  say "② 빌드 건너뜀 (--no-build)"
fi
SRC=target/release
[ -x "$SRC/nexa-beep$EXE" ] && [ -x "$SRC/nbeep-imgdec$EXE" ] || { echo "   ❌ 산출물이 없다: $SRC"; exit 1; }

# ── ③ 설치 자리 덮어쓰기 ─────────────────────────────────────────────────
say "③ 설치 자리 덮어쓰기 → $DEST"
sum() { if command -v md5sum >/dev/null 2>&1; then md5sum "$1" | cut -c1-12; else md5 -q "$1" | cut -c1-12; fi; }
BEFORE=$(sum "$DEST/nexa-beep$EXE")
copy_in() {
  # install = 새 inode로 교체(실행 중 텍스트 잠금·"text file busy" 회피) + 권한 755.
  "$@" install -m 755 "$SRC/nexa-beep$EXE" "$SRC/nbeep-imgdec$EXE" "$DEST/"
}
if [ -w "$DEST/nexa-beep$EXE" ] && [ -w "$DEST" ]; then
  copy_in || { echo "   ❌ 복사 실패"; exit 1; }
else
  echo "   설치 자리가 root 소유 — sudo 1회(비밀번호 입력)"
  copy_in sudo || { echo "   ❌ 복사 실패(sudo)"; exit 1; }
fi
# ★ md5 대조는 **재서명 전에**(08-30 mac 실측 — ad-hoc codesign이 바이너리에 서명을 박아
#   넣어 산출물과 md5가 달라진다 · 서명 뒤 대조는 mac에서 항상 실패).
AFTER=$(sum "$DEST/nexa-beep$EXE")
echo "   nexa-beep: $BEFORE → $AFTER $([ "$BEFORE" = "$AFTER" ] && echo '(동일 — 산출물이 바뀌지 않았다)' || echo '✓ 교체')"
[ "$AFTER" = "$(sum "$SRC/nexa-beep$EXE")" ] || { echo "   ❌ 설치 자리와 산출물이 다르다"; exit 1; }
# mac: 번들 안 실행 파일을 바꾸면 코드 서명이 어긋난다(ad-hoc 재서명 · 격리 속성 제거).
if [ "$OS" = mac ]; then
  codesign --force --deep --sign - "/Applications/Nexa Beep.app" 2>/dev/null || echo "   ⚠ codesign 실패(무시 가능 · 실행이 막히면 xattr -dr com.apple.quarantine)"
  xattr -dr com.apple.quarantine "/Applications/Nexa Beep.app" 2>/dev/null || true
  echo "   codesign: $(codesign -dv "/Applications/Nexa Beep.app" 2>&1 | grep -o 'Signature=.*' || echo '?')"
fi
echo "   버전: $("$DEST/nexa-beep$EXE" --version 2>/dev/null)"

# ── ④ 설치본처럼 실행(런처 경로 = 무인자) ────────────────────────────────
if [ "$RUN" = 1 ]; then
  say "④ 설치본 실행"
  case "$OS" in
    linux)
      if command -v gtk-launch >/dev/null 2>&1 && [ -f /usr/share/applications/nexa-beep.desktop ]; then
        gtk-launch nexa-beep >/dev/null 2>&1 & disown   # 앱 그리드와 같은 경로(.desktop Exec)
      else
        nohup "$DEST/nexa-beep" >/dev/null 2>&1 & disown
      fi ;;
    mac) open -a "Nexa Beep" ;;                             # Finder 실행 = 인자 없음 경로
    win) pw "Start-Process -FilePath '$(cygpath -w "$DEST/nexa-beep.exe")'" >/dev/null ;;
  esac
  sleep 3
  # ── ⑤ 확인 ──
  say "⑤ 확인"
  if [ "$OS" = win ]; then
    N=$(pw "(Get-Process nexa-beep -ErrorAction SilentlyContinue | Measure-Object).Count" | tr -d ' ')
  else
    N=$(pgrep -f "$APP_RE" | wc -l | tr -d ' ')
  fi
  echo "   실행 중 프로세스 = ${N:-0}개"
  [ "${N:-0}" != "0" ] || { echo "   ❌ 창이 뜨지 않았다"; exit 1; }
  "$DEST/nexa-beep$EXE" --whoami 2>/dev/null | sed 's/^/   /' | head -6
else
  say "④ 실행 생략 (--no-run)"
fi
echo; echo "완료 — 설치본($DEST)이 현재 작업 트리 산출물로 바뀌었다. 정식 설치(.deb/brew/NSIS)가 다시 덮어쓴다."
