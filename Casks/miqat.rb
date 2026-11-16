cask "miqat" do
  # version and sha256 are rewritten by .github/workflows/release.yml on every release.
  version "0.1.0"
  sha256 "0a25a8e3ff514e22a4f2ff1eea91b33c451fe9c06b78a971a68a94bd3f77e7f6"

  url "https://github.com/aragabz/miqat/releases/download/v#{version}/Miqat_#{version}_universal.dmg"
  name "Miqat"
  desc "Prayer times in the menu bar"
  homepage "https://github.com/aragabz/miqat"

  depends_on macos: ">= :big_sur"

  app "Miqat.app"

  # The app is ad-hoc signed, not notarized, so Gatekeeper would refuse to open
  # it while the download quarantine flag is set.
  postflight do
    system_command "/usr/bin/xattr",
                   args: ["-dr", "com.apple.quarantine", "#{appdir}/Miqat.app"]
  end

  uninstall quit: "app.miqat.desktop"

  zap trash: [
    "~/Library/Application Support/app.miqat.desktop",
    "~/Library/Caches/app.miqat.desktop",
    "~/Library/LaunchAgents/Miqat.plist",
    "~/Library/WebKit/app.miqat.desktop",
  ]
end
