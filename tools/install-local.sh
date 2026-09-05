#!/usr/bin/env bash
# 설치본 자리 덮어쓰기 실기 — 릴리스 빌드 → 실행 중지 → **설치된 파일 위에** 복사 → 설치본처럼 실행.
#
#   ./tools/install-local.sh              # ① 종료 ② 릴리스 빌드 ③ 설치 자리 덮어쓰기 ④ 설치본 실행 ⑤ 확인
#   ./tools/install-local.sh --debug      # 디버그 프로필(패닉 위치 등 진단 · 배포본과 최적화가 다르다)
#   ./tools/install-local.sh --no-build   # 빌드 건너뜀(직전 산출물 그대로)
#   ./tools/install-local.sh --no-run     # 복사까지만(실행은 사용자가)
#   ./tools/install-local.sh --assets     # (Linux) .desktop·아이콘까지 갱신 — packaging/linux를 고쳤을 때
#   NEXA_INSTALL_DIR=/opt/x ./tools/install-local.sh   # 비표준 설치 자리 강제(Linux · mac은 .app 고정)
#   Windows = `pwsh tools/install-local.ps1`(네이티브) — 이 스크립트를 Git Bash에서 부르면 그쪽으로 위임한다.
#
# 왜: "고쳤는데 설치본에는 언제 들어가나"를 매번 릴리스(태그·CI·다운로드·재설치)로 풀면
# 실기 1회에 10분이 든다(08-29). 설치본 경로·자동 실행 등록·런처(.desktop/.app/시작 메뉴)는
# **설치 자리에서만** 재현되므로, 산출물을 그 자리에 얹어 "새 버전을 설치한 것처럼" 돌린다.
# (09-05 개정 — nexa-clip `scripts/dev-install-*` 와 기능 동등: 프로필 선택 · 설치 자리 탐지 ·
#  `--assets` · 런처 경로 명시 기동 · 설치본 stdout 로그 파일 · sudo 불가 자리의 pkexec 폴백.)
#
# OS별 설치 자리(= 각 포장의 SSOT와 동일해야 한다 — 바뀌면 여기도):
#   Linux   .deb            /usr/bin/{nexa-beep,nbeep-imgdec}  ← `command -v nexa-beep`로 실제 자리를 찾는다(root → sudo/pkexec 1회)
#   macOS   brew cask .app  /Applications/Nexa Beep.app/Contents/MacOS/…     (사용자 소유 · sudo 불요 · ad-hoc 재서명)
#   Windows NSIS(HKCU)      %LOCALAPPDATA%\Programs\NexaBeep\…exe            (→ tools/install-local.ps1)
#
# 데이터: 설치본은 "업그레이드 때 교체되는 자리"(/usr·.app 번들)라 exe 옆이 아니라 사용자 폴더를 쓴다
#   (Linux ~/.config/nexa-beep · mac ~/Library/Application Support/nexa-beep — nexa_conf::is_replaced_on_upgrade).
#   개발 인스턴스(target/*/data · ~/.nexa-beep-multi)와 이력·설정·신원이 **다르다** → `--whoami`로 확인.
# ⚠ 버전 문자열은 Cargo.toml 그대로다 — 패키지 관리자(dpkg/brew)의 등록 버전은 바뀌지 않고,
#   다음 정식 설치가 이 파일을 다시 덮어쓴다. 실기 전용이지 배포 대체가 아니다.
set -u
cd "$(dirname "$0")/.."
ROOT=$PWD
if ! command -v cargo >/dev/null 2>&1 && [ -x "$HOME/.cargo/bin/cargo" ]; then
  export PATH="$HOME/.cargo/bin:$PATH"
fi
BUILD=1 RUN=1 ASSETS=0 PROFILE=release
CARGO_FLAGS=(--release)
for a in "$@"; do
  case "$a" in
    --no-build) BUILD=0 ;;
    --no-run) RUN=0 ;;
    --debug) PROFILE=debug; CARGO_FLAGS=() ;;
    --assets) ASSETS=1 ;;
    -h | --help) sed -n '2,27p' "$0"; exit 0 ;;
    *) echo "알 수 없는 인자: $a (--help)"; exit 2 ;;
  esac
