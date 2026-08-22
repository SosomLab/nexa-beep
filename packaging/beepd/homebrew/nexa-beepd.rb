# Homebrew Formula — nexa-beepd(릴레이 서버 · 단일 실행 파일)
#
# 클라이언트(cask nexa-beep · formula nexa-beep-portable)와 **별도 패키지**다 —
# 서버는 `beepd-v*` 태그로 따로 릴리스된다(W-4 분리 배포).
# 버전·체크섬 자리는 릴리스 워크플로가 채운다(render-manifests-beepd.sh).
class NexaBeepd < Formula
  desc "Blind relay server for Nexa Beep (rendezvous + hole punch + relay fallback)"
  homepage "https://github.com/SosomLab/nexa-beep"
  version "@VERSION@"
  license "PolyForm-Noncommercial-1.0.0"

  on_macos do
    on_arm do
      url "https://github.com/SosomLab/nexa-beep/releases/download/beepd-v@VERSION@/nexa-beepd-@VERSION@-macos-arm64.tar.gz"
      sha256 "@SHA_BEEPD_MAC_ARM64@"
    end
    # ⚠️ Intel Mac 자산은 없다(docs/41 §1) — 필요하면 소스 빌드.
  end

  on_linux do
    on_intel do
      url "https://github.com/SosomLab/nexa-beep/releases/download/beepd-v@VERSION@/nexa-beepd-@VERSION@-linux-x64.tar.gz"
      sha256 "@SHA_BEEPD_LINUX_X64@"
    end
    on_arm do
      url "https://github.com/SosomLab/nexa-beep/releases/download/beepd-v@VERSION@/nexa-beepd-@VERSION@-linux-arm64.tar.gz"
      sha256 "@SHA_BEEPD_LINUX_ARM64@"
    end
  end

  def install
    # 최상위 폴더 유무에 기대지 않는다 — 찾아서 확실히 설치(클라 formula와 같은 방침).
    exe = Dir["nexa-beepd", "*/nexa-beepd"].first
    odie "압축물에서 nexa-beepd 실행 파일을 찾지 못했습니다" if exe.nil?
    bin.install exe => "nexa-beepd"
  end

  def caveats
    <<~EOS
      첫 실행이 서버 신원 키(beepd.key)를 만들고 "서버 신원(핀)"을 출력합니다:
        nexa-beepd --port 47300 --key /usr/local/var/beepd/beepd.key --verbose

      방화벽은 TCP·UDP 둘 다 같은 포트(기본 47300)를 열어야 합니다.
      상주(systemd·launchd)·운영 절차: https://github.com/SosomLab/nexa-beep/blob/main/docs/41-beepd-ops-guide.md
      동작 원리(랑데부·홀펀칭·릴레이): https://github.com/SosomLab/nexa-beep/blob/main/docs/42-relay-rendezvous-walkthrough.md
    EOS
  end

  test do
    # 네트워크를 건드리지 않는 경로만(클라 formula와 같은 규칙).
    assert_match version.to_s, shell_output("#{bin}/nexa-beepd --version")
  end
end
