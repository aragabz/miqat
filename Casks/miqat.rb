cask "miqat" do
  # version and sha256 are rewritten by .github/workflows/release.yml on every release.
  version "0.2.0"
  sha256 "b6b48f7ffff00c5cbc263e00d01c77088b54b47b82d3144961e01a984c9b1e72"

  url "https://github.com/aragabz/miqat/releases/download/v#{version}/Miqat_#{version}_universal.dmg"
  name "Miqat"
  desc "Prayer times in the menu bar"
  homepage "https://github.com/aragabz/miqat"

  depends_on :macos

  app "Miqat.app"

  # The app is ad-hoc signed, not notarized, so Gatekeeper would refuse to open
  # it while the download quarantine flag is set.
  postflight_steps do
    run "/usr/bin/xattr", args: ["-dr", "com.apple.quarantine", "{{appdir}}/Miqat.app"]
  end

  uninstall quit: "app.miqat.desktop"

  zap trash: [
    "~/Library/Application Support/app.miqat.desktop",
    "~/Library/Caches/app.miqat.desktop",
    "~/Library/LaunchAgents/Miqat.plist",
    "~/Library/WebKit/app.miqat.desktop",
  ]
end
