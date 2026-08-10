# Homebrew Cask — Nexa Beep(설치본 채널: .dmg 안의 .app)
#
# 버전과 체크섬 자리는 릴리스 워크플로가 **실제 산출물 해시로** 채운다(render-manifests.sh).
# 참고 프로젝트(sosomlab-tauri-test1)의 기록에 "새 버전마다 cask의 version/sha256를
# 손으로 갱신해야 한다"가 마찰점으로 남아 있었다 — 손으로 적는 해시는 언젠가 틀리고,
# 틀린 해시는 사용자 기기에서 설치 실패로 나타난다. 그래서 여기서는 채우지 않는다.
#
# ★ Homebrew Cask는 설치할 때 quarantine 속성을 떼므로, **서명하지 않은 이 앱도
#   Gatekeeper 경고 없이 실행된다.** macOS 사용자에게 가장 매끄러운 경로다.
cask "nexa-beep" do
  arch arm: "arm64", intel: "x64"

  version "@VERSION@"
  sha256 arm:   "@SHA_MAC_ARM64_DMG@",
         intel: "@SHA_MAC_X64_DMG@"

  url "https://github.com/SosomLab/nexa-beep/releases/download/v#{version}/nexa-beep-#{version}-macos-#{arch}.dmg",
      verified: "github.com/SosomLab/nexa-beep/"
  name "Nexa Beep"
  desc "Zero-config local network messenger"
  homepage "https://github.com/SosomLab/nexa-beep"

  livecheck do
    url :url
    strategy :github_latest
  end

  depends_on macos: ">= :big_sur"

  app "Nexa Beep.app"

  # ⚠️ 앱이 아직 스스로 저장하는 것이 없다(설정 영속 M3-15 대기).
  #    아래는 **macOS가 앱마다 자동으로 만드는** 경로다 — 실제로 생기는 것만 적는다.
  zap trash: [
    "~/Library/Preferences/io.github.sosomlab.nexa-beep.plist",
    "~/Library/Saved Application State/io.github.sosomlab.nexa-beep.savedState",
  ]
end
