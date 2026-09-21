# Plan: LAN clipboard sync (v1)

## Context
Goal: copy text on one computer and paste it on another. Chosen scope for v1: **same Wi‑Fi/LAN only, no server, no accounts**. Every new text clip (text/code/link/color) is sent automatically to paired devices, and the receiving device puts it on its system clipboard. That fits the app's "100% local" promise. Cloud/account sync (Supabase, roadmap v1.2) can be added later as a second transport using the same message format.

## What exists and what we reuse
- The monitor is event-driven: `clipboard/listener.rs::spawn_clipboard_listener` feeds `ClipboardMonitor::handle_clipboard_change` ([monitor.rs](src-tauri/src/clipboard/monitor.rs)). The text path is around L259–298. It dedupes through `last_text_content` and `repo.upsert_item`, then emits `clipboard-item-added`.
- `operations::write_clipboard(text)` ([operations.rs:240](src-tauri/src/clipboard/operations.rs#L240)) writes text to the system clipboard. It does **not** suppress the monitor, so text received from another device would be picked up and sent out again (an echo loop).
- `tokio` (full) is already a dependency, so it provides TCP. `uuid` and `serde_json` are also there.
- `AppState { app_data_dir, .. }` is in managed state ([lib.rs:200](src-tauri/src/lib.rs#L200)).
- Settings: flat `<app_data_dir>/settings.json` via `save_setting`/`get_setting` ([commands/settings.rs](src-tauri/src/commands/settings.rs)).
- UI: [TabAccountSettings.svelte](src/lib/components/settings/tabs/TabAccountSettings.svelte) is an empty "sync across devices" placeholder. The Devices UI goes there.
- The `synced`/`server_id` columns are not used. Leave them alone; they belong to future cloud sync.

## New dependencies (3, all small and pure Rust)
- `mdns-sd`: finds devices on the LAN (`_quakboard._tcp.local.`).
- `spake2`: pairing with a 6‑digit code. A short code alone makes a weak key. SPAKE2 turns it into a strong shared key and blocks offline brute force.
- `chacha20poly1305`: encrypts every clip with the per-peer key.

## Design
**Identity and peers**: `<app_data_dir>/sync.json` holds `{ device_id, device_name, peers: [{ id, name, key_b64, last_addr }] }`.
- `device_id` is a UUID generated on first run.
- `device_name` defaults to the hostname. It is shared only with the other device during pairing, never broadcast.
- `last_addr` is the peer's last known `ip:port`, updated after every successful send or receive.

**Transport**:
- Each device runs a tokio TCP listener on a fixed port, **47823**. It's fixed so users can allow it through their firewall.
- Each device advertises itself over mDNS only while sync is enabled. The service instance name and TXT record carry only an opaque ID (`id=<device_id>`), with no hostname or user name, so people on public Wi‑Fi can't see who is running the app. The Discovered list shows unpaired devices as "Quakboard device (#ab12)", where the suffix is the last 4 characters of the ID.
- Each message uses its own connection (connect → send → close). That's fine at clipboard rates.
  - `ponytail:` one connection per clip; switch to persistent connections if latency ever matters.
- A frame is a 4‑byte length followed by JSON:
  - `Clip { from, nonce, ciphertext }`, where the plaintext is `{ text, content_type }`
  - `PairHello { from, name, spake_msg }` / `PairReply { spake_msg, confirm }`
- Frames over 1 MB are rejected. Frames from unknown peers, or that fail to decrypt, are dropped.

**Pairing flow**:
1. On device A, Settings → Devices lists devices found via mDNS that aren't paired yet. Click **Pair**. If mDNS is blocked on the network, use **Connect by IP** instead: enter B's IP address, which B's Devices screen displays.
2. Device B gets `PairHello`, makes a random 6‑digit code, and emits `sync-pairing-code`. The frontend shows it in a modal.
3. The user types the code on A. Both sides finish SPAKE2 and exchange a key confirmation. Only after that do they send each other their friendly names, encrypted. Each then stores the other as a peer. A wrong code fails the confirmation and nothing is stored.

**Sending** (monitor text path, after a successful save):
- Only if `syncEnabled` is on, the text isn't what we last *received* (echo guard), and the content is text-based.
- Then `sync::broadcast(text, content_type)` is spawned. For each paired peer it tries `last_addr` first, with a short connect timeout (~1s). If that fails, it resolves the peer's current address from the mDNS cache and retries once. Offline peers are skipped (no queue in v1).
- Connecting to the known IP first means mDNS only does real work when a peer's IP changes, which keeps multicast traffic minimal.

**Receiving**:
- Decrypt, set `sync::LAST_RECEIVED = text`, then call `operations::write_clipboard(text)`.
- The monitor then sees the change and saves and emits it as usual, so there's no separate DB or UI code path. Because the text matches `LAST_RECEIVED`, it doesn't send it again.
- The echo guard compares text instead of using `request_skip_events`, because Wayland's `wl-paste --watch` can fire more than once per write.

**Setting**: `syncEnabled` (default **off**) in `settings.json`. When it's off, the listener and mDNS advertisement stop.

## Files
- **New** `src-tauri/src/sync/mod.rs`: the listener, mDNS browse/advertise, `broadcast`, frame encode/decode, encryption, `LAST_RECEIVED`, and sync.json load/save.
- **New** `src-tauri/src/sync/pairing.rs`: the SPAKE2 handshake for both sides.
- **New** `src-tauri/src/commands/sync.rs`: commands `sync_set_enabled`, `sync_get_status` (device name, enabled), `sync_list_discovered`, `sync_list_peers`, `sync_pair_start(device_id)`, `sync_pair_by_address(ip)`, `sync_pair_submit_code(code)`, `sync_get_local_addresses`, `sync_unpair(id)`, `sync_rename_device(name)`.
- **Edit** [lib.rs](src-tauri/src/lib.rs): `mod sync;`, start sync in `setup` if enabled, and register the commands.
- **Edit** [commands/mod.rs](src-tauri/src/commands/mod.rs): export the new commands.
- **Edit** [monitor.rs](src-tauri/src/clipboard/monitor.rs): call `sync::broadcast` in the text branch, behind the echo guard.
- **Edit** [Cargo.toml](src-tauri/Cargo.toml): add the 3 dependencies.
- **Edit** [src/lib/tauri/commands.ts](src/lib/tauri/commands.ts): typed wrappers.
- **Edit** [TabAccountSettings.svelte](src/lib/components/settings/tabs/TabAccountSettings.svelte): replace the stub with a Devices section containing:
  - an enable toggle and the device name
  - the paired list with Unpair
  - the discovered list with Pair
  - a code-entry input
  - "Connect by IP" (an IP input), plus this device's own LAN IPs so they can be read off and typed on the other device
  - a code-display modal, listening to `sync-pairing-code`
- Capabilities: no change. Commands are plain `invoke`.

## Network impact at scale
- mDNS is link-local multicast (224.0.0.251:5353). Routers don't forward it, so load depends on devices **per Wi‑Fi network**, not on the total user count.
- Per device it's one small record (~200–400 bytes). Other devices cache it, searches back off exponentially, and a device doesn't reply with records the asker already has cached. Steady state is well under 1 KB/min per device, the same kind of traffic printers and Chromecasts already produce.
- On large shared Wi‑Fi (offices, campuses), multicast goes out at the slowest Wi‑Fi rate. That's the reason for advertising only while sync is enabled and connecting to `last_addr` first.
- The bigger risk is that **many corporate, campus, hotel and café networks block mDNS** or isolate clients from each other. Connect by IP covers networks that only block mDNS. With full client isolation LAN sync can't work at all; the cloud relay (roadmap v1.2) is the real fix there.
- `mdns-sd` runs its own responder next to Avahi/Bonjour on port 5353. Multiple responders are normal, but check this on Linux with Avahi running.

## Out of scope (v1)
- Images and files
- Syncing across networks, and accounts
- Offline queue and history backfill
- Skipping password-manager copies

Each can be added when needed.

## Verification
1. `cargo test` in `src-tauri` covers:
   - pairing: the same code gives the same key on both sides, and a wrong code fails confirmation
   - frames: an encrypt/decrypt round trip works, and a tampered ciphertext is rejected
   - echo guard: text equal to `LAST_RECEIVED` is not broadcast
2. `bun run check` for types.
3. Manual test with two machines on the same Wi‑Fi (the single-instance plugin blocks running two instances on one box):
   1. On both, enable sync. Each should see the other under "Discovered".
   2. Pair using the code, then copy text on A and paste it on B. It should appear in B's history, and A must not receive it back.
   3. Try a wrong code: pairing fails and nothing is stored.
   4. Turn sync off on B: A's copies stop arriving.
   5. Pair using Connect by IP with mDNS disabled or blocked (for example, stop Avahi, or block UDP 5353 in the firewall).
   6. Change B's IP (reconnect Wi‑Fi or renew DHCP): the next copy from A should still arrive, via the mDNS fallback.
   7. With `tcpdump -i any udp port 5353` running, confirm the app is silent on mDNS in steady state and that no hostname appears in its packets.
   8. On Linux, if nothing is discovered, check firewall rules for TCP 47823 and UDP 5353 (mDNS).
