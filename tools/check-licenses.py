#!/usr/bin/env python3
"""check-licenses.py — 의존성 라이선스 게이트(DR-12 · 11-04 INS-2 · 49 A-4): 퍼미시브 온리 · GPL/LGPL/AGPL 금지.

    python tools/check-licenses.py            # 위반 = 종료 1 · 요약 표
    python tools/check-licenses.py --list     # 패키지별 라이선스 전부

판정(패키지의 SPDX 식을 OR로 나눈 뒤):
  FAIL = 모든 OR 대안이 강한 카피레프트(GPL·LGPL·AGPL·SSPL·EUPL·CDDL·OSL)인 패키지 — 퍼미시브 대안이 하나라도 있으면(예 `MIT OR GPL`) 통과.
  WARN = 약한 카피레프트(MPL-2.0 등)만 있거나 · `license` 없이 `license-file`만 · 모르는 식 — 사람이 본다(종료 코드엔 영향 없음).
입력 = `cargo metadata --locked`(Cargo.lock 그대로 · 네트워크 0) · 워크스페이스 멤버는 제외. 외부 도구 0(cargo-deny 없이).
"""
import json
import re
import subprocess
import sys

PERMISSIVE = {
    "MIT", "APACHE-2.0", "APACHE-2.0 WITH LLVM-EXCEPTION", "BSD-2-CLAUSE", "BSD-3-CLAUSE", "ISC", "ZLIB", "0BSD",
    "UNLICENSE", "CC0-1.0", "BSL-1.0", "UNICODE-DFS-2016", "UNICODE-3.0", "OPENSSL", "WTFPL", "MIT-0", "NCSA",
    "BSD-2-CLAUSE-PATENT", "PYTHON-2.0", "ZLIB-ACKNOWLEDGEMENT", "LICENSEREF-UFL-1.0", "OFL-1.1", "MIT OR APACHE-2.0",
    # 1차 라이선스(우리 것 · DR-12 — 형제 저장소 nexa-ui·nexa-license의 크레이트).
    "LICENSEREF-POLYFORM-NONCOMMERCIAL-1.0.0",
}
STRONG_COPYLEFT = ("GPL", "AGPL", "LGPL", "SSPL", "EUPL", "CDDL", "OSL")
WEAK_COPYLEFT = ("MPL",)


def classify(expr: str) -> str:
    """'ok' · 'warn' · 'fail' — OR 대안 중 하나라도 퍼미시브면 ok."""
    e = expr.strip().upper().replace("/", " OR ")
    alts = [a.strip().strip("()") for a in re.split(r"\bOR\b", e)]
    if any(a in PERMISSIVE or all(p.strip() in PERMISSIVE for p in a.split(" AND ")) for a in alts):
        return "ok"
    if any(k in e for k in STRONG_COPYLEFT) and not any(k in e for k in WEAK_COPYLEFT):
        return "fail"
    return "warn"


def main() -> int:
    # Windows 콘솔(cp949)에서 ⚠·✗가 깨지지 않게(11-04 실측 — UnicodeEncodeError).
    for stream in (sys.stdout, sys.stderr):
        if hasattr(stream, "reconfigure"):
            stream.reconfigure(encoding="utf-8", errors="replace")
    show = "--list" in sys.argv
    out = subprocess.run(
        ["cargo", "metadata", "--format-version", "1", "--locked"],
        capture_output=True, text=True, encoding="utf-8", check=False,
    )
    if out.returncode != 0:
        sys.stderr.write(out.stderr)
        return 2
    meta = json.loads(out.stdout)
    members = set(meta["workspace_members"])
    rows = []
    for p in meta["packages"]:
        if p["id"] in members:
            continue
        lic = p.get("license")
        if not lic:
            rows.append((p["name"], p["version"], f"(license-file: {p.get('license_file')})", "warn"))
            continue
        rows.append((p["name"], p["version"], lic, classify(lic)))
    rows.sort()
    fails = [r for r in rows if r[3] == "fail"]
    warns = [r for r in rows if r[3] == "warn"]
    if show:
        for n, v, l, k in rows:
            print(f"{k:4} {n} {v}  {l}")
    print(f"의존성 {len(rows)}개 · ok {len(rows) - len(fails) - len(warns)} · warn {len(warns)} · fail {len(fails)}")
    for n, v, l, _ in warns:
        print(f"  ⚠ warn {n} {v}  {l}")
    for n, v, l, _ in fails:
        print(f"  ✗ FAIL {n} {v}  {l}")
    if fails:
        print("라이선스 게이트 실패 — DR-12(퍼미시브 온리 · GPL/LGPL/AGPL 금지)")
        return 1
    print("라이선스 게이트 통과(DR-12)")
    return 0


if __name__ == "__main__":
    sys.exit(main())
