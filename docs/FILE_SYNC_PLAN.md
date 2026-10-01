# Plan: file sharing (v1.5.2)

**Status:** steps 1–7 implemented (backend 302 tests, plus `RemoteFilePanel` component tests). Step 8 (manual test on real machines) is next. Builds on LAN sync v1 ([LAN_SYNC_PLAN.md](LAN_SYNC_PLAN.md)) and the binary bodies from image sync ([IMAGE_SYNC_PLAN.md](IMAGE_SYNC_PLAN.md)).

## Goal
Files are **not** synced automatically; you choose which ones to share.
1. On the computer that has the file, the content viewer shows a **Send** button next to "Copy file to clipboard". Clicking it opens a **device picker**, and the file is **offered** to the devices you choose.
2. On the other devices, the file appears in the item list right away, **as information only**: name, size, type, and which device has it. No bytes are transferred yet.
3. Opening that item shows a **Fetch** button. Clicking it downloads the file from the device that offered it.
4. When a device already has the file, identified by content hash, **Fetch is hidden** and the normal Open / Copy buttons show instead.

The bytes only move when someone asks for them, so offering a 2 GB video to 5 computers costs nothing until one of them fetches it.

## Changes to the original idea (why)
- **Fetch by offer id, never by path.** The offering device keeps a list of the files it offered and serves **only those**. A path in a request would let any paired device read any file on your disk.
- **A full SHA-256.** File items today store a *sampled fingerprint* (`fingerprint_external_file`: the length plus the first and last chunk), which is good for spotting duplicates but can't check a download. Send computes a full streamed SHA-256. The receiver checks it after the download, and it's what "already on both devices" compares.
- **The file changed after it was sent:** the owner re-checks size and modification time before serving, and re-hashes if they changed. If the content differs, the fetch is refused with "File changed on Laptop; send it again". It never silently serves different bytes.
- **The owner is offline:** the item says "Unavailable: Laptop is offline", with a retry, instead of a Fetch button that just fails.

## Rule: never touch the user's files
The clipboard history stores **pointers** to the user's own files, not copies (`copy_image_file_to_storage`, `prepare_file_metadata`). Until the fix in 1.5.x, a duplicate check deleted that pointer's path, which permanently deleted users' original pictures (shipped v1.2.0–v1.5.0).

For everything in this plan:
- **Delete** only files the app created itself: fetched files it wrote, `.partial` downloads, thumbnails. **Never** a path that came from the user's clipboard or an offer.
- **Never overwrite:** a fetched file gets a new name (` (1)`, …) rather than replacing an existing one.
- **Tests:** every code path that removes a file gets a test proving a user's original survives it, like `copied_image_file_is_stored_as_a_pointer_to_the_original` and `delete_image_assets_keeps_original_when_source_is_file`.

## Design

### Offers (the owner's side)
- **Stored** in a new `sync_offers` table: `{ offer_id (uuid), item_id, path, size, mtime, sha256, offered_to (device ids), created_at }`. Offers survive restarts, so a fetch the next day still works.
- **Send** (`sync_send_file(item_id, device_ids)`, called by the picker):
  1. Resolve the item's `external_path`.
  2. Compute the SHA-256 in a background thread, reporting progress for big files.
  3. Store the offer.
  4. Seal `FileOffer { offer_id, name, size, mime, sha256 }` to **each chosen device** and send it with the existing `deliver`.
