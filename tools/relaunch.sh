#!/usr/bin/env bash
# 기준 재기동 — 종료 → 재빌드 → 3신원 기동 → 발견 확인 (한 번에)
#
# 왜 스크립트인가: 손으로 하면 매번 한 단계씩 빠졌다(빌드를 잊거나, 옛 프로세스가
# 남아 포트를 물고 있거나, imgdec 복사를 빠뜨려 아바타만 죽거나).
#
# ★ 기본은 **신원 보존**이다. 폴더를 지우면 핀·그룹·설정이 통째로 사라져
#   그룹 테스트를 처음부터 다시 해야 한다. 새 신원이 필요하면 --fresh.
#
# 사용:
#   tools/relaunch.sh            # 종료 → 빌드 → 3신원(기존 신원 유지)
#   tools/relaunch.sh --fresh    # A·B 신원을 새로 만든다(핀·그룹 초기화)
#   tools/relaunch.sh --gate     # 빌드 대신 전체 게이트(fmt·clippy·test·rustdoc)
#   tools/relaunch.sh --no-build # 빌드 건너뛰고 재기동만
#   tools/relaunch.sh --daemon   # 로컬 릴레이 서버(nexa-beepd)도 함께 재기동
#                                #   (포트 47399 — 공식 서버 47300과 안 겹침 ·
#                                #    각 신원은 설정 › Server에서 127.0.0.1:47399
#                                #    + Test 1회로 붙인다 — 검증 게이트 08-22)
#
# 참고: docs/33 §2(신원 3개) · docs/26 §3-4(원리 — data/는 실행 파일을 따라간다)
set -uo pipefail

cd "$(dirname "$0")/.."
ROOT=$(pwd)
# ★ 기본 위치는 **durable**이어야 한다(08-19 진단) — 종전 /tmp/beep-multi는
#   macOS `com.apple.tmp_cleaner`(매일 자정, /private/tmp의 3일 미접근 항목 삭제)에
#   identity.key가 조용히 지워져 재기동마다 새 신원이 생겼다. 새 신원은 옛
#   격리물·핀을 못 연다(sealed는 기기 신원 키에 묶임 · fail-closed). 홈 아래 숨김
#   폴더로 옮긴다. 기존 /tmp 신원은 아래에서 1회 이관해 잃지 않는다.
MULTI=${BEEP_MULTI:-$HOME/.nexa-beep-multi}
LEGACY_MULTI=/tmp/beep-multi
FRESH=0 BUILD=1 GATE=0 DAEMON=0
DPORT=${BEEPD_PORT:-47399}
for a in "$@"; do
  case "$a" in
    --fresh) FRESH=1 ;;
    --no-build) BUILD=0 ;;
    --gate) GATE=1 ;;
    --daemon) DAEMON=1 ;;
    -h | --help) sed -n '2,20p' "$0"; exit 0 ;;
    *) echo "알 수 없는 인자: $a (--help)"; exit 2 ;;
  esac
done

say() { printf '\n\033[1m▶ %s\033[0m\n' "$1"; }

# ── OS 분기(T-relaunch · 08-16) — Git Bash(Windows)엔 pgrep/pkill이 없어
#    ① 종료가 0개 감지(구 인스턴스가 exe를 잠가 링커 os error 5) ④ 창 카운트가
#    0 고정이었다. PowerShell로 대체한다. ⚠ Windows 실기 검증은 Windows 세션 몫.
IS_WIN=0; EXE=""
case "${OSTYPE:-}" in msys* | cygwin*) IS_WIN=1; EXE=".exe" ;; esac
pw() { powershell.exe -NoProfile -Command "$1" 2>/dev/null | tr -d '\r'; }
# ⚠ 패턴은 **경계 일치**여야 한다(08-22 발각 — 'nexa-beep' 부분 일치는
#   `nexa-beepd`(로컬 릴레이 데몬)까지 죽이고 잔존 카운트에도 세어, 데몬을 띄워
#   둔 실기에서 ①이 서버를 조용히 끊거나 '남은 1개'로 오탈락시켰다).
#   Get-Process 이름 일치는 원래 정확(nexa-beep ≠ nexa-beepd)이라 Windows는 무변.
APP_RE='nexa-beep(\.exe)?( |$)'
kill_all() {
  if [ "$IS_WIN" = 1 ]; then
    pw "Get-Process nexa-beep -ErrorAction SilentlyContinue | Stop-Process -Force" >/dev/null
  else
    pkill ${1:-} -f "$APP_RE" 2>/dev/null
  fi
}
count_procs() {
  if [ "$IS_WIN" = 1 ]; then
    pw "(Get-Process nexa-beep -ErrorAction SilentlyContinue | Measure-Object).Count" | tr -d ' '
  else
    pgrep -f "$APP_RE" 2>/dev/null | wc -l | tr -d ' '
  fi
}
kill_daemon() {
  if [ "$IS_WIN" = 1 ]; then
    pw "Get-Process nexa-beepd -ErrorAction SilentlyContinue | Stop-Process -Force" >/dev/null
  else
    pkill -f 'nexa-beepd(\.exe)?( |$)' 2>/dev/null
  fi
}

