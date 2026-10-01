# Image sync

**Status:** implemented and tested on two real machines (Linux on Wayland/Hyprland, and macOS). Builds on LAN sync v1 ([LAN_SYNC_PLAN.md](LAN_SYNC_PLAN.md)): same pairing, keys, transport and discovery.
- **Automated:** `cargo test --lib sync::` (147 tests), plus the clipboard tests below.
- **Manual:** tests 1–6 and 8 passed; test 7 (toggle off) hasn't been run.

## Goal
When you copy an image (a screenshot, or "Copy image" in a browser), it appears on every paired device's clipboard, ready to paste, and in its history, exactly like text. Built to be robust:
- no echo loops
- no duplicates
- no hangs on slow Wi‑Fi
- no memory spikes
- no crashes on malformed input from the network

## Scope
- **In:** images that arrive as **pixels on the clipboard**, the monitor's `ClipboardContent::Image` branch. That covers screenshots to the clipboard (hyprshot, ⌘⌃⇧3/4) and "Copy image" in browsers.
- **Out:** image **files** copied in a file manager or Finder (the `ImageFile` branch). Those are shared **manually** through file sharing ([FILE_SYNC_PLAN.md](FILE_SYNC_PLAN.md)).
  - An optional "sync copied image files automatically" switch is planned for later (see [FILE_SYNC_PLAN.md](FILE_SYNC_PLAN.md#later-image-files-after-file-sharing-ships)).
- **Also out:** animated images (only the first frame reaches the clipboard), and a queue for offline devices.

## How it works

### Wire format (`sync/body.rs`, `sync/image.rs`)
- **`Frame::Image { from, meta }`** is followed on the same connection by a **raw binary body**, with no base64 or JSON:
  - `meta` is sealed and holds `{ width, height, byte_len, sha256, is_screenshot }`
  - the body is the PNG, encrypted as one ChaCha20-Poly1305 message with a fresh nonce
- **Binding:** the body's associated data is `image-body:<sender id>:<sha256>`. A body can't be moved under another image's header or another device's id.
- **Checks after decrypting:** the receiver re-checks length and SHA-256, so a paired device can't describe different bytes than it sends (`SyncError::Integrity`).
- **`seal_image`** computes the hash and length itself, so the header always matches.

**Body transfer limits** (`BodyLimits`, `IMAGE_BODY_LIMITS`):

| Limit | Value | Stops |
|---|---|---|
| Size cap | 20 MB of PNG + 16-byte tag, checked **before** reading | A huge body |
| Memory growth | only as bytes arrive, never the announced size up front | A peer announcing 20 MB and then stalling |
| Idle timeout | 5 s without any bytes | A stalled sender |
| Total cap | 60 s | A trickle staying just under the idle timeout |

### Safe decode (`decode_png`)
- **Only PNG is accepted:** the PNG decoder is forced, so a valid JPEG is refused.
- **Size limits:** at most 16384 px on either side, and at most 256 MB as RGBA. A tiny PNG claiming 9000×9000 (a "decompression bomb") is refused before allocating.
- **Converting to RGBA is also checked:** the decoder's limit doesn't cover it, and converting a grayscale image quadruples its size. Without this check, a 9000×9000 grayscale PNG took about 324 MB and 9 s.
- **The decoded size must match the header.**
- **`pixel_hash`** is SHA-256 of `width ‖ height ‖ RGBA`. The same picture re-encoded through a clipboard keeps its pixel hash; that's what the echo guard compares.

### Sending
- **Hooked into the monitor's `Image` branch** after the image is saved, or bumped if it's already in history. Re-copying an image syncs it too, like text.
- **The PNG is read from its stored file,** in the background.
- **`broadcast_image`** seals once per paired device and delivers through the same path as text: last address first, then an mDNS lookup.
- **With the folder watcher on** (it stores screenshots as file pointers): the monitor still doesn't save a screenshot's bytes, but **still sends them** (`sync_pixels`, which encodes the pixels to PNG in memory).
- **Images over 20 MB are skipped,** logged as `Not syncing a N MB image`.

### Receiving (`sync/service.rs`, `sync/received.rs`, `sync/runtime.rs`)
1. The sender must be paired **before the body is read**, and image sync must be on.
2. Read the body under the limits, then open and verify it, then decode safely in a background thread.
3. **Store the original PNG bytes** (never re-encoded), so the stored hash equals the sender's:
   - a duplicate hash bumps the existing item
   - a duplicate whose file was deleted gets its file back
   - the PNG is written atomically, and nothing is left behind on failure
   - source: "Synced from `<device>`"
   - metadata `"source": "clipboard"`, so the viewer treats it like a local copy
4. Emit `clipboard-item-added`, then `write_clipboard_image`.

### Echo guard (two layers)
1. **Skip events:** `write_clipboard_image` asks the monitor to ignore its own write. That's **2 events on Wayland and 1 elsewhere**, and requests **expire after 2 s** (`SkipWindow` in `clipboard/state.rs`).
2. **Pixel hash:** the receiver remembers the last received image's pixel hash. The monitor calls `take_image_echo` **before storing**, so an echo is neither stored nor sent back.
   - It's used up on a match, and only counts for **5 s**. When layer 1 works, the monitor never sees the write, so a later deliberate copy of the same picture must still sync.
   - It runs after the monitor's own duplicate check, so a second duplicate Wayland event for the same write is still filtered.

### Screenshot vs. app copy on Wayland (`looks_like_screenshot_types`)
- **Screenshot tools offer only `image/png`:** hyprshot runs `wl-copy --type image/png`.
- **Browsers also offer** `text/html`, `text/x-moz-url` and `chromium/x-source-url`.
- **The Wayland screenshot hint** is "no HTML or URL types". With the watcher on, only screenshots skip the local save, so **browser copies are saved locally and synced.**
- **macOS, Windows and X11 are unchanged:** there's no such signal there, so all clipboard images still skip the local save while the watcher is on.

### Setting and UI
- **`syncImages`** in `settings.json`, **on by default**. A missing or malformed value counts as on.
- **The "Also sync images" toggle** is in Settings → Account → Devices, with help text showing the real limit (`maxImageBytes`, taken from the backend).
- **Off means off in both directions:** nothing is sent, and incoming images are refused before their body is read. Text keeps syncing.

### Delivery logs
- Sending logs `Sync sent text|image to N of M paired device(s)`.
- Receiving logs `Sync received text from <name>` and `Sync received image WxH (N KB) from <name>`.
- Counts and kinds only, **never content**, since clips can be passwords.

## Files
| File | Purpose |
|---|---|
| `src-tauri/src/sync/body.rs` | Binary bodies: `write_body`, `read_body`, `BodyLimits`, `MAX_IMAGE_BYTES` |
| `src-tauri/src/sync/image.rs` | `seal_image` / `open_image`, `decode_png`, `encode_png`, `pixel_hash` |
| `src-tauri/src/sync/received.rs` | `store_received_image`: history item, duplicates, atomic write |
| `src-tauri/src/sync/service.rs` | `broadcast_image`, receiving `Frame::Image`, `take_image_echo`, the `syncs_images` flag, delivery logs, shared `send_to_peers` / `deliver` |
| `src-tauri/src/sync/runtime.rs` | The `apply_image` hook (store, emit, clipboard), the `syncImages` setting |
| `src-tauri/src/commands/sync.rs` | `sync_set_images_enabled`; the status gains `syncImages` and `maxImageBytes` |
| `src-tauri/src/clipboard/monitor.rs` | Image branch: duplicate check, then echo check, then the watcher decision, then save and send |
| `src-tauri/src/clipboard/operations.rs` | Wayland screenshot hint from offered types; skip count per backend |
| `src-tauri/src/clipboard/state.rs` | Expiring skip requests (`SkipWindow`) |
| `src/lib/components/settings/DevicesSection.svelte` | The "Also sync images" toggle |

## Bugs found by testing on real machines (all fixed)
1. **With the folder watcher on, clipboard images were ignored entirely,** so they were never saved or sent. They're now sent even when the local save is skipped.
2. **On Wayland the screenshot hint was always `false`,** so the watcher skip hit browser copies too. It's now decided from the offered types.
3. **Skip requests accumulated and never expired.** Receiving images on macOS left one skip behind each time, which silently ignored the user's next copies. The same thing happened after "Copy image to clipboard" on macOS, Windows and X11. Skips are now counted per backend and expire after 2 s.
4. **Copying an already-known image file deleted the user's original,** permanently, in releases **v1.2.0–v1.5.0**. It was an older bug, found during this testing: the duplicate branch deleted `info.full_path`, which in pointer mode is the user's file. The delete is removed, and a regression test was added (`copied_image_file_is_stored_as_a_pointer_to_the_original`). See the "never touch the user's files" rule in [FILE_SYNC_PLAN.md](FILE_SYNC_PLAN.md).
5. **A successful send logged nothing,** so the "sent but not received" case was invisible. Delivery logs were added.
6. **Re-copying the same image did nothing,** even minutes later, so it couldn't be re-sent to a device that was offline. The duplicate-event check now only applies within 1 s (`is_repeat_image_event`).
7. **Some logs showed device ids instead of names.** Logs now name paired devices; unpaired ones are identified by IP.

## Changes from the original plan
- **Binary bodies live in their own module** (`body.rs`), with limits passed in as a value, so files can reuse them with different limits and tests can use millisecond timeouts.
- **The image echo guard expires (5 s)**, which the plan didn't foresee. Without it, the normal case (the skip works) would block a later deliberate copy of the same picture.
- **The echo expiry is tested** by passing the time in (`take_image_echo_at`), rather than with sleeps or a test-only setter.
- **Received images are stored as their original bytes.** The pixels are decoded only to check them and hash them, then dropped.
- **Sending with the watcher on**, the Wayland screenshot detection, and the expiring skips were all added after testing on real machines (see above).

## Known gaps
- **macOS with its folder watcher on:** a browser "Copy image" syncs to other devices, but **isn't saved in the Mac's own history.** The clipboard library doesn't expose pasteboard types there; reading `public.html` / `public.url` directly would allow the same fix as on Wayland.
- **Offline devices miss images** (there's no queue), the same as text.
- **Big images need a reasonable connection:** about 15 MB needs at least roughly 2.5 Mbit/s to finish inside the 60 s cap (measured in test 6).
- **Manual test 7 hasn't been run.**

## Verification
- `cd src-tauri && cargo test --lib sync:: clipboard::`, `bun run check`, `bunx vitest run`.
- **Manual, two machines** (this is how the bugs above were found):

| # | Test | Result |
|---|---|---|
| 1 | Linux screenshot (hyprshot, watcher on) → Mac | ✅ |
| 2 | Mac screenshot to clipboard → Linux, Wayland echo | ✅ One item each side |
| 3 | Same image again | ✅ No duplicates |
| 4 | 15.7 MB image → Mac | ✅ |
| 5 | 27 MB image refused (`Not syncing a 25 MB image`) | ✅ |
| 6 | Slow network, limit set with `sudo tc qdisc replace dev <if> root tbf rate <R> burst 32kbit latency 400ms` and removed with `sudo tc qdisc del dev <if> root`. At **5 Mbit/s** the 15.7 MB image arrives. At **2 Mbit/s** (about 63 s needed) both sides give up cleanly at the 60 s cap: `sync transfer took too long`, `sent image to 0 of 1`, nothing stored, no pointless retry, and text keeps syncing | ✅ |
| 7 | "Also sync images" off: images stop both ways, text still syncs | ⏭ Not run |
| 8 | Browser "Copy image" (Linux, watcher on): arrives on the Mac **and** in local history | ✅ After fix 2 |
| — | Browser "Copy image" on the Mac → Linux | ✅ After fix 3 |

Generating big test images: noise PNGs barely compress, so their size is predictable. Put one on the clipboard as pixels with `wl-copy --type image/png < file.png`.
