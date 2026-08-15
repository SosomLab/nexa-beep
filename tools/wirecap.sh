#!/usr/bin/env bash
# wirecap.sh — 발견 멀티캐스트 **실소켓** 캡처 + 판정 보조 (M1-12 · [docs/29 §4-2] · SEC-T1).
#
# 왜 있나: 발견 패킷은 이 앱의 유일한 평문 구간이다. 자동 회귀(M1-11)는 **인코더가
# 만든 바이트**를 검사한다 — 이 스크립트는 **실제 소켓에서 망으로 나간 바이트**를 본다
# ("인코더가 맞다 ≠ 망에 나간 게 맞다" — 추정 금지·실측 필수의 마지막 절반).
#
# 실행 시점(SEC-T1): **발견 와이어 포맷을 바꾼 커밋마다** 재실행하고 판정을 journal에
# 남긴다. 발신 경로가 늘어난 커밋(유니캐스트·인터페이스별 발신 등)도 같은 트리거다.
#
# 판정(사람 몫 — 출력을 눈으로 본다):
#   ① ASCII 열에 사람이 읽을 것이 **표시 이름 하나뿐**인가(이메일·호스트명 조각·전화 = 유출)
#   ② "고정 바이트" 요약에서 **새 재식별자**가 생기지 않았는가(알려진 고정값 = 매직·버전·
#      종류·키 지문(R-18 등록)·tcp_port·epoch·instance(기동 무작위 · D-22) · 그 외 = 심사)
#
# sudo 불필요 — tcpdump(원시 소켓)가 아니라 앱과 같은 방식(그룹 조인)으로 받는다(T0 원칙).
# 사용법: tools/wirecap.sh [초]   (기본 6초 · 실행 중인 인스턴스가 있어야 잡힌다)

set -euo pipefail
SECS="${1:-6}"

python3 - "$SECS" <<'PYEOF'
import socket, struct, sys, time, collections

GROUP, PORT = "239.255.77.77", 47100  # docs/08 §2 — 프로토콜 헌법(불변)
secs = float(sys.argv[1])

s = socket.socket(socket.AF_INET, socket.SOCK_DGRAM, socket.IPPROTO_UDP)
s.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
if hasattr(socket, "SO_REUSEPORT"):
    s.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEPORT, 1)
s.bind(("", PORT))
s.setsockopt(socket.IPPROTO_IP, socket.IP_ADD_MEMBERSHIP,
             struct.pack("4s4s", socket.inet_aton(GROUP), socket.inet_aton("0.0.0.0")))
s.settimeout(0.3)

def hexdump(b):
    for off in range(0, len(b), 16):
        row = b[off:off + 16]
        hx = " ".join(f"{c:02x}" for c in row)
        asc = "".join(chr(c) if 32 <= c < 127 else "." for c in row)
        print(f"  {off:04x}  {hx:<47}  |{asc}|")

by_src = collections.defaultdict(list)
deadline = time.time() + secs
print(f"WIRECAP {GROUP}:{PORT} {secs:.0f}s — 실소켓 수신(그룹 조인 · sudo 불필요)")
while time.time() < deadline:
    try:
        data, addr = s.recvfrom(2048)
    except socket.timeout:
        continue
    by_src[addr].append(data)

first_of = {}
for addr, pkts in sorted(by_src.items()):
    print(f"\n== from={addr[0]}:{addr[1]}  packets={len(pkts)}  len={sorted(set(len(p) for p in pkts))}")
    p0 = pkts[0]
    first_of[addr] = p0
    hexdump(p0)
    # 판정 ② 보조 — 같은 발신원의 전 패킷에서 값이 안 변한 바이트 구간(= 고정 필드 후보).
    if len(pkts) > 1:
        n = min(len(p) for p in pkts)
        fixed = [all(p[i] == p0[i] for p in pkts) for i in range(n)]
        runs, i = [], 0
        while i < n:
            if fixed[i]:
                j = i
                while j < n and fixed[j]:
                    j += 1
                runs.append((i, j - 1))
                i = j
            else:
                i += 1
        print("  고정 구간(패킷 간 불변 = 재식별자 후보): "
              + " ".join(f"[{a:#04x}..{b:#04x}]" for a, b in runs))

if not by_src:
    print("수신 0 — 실행 중인 인스턴스가 없거나 이 망이 멀티캐스트를 막는다")
    sys.exit(1)
print(f"\n발신원 {len(by_src)}곳 — 판정 ①(ASCII 열)·②(고정 구간)를 눈으로 확인할 것")
PYEOF