# ── ① 종료 ────────────────────────────────────────────────────────────────
say "① 실행 중인 nexa-beep 종료$([ "$DAEMON" = 1 ] && echo ' (+로컬 beepd)')"
BEFORE=$(count_procs)
kill_all
[ "$DAEMON" = 1 ] && kill_daemon
sleep 2
# 2차 — 안 죽은 것(끊긴 세션의 프로브 잔재 등)은 강제 종료한다. 하나라도 남으면
# 포트·발견이 겹쳐 다음 단계 판정이 흐려진다(08-14: 고아 프로브 1개가 가드를 막았다).
if [ "$(count_procs)" != "0" ]; then
  kill_all -9
  sleep 1
fi
LEFT=$(count_procs)
echo "   종료 전 ${BEFORE:-0}개 → 남은 ${LEFT:-0}개"
[ "${LEFT:-0}" != "0" ] && { echo "   ⚠️ 강제 종료 후에도 남았다 — 확인 후 다시"; exit 1; }

# ── ② 빌드 ────────────────────────────────────────────────────────────────
if [ "$GATE" = "1" ]; then
  say "② 전체 게이트 (fmt · clippy · test · rustdoc)"
  cargo fmt --all --check || { echo "   ❌ fmt"; exit 1; }
  cargo clippy --workspace --all-targets -q 2>&1 | grep -E "^(warning|error)" && { echo "   ❌ clippy"; exit 1; }
  cargo test --workspace -q 2>&1 | grep -E "^test result" |
    awk -F'[ ;]' '{p+=$4;f+=$6} END{printf "   테스트 %d 통과 / %d 실패\n",p,f; if(f>0) exit 1}' || exit 1
  cargo doc --workspace --no-deps -q 2>&1 | grep -E "^(error|warning)" && { echo "   ❌ rustdoc"; exit 1; }
  echo "   게이트 통과 ✅"
elif [ "$BUILD" = "1" ]; then
  say "② 릴리스 빌드 (nexa-beep + nbeep-imgdec$([ "$DAEMON" = 1 ] && echo ' + nexa-beepd'))"
  # ★ rc를 직접 검사한다(T-relaunch · 08-16) — 종전 `… | tail -1`은 실패를
  #   삼켰고(set -e 부재) ⑤의 산출물 검사는 **구 바이너리**로 통과해, 빌드가
  #   깨졌는데 "성공"을 두 번 오보했다(08-13 os error 5 · 08-16 E0432).
  BUILD_LOG=$(mktemp)
  PKGS="-p nexa-beep -p nbeep-imgdec"
  [ "$DAEMON" = 1 ] && PKGS="$PKGS -p nexa-beepd"
  # shellcheck disable=SC2086
  if ! cargo build --release $PKGS > "$BUILD_LOG" 2>&1; then
    echo "   ❌ 빌드 실패 — 마지막 20줄:"
    tail -20 "$BUILD_LOG" | sed 's/^/   /'
    rm -f "$BUILD_LOG"
    exit 1
  fi
  tail -1 "$BUILD_LOG"
  rm -f "$BUILD_LOG"
