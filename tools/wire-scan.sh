#!/usr/bin/env bash
# wire-scan.sh — 와이어 평문 **실소켓 자동 회귀**(SEC-1 잔여 · 11-05 사용자 결정 · docs/29 §4 · W-1·W-2·W-3·W-5·W-7·W-8):
#   `tools/wirecap.sh`(08-15 · 사람이 hex를 눈으로 판정)를 **자동 판정**으로 확장한다. 인코더 단위 테스트는 인코더가 만든 바이트만 보고,
#   이 스크립트는 **실제 소켓으로 나간 바이트**를 본다("인코더가 맞다 ≠ 망에 나간 게 맞다" — 추정 금지·실측 필수).
#
#   ① 발견(UDP 멀티캐스트 239.255.77.77:47100 · 평문 구간) — 앱과 같은 방식(그룹 조인 · sudo 불필요 · 3-OS)으로 받아 패킷마다:
#      W-1 머리 NXBP · 길이 == 71 + name_len(허용 목록 밖 바이트 0) · W-3 ≤512B · 이름 = 표시 이름 또는 기본 `beep-xxxx`(UserHint는 hex 32자)
#      · W-7 epoch가 실시각이 아니다(< 10^9) · W-2·W-5 금칙 문자열(메시지 본문·소개글·호스트명·OS 사용자명) 부재 · 고정 구간 요약(사람 참고).
#   ② 세션(TCP · 암호 구간 · W-8) — Linux + tcpdump + 무암호 sudo일 때만(CI ubuntu): `-i any` pcap에 메시지 본문·소개글이 **한 조각도 없고**
#      TCP PSH 세그먼트는 **있어야**(비어서 통과하는 공허 판정 방지). 그 밖의 OS에서는 "pcap 없음 — 건너뜀"(실패 아님 · 발견 판정만).
#   신원 3개(ws-x 표시 이름+소개글 공유 on · ws-y · ws-z 기본 이름)를 새로 만들어 X→Y 메시지 1건을 보낸다(Y dump로 도착 확인 = 세션 트래픽 존재).
#
# 사용:  tools/wire-scan.sh [출력 폴더] [바이너리 폴더]   # 기본 = $BEEP_MULTI/wire-out · target/debug   ≈ 35s
# 전제:  빌드 산출물(nexa-beep + nbeep-imgdec) · python3 · 실행 중인 nexa-beep은 미리 종료(발견·포트가 겹친다).
# 결과:  disc.bin(길이 접두 원본 패킷) · disc.txt(판정·hex) · wire.pcap(있을 때) · {x,y,z}.log · 마지막 줄 PASS/FAIL · 종료 코드.
set -uo pipefail
cd "$(dirname "${BASH_SOURCE[0]}")/.."
export NEXA_TEST_HEADLESS=${NEXA_TEST_HEADLESS:-1}
. "$(dirname "${BASH_SOURCE[0]}")/lib/xvfb.sh"
M=${BEEP_MULTI:-$HOME/.nexa-beep-multi}
OUT=${1:-$M/wire-out}; mkdir -p "$OUT"; OUT=$(cd "$OUT" && pwd)
BINDIR=${2:-target/debug}
export NEXA_NO_AUTOSTART=1
trap 'BEEP_MULTI="$M" ./tools/autostart-clean.sh -q' EXIT
EXE=""; case "${OSTYPE:-}" in msys* | cygwin*) EXE=".exe" ;; esac
[ -x "$BINDIR/nexa-beep$EXE" ] || { echo "❌ $BINDIR/nexa-beep 없음 — cargo build -p nexa-beep -p nbeep-imgdec"; exit 1; }
PY=python3; command -v python3 >/dev/null 2>&1 || PY=python
command -v "$PY" >/dev/null 2>&1 || { echo "❌ python3 없음(발견 패킷 수신기)"; exit 1; }
export PYTHONUTF8=1 PYTHONIOENCODING=utf-8 # Windows 콘솔 cp949에서 ✗·… 출력이 UnicodeEncodeError로 죽는다(11-05 첫 실행)

# 금칙 문자열 — 와이어 어디에도(발견 · 세션 pcap) 평문으로 나오면 안 되는 것. 짧은 우연 일치를 피해 전부 ≥20자.
MSG="wire-scan-secret-body-7f3a9c2e"
BIO="wire-scan-bio-secret-51d2e8b0"
HOSTN=$(hostname 2>/dev/null || echo ""); USERN=${USER:-${USERNAME:-}}
DUR=26

