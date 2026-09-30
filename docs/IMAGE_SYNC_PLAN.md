# Plan: image sync (v1.5.1)

**Status:** planned. Builds on LAN sync v1 ([LAN_SYNC_PLAN.md](LAN_SYNC_PLAN.md)): same pairing, keys, transport and discovery.

## Goal
When you copy an image (a screenshot, "Copy image" in a browser, an image copied from an editor), it appears on every paired device's clipboard, ready to paste, and in their history, exactly like text does today. It must be **robust**:
- no echo loops
- no duplicates
- no hangs on slow Wi‑Fi
- no memory spikes
- no crashes on malformed input from the network

## Scope
- **In:** images that arrive as **pixels on the clipboard**, the monitor's `ClipboardContent::Image` branch.
- **Out:** image *files* copied in a file manager (the `ImageFile` branch). Those follow the file flow in v1.5.2.
- **Also out:** GIF/animated images (only the first frame reaches the clipboard anyway), and a queue for offline devices.

## What exists and what to reuse
| Existing code | Reuse for |
|---|---|
| `image_handler::save_image_to_disk(image_data, images_dir, is_screenshot)` | Saves `<uuid>.png` plus a thumbnail and returns `StoredImageInfo { full_path, width, height, file_size, file_hash (full SHA-256), .. }`. The sender sends this PNG; the receiver stores through the same function |
| `repo.find_by_file_hash` + `bump_item` | Duplicate detection on the receiver: an image it already has is bumped, not duplicated |
| `operations::write_clipboard_image(path)` | Puts a PNG on the system clipboard. It already calls `state::request_skip_events(2)` so the monitor ignores the write |
| Monitor's `last_image_hash` (a hash of the raw RGBA pixels) | Already skips the same pixels arriving twice |
| `SyncService::broadcast` / `deliver` / address fallback | Sending to all paired devices, unchanged |

## Design

### Wire format: a binary body, not base64 JSON
Text clips are small JSON frames capped at 1 MB. Images are often 1–10 MB, and base64 would add 33% to that.
- **New frame:** `Frame::Image { from, meta: Sealed }`, followed on the same connection by a **binary body**.
  - `meta` (sealed) holds `{ width, height, byte_len, sha256, is_screenshot }`.
  - The body is the PNG, encrypted as one ChaCha20-Poly1305 message with a fresh nonce, with the sender id plus `sha256` as associated data. That binds the body to its header.
- **Transport:** add `write_body` / `read_body`, a 4-byte length followed by the raw bytes. `read_body` enforces `MAX_IMAGE_BYTES` (default **20 MB**) **before** allocating.
- **Timeouts:** the fixed 5 s read timeout becomes an **idle timeout** (5 s without receiving any bytes), plus a total cap (60 s). A 20 MB image on slow Wi‑Fi doesn't fail, and a stalled sender still can't hold a connection forever.
- ponytail: the body is in memory as one message, which is fine up to 20 MB. Use chunked encryption (the files plan needs it anyway) if the limit ever grows.

### Sending
- Hook the monitor's `Image` branch **after** `save_image_to_disk` and the database save succeed, the same place as text.
- `SyncService::broadcast_image(stored: &StoredImageInfo)`:
  1. Skip it if image sync is off or the image is over the limit.
  2. Check the echo guard (below).
  3. Read the PNG from `full_path` in a background thread, since file I/O shouldn't block the async runtime.
  4. Seal it once per peer, then send through the existing `deliver` (last address, then lookup).
- A copy while a send is still in progress just starts another send. Each connection is independent.

### Receiving (each step can reject; rejection is logged, never crashes)
1. The header must come from a paired device and its `meta` must decrypt, same as text.
2. `byte_len` must be ≤ `MAX_IMAGE_BYTES` and match the body length. The body must decrypt with the header-bound associated data.
3. The body's SHA-256 must equal `meta.sha256`.
4. **Decode safely.**
   - Use the `image` crate with limits: max width/height (for example 16384) and a max decoded size (for example 256 MB of RGBA), so a tiny "decompression bomb" PNG can't exhaust memory.
   - Reject anything that isn't a PNG.
