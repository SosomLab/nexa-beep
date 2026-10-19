#!/usr/bin/env bash
# 테스트 인스턴스의 로그인 자동 실행 등록 정리(10-19)
#
# 왜: 자동 실행 슬롯은 실행 파일 경로마다 따로다(08-29 인스턴스 슬롯). 3신원·S4·perf
#     시나리오가 폴더마다 복사한 nexa-beep이 기본값 on으로 등록하면, 테스트가 끝나도
#     Run 값·plist·.desktop이 남고 폴더를 지운 뒤엔 없는 파일을 가리키는 유령이 된다.
#     앱은 이제 테스트 인스턴스(NEXA_NO_AUTOSTART · cargo 산출 폴더)에서 부팅 등록을
#     하지 않지만, 테스트 중 사용자가 설정에서 직접 켠 등록은 남는다 — 그걸 여기서 지운다.
#
# 대상 = **테스트 뿌리 아래를 가리키는 등록만**: $BEEP_MULTI(기본 ~/.nexa-beep-multi) ·
#        저장소 target/. 설치본(Programs·/usr/bin·.app)은 건드리지 않는다.
#   ① 실행 파일이 남아 있으면 그 exe의 --autostart-clear(앱과 같은 슬롯 규칙)
#   ② 실행 파일이 사라진 유령은 등록 내용의 경로 접두로 찾아 직접 지운다
#
# 사용: tools/autostart-clean.sh [-q]     (-q = 지운 것이 없으면 조용히)
# 호출: relaunch.sh(① 종료 직후 · --stop) · s4-scenario.sh·perf-scenario.sh(EXIT trap)
set -uo pipefail
cd "$(dirname "$0")/.."
ROOT=$(pwd)
M=${BEEP_MULTI:-$HOME/.nexa-beep-multi}
QUIET=0; [ "${1:-}" = "-q" ] && QUIET=1
IS_WIN=0; EXE=""
case "${OSTYPE:-}" in msys* | cygwin*) IS_WIN=1; EXE=".exe" ;; esac
N=0
note() { N=$((N + 1)); echo "   자동 실행 정리: $1"; }

# ① 남아 있는 실행 파일 — 앱 자신에게 지우게 한다(슬롯 이름 규칙 중복 구현 금지).
for exe in "$M"/*/nexa-beep"$EXE" "$ROOT"/target/debug/nexa-beep"$EXE" "$ROOT"/target/release/nexa-beep"$EXE"; do
  [ -x "$exe" ] || continue
  out=$("$exe" --autostart-clear 2>&1 | tr -d '\r')
  # 구버전 복사본(인자 미지원 = 도움말 안내)은 조용히 넘긴다 — ②의 경로 접두 정리가 맡는다.
  case "$out" in *removed*) note "$exe" ;; *"clear failed"*) echo "   ⚠ $exe: $out" ;; esac
done

# ② 유령(실행 파일이 없어진 등록) — 경로 접두 대조로 직접 지운다.
if [ "$IS_WIN" = 1 ]; then
  WM=$(cygpath -w "$M"); WT=$(cygpath -w "$ROOT/target")
  gone=$(powershell.exe -NoProfile -Command "
    \$k='HKCU:\Software\Microsoft\Windows\CurrentVersion\Run'
    \$roots=@('$WM\','$WT\')
    (Get-ItemProperty \$k -ErrorAction SilentlyContinue).psobject.Properties |
      Where-Object { \$_.Name -like 'NexaBeep*' } | ForEach-Object {
        \$p = (\$_.Value -as [string]).Trim().Trim('\"')
        foreach (\$r in \$roots) { if (\$p.StartsWith(\$r, [StringComparison]::OrdinalIgnoreCase)) {
          Remove-ItemProperty -Path \$k -Name \$_.Name; \"\$(\$_.Name) -> \$p\"; break } }
      }" 2>/dev/null | tr -d '\r')
  [ -n "$gone" ] && while IFS= read -r l; do note "$l"; done <<< "$gone"
else
  if [ "$(uname)" = Darwin ]; then
    files=("$HOME"/Library/LaunchAgents/com.sosomlab.nexa-beep*.plist)
    path_of() { sed -n '/<key>ProgramArguments<\/key>/,/<\/array>/{s/.*<string>\(.*\)<\/string>.*/\1/p;}' "$1" | head -1 | sed 's/&lt;/</g;s/&gt;/>/g;s/&amp;/\&/g'; }
  else
    files=("${XDG_CONFIG_HOME:-$HOME/.config}"/autostart/nexa-beep*.desktop)
    path_of() { sed -n 's/^Exec="\(.*\)"$/\1/p' "$1" | head -1 | sed 's/\\\(.\)/\1/g'; }
  fi
  for f in "${files[@]}"; do
    [ -f "$f" ] || continue
    p=$(path_of "$f")
    case "$p" in "$M"/* | "$ROOT"/target/*) rm -f "$f" && note "$f -> $p" ;; esac
  done
fi

[ "$N" = 0 ] && [ "$QUIET" = 0 ] && echo "   자동 실행 정리: 테스트 등록 없음"
exit 0
