# Miqat

Prayer times in your macOS menu bar.

Miqat sits in the menu bar and shows a countdown to the next prayer. Click it
for today's times and the qibla direction. Times are calculated on your Mac
from your coordinates, so it works offline once a location is set.

- Countdown, time of the next prayer, or icon only in the menu bar; the text
  turns red in the last 15 minutes
- Twelve calculation methods (Muslim World League, Umm al-Qura, Egyptian,
  ISNA, Diyanet and more) and both Asr conventions
- Notifications and adhan at prayer time, with a per-prayer mute and an
  optional reminder 5 to 30 minutes before
- Bundled adhan recording, or pick your own audio file
- City search or automatic location, 12 or 24-hour clock, launch at login

## Install

Requires macOS 11 or later, Apple silicon or Intel.

```sh
brew tap aragabz/miqat https://github.com/aragabz/miqat
brew install --cask miqat
```

Upgrade later with `brew upgrade --cask miqat`.

You can also download the DMG from the
[latest release](https://github.com/aragabz/miqat/releases/latest). Miqat is
not notarized by Apple, so after a manual download macOS will refuse to open
it until you clear the quarantine flag:

```sh
xattr -dr com.apple.quarantine /Applications/Miqat.app
```

The Homebrew cask does this for you.

## Privacy

Miqat has no account and no analytics. It makes network requests only when you
ask it to find a location: city search goes to
[Open-Meteo geocoding](https://open-meteo.com/en/docs/geocoding-api), and
"Detect from my internet connection" looks up your approximate position from
your IP address via
[ipwho.is](https://ipwho.is). Entering coordinates by hand makes no requests.

## Build from source

You need [Rust](https://rustup.rs), [Bun](https://bun.sh) and the Xcode
command line tools.

```sh
bun install
bun run dev      # run with live reload
bun run build    # produce Miqat.app and a DMG under src-tauri/target/release/bundle
```

Run the tests with `cargo test --manifest-path src-tauri/Cargo.toml`.

## Releasing

1. Set the new version in `src-tauri/tauri.conf.json`, `src-tauri/Cargo.toml`
   and `package.json`, and commit.
2. Tag and push: `git tag v0.2.0 && git push origin main v0.2.0`.

The release workflow builds a universal DMG, publishes it as a GitHub release
and updates `Casks/miqat.rb` with the new version and checksum.

## License

Code is [MIT](LICENSE). The bundled adhan recording is by Atcovi, distributed
unmodified under CC BY-SA 4.0; see
[ADHAN-LICENSE.txt](src-tauri/resources/ADHAN-LICENSE.txt).