- **Revoked** when the item is deleted, or with a "Stop sharing" action. Offers never expire otherwise.
- **Offline devices miss the offer** (there's no queue), the same as clips. The owner can press Send again later. ponytail: re-send pending offers when a device comes back if that turns out to matter.

### Remote items (the receiving side)
- **Receiving `FileOffer`:** check its size and name, then create a clipboard item with `content_type = "file"` and `content_metadata = { remote: { offer_id, origin_device_id, origin_name, sha256, size }, original_name, mime }`. There's no local path yet. Emit `clipboard-item-added`.
- **Duplicates:** if an item with the same `sha256` already exists locally, it becomes that item's "also on Laptop" note instead of a second item. Fetch is then hidden immediately.
- **The item list** shows the name with a "remote" indicator and the origin device. The content viewer shows name, size, type and origin, plus **Fetch** or **Unavailable**.

### Fetch (the transfer)
1. **Request:** the receiver connects to the origin's last address (with the mDNS fallback) and sends `FileFetch { offer_id }`, sealed.
2. **Checks on the owner's side:** the requester must be a paired device, the offer must exist, the file must still be at its path, and it must still match (see above).
3. **Transfer:** a header `FileStart { size, sha256 }`, then the bytes in **chunks** (for example 256 KB), each encrypted on its own. The chunk nonce includes the chunk number, so chunks can't be reordered, dropped or replayed. A final chunk marks the end, so a file cut short is detected.
   - It **streams**, never holding the whole file in memory.
   - It uses the idle timeout from image sync, not a total limit.
4. **Receiver:**
   1. Write to `<downloads>/Quakboard/.partial/<offer_id>`, hashing as it goes.
   2. When done, check the size and SHA-256.
   3. **Only then** move it to its final name (an atomic rename).
   4. Update the item: `external_path`, clear `remote`, keep `origin_name`.
   - On any failure, delete the partial file and show the error.
5. **Progress:** a `sync-file-progress { itemId, received, total }` event, throttled to about 10 per second. A **Cancel** button drops the connection and deletes the partial file.

### Where fetched files go, and name safety
- The folder is `<Downloads>/Quakboard/`, from Tauri's `download_dir`, created on first fetch. A setting to change it can come later.
- **The received name is untrusted.** Keep only its last path part, strip `/`, `\`, `..`, control characters and Windows-reserved names (`CON`, `NUL`, …), cap its length, and fall back to `file` if nothing is left.
- **Name clashes** get ` (1)`, ` (2)` and so on. The app never overwrites an existing file.
- **The path is always built as** `downloads/Quakboard/<cleaned name>`. As a last check, confirm the result is still inside that folder.

### Limits
- There's no hard size limit, since the user chooses to fetch. It warns above 1 GB ("Fetch 2.3 GB from Laptop?").
- **Free disk space** is checked before starting.
- **One transfer per file at a time,** with at most 3 concurrent fetches.

### UI
- **ContentViewer, file item, local:** `[Open file] [Copy file to clipboard] [Send]`, with the send icon from the design. After clicking, it shows "Sent to 3 devices" (or "No paired device reachable").
- **ContentViewer, file item, remote:**
  - `[Fetch]` with progress and Cancel
  - or "Unavailable: Laptop is offline" with Retry
  - or "File changed on Laptop"
- **ContentViewer, file already here:** normal local buttons. Fetch is hidden.
- **Sidebar:** remote items show their name and a small "on Laptop" label.
- **Send is disabled** when sync is off or there are no paired devices, with a tooltip explaining why.

### Known limitation to fix alongside
"Copy file to clipboard" (`write_file_to_clipboard`) is **macOS-only** today. A fetched file on Linux or Windows could be opened but not copied. v1.5.2 should add the Linux (`text/uri-list` through `wl-copy` / X11) and Windows (`CF_HDROP`) versions, or hide the button on those platforms until then.

## Steps (each ends with something checkable)
1. **Chunked, encrypted transfer** in `transport`: chunk nonces, the end marker and the idle timeout. Tests: round trip, reordered chunk, dropped chunk, cut-short file, tampered chunk.
2. **Offers and full hashing:** the `sync_offers` table, a streamed SHA-256 with progress, and the `FileOffer` frame. Tests: an offer is stored and survives a restart, deleting the item revokes it.
3. **Remote items:** receiving an offer creates or merges an item, with name and size checks. Tests: a new offer, a duplicate merged by hash, an oversized or bad name rejected.
4. **Serving fetches:** checks on the owner's side (paired, offer exists, file unchanged). Tests: an unknown offer, an unpaired requester, a file changed since sending, a file moved.
5. **Fetching:** the partial file, verify then rename, the item update, progress and cancel. Tests: a successful fetch over TCP, a hash mismatch deletes the partial, cancel cleans up, name clashes get ` (1)`, a `../` name stays inside the folder.
6. **UI:** Send, Fetch, progress, Unavailable, and the sidebar label, with frontend tests for the button states.
7. **Cross-platform file clipboard,** or hiding the button on Linux and Windows.
8. **Manual test:** a 1 KB file, a 500 MB file, and 2+ GB with the warning; fetching while the owner goes offline; changing the file after sending; three devices.

## Notes from building
- **Step 1:** `send_stream` reads its source to the end, so step 4 must pass the file limited to the offered size (`File::take`). Otherwise a file still being written streams forever. `hash_file` already does this: a growing file once made a test hash for 10 minutes and write 6.8 GB.
- **Step 3: "already on this device" only works by full SHA-256.** It matches the same file offered again, files fetched before (step 5 must store the full SHA-256 as `file_hash`), and clipboard images. A file copied here from the file manager stores a *sampled* fingerprint (`fingerprint_external_file`), so it isn't recognized and shows a Fetch button anyway. A fix would hash local file items of the same size when an offer arrives; that can be expensive, so it's deferred.
- **Step 3:** remote items have **no `file_url`** until fetched. That keeps them out of `cleanup_missing_file_records` (it only looks at items that have one), and a test pins this down.
- **Existing behavior:** the "Remove duplicates" action groups items by `content_text`, so two different remote files with the same name are merged into one history entry. Only history entries are affected, never files.

## What was built (steps 1–7)
| Module | Role |
|---|---|
| `sync/stream.rs` | Chunked encrypted transfer (STREAM construction). It uses a key per transfer from a random salt, the chunk number as nonce, and an authenticated "last" flag plus context; it reports progress |
| `sync/offers.rs` | The `sync_offers` table (revoked by a trigger whenever the item is deleted, on every path), `hash_file` (full SHA-256 that stops at the starting size and fails if the file changes), and `FileOffer` sealing |
| `sync/remote_files.rs` | `validate_offer`, `clean_file_name`, and `store_file_offer` (new / refreshed / already here) |
| `sync/fetch.rs` | `FileFetch` / `FileReply` and the owner's `decide`: paired, offered to this device, still there, unchanged (re-hashed when only touched) |
| `sync/fetching.rs` | The requester side: download into `.partial`, which deletes itself on failure, cancel or abort; verify size and SHA-256; then `place_without_overwrite` (hard link, so taking a name is atomic; ` (1)`, ` (2)`…; never outside the folder) |
| `sync/sharing.rs` | App glue: Send (hash with progress, then offer the chosen devices), Fetch (at most 3 at once, cancellable), finishing a fetch (the item becomes a local file and the file goes on the clipboard), and the download folder |
| `clipboard/operations.rs` | `write_file_list` on every platform: `wl-copy text/uri-list` on Wayland, NSPasteboard on macOS, arboard `file_list` on X11 and Windows. It asks the monitor to skip its own write. "Copy file to clipboard" now works on Linux and Windows too |
| UI | Send button and device picker (online/offline, hashing progress, "Sent to N of M"); `RemoteFilePanel` (Fetch, progress bar, Cancel, reasons, Try again); "on Laptop" in the sidebar; "Fetched files go to … Change…" in Devices |

**Commands:** `sync_send_file`, `sync_fetch_file`, `sync_cancel_fetch`, `sync_get_download_dir`, `sync_set_download_dir`. `sync_list_peers` gains `isOnline`.

**Events:** `sync-file-progress { itemId, phase: hashing|fetching, done, total }` and `sync-file-done { itemId, ok, cancelled, error }`.

**Not built (yet):**
- **"Stop sharing":** an offer lives until its item is deleted (decision 1); the unused function was removed.
- **Free disk space check** before fetching: a full disk fails the write, and the partial file is deleted.
- **The 1 GB confirmation** before fetching.
- **"Already on this device" for files copied here from the file manager** (see Notes).

## Step 8: manual test (two machines)
1. **Send** a small file to the Mac: the picker shows the Mac as Online, and you get "Sent to 1 device". The Mac shows the item "· on cachyos-…" with a Fetch button.
2. **Fetch** on the Mac: a progress bar, then the file is in `Downloads/Quakboard`, the item becomes a normal file (Open / Copy), and the file is on the clipboard (paste it into Finder).
3. **A big file** (≥ 500 MB): progress moves, Cancel works and leaves no `.partial` file, and fetching again completes.
4. **Edit the file after sending,** then fetch: "the file changed on the other device; send it again". **Delete it:** "moved or deleted".
5. **Quit the sending app,** then fetch: "… is offline or unreachable", and Try again works once it's back.
6. **Name clash:** fetch the same name twice, and the second becomes `name (1).ext`. The first is untouched.
7. **Change the folder** in Devices: the next fetch lands there.
8. **Three devices:** send to one of two other devices; only that one gets the item, and the other can't fetch it.
9. "Copy file to clipboard" on Linux: pasting into the file manager pastes the file.

## Decisions (confirmed)
1. **Offers never expire.** An offer lives until its item is deleted on the offering device, or the user stops sharing it. There's no 30-day expiry.
2. **Send opens a device picker.** You choose which paired devices get the offer, not all of them.
   - The picker lists paired devices and whether each one is reachable right now.
   - Only the chosen devices receive `FileOffer`.
   - The offer records who it was sent to. **Fetch is only served to those devices**, and anyone else gets "not offered to you".
3. **The download folder is configurable.** The default is `<Downloads>/Quakboard/`, with a setting to choose another folder (a folder picker via `tauri-plugin-dialog`, already a dependency). Name cleaning and "stays inside the chosen folder" apply to whatever folder is set.
4. **A fetched file also goes on the clipboard, as a file**, the way it would on the original device, so it can be pasted into a file manager or chat app straight away. That makes the cross-platform file clipboard (step 7) **required** for v1.5.2, not optional:
   - Linux: `text/uri-list`
   - Windows: `CF_HDROP`
   - macOS: already works

## Later: image files (after file sharing ships)
**Decided:** an image *file* copied in a file manager (png, jpg, webp…) is a file like any other in v1.5.2. It is shared **manually** (Send, then Fetch), and nothing syncs automatically.

A later release (planned as **v1.5.3**) adds settings in the Devices section:
1. **"Sync copied image files automatically" (on/off).** When on, copying an image file sends its pixels like a screenshot:
   - Read the file, and convert it to PNG if it isn't one, off the async workers.
   - Reuse `broadcast_image`, with the same 20 MB limit (checked after conversion) and the same echo guard.
   - Skip formats the `image` crate can't read (HEIC and others), with a log message.
   - The other device gets a pasteable image, not a file.
2. **"Fetch the original" for synced images.** An image that arrived as pixels can also fetch the original file (full quality, original format and name) through the file-sharing Fetch flow.
   - This needs the sender to create an offer for that file when it syncs the pixels. Tie them together with the file's full SHA-256.

Open question: whether "offer the original" should follow the same device picker as Send, or go to every device that received the pixels.
