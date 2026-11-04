#!/usr/bin/env bash
# scan-plaintext.sh — 데이터 폴더·로그 평문 스캐너(INS-7 · 11-04 · 49 J-3·J-4·J-5 · 17 §4-a 실증 3면 · W-11·W-12):
#   ① 헤더 매직 — 비밀 파일은 전부 봉인 봉투여야 한다: `*.seg`(history·pending·keys = NBSE · trust.seg = NBTS · groups.seg = NBGS) ·
#      `profile.sec` NBSE · `quarantine/**/*.beepq`·`.beepq.meta` NBSE · `identity.key` NBK1 · `user.key` NBSE. 평문 허용 = `settings.cfg`
#      (단 PII 키 `profile.email`·`profile.phone`·`user.passphrase`는 cfg에 없어야 — 사이드카 `profile.sec`가 맡는다) · `history/*.id`
#      (공개 PeerId 32B) · `profiles/*.meta`(공개 아바타 키·보더) · `logs/*.log`(상태 문구만).
#   ② 금칙 문자열 — 인자로 준 평문(메시지 본문·이메일·전화·파일명)이 **어느 파일에도**(로그 포함) 나오면 실패(grep -F · 바이너리 포함).
#   ③ 권한 — mac/Linux에서 비밀 파일은 0600(Windows = no-op · `tighten_data_dir` 스윕과 같은 규칙).
#
# 사용:  tools/scan-plaintext.sh <data 폴더> [금칙 문자열…]     # 위반 = 종료 1
# 예:    tools/scan-plaintext.sh ~/.nexa-beep-multi/rp-x/data "원본 글 첫 줄" "답장이야"
set -uo pipefail
[ $# -ge 1 ] && [ -d "$1" ] || { echo "사용: $0 <data 폴더> [금칙 문자열…]" >&2; exit 2; }
D=$1; shift
bad=0; files=0
magic() { head -c 4 "$1" 2>/dev/null | od -An -c | tr -d ' \n'; }
expect() { # 파일 · 기대 매직
  local m; m=$(magic "$1"); files=$((files+1))
  if [ "$m" = "$2" ]; then printf '  ✓ %-48s %s\n' "${1#$D/}" "$2"; else printf '  ✗ %-48s 헤더 "%s" ≠ %s(평문 또는 손상)\n' "${1#$D/}" "$m" "$2"; bad=1; fi
}
echo "▶ ① 헤더 매직: $D"
[ -f "$D/identity.key" ] && expect "$D/identity.key" NBK1
[ -f "$D/trust.seg" ] && expect "$D/trust.seg" NBTS
[ -f "$D/groups.seg" ] && expect "$D/groups.seg" NBGS
for f in "$D"/keys.seg "$D"/user.key "$D"/profile.sec "$D"/history/*.seg "$D"/pending/*.seg "$D"/quarantine/*/*.beepq "$D"/quarantine/*/*.beepq.meta; do
  [ -f "$f" ] && expect "$f" NBSE
done
# 그 밖의 파일은 허용 목록 밖이면 알린다(새 파일 = 봉인 여부를 사람이 본다).
while IFS= read -r -d '' f; do
  rel=${f#$D/}
  case "$rel" in
    identity.key|trust.seg|groups.seg|keys.seg|user.key|profile.sec|settings.cfg|history/*.seg|history/*.id|pending/*.seg|quarantine/*/*.beepq|quarantine/*/*.beepq.meta|profiles/*.meta|profiles/*.img|logs/*.log|*.tmp.*) ;;
    *) printf '  ⚠ %-48s 허용 목록 밖(봉인 여부 확인 필요)\n' "$rel" ;;
  esac
done < <(find "$D" -type f -print0 2>/dev/null)
# settings.cfg에 PII 키가 남아 있으면 평문 노출(08-17 사이드카 규칙 위반).
if [ -f "$D/settings.cfg" ]; then
  if grep -E '^(profile\.email|profile\.phone|user\.passphrase)=.+' "$D/settings.cfg" >/dev/null; then echo "  ✗ settings.cfg에 PII 키 평문(profile.email/phone · user.passphrase)"; bad=1; else echo "  ✓ settings.cfg PII 키 없음"; fi
fi

if [ $# -gt 0 ]; then
  echo "▶ ② 금칙 문자열 $#개(로그 포함 · 바이너리도 본다)"
  for n in "$@"; do
    # 짧은 문자열은 암호문 바이트·설정 키에 우연히 맞는다("hi" = ui.scrollbar_hide · 11-04 실측) — 6바이트 미만은 경고만.
    if [ "${#n}" -lt 6 ] && [ "$(printf '%s' "$n" | wc -c)" -lt 6 ]; then printf '  ⚠ "%s" 너무 짧아(6바이트 미만) 우연 일치 — 건너뜀\n' "$n"; continue; fi
    hits=$(grep -rlF --binary-files=text -- "$n" "$D" 2>/dev/null | sed "s|^$D/||")
    if [ -n "$hits" ]; then printf '  ✗ "%s" 발견: %s\n' "$n" "$(echo "$hits" | tr '\n' ' ')"; bad=1; else printf '  ✓ "%s" 없음\n' "$n"; fi
  done
fi

echo "▶ ③ 권한(0600)"
case "$(uname -s)" in
  MINGW*|MSYS*|CYGWIN*) echo "  (Windows — ACL은 사용자 전용 폴더 상속 · 생략)" ;;
  *)
    while IFS= read -r -d '' f; do
      case "$(uname -s)" in Darwin) p=$(stat -f %Lp "$f") ;; *) p=$(stat -c %a "$f") ;; esac
      if [ "$p" = "600" ]; then printf '  ✓ %-48s %s\n' "${f#$D/}" "$p"; else printf '  ✗ %-48s %s(0600 아님)\n' "${f#$D/}" "$p"; bad=1; fi
    done < <(find "$D" -type f \( -name '*.seg' -o -name '*.sec' -o -name '*.key' -o -name '*.beepq' -o -name '*.beepq.meta' \) -print0 2>/dev/null)
    ;;
esac
[ "$bad" = 0 ] && echo "PASS scan-plaintext ($files 봉인 파일 · $D)" || { echo "FAIL scan-plaintext — $D"; exit 1; }