else
  say "② 빌드 건너뜀 (--no-build)"
fi
[ -x "target/release/nexa-beep$EXE" ] || { echo "   ❌ 빌드 산출물이 없다"; exit 1; }

# ── ③ 3신원 준비 ──────────────────────────────────────────────────────────
# 신원은 '실행 파일 옆 data/'에 붙는다(포터블 규칙 DR-4) — 폴더가 곧 신원이다.
say "③ 신원 3개 준비 ($([ "$FRESH" = 1 ] && echo '새 신원' || echo '기존 신원 유지'))"
[ "$FRESH" = "1" ] && rm -rf "$MULTI"
# 1회 이관 — 종전 /tmp/beep-multi 신원을 durable 위치로 옮겨 지문을 보존한다.
# 목적지가 아직 없을 때만(경합·재이관 방지). --fresh면 새로 시작하므로 건너뛴다.
if [ "$FRESH" != "1" ] && [ "$MULTI" != "$LEGACY_MULTI" ] && [ -d "$LEGACY_MULTI" ] && [ ! -d "$MULTI" ]; then
  if mv "$LEGACY_MULTI" "$MULTI" 2>/dev/null; then
    echo "   이관: $LEGACY_MULTI → $MULTI (기존 신원 보존 · /tmp 청소 회피)"
  fi
fi
for d in A B; do
  mkdir -p "$MULTI/$d"
  cp -f "target/release/nexa-beep$EXE" "target/release/nbeep-imgdec$EXE" "$MULTI/$d/" # imgdec 동거 필수
done
echo "   기본 = $ROOT/target/release  ·  A·B = $MULTI/{A,B}"

# ── ④ 기동 ────────────────────────────────────────────────────────────────
# ★ nohup + disown 으로 **셸에서 떼어낸다**. 그냥 `&`로 띄우면 이 스크립트를 부른
#   셸의 프로세스 그룹에 남아, 그 셸이 종료·중단될 때 창이 같이 죽는다(08-14 실측 —
#   크래시 로그 없이 조용히 사라져 원인 찾는 데 시간을 썼다).
# ── ③-b 로컬 릴레이 데몬(--daemon · 08-22 서버 축 실기) ─────────────────
# 공식 서버(beepd.sosomlab.com:47300)를 시험 트래픽으로 오염시키지 않으려면
# 로컬 데몬이 맞다. 클라보다 **먼저** 띄운다 — Managed 저장분(검증 게이트 통과
# 이력)이 있으면 부팅 자동 접속이 첫 틱에 붙는다. 키/핀은 $MULTI/beepd/에 영속
# (재기동해도 핀 불변 — 지우면 전 신원이 PinMismatch로 시끄럽다).
if [ "$DAEMON" = 1 ]; then
  say "③-b 로컬 beepd 기동 (포트 $DPORT)"
  mkdir -p "$MULTI/beepd"
  if [ "$IS_WIN" = 1 ]; then
    WROOT_D=$(cygpath -w "$ROOT")
    WKEY=$(cygpath -w "$MULTI/beepd/beepd.key")
    pw "Start-Process -FilePath '$WROOT_D\\target\\release\\nexa-beepd.exe' -ArgumentList '--port','$DPORT','--key','$WKEY'" >/dev/null
  else
    nohup ./target/release/nexa-beepd --port "$DPORT" --key "$MULTI/beepd/beepd.key" \
      > "$MULTI/beepd/out.log" 2>&1 &
    disown
  fi
  sleep 1
  if [ "$IS_WIN" != 1 ]; then
    PIN=$(grep -oE '[0-9a-f]{64}' "$MULTI/beepd/out.log" | head -1)
    echo "   주소 = 127.0.0.1:$DPORT · 핀 = ${PIN:-(로그에서 미검출 — $MULTI/beepd/out.log)}"
    echo "   각 신원: 설정 › Server → Managed · 127.0.0.1 · $DPORT → [Test] 1회"
  fi
fi

