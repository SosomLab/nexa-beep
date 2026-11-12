#!/usr/bin/env python3
"""disc-flood.py — 발견 광고 폭주 주입기(M5-3 2차 ⓑ 감쇠 시나리오 · tools/flood-scenario.sh 전용 · 11-12).

같은 PC의 시험 인스턴스가 내는 **진짜 Announce 한 장**(표시 이름으로 고른다)을 멀티캐스트 수신으로 받아,
그 바이트 그대로 발견 그룹(239.255.77.77:47100 · TTL 1)에 <pps>로 <초> 동안 다시 뿌린다.
발견 패킷은 서명 없는 힌트라(docs/08 §2) 재전송이 곧 "같은 상대의 광고 폭주"다 — 패킷을 지어내지 않는다.

와이어(crates/nbeep-net/src/wire.rs): magic "NXBP" · ver · type(2 = Announce) · … · name_len@70 · name@71.

사용: disc-flood.py <표시 이름> <pps> <초> [대기 상한 초=20]
출력: `captured len=<n>` · `sent=<n> secs=<s> pps=<실측>` · 실패 = 종료 코드 1.
"""
import socket
import struct
import sys
import time

GROUP, PORT = "239.255.77.77", 47100


def main() -> int:
    if len(sys.argv) < 4:
        print(__doc__)
        return 2
    name = sys.argv[1].encode("utf-8")
    pps, secs = int(sys.argv[2]), float(sys.argv[3])
    wait = float(sys.argv[4]) if len(sys.argv) > 4 else 20.0

    rx = socket.socket(socket.AF_INET, socket.SOCK_DGRAM, socket.IPPROTO_UDP)
    rx.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
    if hasattr(socket, "SO_REUSEPORT"):
        rx.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEPORT, 1)
    rx.bind(("", PORT))
    rx.setsockopt(socket.IPPROTO_IP, socket.IP_ADD_MEMBERSHIP,
                  struct.pack("4s4s", socket.inet_aton(GROUP), socket.inet_aton("0.0.0.0")))
    rx.settimeout(0.5)

    pkt = None
    end = time.monotonic() + wait
    while time.monotonic() < end:
        try:
            buf, _ = rx.recvfrom(2048)
        except socket.timeout:
            continue
        if len(buf) < 71 or buf[:4] != b"NXBP" or buf[5] != 2:
            continue
        if buf[71:71 + buf[70]] == name:
            pkt = buf
            break
    rx.close()
    if pkt is None:
        print(f"no Announce from name={sys.argv[1]} within {wait:.0f}s")
        return 1
    print(f"captured len={len(pkt)}", flush=True)

    tx = socket.socket(socket.AF_INET, socket.SOCK_DGRAM, socket.IPPROTO_UDP)
    tx.setsockopt(socket.IPPROTO_IP, socket.IP_MULTICAST_TTL, 1)  # 링크 밖으로 나가지 않는다
    tx.setsockopt(socket.IPPROTO_IP, socket.IP_MULTICAST_LOOP, 1)
    gap, sent, t0 = 1.0 / pps, 0, time.monotonic()
    while True:
        now = time.monotonic()
        if now - t0 >= secs:
            break
        tx.sendto(pkt, (GROUP, PORT))
        sent += 1
        nxt = t0 + sent * gap
        if nxt > now:
            time.sleep(nxt - now)
    el = time.monotonic() - t0
    print(f"sent={sent} secs={el:.1f} pps={sent / el:.0f}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
