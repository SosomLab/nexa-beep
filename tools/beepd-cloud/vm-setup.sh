#!/usr/bin/env bash
# nexa-beepd 단발 테스트 VM 셋업 — deploy-gcp.ps1이 소스 tarball과 함께 업로드해 root로 실행한다.
# (GCP Debian 12 기준 · 소스에서 서버만 빌드 — 이 PC에서 리눅스 교차 빌드 수단이 없을 때의 경로)
#
# 하는 일: 빌드 도구 → 저메모리 VM 스왑 → rustup(최소) → cargo build -p nexa-beepd →
#          systemd 상주(beepd.service · 비루트 계정) → 서버 신원(핀) 출력.
set -euo pipefail

SRC=/tmp/beepd-src.tar.gz
APP=/opt/beepd

export DEBIAN_FRONTEND=noninteractive
apt-get update -qq
apt-get install -y -qq gcc curl tar >/dev/null

# e2-micro(1GB)에서도 빌드가 죽지 않게 — 스왑 2G(있으면 건너뜀). e2-small(2GB)은 불필요.
if [ "$(free -m | awk '/^Mem:/{print $2}')" -lt 1500 ] && [ ! -f /swapfile ]; then
  fallocate -l 2G /swapfile && chmod 600 /swapfile && mkswap /swapfile && swapon /swapfile
  echo "[셋업] 저메모리 VM — 스왑 2G 활성"
fi

# 상주 계정(비루트) — 빌드와 서버 실행을 같은 계정으로.
mkdir -p "$APP/src"
id -u beepd >/dev/null 2>&1 || useradd -r -d "$APP" -s /usr/sbin/nologin beepd
tar -xzf "$SRC" -C "$APP/src"
# rust-toolchain.toml은 CI 재현용으로 5타깃을 강제 설치시킨다 — VM에선 호스트 타깃만
# 필요하므로 지운다(기본 stable로 빌드 · MSRV 1.82 충족).
rm -f "$APP/src/rust-toolchain.toml"
chown -R beepd:beepd "$APP"

sudo -u beepd env HOME="$APP" sh -c \
  'curl -sSf https://sh.rustup.rs | sh -s -- -y --profile minimal --no-modify-path' >/dev/null
echo "[셋업] rust 설치 완료 — 서버 빌드 시작(수 분 소요)"
sudo -u beepd env HOME="$APP" sh -c \
  "cd '$APP/src' && '$APP/.cargo/bin/cargo' build --release -p nexa-beepd"
install -m 0755 "$APP/src/target/release/nexa-beepd" "$APP/nexa-beepd"

cat > /etc/systemd/system/beepd.service <<EOF
[Unit]
Description=nexa-beepd relay server (test session)
After=network-online.target
Wants=network-online.target

[Service]
User=beepd
WorkingDirectory=$APP
ExecStart=$APP/nexa-beepd --port 47300 --key $APP/beepd.key --verbose
Restart=on-failure

[Install]
WantedBy=multi-user.target
EOF
systemctl daemon-reload
systemctl enable --now beepd
sleep 2

echo "──────────────────────────────────────────────"
echo "[셋업] 완료 — 서버 상태·핀:"
systemctl is-active beepd
# 클라이언트가 핀할 서버 신원 키(첫 접속은 자동 TOFU라 참고용 대조 값이다).
journalctl -u beepd --no-pager | grep -m1 '서버 신원' || journalctl -u beepd --no-pager | tail -8