done
say() { printf '\n\033[1m▶ %s\033[0m\n' "$1"; }

# ── OS 판별 ──────────────────────────────────────────────────────────────
OS=linux
case "${OSTYPE:-}" in
  darwin*) OS=mac ;;
  msys* | cygwin*)
    # Windows = 네이티브 ps1로 위임(Git Bash 의존 제거 · 09-05). 인자 매핑만 한다.
    PSARGS=()
    [ "$BUILD" = 0 ] && PSARGS+=(-NoBuild)
    [ "$RUN" = 0 ] && PSARGS+=(-NoRun)
    [ "$PROFILE" = debug ] && PSARGS+=(-Debug)
    PS=$(command -v pwsh || command -v powershell.exe || true)
    [ -n "$PS" ] || { echo "❌ pwsh/powershell을 찾을 수 없다 — tools/install-local.ps1을 직접 실행"; exit 1; }
    exec "$PS" -NoProfile -File "$(cygpath -w "$ROOT/tools/install-local.ps1" 2>/dev/null || echo "$ROOT/tools/install-local.ps1")" "${PSARGS[@]}" ;;
esac

# ── 설치 자리 ────────────────────────────────────────────────────────────
case "$OS" in
  linux)
    if [ -n "${NEXA_INSTALL_DIR:-}" ]; then
      DEST=$NEXA_INSTALL_DIR
    else
      INSTALLED=$(command -v nexa-beep 2>/dev/null || true)
      if [ -n "$INSTALLED" ]; then
        DEST=$(dirname "$(readlink -f "$INSTALLED")")
      else
        DEST=/usr/bin
      fi
    fi ;;
  mac) DEST="/Applications/Nexa Beep.app/Contents/MacOS" ;;