5. **Store through the normal path.**
   - If `find_by_file_hash(sha256)` finds it, bump the existing item.
   - Otherwise save through `save_image_to_disk` (thumbnail included) and create the item with `source_app = "Synced from <peer name>"`.
   - Emit `clipboard-item-added`.
6. **Put it on the clipboard** with `write_clipboard_image`, which asks the monitor to skip the next events.

### Echo guard (the tricky part)
For text, the guard compares strings. Images can be re-encoded on their way through the clipboard, so the file bytes seen on the next copy may differ. Two layers:
1. **Primary:** the receiver writes with `request_skip_events`, so the monitor ignores the clipboard change and nothing is re-saved or re-sent. The receiver stored the item itself in step 5.
2. **Backup:** a pixel hash, SHA-256 of `width ‖ height ‖ RGBA bytes`.
   - The receiver computes it from the decoded image and keeps it as `last_received_image`.
   - `broadcast_image` computes the same hash from the monitor's `ImageData` and skips (and clears the guard) on a match.
   - This catches the cases where skip counting is off, for example Wayland firing an unexpected number of events.
   - Pixels survive a lossless round trip even when the PNG bytes don't.

**Verify on each OS:** macOS may convert the colour profile when writing the clipboard. If pixels change there, fall back to a short time window: ignore image changes for 2 s after writing a received one.

### Settings and UI
- **Setting:** `syncImages` (default **on** once sync itself is on) with a toggle "Also sync images" in the Devices section, under the main toggle.
  - It's a separate switch because screenshots are the most likely thing to contain something private.
- **Size limit:** a fixed 20 MB in v1.5.1, shown in the toggle's help text. A setting can come later.
- **History:** synced items show "Synced from Laptop" as their source, using the existing `source_app` field.

## Steps (each ends with something checkable)
1. **Transport:** binary bodies, the size limit and the idle timeout. Tests: round trip, oversized length rejected, stalled sender times out, body exactly at the limit accepted.
2. **Frame and sealing:** `Frame::Image`, sealing and opening the header and body with bound associated data. Tests: tampered body, body swapped from another image, wrong sender, hash mismatch.
3. **Safe decode and pixel hash:** tests for a decompression bomb, a non-PNG, a truncated PNG, and the same pixel hash from a PNG and from raw RGBA.
4. **Receiver pipeline:** store with duplicates bumped, the event, then the clipboard write through the injected `SyncHooks` (a new `apply_image` hook). Tests: a new image is stored, a duplicate is bumped rather than stored twice, the source says "Synced from".
5. **Sender hook and echo guard:** `broadcast_image` from the monitor's `Image` branch. Tests: two devices over TCP, the image arrives, the receiver doesn't send it back, the same image copied later still syncs.
6. **Setting and UI:** the `syncImages` toggle and its help text. Test: with it off, nothing is sent and received images are ignored.
7. **Manual cross-OS test (below):** fix whatever turns up, especially the echo guard.

## Verification
- `cargo test --lib sync::`, `bun run check`, `bunx vitest run`.
- **Two machines**, both directions:
  - a small screenshot (under 1 MB), a large one (about 15 MB), and a 25 MB image, which must be refused and logged
  - a browser "Copy image"
  - the same image copied twice, which gives one history item
- **Echo:** after receiving, the history on both sides has exactly one new item, and there's no ping-pong in the logs.
- **Slow network:** limit bandwidth (for example `sudo tc qdisc add dev <if> root tbf rate 2mbit burst 32kbit latency 400ms`). A 15 MB image still arrives and doesn't time out.
- **Platform matrix for the echo guard:** Linux Wayland (Hyprland), Linux X11, macOS and Windows.