seed() { # 폴더 · 표시 이름(빈 값 = 기본 이름 · W-5) · 소개글
  rm -rf "$M/$1"; mkdir -p "$M/$1/data"
  cp -f "$BINDIR/nexa-beep$EXE" "$BINDIR/nbeep-imgdec$EXE" "$M/$1/"
  { printf '_schema=1\nchat.window_mode=single\napp.autostart=off\n'
    [ -n "$2" ] && printf 'profile.display_name=%s\n' "$2"
    [ -n "$3" ] && printf 'profile.bio=%s\nprofile.share.basic=on\n' "$3"; } > "$M/$1/data/settings.cfg"
}
run() { # 폴더 · 스크립트 · 로그
  ( cd "$M/$1" && NEXA_SCRIPT="$2" "./nexa-beep$EXE" --window --live 2> "$3" >/dev/null ) &
}
short() { "$M/$1/nexa-beep$EXE" --whoami 2>/dev/null | awk -F'= ' '/^fingerprint/{print substr($2,1,8); exit}'; }

echo "▶ 신원 준비(새로) — $M/{ws-x,ws-y,ws-z} · 출력 $OUT"
seed ws-x wsx "$BIO"; seed ws-y wsy ""; seed ws-z "" ""

# ② 세션 pcap(Linux · tcpdump · 무암호 sudo) — 없으면 건너뜀.
PCAP=""; TD_PID=""
if [ "$(uname -s)" = Linux ] && command -v tcpdump >/dev/null 2>&1 && sudo -n true 2>/dev/null; then
  PCAP="$OUT/wire.pcap"; rm -f "$PCAP"
  sudo tcpdump -i any -U -q -w "$PCAP" 'tcp or udp port 47100' 2> "$OUT/tcpdump.log" &
  TD_PID=$!
  sleep 1
  echo "▶ ② tcpdump -i any → $PCAP (pid $TD_PID)"
else
  echo "▶ ② 세션 pcap 없음(Linux+tcpdump+sudo -n 아님) — 발견 판정만"
fi

# ① 발견 수신기(그룹 조인 · 백그라운드 · DUR초) — 원본을 disc.bin에 [len u16 BE][bytes]로 남긴다.
"$PY" - "$DUR" "$OUT/disc.bin" <<'PYEOF' > "$OUT/recv.log" 2>&1 &
import socket, struct, sys, time
secs, out = float(sys.argv[1]), sys.argv[2]
GROUP, PORT = "239.255.77.77", 47100
s = socket.socket(socket.AF_INET, socket.SOCK_DGRAM, socket.IPPROTO_UDP)
s.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
if hasattr(socket, "SO_REUSEPORT"):
    s.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEPORT, 1)
s.bind(("", PORT))
s.setsockopt(socket.IPPROTO_IP, socket.IP_ADD_MEMBERSHIP,
             struct.pack("4s4s", socket.inet_aton(GROUP), socket.inet_aton("0.0.0.0")))
s.settimeout(0.3)
n = 0
with open(out, "wb") as f:
    deadline = time.time() + secs
    while time.time() < deadline:
        try:
            data, addr = s.recvfrom(2048)
        except socket.timeout:
            continue
        f.write(struct.pack(">H", len(data)) + data); n += 1
print(f"recv packets={n}")
PYEOF
RECV_PID=$!
sleep 1

echo "▶ ① 기동 — X: 8s 연결·12s send / Y: 20s dump / 24s quit(3곳) · 수신기 ${DUR}s"
run ws-x "8000:activate=wsy;12000:send=$MSG;20000:dump;24000:quit" "$OUT/x.log"
run ws-y "20000:dump;24000:quit" "$OUT/y.log"
run ws-z "24000:quit" "$OUT/z.log"
wait "$RECV_PID" 2>/dev/null
wait
if [ -n "$TD_PID" ]; then sudo kill -INT "$TD_PID" 2>/dev/null; sleep 1; sudo chmod a+r "$PCAP" 2>/dev/null || true; fi
cat "$OUT/recv.log"
# 우리 신원의 키 접두(지문 short = 키 첫 4B hex) — 같은 LAN의 다른 인스턴스(사용자 실기 설치본 등) 패킷은 판정에서 뺀다(수만 센다).
XS=$(short ws-x); YS=$(short ws-y); ZS=$(short ws-z); echo "   X=$XS Y=$YS Z=$ZS"

echo; echo "═══ 판정 ═══"
ok=0; bad=0
pass() { printf '  ✓ %s\n' "$1"; ok=$((ok+1)); }
fail() { printf '  ✗ %s\n' "$1"; bad=$((bad+1)); }

