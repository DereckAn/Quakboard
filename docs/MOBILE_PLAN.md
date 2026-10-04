# Plan: iPhone/iPad and Android apps

**Status:** planned, nothing built yet.

## Goal
Quakboard on iPhone, iPad and later Android, syncing with the desktop app: text, images and files, using the same pairing and encryption.

## The approach in one picture
```
              ┌────────────────────────────────────┐
              │  quakboard-sync (Rust crate)        │  pairing, encryption, transfers,
              │  ONE copy, already tested (~300)    │  echo guard, file offers/fetch
              └──────┬──────────────────┬───────────┘
                     │                  │
        ┌────────────┘                  └──────────────┐
  quakboard (desktop)                     quakboard-sync-ffi (UniFFI)
  Tauri + Svelte, as today                      │
                                   ┌────────────┴────────────┐
                              iOS / iPadOS                Android
                           SwiftUI + Swift package     Kotlin + Compose
```
- **One sync core in Rust.** Pairing (SPAKE2), encryption (ChaCha20-Poly1305), the chunked file stream and all the checks stay exactly as tested. They're never rewritten in Swift or Kotlin; three copies of security code that must agree byte for byte is how bugs and holes happen.
- **Native UIs:** SwiftUI on iOS, then Jetpack Compose on Android. On a phone, the app is mostly operating-system integration (share sheet, clipboard rules, local-network permission, background limits). Native code does that directly, and the UI itself is small.
- **UniFFI** (Mozilla) generates the Swift and Kotlin bindings from the Rust API, so the apps call `pair()`, `sendText()`, `fetchFile()` as ordinary native functions.

## What the code looks like today (measured)
| Module | Lines | Depends on | Goes to |
|---|---|---|---|
| `mod.rs` (Frame, Sealed), `body`, `transport`, `stream`, `pairing`, `store` | ~1,930 | nothing app-specific | **core** |
| `image` (seal/open, safe decode, `pixel_hash`) | 603 | app code in one test only | **core** |
| `fetch` (protocol, `decide`) | 343 | nothing | **core** |
| `service` (listener, broadcast, pairing, receiving, serving fetches) | 1,710 | nothing (the outside world comes in through `SyncHooks`) | **core** |
| `discovery` (mDNS via `mdns-sd`) | 232 | nothing | **core, optional feature** (see iOS below) |
| `offers` | 441 | hashing/stamps/sealing are pure; the `sync_offers` table is SQLite | split: pure parts to **core**, table to **desktop** |
| `remote_files` | 510 | `validate_offer`, `clean_file_name` pure; `store_file_offer` uses the history DB | split |
| `fetching` | 360 | `fetch_into`, `place_without_overwrite` pure; `RemoteFile::of(ClipboardItem)` reads a history item | split |
| `received` | 299 | the history DB and image thumbnails | **desktop** |
| `runtime`, `sharing` | 797 | Tauri | **desktop** |

**`SyncHooks` is already the boundary.** The service never touches the clipboard, history or UI itself; it calls `apply_clip`, `apply_image`, `apply_offer`, `lookup_offer`, `resolve_addr` and `on_pairing`. The desktop implements them with Tauri and SQLite; the phone apps will implement them in Swift or Kotlin.

## Phase 0: decisions and prerequisites (before any code)
1. **An Apple Developer account** ($99/year). It's needed to run on a real iPhone for more than 7 days, for TestFlight, and for the App Store. Development needs the Mac with Xcode.
2. **What the phone does in v1** (proposed):
   - **Receive** text, images and file offers **while the app is open**, and fetch files.
   - **Send** text and images through **"Paste & Send"** in the app (`UIPasteControl`, which avoids the "Allow Paste?" prompt) and through the **Share Extension** ("Share to Quakboard" from any app).
   - **Not in v1:** receiving while the app is closed. iOS suspends it, so that needs push notifications and a server (see "Later").
3. **The minimum iOS version:** iOS 16, for `UIPasteControl` and current SwiftUI.
4. **The Android minimum,** decided at Phase 6 (proposed: Android 10 / API 29).

## Phase 1: extract the shared crate (desktop behaves exactly the same)
The goal is a Cargo workspace with `quakboard-sync`, which knows nothing about Tauri, SQLite history or the desktop clipboard.

1. **Workspace:** `src-tauri/` becomes a workspace member, and `crates/quakboard-sync/` is created. Move the pure modules unchanged, along with their tests.
   - Check: `cargo test -p quakboard-sync` passes and the desktop builds.
2. **Split the mixed modules:**
   - `offers.rs`: `hash_file`, `FileStamp`, `Offer`, and `FileOffer` sealing go to the core. The `sync_offers` SQL (table, trigger, `create_offer`, `find_offer`) stays in the desktop, behind `lookup_offer`, which is already a hook.
   - `remote_files.rs`: `validate_offer` and `clean_file_name` go to the core. `store_file_offer` stays in the desktop.
   - `fetching.rs`: the core keeps `fetch_into`, `place_without_overwrite` and the `RemoteFile` struct. `RemoteFile::of(&ClipboardItem)` becomes a desktop function.
   - `received.rs` stays in the desktop.
   - Check: all of today's ~300 tests pass, split between the two crates.