esac
# 개발 트리를 설치 자리로 오인하지 않는다(target/ 을 자기 자신에 덮어쓰는 사고 방지).
case "$DEST" in
  "$ROOT"/target/*) echo "❌ 설치 자리가 개발 트리다($DEST) — 배포본을 설치한 뒤 다시(또는 NEXA_INSTALL_DIR 지정)"; exit 1 ;;
esac
[ -d "$DEST" ] || { echo "❌ 설치 자리가 없다: $DEST — 먼저 정식 설치본을 한 번 설치한다(.deb / brew cask)"; exit 1; }
[ -x "$DEST/nexa-beep" ] || { echo "❌ 설치본 실행 파일이 없다: $DEST/nexa-beep"; exit 1; }
echo "설치 자리 = $DEST ($OS · $PROFILE)"

# ── 권한 승격 — 대상이 쓰기 가능하면 안 쓴다. 터미널 없는 자리(에디터 통합 셸)에서는 sudo가
#    "A terminal is required"로 죽으므로 데스크톱 세션이면 pkexec(GUI 암호창)로 넘긴다.
#    ⚠ pkexec는 호출자의 cwd를 물려주지 않는다 → 넘기는 경로는 전부 절대 경로.
ELEVATE=""
pick_elevator() {
  [ -n "$ELEVATE" ] && return 0
  if sudo -n true 2>/dev/null; then ELEVATE=sudo
  elif [ -t 0 ] && command -v sudo >/dev/null 2>&1; then ELEVATE=sudo; echo "   설치 자리가 root 소유 — sudo 1회(비밀번호 입력)"
  elif command -v pkexec >/dev/null 2>&1 && [ -n "${DISPLAY:-}${WAYLAND_DISPLAY:-}" ]; then ELEVATE=pkexec; echo "   권한: pkexec — 화면의 인증 창에 암호를 입력"
  elif command -v sudo >/dev/null 2>&1; then ELEVATE=sudo
  else echo "❌ 권한 승격 수단이 없다(sudo·pkexec 부재) — NEXA_INSTALL_DIR로 쓰기 가능한 자리를 지정"; exit 1
  fi
}
as_owner() { # as_owner <대상 디렉터리> <명령…>
  local dir=$1; shift
  if [ -w "$dir" ]; then "$@"; else pick_elevator; "$ELEVATE" "$@"; fi
}

# ── ① 종료 ───────────────────────────────────────────────────────────────
say "① 실행 중인 nexa-beep 종료"
APP_RE='nexa-beep( |$)'
pkill -f "$APP_RE" 2>/dev/null
for _ in 1 2 3 4 5 6 7 8 9 10; do pgrep -f "$APP_RE" >/dev/null 2>&1 || break; sleep 0.3; done
if pgrep -f "$APP_RE" >/dev/null 2>&1; then pkill -9 -f "$APP_RE"; sleep 0.5; fi
pkill -x nbeep-imgdec 2>/dev/null || true
echo "   종료 완료"

# ── ② 빌드 ───────────────────────────────────────────────────────────────
if [ "$BUILD" = 1 ]; then
  say "② $PROFILE 빌드 (nexa-beep + nbeep-imgdec)"
  LOG=$(mktemp)
  if ! cargo build "${CARGO_FLAGS[@]}" -p nexa-beep -p nbeep-imgdec > "$LOG" 2>&1; then
    echo "   ❌ 빌드 실패 — 마지막 20줄:"; tail -20 "$LOG" | sed 's/^/   /'; rm -f "$LOG"; exit 1
  fi
  tail -1 "$LOG"; rm -f "$LOG"
else
  say "② 빌드 건너뜀 (--no-build)"
fi
SRC="$ROOT/target/$PROFILE"   # 절대 경로 — pkexec 대비
[ -x "$SRC/nexa-beep" ] && [ -x "$SRC/nbeep-imgdec" ] || { echo "   ❌ 산출물이 없다: $SRC"; exit 1; }

# ── ③ 설치 자리 덮어쓰기 ─────────────────────────────────────────────────
say "③ 설치 자리 덮어쓰기 → $DEST"
sum() { if command -v md5sum >/dev/null 2>&1; then md5sum "$1" | cut -c1-12; else md5 -q "$1" | cut -c1-12; fi; }
BEFORE=$(sum "$DEST/nexa-beep")
# install(1) = 새 inode로 교체(실행 중 텍스트 잠금·"text file busy" 회피) + 권한 755.
as_owner "$DEST" install -m 755 "$SRC/nexa-beep" "$SRC/nbeep-imgdec" "$DEST/" || { echo "   ❌ 복사 실패"; exit 1; }
# ★ md5 대조는 **재서명 전에**(08-30 mac 실측 — ad-hoc codesign이 바이너리에 서명을 박아
#   넣어 산출물과 md5가 달라진다 · 서명 뒤 대조는 mac에서 항상 실패).
AFTER=$(sum "$DEST/nexa-beep")
echo "   nexa-beep: $BEFORE → $AFTER $([ "$BEFORE" = "$AFTER" ] && echo '(동일 — 산출물이 바뀌지 않았다)' || echo '✓ 교체')"
[ "$AFTER" = "$(sum "$SRC/nexa-beep")" ] || { echo "   ❌ 설치 자리와 산출물이 다르다"; exit 1; }
if [ "$OS" = mac ]; then
  # 번들 안 실행 파일을 바꾸면 코드 서명 봉인이 깨진다(ad-hoc 재서명 · 격리 속성 제거 — 배포본과 같은 adhoc).
  codesign --force --deep --sign - "/Applications/Nexa Beep.app" 2>/dev/null || echo "   ⚠ codesign 실패(무시 가능 · 실행이 막히면 xattr -dr com.apple.quarantine)"
  xattr -dr com.apple.quarantine "/Applications/Nexa Beep.app" 2>/dev/null || true
  echo "   codesign: $(codesign -dv "/Applications/Nexa Beep.app" 2>&1 | grep -o 'Signature=.*' || echo '?')"
fi
if [ "$OS" = linux ] && [ "$ASSETS" = 1 ]; then
  SHARE="$(dirname "$DEST")/share"   # /usr/bin → /usr/share (.deb 배치와 동일 — release.yml)
  if [ -d "$SHARE/applications" ]; then
    as_owner "$SHARE/applications" install -m 644 "$ROOT/packaging/linux/nexa-beep.desktop" "$SHARE/applications/nexa-beep.desktop"
    ICON="$SHARE/icons/hicolor/256x256/apps"
    as_owner "$SHARE/applications" install -D -m 644 "$ROOT/packaging/branding/nexa-beep-256.png" "$ICON/nexa-beep.png"
    as_owner "$SHARE/applications" update-desktop-database "$SHARE/applications" 2>/dev/null || true
    echo "   자산: .desktop · 아이콘 → $SHARE"
  else
    echo "   ⚠ 자산 자리를 못 찾음($SHARE) — 실행 파일만 갈았다"
  fi
fi
echo "   버전: $("$DEST/nexa-beep" --version 2>/dev/null)"

# ── ④ 설치본처럼 실행(런처 경로 = 무인자) ────────────────────────────────
if [ "$RUN" = 1 ]; then
  say "④ 설치본 실행"
  # 설치본은 콘솔이 없다 — 여기서 띄운 인스턴스의 stdout/stderr는 이 파일로 모아 진단을 살린다.
  RUNLOG="$ROOT/target/installed-nexa-beep.log"; : > "$RUNLOG"
  case "$OS" in
    linux)
      DESKTOP="$(dirname "$DEST")/share/applications/nexa-beep.desktop"
      # ★ 설치된 .desktop을 **경로로** 지정해 띄운다(앱 그리드와 같은 Exec 경로). `gtk-launch nexa-beep`은 ID로
      #   찾아 앱이 제 손으로 쓴 사용자 런처(~/.local/share/applications)가 /usr/share 것을 가리면 개발 빌드가
      #   대신 뜰 수 있다(nexa-clip 09-05 실측 · beep도 자동 실행 슬롯 .desktop을 쓴다).
      if command -v gio >/dev/null 2>&1 && [ -f "$DESKTOP" ]; then
        gio launch "$DESKTOP" >"$RUNLOG" 2>&1 || true
      else
        echo "   ⚠ gio/.desktop 없음 — 바이너리를 직접 띄운다"
        setsid "$DEST/nexa-beep" >"$RUNLOG" 2>&1 < /dev/null &
      fi ;;
    mac) open -a "Nexa Beep" ;;   # Finder 실행 = 인자 없음 경로(LaunchServices라 stdout은 오지 않는다)
  esac
  for _ in $(seq 1 20); do pgrep -f "$APP_RE" >/dev/null 2>&1 && break; sleep 0.3; done
  sleep 1
  # ── ⑤ 확인 ──
  say "⑤ 확인"
  N=$(pgrep -f "$APP_RE" | wc -l | tr -d ' ')
  echo "   실행 중 프로세스 = ${N:-0}개"
  [ "${N:-0}" != "0" ] || { echo "   ❌ 창이 뜨지 않았다 — 수동: ${DESKTOP:+gio launch $DESKTOP}${DESKTOP:-open -a 'Nexa Beep'}"; exit 1; }
  if [ "$OS" = linux ]; then
    EXE=$(readlink -f "/proc/$(pgrep -f "$APP_RE" | head -1)/exe" 2>/dev/null || true)
    echo "   실행 파일 = ${EXE:-?} $([ "$EXE" = "$DEST/nexa-beep" ] && echo '✓ 설치본' || echo '⚠ 설치본이 아니다(사용자 런처가 가렸나?)')"
  fi
  "$DEST/nexa-beep" --whoami 2>/dev/null | sed 's/^/   /' | head -6
  [ "$OS" = linux ] && echo "   로그: tail -f $RUNLOG"
else
  say "④ 실행 생략 (--no-run)"
fi
echo; echo "완료 — 설치본($DEST)이 현재 작업 트리 산출물로 바뀌었다. 정식 설치(.deb/brew)가 다시 덮어쓴다."