# 세션 트래픽이 실제로 있었나(W-8 공허 판정 방지) — Y가 본문을 받았어야 한다.
if grep -q "thread .* mine=false .* text=$MSG" "$OUT/y.log"; then pass "X→Y 메시지 도착(세션 트래픽 존재)"; else fail "X→Y 메시지 미도착 — 세션이 서지 않았다(y.log)"; fi

# ① 발견 패킷 자동 판정(python — 레이아웃 docs/08 §2 · wire.rs: 71B 고정부 + name).
"$PY" - "$OUT/disc.bin" "$MSG" "$BIO" "$HOSTN" "$USERN" "$XS,$YS,$ZS" <<'PYEOF' > "$OUT/disc.txt" 2>&1
import struct, sys, re, collections
path, msg, bio, hostn, usern, ours = sys.argv[1:7]
ks = ours.split(",")
# 키 접두 → 기대 이름: X=wsx · Y=wsy · Z=None(기본 이름 — FR-S-50 정제 호스트명 또는 지문 라벨 `beep-xxxx` · 사람이 본다).
expect = {}
for k, nm in zip(ks, ("wsx", "wsy", None)):
    if len(k) == 8: expect[k] = nm
ours = set(expect)
raw = open(path, "rb").read()
allp, i = [], 0
while i + 2 <= len(raw):
    (n,) = struct.unpack(">H", raw[i:i+2]); i += 2
    allp.append(raw[i:i+n]); i += n
# 우리 3신원 패킷만 판정(키 첫 4B hex ∈ ours). 머리가 깨진 패킷은 키를 못 읽으니 전부 판정에 넣는다(W-1 위반으로 잡히게).
pkts = [p for p in allp if len(p) < 40 or p[:4] != b"NXBP" or p[8:12].hex() in ours]
print(f"packets={len(allp)} ours={len(pkts)} foreign={len(allp) - len(pkts)}")
bad = []
names = collections.Counter(); kinds = collections.Counter(); default_names = set()
# 금칙 = 메시지 본문·소개글(세션 전용이어야) · OS 사용자명(W-5 — 실명 신호). 호스트명은 금칙이 아니다: 기본 이름 = **정제된** 호스트명
# (FR-D-9 · FR-S-50 · R-19 해소 08-12 M1-10) — 정제가 실명을 걸러냈는지는 ws-z 이름을 사람이 본다(disc.txt `default=`).
forbid = [x for x in (msg, bio, usern) if x and len(x) >= 4]
for k, p in enumerate(pkts):
    tag = f"#{k} len={len(p)}"
    if len(p) > 512: bad.append(f"{tag} W-3 >512B")
    if p[:4] != b"NXBP": bad.append(f"{tag} W-1 magic {p[:4]!r}"); continue
    if len(p) < 71: bad.append(f"{tag} W-1 고정부 71B 미만"); continue
    ver, kind = p[4], p[5]
    kinds[kind] += 1
    epoch = struct.unpack(">Q", p[42:50])[0]
    nlen = p[70]
    if len(p) != 71 + nlen: bad.append(f"{tag} W-1 길이 {len(p)} != 71+name_len {nlen}(허용 목록 밖 바이트)")
    name = p[71:71+nlen]
    try: ns = name.decode("utf-8")
    except UnicodeDecodeError: bad.append(f"{tag} W-4 name UTF-8 아님"); continue
    want = expect.get(p[8:12].hex(), "?")
    if kind == 5:
        if not re.fullmatch(r"[0-9a-f]{32}", ns): bad.append(f"{tag} UserHint name이 hex32가 아님 {ns!r}")
    else:
        names[ns] += 1
        if want is None:
            default_names.add(ns)
            if usern and len(usern) >= 4 and usern.lower() in ns.lower(): bad.append(f"{tag} W-5 기본 이름 {ns!r}에 OS 사용자명")
        elif ns != want: bad.append(f"{tag} W-1 이름 {ns!r} != 설정한 표시 이름 {want!r}(이름 자리에 다른 것)")
    if epoch >= 1_000_000_000: bad.append(f"{tag} W-7 epoch={epoch} 실시각으로 보임(가동 시간 유출)")
    if ver != 1: print(f"  note {tag} ver={ver}(dual-announce?)")
    for fz in forbid:
        if fz.encode() in p: bad.append(f"{tag} W-2/W-5 금칙 문자열 {fz!r} 평문 노출")
