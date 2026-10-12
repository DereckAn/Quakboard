# Quakboard

<div align="center">
  <img src=".github/pato.webp" alt="Quakboard icon" width="128">

  <p><strong>A fast, private clipboard manager for macOS, Windows and Linux,<br>with encrypted sync to your other devices on the same network.</strong></p>

  [![Latest release](https://img.shields.io/github/v/release/DereckAn/Quakboard)](https://github.com/DereckAn/Quakboard/releases/latest)
  [![License: MIT](https://img.shields.io/badge/License-MIT-blue.svg)](LICENSE)
  [![Built with Tauri](https://img.shields.io/badge/Built%20with-Tauri%20v2-FFC131?logo=tauri&logoColor=white)](https://tauri.app)
  [![Svelte 5](https://img.shields.io/badge/Svelte-5-FF3E00?logo=svelte&logoColor=white)](https://svelte.dev)
  [![Rust](https://img.shields.io/badge/Rust-stable-orange?logo=rust&logoColor=white)](https://www.rust-lang.org)
</div>

---

## Features

### History
- **Everything you copy, saved:** text, code, links, colors, images and files.
- **Smart detection:** code (with its language), links (with title and preview), and colors (HEX, RGB, HSL, OKLCH, with conversion between them).
- **Full-text search** across your whole history, with matches highlighted.
- **Grouped by date:** Today, Yesterday, This Week, This Month, Older.
- **Favorites**, and duplicates move to the top instead of piling up.
- **Paste straight back:** pick an item and press Enter to paste it into the app you were using.
- **Retention controls:** limit how many items are kept or for how long. Favorites are never deleted.

### Sync between your devices
- **Pair once with a 6-digit code.** Devices on the same network find each other automatically, or you can connect by IP.
- **Text and images sync as you copy.** Image sync can be turned off.
- **Files are offered, not pushed:** send a file to a device, and it downloads only when you choose Fetch.
- **No server and no account:** devices talk directly, encrypted end to end.

### Desktop
- **Global hotkey** to show or hide the window (default `Ctrl/Cmd + Shift + V`), changeable in Settings.
- **Screenshots** (Lab feature): full-screen or region capture, straight to your history.
- **Capture-folder watcher** (optional): new screenshots and recordings appear in history without being copied.
- **Launch at login, tray icon and in-app updates.**

---

## Screenshots

<div align="center">
  <img src=".github/app.png" alt="Quakboard on macOS" width="500">
</div>

---

## Download

Get the latest version from **[Releases](https://github.com/DereckAn/Quakboard/releases/latest)**.

| System | File |
|---|---|
| macOS (Apple Silicon) | `Quakboard-<version>-macOS-AppleSilicon.dmg` |
| macOS (Intel) | `Quakboard-<version>-macOS-Intel.dmg` |
| Windows (x64 / ARM64) | `Quakboard-<version>-Windows-<arch>-Setup.exe` or `.msi` |
| Linux (x64 / ARM64) | `.deb`, `.rpm` or `.AppImage` |
| Arch Linux | `packaging/arch/PKGBUILD` (see [its README](packaging/arch/README.md)) |

Once installed, Quakboard checks for new versions and offers to install them.

> **Arch Linux on Wayland:** if the AppImage crashes on start, use the `PKGBUILD`. It uses your system's WebKit instead of the bundled one.

### Quakboard on iPhone and iPad (in development)

A native iOS app is being built with the same sync core: history, pairing, Paste & Send, and a Share extension. Progress is tracked in [docs/MOBILE_PLAN.md](docs/MOBILE_PLAN.md). Android comes after iOS.

---

## Keyboard shortcuts

| Shortcut | Action |
|---|---|
| `Ctrl/Cmd + Shift + V` | Show or hide Quakboard (changeable in Settings) |
| `↑` / `↓` | Move through your history |
| `Enter` | Paste the selected item into the previous app |
| `Esc` | Clear the search or filter; press again to hide the window |

---

## Privacy

- **Local first:** your history stays in a SQLite database on your device.
- **No account, no telemetry, no cloud.**
- **Sync is device to device** on your local network. Pairing uses SPAKE2, so the code is never sent over the network, and everything after that is encrypted with ChaCha20-Poly1305.
- **Internet is only used** for link previews and update checks.

**Where your data lives:**

| System | Folder |
|---|---|
| macOS | `~/Library/Application Support/com.dereckan.quakboard/` |
| Linux | `~/.local/share/com.dereckan.quakboard/` |
| Windows | `%APPDATA%\com.dereckan.quakboard\` |

---

## Build from source

### Prerequisites
- [Rust](https://rustup.rs) (stable) and [Bun](https://bun.sh)
- **macOS:** Xcode Command Line Tools (`xcode-select --install`)
- **Windows:** the [Tauri prerequisites](https://v2.tauri.app/start/prerequisites/) (Microsoft C++ Build Tools and WebView2)
- **Linux (Debian/Ubuntu):**
  ```bash
  sudo apt install libwebkit2gtk-4.1-dev libgtk-3-dev libayatana-appindicator3-dev \
    librsvg2-dev libssl-dev pkg-config build-essential libxdo-dev \
    libxcb-shape0-dev libxcb-xfixes0-dev liblcms2-dev libjpeg-dev
  ```

### Run and build
```bash
git clone https://github.com/DereckAn/Quakboard.git
cd Quakboard
bun install

bun run tauri dev     # run with hot reload
bun run tauri build   # installers in src-tauri/target/release/bundle/
```

### Tests and checks
These are the same checks CI runs:
```bash
bun run check                                   # Svelte and TypeScript
bun run test:frontend                           # Vitest
cargo test  --manifest-path src-tauri/Cargo.toml
cargo clippy --manifest-path src-tauri/Cargo.toml --all-targets -- -D warnings
cargo test  --manifest-path crates/quakboard-sync/Cargo.toml
```

### iOS (on a Mac with Xcode)
```bash
./scripts/build-ios.sh      # builds the sync core for iOS and its Swift package
open ios/Quakboard/Quakboard.xcodeproj
```
Run the script again after changing Rust code. Xcode doesn't run it for you.

---

## Project structure

```
src/                        Desktop UI (SvelteKit + Svelte 5, TailwindCSS v4)
src-tauri/                  Desktop backend (Rust + Tauri v2): clipboard, database, hotkeys
crates/
  quakboard-sync/           Sync core shared by every app: pairing, encryption, transfers
  quakboard-sync-ffi/       Swift and Kotlin bindings for the core (UniFFI)
ios/
  QuakboardSync/            Swift package wrapping the core (generated by scripts/build-ios.sh)
  Quakboard/                iOS app (SwiftUI)
packaging/arch/             Arch Linux PKGBUILD
docs/                       Design plans: LAN sync, image sync, file sync, mobile
```

---

## Troubleshooting

**Devices don't find each other**
- Both need sync turned on and must be on the same network.
- Some networks (guest or office Wi-Fi) block discovery. Use **Connect by IP** instead.
- Allow Quakboard through your firewall. Sync uses TCP port **47823**.
- On iPhone, allow **Local Network** for Quakboard in Settings > Privacy & Security.

**The hotkey doesn't work:** another app may be using it. Pick a different one in Settings.

**Port 1420 already in use** (development):
```bash
lsof -ti:1420 | xargs kill -9
```

**Start over** (deletes your history and pairings):
quit Quakboard, then delete the data folder for your system listed under [Privacy](#privacy).

---

## Roadmap

- [docs/MOBILE_PLAN.md](docs/MOBILE_PLAN.md): iPhone, iPad and Android apps
- [documentation/ROADMAP.md](documentation/ROADMAP.md): the longer-term plan

---

## Contributing

Contributions are welcome. See [CONTRIBUTING.md](CONTRIBUTING.md) for the workflow. In short:
1. Fork the repository and create a branch (`git checkout -b feat/my-change`).
2. Run the [tests and checks](#tests-and-checks) before opening a pull request.
3. Use [conventional commits](https://www.conventionalcommits.org).

---

## License

MIT. See [LICENSE](LICENSE).

## Contact

Created by [@DereckAn](https://github.com/DereckAn). Report bugs and ideas in [GitHub Issues](https://github.com/DereckAn/Quakboard/issues).

<div align="center">
  <br>
  Made with Tauri, Svelte 5 and Rust
</div>
