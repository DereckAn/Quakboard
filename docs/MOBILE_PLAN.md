# Plan: iPhone/iPad and Android apps

**Status:** Phases 1 and 2 done (see [Phase 1](#what-was-built-in-phase-1) and [Phase 2](#what-was-built-in-phase-2)). Phase 3, on a Mac, is next.

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

## What the code looked like before Phase 1 (measured)
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
**Decided (2026-10-10):** the v1 scope below as proposed, and iOS 16 as the minimum. The Apple Developer account waits until the Phase 3 proof works. Until then, the simulator and Xcode's free signing (an app on your own iPhone that expires after 7 days) are enough.

1. **An Apple Developer account** ($99/year). **Deferred:** buy it once Phase 3 proves the approach. It's needed to run on a real iPhone for more than 7 days, for TestFlight, and for the App Store. Development needs the Mac with Xcode.
2. **What the phone does in v1** (**decided**):
   - **Receive** text, images and file offers **while the app is open**, and fetch files.
   - **Send** text and images through **"Paste & Send"** in the app (`UIPasteControl`, which avoids the "Allow Paste?" prompt) and through the **Share Extension** ("Share to Quakboard" from any app).
   - **Not in v1:** receiving while the app is closed. iOS suspends it, so that needs push notifications and a server (see "Later").
3. **The minimum iOS version:** iOS 16 (**decided**), for `UIPasteControl` and current SwiftUI.
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

### What was built in Phase 1
Where the result differs from the steps above, this section is what's true.

1. **A path dependency, not a workspace.** `crates/quakboard-sync` is listed in `src-tauri/Cargo.toml` as `path = "../crates/quakboard-sync"`. This keeps `src-tauri/target`, its lockfile and the CI artifact paths unchanged. The core has its own `Cargo.lock`. Moving to a workspace later is optional.
2. **What's in the core:** `lib.rs` (`Frame`, `Sealed`, `ClipPayload`, `PeerKey`, `SyncError`), `body`, `discovery`, `fetch`, `fetching`, `image`, `offers`, `pairing`, `protocol`, `remote_files`, `store`, `stream` and `transport`. The desktop's `sync/mod.rs` re-exports them, so desktop code still uses `crate::sync::…`.
   - **Moved whole:** `fetch.rs`, since everything it needs moved too. `fetching.rs` also moved, except `RemoteFile::of`, which became `remote_files::remote_file_of` on the desktop next to the code that writes that metadata. The desktop `fetching.rs` is gone.
   - **Still on the desktop:** `service.rs` and `runtime.rs`, plus `received.rs`, `sharing.rs`, and the SQL halves of `offers.rs` and `remote_files.rs`. **`service.rs` moves in Phase 2,** where it gets the FFI's hooks.
   - **One cross-crate test changed:** `image.rs` compared against the desktop's `calculate_file_hash`. Both sides now pin the SHA-256 of `"hello world"` instead.
3. **The `mdns` feature** (on by default) covers only the mDNS daemon. These stay available without it, so the phone apps advertise in a way the desktop recognizes:
   - `SERVICE_TYPE` (`_quakboard._tcp.local.`)
   - `ID_PROPERTY` (the `"id"` TXT key, now public)
   - `DiscoveredDevice`
   - `short_id`

   **Deferred to Phase 2:** the "list nearby devices" hook. Nothing can use it until `service.rs` is in the core.
4. **Protocol version: a stamp on every frame, not just pairing.**
   - **The stamp:** `write_frame` adds `"protocol": {"version", "min_peer"}` to every frame (`protocol.rs`, `Protocol::CURRENT` = 1/1). A frame without it is from v1.5.x and reads as version 1. Older devices ignore the extra key; a test parses a stamped frame as an old device would.
   - **Checking order:** `read_frame` checks compatibility **before** parsing, because a newer device's frame may not parse at all. It fails with an `Incompatible` (`PeerTooOld` or `ThisTooOld`) inside the error, so a fetch says "the other device needs a newer Quakboard; update this one".
   - **The listener's first frame:** `read_stamped` returns `Incoming::Frame(frame, protocol)` unjudged, so pairing can still answer. If the frame doesn't parse, it returns `Incoming::Incompatible { sender, reason }`, keeping the claimed `from`.
   - **Pairing:** the responder sends its challenge **before** refusing, so the initiator also learns why. The initiator refuses as soon as it reads the challenge, before anyone types a code. The UI's `failed` event has `needsUpdate`, and Devices shows it even when no dialog was open.
   - **Paired devices:** an incompatible frame is dropped and never interpreted. The peer row shows "Not syncing: update Quakboard on …" (`updateNeeded` from `sync_list_peers`).
   - **Security:** the stamp travels outside the encryption, so anyone on the LAN can forge it. The badge is therefore kept **in memory only, never saved**, and only for ids that are already paired. A forged stamp can drop that one frame or show a badge until the next real frame, but it can't stop sync with a device.
   - **One limit:** an unparseable first frame with no `from` can't be tied to a device, so it's only logged.
5. **CI:** the `tests` job runs, for the core:
   - `cargo test`, with default features and with `--no-default-features`
   - `cargo clippy --all-targets -- -D warnings` in both configurations

   These steps share `src-tauri/target`.

**Tests:** 167 in the core (158 with `--no-default-features`) and 160 on the desktop; there were 303 before Phase 1.

**Manual smoke test (Linux ↔ Mac, both on this branch):**

| # | Test | Result |
|---|---|---|
| 1 | Pair again | ✅ |
| 2 | Text both ways | ✅ |
| 3 | Screenshots, Linux → Mac and Mac → Linux | ✅ |
| 4 | Send and Fetch a file, including an image file | ✅ |
| 5 | A paired v1.5.2 desktop against this branch | not run; covered by `older_devices_can_still_read_a_stamped_frame` |

**Found and fixed during the smoke test (desktop only, not mobile work):**
- **WebKitGTK 2.54 blocks `asset://` images in `tauri dev` on Linux.** The page comes from `http://localhost:1420`, a different origin. A debug-only CORS allowlist in `lib.rs` works around it. It's marked `HACK(dereck)` (in `lib.rs` and `Cargo.toml`); remove it once [tauri#16201](https://github.com/tauri-apps/tauri/issues/16201) is fixed. Release builds aren't affected.
- **Image files copied in a file manager** now have Copy image, Open file and Send, styled like the other copy buttons. The "Also sync images" text says these files aren't sent automatically.
- **Fetched files** now get the same preview (thumbnail or text excerpt) as files copied locally.

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
- **Left over from Phase 1:** move `service.rs` into the core behind its hooks, and add the nearby-devices hook (for Bonjour/NSD) that step 3 deferred. `PeerInfo` should carry the in-memory `update_needed` status, so phones show the same "update Quakboard" badge.
- **Files on phones:** sending a file needs an offer table on the phone too. Phase 5 adds it inside the FFI crate as a small JSON file, since phones don't have the desktop's SQLite history.

### What was built in Phase 2
Built on `feat/sync-ffi`. Where it differs from the sketch above, this section is what's true.

1. **`service.rs` moved into the core unchanged.** Only its test paths changed. It only reaches the outside world through `SyncHooks`, which is the boundary the FFI layer implements.
2. **No new discovery hook was needed.** The service only calls `resolve_addr` (when a device's last address fails). The nearby list and "online" status are app state. On the desktop they're in `runtime.rs` (from `Discovery`); on phones they're in `SyncNode`, fed by `set_nearby`.
3. **`crates/quakboard-sync-ffi`** (UniFFI 0.32.2, proc macros, no `.udl`). It uses the core with `default-features = false`, which is the phone configuration with no `mdns-sd`.
   - **The runtime:** one Tokio runtime per process, in a `OnceLock`, so a node can be dropped from any thread. Every async call runs there through `run()`. If the caller stops waiting (a cancelled Swift task), the work is aborted; a fetch then deletes its partial file.
   - **`SyncNode`:**
     - `new(data_dir, device_name, listen_port, delegate)`; `device_name` is used only on first run
     - `start` / `stop`, `identity`, `peers` (never keys; includes `is_online` and `update_needed`) and `set_nearby`
     - `send_text`, `send_image` (PNG bytes; size read from the header) and `set_syncing_images`
     - `fetch_file(file, dest_dir, progress)`
     - `pair_with_address`, `pair_with_device`, `submit_pairing_code`, `cancel_pairing`, `set_accepting_pairing` and `unpair`
   - **`SyncDelegate`:** `on_text`, `on_image(png, width, height, from_name)`, `on_file_offer(RemoteFileInfo)` and `on_pairing_event`. Called on a background thread. **`FetchProgress`:** `on_progress(done, total)`.
   - **Free functions:** `default_sync_port()`, plus `discovery_service_type()` (`_quakboard._tcp`, the form `NWBrowser` and NSD take) and `discovery_id_key()` (`"id"`), so phones advertise exactly like desktops.
   - **Errors:** `SyncNodeError` with `Storage`, `Network`, `InvalidInput` and `Fetch`. Each message is ready to show.
4. **Core additions:**
   - `send_clip`, which sends without the echo guard. Phones have no clipboard monitor, so every send is deliberate. `broadcast` keeps the guard and calls it.
   - `image::png_dimensions`.
   - `parse_peer_address`, moved from the desktop's `runtime.rs` to `transport`.
5. **Start/stop lifecycle:** `Stopped` → `Starting(token)` → `Running`.
   - A `stop` during the bind wins, and a second `start` doesn't bind again.
   - A `start` cancelled mid-bind gives its claim back (`StartClaim`) without undoing a newer start.
   - A restart right after `stop` retries the bind for up to 0.5 s, while the old socket closes.
6. **CI:** the FFI crate's tests and clippy (`-D warnings`), plus generating the **Swift and Kotlin** bindings with its `uniffi-bindgen` binary (`--features bindgen`). The generator reads `cargo metadata`, so it runs from the crate's folder.

**Tests:** 30 in the FFI crate. They use the API as Swift will: pairing (right code, wrong code, hostname), text, images, an offer → fetch → verify against a desktop-like owner, progress, the nearby list, and the lifecycle cases. The core has 238 and the desktop 91.

**Left for later:**
- **The phone's offer table** (sending files from a phone) waits for Phase 5.
- **`on_text` doesn't say who sent the text.** The core's `apply_clip` hook doesn't carry the sender; add it if the phone UI wants "from Laptop" on text.
- **Kotlin bindings** are generated in the default `uniffi.quakboard_sync_ffi` package. Set a real package name in a `uniffi.toml` at Phase 6.
- **`staticlib`** for iOS is added in Phase 3.

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