print("kinds=" + ",".join(f"{k}:{v}" for k, v in sorted(kinds.items())))
print("names=" + ",".join(f"{k}:{v}" for k, v in sorted(names.items())))
# 고정 구간 요약(사람 참고 — 알려진 고정값 = 매직·ver·kind·flags·키·tcp_port·epoch·instance 외 새 재식별자가 있는가 · docs/29 §4-2 ②).
by_key = collections.defaultdict(list)
for p in pkts:
    if len(p) >= 71 and p[:4] == b"NXBP": by_key[p[8:40]].append(p)
for key, ps in by_key.items():
    if len(ps) < 2: continue
    n = min(len(p) for p in ps); p0 = ps[0]
    fixed = [all(p[j] == p0[j] for p in ps) for j in range(n)]
    runs, j = [], 0
    while j < n:
        if fixed[j]:
            a = j
            while j < n and fixed[j]: j += 1
            runs.append((a, j - 1))
        else: j += 1
    print(f"  key={key[:4].hex()}… packets={len(ps)} fixed=" + " ".join(f"[{a:#04x}..{b:#04x}]" for a, b in runs))
seen_need = {"wsx", "wsy"} - set(names)
if seen_need: bad.append(f"표시 이름 미관측 {sorted(seen_need)} — 수신기가 우리 인스턴스를 못 봤다")
if not default_names: bad.append("W-5 기본 이름 미관측 — ws-z 패킷 없음")
else: print(f"default={sorted(default_names)} hostname={hostn!r}  ← W-5: 정제 호스트명/지문 라벨인지 · 실명이 남았는지 사람이 본다(FR-S-50)")
for b in bad: print("  ✗ " + b)
if not ours: bad.append("우리 신원 지문을 못 읽었다(--whoami) — 판정 불가")
print("DISC_BAD=" + str(len(bad)) + " DISC_PKTS=" + str(len(pkts)))
PYEOF
DISC_PKTS=$(grep -o 'DISC_PKTS=[0-9]*' "$OUT/disc.txt" | cut -d= -f2); DISC_BAD=$(grep -o 'DISC_BAD=[0-9]*' "$OUT/disc.txt" | cut -d= -f2)
grep -v '^DISC_BAD=' "$OUT/disc.txt" | sed 's/^/    /'
if [ "${DISC_PKTS:-0}" -ge 1 ]; then pass "발견 패킷 수신 ${DISC_PKTS}건(그룹 조인 · sudo 없이 = 공격자도 이만큼 본다)"; else fail "발견 패킷 0건 — 멀티캐스트 루프백/수신기 확인(recv.log)"; fi
if [ "${DISC_BAD:-1}" = 0 ]; then pass "발견 W-1·W-2·W-3·W-4·W-5·W-7 전부 통과(패킷마다)"; else fail "발견 판정 위반 ${DISC_BAD:-?}건 — $OUT/disc.txt"; fi

# ② 세션 pcap(W-8) — 금칙 문자열 부재 + TCP PSH 존재.
if [ -n "$PCAP" ] && [ -s "$PCAP" ]; then
  psh=$(tcpdump -nn -r "$PCAP" 'tcp[tcpflags] & tcp-push != 0' 2>/dev/null | wc -l | tr -d ' ')
  if [ "${psh:-0}" -ge 1 ]; then pass "pcap TCP PSH 세그먼트 ${psh}건(세션 바이트가 잡혔다)"; else fail "pcap에 TCP PSH 0건 — 세션 캡처 실패(공허 판정 거부)"; fi
  leak=0
  for fz in "$MSG" "$BIO"; do
    if grep -q -F --binary-files=text -- "$fz" "$PCAP"; then printf '    ✗ pcap에 "%s" 평문\n' "$fz"; leak=1; fi
  done
  if [ "$leak" = 0 ]; then pass "W-8 세션 평문 부재(메시지 본문·소개글이 pcap 어디에도 없음 · $(wc -c < "$PCAP")B)"; else fail "W-8 세션 평문 노출 — $PCAP"; fi
elif [ -n "$PCAP" ]; then
  fail "tcpdump가 pcap을 남기지 않았다 — $OUT/tcpdump.log"
else
  echo "  – W-8 세션 pcap 건너뜀(이 OS · 발견 판정만 — 세션 평문 부재는 단위 tap + Linux CI가 본다)"
fi

echo; echo "결과: ✓ $ok · ✗ $bad   (로그 = $OUT/{disc.txt,recv.log,x,y,z}.log)"
if [ "$bad" = 0 ]; then echo "PASS wire-scan ($OUT)"; else echo "FAIL wire-scan ($OUT)"; exit 1; fi