3. **Make discovery pluggable:** `mdns-sd` goes behind a cargo feature, `mdns`, on by default for desktop and off for mobile. The service only needs `resolve_addr` (already a hook) plus a way to list nearby devices, so the phone apps supply both from native Bonjour or NSD.
4. **Add a protocol version to pairing and frames.** This must ship **before** the first mobile release:
   - Pairing exchanges a `protocol_version`, and an older device gets "update Quakboard" instead of confusing failures.
   - Frames gain an optional field that's ignored when absent, so v1.5.x desktops keep working.
   - Check: tests that an old-version peer gets a clear refusal, and that today's frames still parse.
5. **CI:** run `cargo test -p quakboard-sync` and `cargo clippy -p quakboard-sync` in the existing workflow.

**Done when** the desktop works exactly as before (same manual smoke test as image test 1 and file test 1) and the core builds with `--no-default-features`.

## Phase 2: the FFI layer (`quakboard-sync-ffi`)
A thin crate exposing a small, stable API through UniFFI. The core stays idiomatic Rust, and only this layer deals with FFI types.

**The object the apps hold:**
```rust
#[uniffi::export]
impl SyncNode {
    #[uniffi::constructor]
    fn new(data_dir: String, device_name: String, delegate: Box<dyn SyncDelegate>) -> Arc<Self>;
    async fn start(&self) -> Result<(), SyncError>;        // listen on 47823
    fn stop(&self);
    fn identity(&self) -> DeviceIdentity;                    // id, name, short id
    fn peers(&self) -> Vec<PeerInfo>;                        // never keys
    async fn pair_with(&self, addr: String) -> Result<PeerInfo, SyncError>;
    fn submit_pairing_code(&self, code: String);
    fn set_accepting_pairing(&self, on: bool);
    fn unpair(&self, peer_id: String) -> Result<(), SyncError>;
    async fn send_text(&self, text: String) -> u32;          // devices reached
    async fn send_image(&self, png: Vec<u8>, width: u32, height: u32) -> u32;
    async fn fetch_file(&self, offer: RemoteFileInfo, dest_dir: String,
                        progress: Box<dyn ProgressListener>) -> Result<String, SyncError>;
    fn set_nearby(&self, devices: Vec<NearbyDevice>);         // fed by Bonjour/NSD
}
```

**The callbacks the apps implement** (UniFFI callback interfaces, mirroring `SyncHooks`):
```rust
#[uniffi::export(callback_interface)]
pub trait SyncDelegate: Send + Sync {
    fn on_text(&self, text: String, from: String);
    fn on_image(&self, png: Vec<u8>, width: u32, height: u32, from: String);
    fn on_file_offer(&self, offer: RemoteFileInfo, from: String);
    fn on_pairing_event(&self, event: PairingEvent);
}
```

- **The async runtime:** the FFI crate owns a Tokio runtime. UniFFI's async support maps `async fn` to Swift `async` and Kotlin `suspend`.
- **Errors:** one `SyncError` enum with user-facing messages, the same ones the desktop already shows ("offline", "the file changed on the other device", …).
- **Tests:** Rust tests drive two `SyncNode`s over localhost (the same scenarios as `service.rs`), plus a generated-bindings smoke test on CI.
- **Files on phones:** sending a file needs an offer table on the phone too. Phase 5 adds it inside the FFI crate as a small JSON file, since phones don't have the desktop's SQLite history.

## Phase 3: iOS build pipeline
1. **Rust targets:** `aarch64-apple-ios` (devices), plus `aarch64-apple-ios-sim` and `x86_64-apple-ios` (simulators).
2. **The build script** (`scripts/build-ios.sh`) runs `cargo build --release` for each target, then `xcodebuild -create-xcframework`, then UniFFI to generate the Swift bindings, and packages everything as a local **Swift Package** (`ios/QuakboardSync`).
3. **CI:** a macOS job builds the XCFramework on every PR that touches the core.
4. **Check:** a tiny SwiftUI test app on the simulator pairs with the desktop over the LAN and shows the next copied text. **That's the moment the whole approach is proven.**

## Phase 4: iOS app MVP (text and images)
**Discovery (native, no special entitlement):**
- Advertise with `NWListener` and find others with `NWBrowser`, using **Apple's Network framework** (Bonjour) for `_quakboard._tcp`.
- Declare `NSBonjourServices = ["_quakboard._tcp"]` and an honest `NSLocalNetworkUsageDescription`, which iOS shows in its permission prompt.
- Feed results into `SyncNode.set_nearby(...)`.
- The Rust `mdns-sd` path isn't used on iOS: raw multicast there needs Apple's `com.apple.developer.networking.multicast` entitlement, which requires a request and approval.