say "④ 기동 (셸에서 분리)"
if [ "$IS_WIN" = 1 ]; then
  # Windows — Start-Process(분리 기동 · 로그 리다이렉트는 미지원: 이벤트 관찰은
  # 앱 상태바/실기로). ★ 경로는 반드시 cygpath -w로 변환한다(08-18 실기 —
  # PowerShell은 POSIX 경로 '/d/…'를 못 열고, pw()가 stderr를 버려 조용히
  # 실패했다: ④ 창 0개의 진범).
  WROOT=$(cygpath -w "$ROOT")
  WMULTI=$(cygpath -w "$MULTI")
  pw "Start-Process -FilePath '$WROOT\\target\\release\\nexa-beep.exe' -ArgumentList '--window','--live'" >/dev/null
  pw "Start-Process -FilePath '$WMULTI\\A\\nexa-beep.exe' -ArgumentList '--window','--live'" >/dev/null
  pw "Start-Process -FilePath '$WMULTI\\B\\nexa-beep.exe' -ArgumentList '--window','--live'" >/dev/null
else
  nohup ./target/release/nexa-beep --window --live > /tmp/beep-base.log 2>&1 &
  disown
  nohup "$MULTI/A/nexa-beep" --window --live > "$MULTI/A/out.log" 2>&1 &
  disown
  nohup "$MULTI/B/nexa-beep" --window --live > "$MULTI/B/out.log" 2>&1 &
  disown
fi
sleep 6
if [ "$IS_WIN" = 1 ]; then
  RUNNING=$(pw "(Get-Process nexa-beep -ErrorAction SilentlyContinue | Where-Object { \$_.MainWindowTitle }).Count" | tr -d ' ')
else
  RUNNING=$(pgrep -f "nexa-beep --window" 2>/dev/null | wc -l | tr -d ' ')
fi
echo "   창 ${RUNNING:-0}개"

# ── ④-b 신원 표 ───────────────────────────────────────────────────────────
# 각 실행 파일이 '실제로 로드한' 신원을 읽기 전용으로 찍는다(--whoami는 키를
# 생성하지 않는다). 재기동마다 지문이 유지되는지 여기서 눈으로 확인한다 —
# 지문이 바뀌면 격리물·핀이 옛 신원에 묶여 조용히 사라진다(08-19 진단).
if [ "$IS_WIN" != 1 ]; then
  say "④-b 신원 (지문 · 이름 · 실행경로)"
  for pair in "기본:./target/release/nexa-beep" "A:$MULTI/A/nexa-beep" "B:$MULTI/B/nexa-beep"; do
    label=${pair%%:*}
    exe=${pair#*:}
    line=$("$exe" --whoami 2>/dev/null)
    fp=$(echo "$line" | sed -n 's/^fingerprint = \([0-9a-f]*\).*/\1/p')
    nm=$(echo "$line" | sed -n 's/^name        = //p')
    ex=$(echo "$line" | sed -n 's/^exe         = //p')
    printf '   %-4s %s  %-12s %s\n' "$label" "${fp:---------}" "${nm:-?}" "$ex"
  done
fi

# ── ⑤ 발견 확인 ───────────────────────────────────────────────────────────
# 셋이 서로 보이는지까지 봐야 "기동됐다"고 말할 수 있다(창만 떠도 발견이 막힐 수 있다).
say "⑤ 발견 확인 (프로브 8초)"
SEEN=$("./target/release/nexa-beep$EXE" --discover-probe 8 2>&1 |
  grep -oE "peer=[0-9a-f]+ .*name=[^ ]*" | sort -u)
echo "$SEEN" | sed 's/^/   /'
N=$(echo "$SEEN" | grep -c "peer=" || true)
if [ "$RUNNING" = "3" ] && [ "$N" -ge 3 ]; then
  printf '\n\033[32m✅ 3신원 기동·상호 발견 완료\033[0m — 정리: pkill -f "nexa-beep( |$)"%s\n' "$([ "$DAEMON" = 1 ] && echo ' · 데몬: pkill -f nexa-beepd')"
else
  printf '\n\033[33m⚠️ 창 %s개 · 발견 %s명\033[0m — 로그: /tmp/beep-base.log · %s/{A,B}/out.log\n' "$RUNNING" "$N" "$MULTI"
  exit 1
fi
