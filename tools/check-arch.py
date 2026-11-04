#!/usr/bin/env python3
"""check-arch.py — 아키텍처 단언(INS-8 · 11-04 · 49 M-1·M-2 · DR-21 · 13 §2-4 규칙 7): 의존 방향 + 포트 시그니처 외부 타입 0.

    python tools/check-arch.py            # 위반 = 종료 1
    python tools/check-arch.py --list     # 워크스페이스 크레이트별 normal 의존 전부

① 의존 방향(cargo metadata --locked · normal 의존 · 전이 포함):
   - nbeep-core = 의존 **0**(순수 도메인 — 외부 크레이트 타입이 포트에 들어올 길 자체가 없다).
   - nbeep-{net,store,crypto,plat,safe,relay} = 표현·앱 금지(nbeep-ui · nexa-beep · winit · softbuffer · nexa-ctl · nexa-dlg) —
     래스터 원시 nexa-gfx·폰트 nexa-font는 재료라 플랫폼 층이 써도 된다.
   - nbeep-ui(표현) = 전송·저장·암호·앱 금지(nbeep-net · nbeep-store · nbeep-crypto · nbeep-relay · nexa-beep · winit).
   - nexa-ctl(공용 컨트롤 · 형제) = 앱 도메인 금지(nbeep-* · nexa-beep).
② 포트 파일(nbeep-core의 trait 정의: ports.rs · link.rs · session.rs · pipeline.rs · trust.rs)에 외부 크레이트 경로 0 —
   `use`는 std/core/alloc/crate/super/self만 · 본문에 `<외부크레이트>::` 0(외부 = 워크스페이스 비멤버 패키지 이름 전부).
"""
import json
import os
import re
import subprocess
import sys

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
FORBID = {
    "nbeep-core": None,  # None = 의존 0
    # 표현 = 위젯(nbeep-ui)·컨트롤(nexa-ctl)·대화상자(nexa-dlg)·창(winit·softbuffer)·앱. 래스터 원시(nexa-gfx)·폰트(nexa-font)는
    # 표현이 아니라 재료라 플랫폼 층(nbeep-plat → nexa-font → nexa-gfx)이 써도 된다(11-04 첫 실행에서 가른 경계).
    "nbeep-net": {"nbeep-ui", "nexa-beep", "winit", "softbuffer", "nexa-ctl", "nexa-dlg"},
    "nbeep-store": {"nbeep-ui", "nexa-beep", "winit", "softbuffer", "nexa-ctl", "nexa-dlg"},
    "nbeep-crypto": {"nbeep-ui", "nexa-beep", "winit", "softbuffer", "nexa-ctl", "nexa-dlg"},
    "nbeep-plat": {"nbeep-ui", "nexa-beep", "winit", "softbuffer", "nexa-ctl", "nexa-dlg"},
    "nbeep-safe": {"nbeep-ui", "nexa-beep", "winit", "softbuffer", "nexa-ctl", "nexa-dlg"},
    "nbeep-relay": {"nbeep-ui", "nexa-beep", "winit", "softbuffer", "nexa-ctl", "nexa-dlg"},
    "nbeep-ui": {"nbeep-net", "nbeep-store", "nbeep-crypto", "nbeep-relay", "nexa-beep", "winit", "softbuffer"},
    "nexa-ctl": {"nexa-beep", "nbeep-core", "nbeep-ui", "nbeep-net", "nbeep-store", "nbeep-crypto", "nbeep-plat"},
}
PORT_FILES = ["ports.rs", "link.rs", "session.rs", "pipeline.rs", "trust.rs"]
STD_ROOTS = ("std", "core", "alloc", "crate", "super", "self")


def main() -> int:
    for stream in (sys.stdout, sys.stderr):
        if hasattr(stream, "reconfigure"):
            stream.reconfigure(encoding="utf-8", errors="replace")
    show = "--list" in sys.argv
    out = subprocess.run(
        ["cargo", "metadata", "--format-version", "1", "--locked"],
        capture_output=True, text=True, encoding="utf-8", check=False, cwd=ROOT,
    )
    if out.returncode != 0:
        sys.stderr.write(out.stderr)
        return 2
    meta = json.loads(out.stdout)
    by_id = {p["id"]: p for p in meta["packages"]}
    members = set(meta["workspace_members"])
    nodes = {n["id"]: n for n in meta["resolve"]["nodes"]}

    def normal_deps(pid: str) -> set:
        """전이 normal 의존(패키지 이름 집합)."""
        seen, stack = set(), [pid]
        while stack:
            cur = stack.pop()
            for d in nodes.get(cur, {}).get("deps", []):
                kinds = {k.get("kind") for k in d.get("dep_kinds", [])} or {None}
                if None not in kinds and "normal" not in kinds:
                    continue  # dev/build 전용
                name = by_id[d["pkg"]]["name"]
                if name not in seen:
                    seen.add(name)
                    stack.append(d["pkg"])
        return seen

    bad = 0
    # ① 의존 방향 — 워크스페이스 멤버 + 형제 nexa-ctl(path 의존 · 비멤버지만 메타데이터에 있다).
    targets = {by_id[m]["name"]: m for m in members}
    for p in meta["packages"]:
        if p["name"] == "nexa-ctl":
            targets["nexa-ctl"] = p["id"]
    for name, pid in sorted(targets.items()):
        deps = normal_deps(pid)
        if show:
            print(f"{name}: {', '.join(sorted(deps)) or '(없음)'}")
        rule = FORBID.get(name, "skip")
        if rule == "skip":
            continue
        if rule is None:
            if deps:
                print(f"  ✗ {name}: 의존 0이어야 하는데 {sorted(deps)}")
                bad = 1
            else:
                print(f"  ✓ {name}: 의존 0")
        else:
            hit = sorted(deps & rule)
            if hit:
                print(f"  ✗ {name}: 금지 의존 {hit}")
                bad = 1
            else:
                print(f"  ✓ {name}: 금지 의존 없음({len(deps)}개 검사)")
    # ② 포트 파일 외부 타입 0.
    external = {p["name"].replace("-", "_") for p in meta["packages"] if p["id"] not in members}
    port_dir = os.path.join(ROOT, "crates", "nbeep-core", "src")
    for f in PORT_FILES:
        path = os.path.join(port_dir, f)
        if not os.path.exists(path):
            continue
        text = open(path, encoding="utf-8").read()
        hits = []
        for m in re.finditer(r"^\s*(?:pub(?:\([^)]*\))?\s+)?use\s+([A-Za-z_][A-Za-z0-9_]*)", text, re.M):
            if m.group(1) not in STD_ROOTS:
                hits.append(f"use {m.group(1)}")
        for m in re.finditer(r"\b([a-z][a-z0-9_]*)::", text):
            root = m.group(1)
            if root in external:
                hits.append(f"{root}::")
        if hits:
            print(f"  ✗ 포트 {f}: 외부 크레이트 참조 {sorted(set(hits))}")
            bad = 1
        else:
            print(f"  ✓ 포트 {f}: 외부 크레이트 참조 0")
    if bad:
        print("아키텍처 단언 실패(49 M-1·M-2 · DR-21)")
        return 1
    print("아키텍처 단언 통과(의존 방향 · 포트 외부 타입 0)")
    return 0


if __name__ == "__main__":
    sys.exit(main())