**Storage:**
- Pairing keys (`sync.json`) go in an **App Group** container, so the Share Extension can use them too, with Data Protection set to `complete`.
- History lives in **SwiftData** (or GRDB/SQLite): text, images (as files), and remote file items.

**Screens (SwiftUI):**
- **History:** a list grouped by date like the desktop, with search, plus detail views for text, images and files.
- **Devices:** the same flows as desktop. A sync toggle, this device's name and IP, paired devices with Unpair, nearby devices with Pair, Connect by IP, and the code-entry and code-display sheets.
- **Paste & Send:** a `UIPasteControl` button that sends the clipboard to paired devices. No prompt, since the user tapped a system paste button.

**The Share Extension ("Share to Quakboard"):**
- A separate target that accepts text, URLs and images from any app's share sheet.
- It sends them through `SyncNode` and closes. iOS gives extensions limited memory and time, so it sends and exits rather than listening.

**Receiving:**
- While the app is open, received text and images go into history. Text is copied to `UIPasteboard`, the same as the desktop. Images go to history, with a "Copy" button.
- When the app is backgrounded, it stops listening cleanly. Desktops see it as offline, the same as a closed laptop today.

**App Store notes:** explain the local-network permission and the clipboard use in the review notes.

**Check:** the manual tests from IMAGE_SYNC_PLAN, run Mac/Linux ↔ iPhone in both directions, plus the Share Extension from Safari and Photos.

## Phase 5: iOS files
- **Receiving:** file offers become items "on Laptop". **Fetch** saves into the app's Documents folder (visible in the Files app, via `UIFileSharingEnabled` and `LSSupportsOpeningDocumentsInPlace`), with a progress bar and Cancel.
- **Sending:** the Share Extension or a document picker creates an offer, using the same device picker as the desktop. The phone serves the fetch only while the app is open, and the UI says so.
- **Big files:** use `beginBackgroundTask` to finish a running transfer if the user switches apps, which iOS allows for a short time.

## Phase 6: Android
- **Build:** `cargo-ndk` for `arm64-v8a`, `armeabi-v7a` and `x86_64`, with UniFFI Kotlin bindings, as a Gradle module `:quakboard-sync`.
- **Discovery:** **NsdManager**, Android's native DNS-SD. The Rust `mdns` feature would also work there, with a `WifiManager.MulticastLock`.
- **UI:** Jetpack Compose, mirroring the iOS screens.
- **Sending:** a **share target** (an `ACTION_SEND` intent filter), plus an in-app "Paste & Send". Android 10+ only lets the focused app read the clipboard.
- **Receiving in the background is possible on Android:** an optional **foreground service**, with a persistent notification, keeps the listener running. That's something iOS can't do. Make it a user setting.
- **Check:** the same manual matrix as iOS, plus Android ↔ iPhone.

## Phase 7: release
- **iOS:** TestFlight beta, then App Store review (privacy labels: no data collected, everything stays on the local network).
- **Android:** Play Store internal testing, then production. Optionally F-Droid, since it fits the "100% local" story.
- **Desktop:** release notes saying which desktop version pairs with the phones (the protocol version from Phase 1, step 4).

## Later (needs a server)
- **Receiving while the iPhone app is closed:** push notifications through APNs need a small relay server. That's where the earlier relay/cloud idea comes back. The same server would also cover syncing between different networks.
- **Sync groups** ("pair once, join all"), which matter more once you have phones plus several computers.

## Risks and how the plan handles them
| Risk | Mitigation |
|---|---|
| iOS multicast entitlement | Not needed: native Bonjour (`NWBrowser`/`NWListener`) does discovery on iOS |
| Phones are offline most of the time | v1 is explicit about it ("receives while open"); push and the relay come later |
| Old desktop vs new phone | The protocol version in Phase 1, step 4, ships before any mobile build |
| UniFFI async or callback edge cases | Phase 3 proves the whole chain with a tiny app before any real UI work |
| Memory and time limits in the Share Extension | It only sends small payloads (text, images ≤ 20 MB) and exits; files go through the main app |
| Two UIs to maintain | They're small (history, detail, devices); all the logic is in the shared core |

## Rough effort (one developer, part-time)
| Phase | Estimate |
|---|---|
| 1: Extract the crate + protocol version | 1–2 weeks |
| 2: FFI layer | 1–2 weeks |
| 3: iOS pipeline + proof app | ~1 week |
| 4: iOS MVP (text, images, share extension) | 3–5 weeks |
| 5: iOS files | 1–2 weeks |
| 6: Android | 4–6 weeks |
| 7: Release | 1–2 weeks + review time |

These are estimates for planning, not commitments. The first real signal comes at the end of Phase 3.
