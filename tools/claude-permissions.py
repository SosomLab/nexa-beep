# -*- coding: utf-8 -*-
"""claude-permissions.py — Claude Code 세션이 tools/*.sh(verify.sh · 시나리오 · 스모크 · tmp-clean)를 **묻지 않고** 돌리도록
`.claude/settings.json`의 permissions.allow에 스크립트 실행 규칙을 병합한다(사용자 확정 2026-11-04 "스크립트 실행은 자동화").

⚠ 세션(개발·협업)은 자기 권한 파일을 고칠 수 없다(하네스 자기 수정 차단) → **사용자가 한 번** 실행한다:
    python tools/claude-permissions.py            # 병합(기존 항목 보존 · 중복 없음 · 들여쓰기 2 · CRLF 보존)
    python tools/claude-permissions.py --dry-run  # 더할 항목만 출력
새 스크립트 종류를 더하면 아래 RULES에 접두를 추가하고 다시 실행한다(docs/18 §3).
"""
import io
import json
import os
import sys
from collections import OrderedDict

RULES = [
    "Bash(bash:*)",
    "Bash(sh:*)",
    "Bash(./tools/:*)",
    "Bash(tools/:*)",
    "Bash(env:*)",
    "Bash(NEXA_TEST_HEADLESS=1 bash:*)",
    "Bash(NEXA_TEST_HEADLESS=1 ./tools/:*)",
    "Bash(cygpath:*)",
    "Bash(unzip:*)",
    "Bash(tar:*)",
    "Bash(du:*)",
    "Bash(sleep:*)",
    "Bash(timeout:*)",
    "Bash(sort:*)",
    "Bash(uniq:*)",
    "Bash(cut:*)",
    "Bash(sed:*)",
    "Bash(tr:*)",
    "Bash(diff:*)",
    "Bash(stat:*)",
    "Bash(od:*)",
    "Bash(xargs:*)",
    "Bash(dirname:*)",
    "Bash(basename:*)",
    "Bash(uname:*)",
    "Bash(tasklist:*)",
    "Bash(pgrep:*)",
    "PowerShell(bash:*)",
    "PowerShell(Get-Date:*)",
    "PowerShell(Get-TimeZone:*)",
]


def main() -> int:
    root = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
    path = os.path.join(root, ".claude", "settings.json")
    dry = "--dry-run" in sys.argv
    raw = io.open(path, encoding="utf-8", newline="").read()
    crlf = "\r\n" in raw
    data = json.loads(raw, object_pairs_hook=OrderedDict)
    perms = data.setdefault("permissions", OrderedDict())
    allow = perms.setdefault("allow", [])
    missing = [r for r in RULES if r not in allow]
    if not missing:
        print("추가할 규칙 없음 — 이미 등재됨")
        return 0
    for r in missing:
        print(("  (dry) " if dry else "  + ") + r)
    if dry:
        return 0
    allow.extend(missing)
    text = json.dumps(data, ensure_ascii=False, indent=2) + "\n"
    io.open(path, "w", encoding="utf-8", newline="").write(text.replace("\n", "\r\n") if crlf else text)
    print(f"{len(missing)}개 규칙 병합 → {path} (세션 재시작 또는 /hooks 한 번으로 반영)")
    return 0


if __name__ == "__main__":
    sys.exit(main())
