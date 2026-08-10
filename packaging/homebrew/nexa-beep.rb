# Homebrew Cask — Nexa Beep(설치본 채널: .dmg 안의 .app)
#
# 버전과 체크섬 자리는 릴리스 워크플로가 **실제 산출물 해시로** 채운다(render-manifests.sh).
# 참고 프로젝트(sosomlab-tauri-test1)의 기록에 "새 버전마다 cask의 version/sha256를
# 손으로 갱신해야 한다"가 마찰점으로 남아 있었다 — 손으로 적는 해시는 언젠가 틀리고,
# 틀린 해시는 사용자 기기에서 설치 실패로 나타난다. 그래서 여기서는 채우지 않는다.
#
# ★ **quarantine을 postflight에서 명시적으로 뗀다** — 그러지 않으면 앱이 실행되지 않는다.
#   실측(08-11, macOS 15 · Intel): 서명 없는 앱 + quarantine → **SIGKILL(exit 137)**,
#   그리고 macOS가 /Applications에서 앱을 치워 버린다. 애드혹 서명을 붙여도 결과는 같았다
#   (quarantine 자체가 원인 · 공증(notarization) 없이는 우회 불가).
#   → Homebrew Cask는 기본적으로 quarantine을 **붙인다**(브라우저 다운로드와 같게).
#     인증서를 갖추기 전까지는 여기서 떼는 것 외에 방법이 없다. 무엇을 왜 하는지
#     caveats에 그대로 밝힌다 — 사용자가 모르는 채로 보안 검사를 끄지는 않는다.
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

  depends_on macos: :big_sur

  app "Nexa Beep.app"

  # 서명·공증이 없어 macOS가 실행을 막는다 — 설치 시점에 격리 표식을 뗀다.
  # (이 저장소에서 받은 것이 맞는지는 릴리스의 SHA256SUMS.txt로 확인할 수 있다.)
  postflight do
    system_command "/usr/bin/xattr",
                   args: ["-dr", "com.apple.quarantine", "#{appdir}/Nexa Beep.app"],
                   sudo: false
  end

  caveats <<~EOS
    이 앱은 코드 서명·공증이 되어 있지 않습니다(v1 · 인증서 미보유).
    설치 과정에서 macOS 격리 표식(com.apple.quarantine)을 제거해 바로 실행되도록 했습니다.

    받은 파일이 맞는지 확인하려면 릴리스의 SHA256SUMS.txt와 대조하세요:
      https://github.com/SosomLab/nexa-beep/releases

    이 앱은 같은 로컬 네트워크의 사용자를 찾기 위해 첫 실행 시 네트워크 접근 권한을 요청합니다.
  EOS

  # ⚠️ 앱이 아직 스스로 저장하는 것이 없다(설정 영속 M3-15 대기).
  #    아래는 **macOS가 앱마다 자동으로 만드는** 경로다 — 실제로 생기는 것만 적는다.
  zap trash: [
    "~/Library/Preferences/io.github.sosomlab.nexa-beep.plist",
    "~/Library/Saved Application State/io.github.sosomlab.nexa-beep.savedState",
  ]
end
