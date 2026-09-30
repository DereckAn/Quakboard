# LAN clipboard sync (v1)

**Status:** implemented in 7 steps. Covered by `cargo test --lib sync::` (79 tests); not yet exercised end to end on two real machines.

## Context
Goal: copy text on one computer and paste it on another. Scope for v1: **same Wi‑Fi/LAN only, no server, no accounts**. Every new text clip (text/code/link/color/svg) is sent automatically to paired devices, and the receiving device puts it on its system clipboard. That fits the app's "100% local" promise. Cloud/account sync (Supabase, roadmap v1.2) can be added later as a second transport using the same frames and keys.

## How it works

### Identity and paired devices
`<app_data_dir>/sync.json` holds `{ device_id, device_name, peers: [{ id, name, key, last_addr }] }`.
- `device_id` is a UUID generated on first run and never changes.
- `device_name` is the hostname (`.local` stripped), falling back to "Quakboard device". It's sent to the other device only during pairing, encrypted. Never broadcast.
- `key` is the 32-byte per-peer key, stored as base64.
- `last_addr` is the peer's last known `ip:port`.

The file holds keys, so:
- It's written owner-only (`0600`) on Linux and macOS. On Windows it relies on per-user AppData permissions.
- Saves are atomic (a temp file, then a rename).
- A file that fails to parse is reported and **never overwritten**, so pairings can't be silently wiped.
- `Peer`'s `Debug` output redacts the key.

### Wire format
- **Transport:** TCP on fixed port **47823**, so users can allow it through a firewall.
  - One connection per clip. `ponytail:` switch to persistent connections if latency ever matters.
  - Pairing reuses a single connection for the whole handshake.
- **Framing:** each frame is a 4‑byte big-endian length followed by JSON tagged by `type`.
  - `Clip { from, sealed }`
  - `PairRequest`, `PairChallenge`, `PairSpake`, `PairAccept`, `PairFinish`, `PairDone`
- **Encryption:** `Sealed { nonce, ciphertext }` is ChaCha20-Poly1305 with a fresh random nonce every time.
  - Clips are sealed with the sender's id as associated data, so a frame can't be relabelled as coming from another device.
- **Listener hardening,** since it faces the LAN:

| Protection | Value |
|---|---|
| Frame size limit (checked before allocating) | 1 MB |
| Read timeout | 5 s |
| Concurrent connections | at most 16 |
| Connect timeout when sending | 1 s |

Frames from unpaired devices, and frames that fail to decrypt, are dropped and logged.

### Pairing (SPAKE2 with a 6-digit code)
```
A (initiator)                              B (responder, shows the code)
  PairRequest   { from: A }        ->
                                   <-  PairChallenge { from: B }
  ... user reads B's code and types it on A (2 min limit) ...
  PairSpake     { A's SPAKE2 msg } ->
                                   <-  PairAccept { B's msg, sealed B name+port }
  (A opening B's info proves both sides used the same code)
  PairFinish    { sealed A name+port } ->
                                   <-  PairDone
```
- **Guessing:** an eavesdropper learns nothing, and an active attacker gets one online guess per attempt (1 in 1,000,000), with a fresh code each time.
- **Wrong codes:** SPAKE2 doesn't report a wrong code, so each side proves it has the key by sealing its info for the other to open. A failure means `WrongCode`, and nothing is stored.
- **Keys:** the SPAKE2 secret is hashed with domain separation into a handshake key and a separate long-term peer key.
- **The code:** a uniformly random 6-digit number from the OS RNG, using rejection sampling to avoid bias.
- **Untrusted input from the other device:**
  - its id must be a UUID
  - its name has control characters stripped, is capped at 64 characters, and blank names become "Unnamed device"
- **One pairing at a time** per device.
- **B only answers pairing requests while its Devices screen is open**, so nobody on the LAN can pop up a code prompt at other times.
- **Saving:** a new peer is saved before it goes live in memory. Unpair works the same way.

### Discovery (mDNS)
- **What gets advertised:** `_quakboard._tcp.local.`, only while sync is on. The instance name, host name (`<uuid>.local.`) and TXT record (`id=<uuid>`) carry only the opaque id. Verified with `avahi-browse`: no hostname or user name leaks.
- **What gets trusted:** browse results must carry a UUID id, the device's own advertisement is ignored, and only IPv4 is used (`ponytail:` IPv6 link-local needs interface scopes).
- **Address choice:** LAN addresses are preferred over loopback. A device can advertise 127.0.0.1, and that only works from the same machine.
- **Labels:** unpaired devices show as "Quakboard device #ab12", where the suffix is the last 4 characters of the id.

### Sending
- The monitor's text branch, after saving the item, calls `SyncService::broadcast` through `SyncRuntime`, only while sync is running.
- **Per device:** try `last_addr` first. If that fails, or there's no address, look the id up through mDNS and retry once at a *different* address.
- **Remembering moves:** a newly found address is saved to `sync.json`.
- **Offline devices** are skipped. There's no queue in v1.

### Receiving and the echo guard
- **Receiving:** decrypt, remember the text as `last_received`, then write it to the system clipboard. The monitor then saves it and updates the UI as usual, so there's no separate code path.
- **Echo guard:** the next `broadcast` *consumes* `last_received`. If it matches, that one echo is suppressed. Copying the same text again later still syncs.
  - This compares text rather than skipping clipboard events, because Wayland can fire more than one event per write. The monitor's own duplicate check already absorbs those.
- **Address learning:** a successfully decrypted clip proves its sender, so if it came from a new IP, that IP is stored, keeping the port the peer gave at pairing.

### Turning it on and off
- **The setting:** `syncEnabled` in `settings.json`, off by default.
- **`SyncRuntime`** is registered at startup and starts or stops the service live, without a restart.
  - On start it binds the port first. If the port is taken, you get an error and the setting stays off.
  - On stop it frees the port, sends mDNS goodbyes and cancels any pending code entry.
- **Where to find it:** Settings → Account → **Devices**.
  - An enable toggle.
  - This device's name, `#id`, and IP.
  - Paired devices, with Unpair (asks for confirmation).
  - Nearby devices, with Pair.
  - Connect by IP.
  - The code-entry dialog and the code-display dialog.
  - The screen refreshes every 2 s while open.

### Connect by IP
For networks that block mDNS. Type the IP the other device shows under "This device", for example `192.168.1.20` or `192.168.1.20:5000`. Hostnames are rejected, so nothing leaks to DNS. Pairing then works as above.

## Files
| File | Purpose |
|---|---|
| `src-tauri/src/sync/mod.rs` | `ClipPayload`, `Sealed`, `Frame`, `SyncError` |
| `src-tauri/src/sync/store.rs` | `SyncStore` / `Peer`, persisting `sync.json` |
| `src-tauri/src/sync/transport.rs` | Framing, `connect`, `send_frame`, `serve` |
| `src-tauri/src/sync/pairing.rs` | SPAKE2 handshake (`initiate`, `respond`), `generate_code` |
| `src-tauri/src/sync/discovery.rs` | mDNS advertise and browse, `short_id` |
| `src-tauri/src/sync/service.rs` | `SyncService`: broadcast, receive, pairing, unpair. Effects come in through `SyncHooks` (`apply_clip`, `on_pairing`, `resolve_addr`), so tests don't touch the clipboard, UI or network |
| `src-tauri/src/sync/runtime.rs` | `SyncRuntime`: start and stop, pairing entry points, `parse_peer_address`, `local_addresses` |
| `src-tauri/src/commands/sync.rs` | Tauri commands (below) |
| `src-tauri/src/clipboard/monitor.rs` | Calls `broadcast` after a text copy is saved |
| `src-tauri/src/lib.rs` | Registers `SyncRuntime`, starts it if enabled, registers the commands |
| `src/lib/types/sync.ts`, `src/lib/tauri/commands.ts` | Frontend types and wrappers |
| `src/lib/components/settings/DevicesSection.svelte` | The Devices UI, inside `TabAccountSettings.svelte` |
| `src/lib/components/ui/Modal.svelte` | Fixed before first use: Esc no longer also closes the settings page, and Enter/Space in inputs no longer closes the modal |

**Commands:**
- `sync_get_status` (`enabled`, `running`, `deviceName`, `shortId`, `addresses`)
- `sync_set_enabled`
- `sync_list_peers`: never returns keys
- `sync_list_nearby`: unpaired only
- `sync_set_accepting_pairing`
- `sync_pair_start(deviceId)`, `sync_pair_by_address(address)`, `sync_pair_submit_code(code)`, `sync_pair_cancel`
- `sync_unpair(peerId)`

**Event:** `sync-pairing` with `{ kind: "codeShown", code }`, `{ kind: "paired", peerId, name }` or `{ kind: "failed", reason }`.

**Dependencies added:**
- `chacha20poly1305` 0.11
- `spake2` 0.4
- `mdns-sd` 0.21
- `base64` 0.22, `getrandom` 0.3 and `if-addrs` 0.15, all already compiled in as transitive dependencies

## Changes from the original plan
- **Runtime module added:** Tauri state can't be removed once registered, so a `SyncRuntime` holder owns the service, and the toggle works without a restart.
- **Pairing is only accepted while the Devices screen is open.** This wasn't in the plan; it limits code-prompt spam from other devices on the LAN.
- **The protocol has an extra message:** the initiator can't start SPAKE2 until the user types the code, so the flow opens with `PairRequest`/`PairChallenge`. Device ports are exchanged sealed, so both sides know where to reach each other.
- **Address learning on receive moved from step 3 to step 7,** because it needs the port exchanged at pairing.
- **Split into more files** than the planned `mod.rs` + `pairing.rs`, one job per file.
- **No `sync_rename_device`.** The hostname is used; add renaming if users ask.
- **The Account placeholder (email / Upgrade) was kept** above the Devices card for future cloud accounts, instead of being replaced.

## Network impact at scale
- mDNS is link-local multicast (224.0.0.251:5353). Routers don't forward it, so load depends on devices **per Wi‑Fi network**, not on the total user count.
- Per device it's one small record (~200–400 bytes). Other devices cache it, searches back off exponentially, and a device doesn't reply with records the asker already has cached. Steady state is well under 1 KB/min per device, the same kind of traffic printers and Chromecasts already produce.
- On large shared Wi‑Fi (offices, campuses), multicast goes out at the slowest Wi‑Fi rate. That's why the app advertises only while sync is on and connects to `last_addr` first.
- **Many corporate, campus, hotel and café networks block mDNS** or isolate clients from each other.
  - Connect by IP covers networks that only block mDNS.
  - With full client isolation LAN sync can't work at all. The fix there is a relay (or iroh), then cloud sync.
- `mdns-sd` runs its own responder next to Avahi or Bonjour on port 5353. Verified working with Avahi running.

## Firewall
Linux users running ufw or firewalld must allow both ports. On ufw, for a `192.168.18.x` network:
```
sudo ufw allow from 192.168.18.0/24 to any port 47823 proto tcp
sudo ufw allow from 192.168.18.0/24 to any port 5353 proto udp
```
The Devices screen mentions this when nothing is found. A future improvement is to detect it and show these exact commands.

## Out of scope (v1)
- Images and files. The frame format has room: add a new `Frame` variant plus a size limit, and stream large payloads.
- Syncing across networks, and accounts (relay or cloud).
- An offline queue and history backfill.
- Skipping password-manager copies.
- IPv6.
- Renaming the device.
- **Known gap:** with mDNS blocked *and* a peer's IP changed, neither fallback finds it. Pairing again by IP fixes it.

## Verification
1. `cd src-tauri && cargo test --lib sync::` covers:
   - framing and encryption
   - store safety
   - listener limits
   - pairing: right code, wrong code, cancel, one at a time, only while the screen is open
   - real-TCP flows (pair, then sync both ways, echo guard, unpaired devices dropped, unpair)
   - address fallback and learning
   - discovery tracking
   - address parsing
2. `bun run check` and `bunx vitest run`.
3. Manual test with two machines on the same Wi‑Fi (the single-instance plugin blocks running two instances on one box). Open the firewall first.
   1. Turn sync on in Settings → Account → Devices on both. Each should list the other under Nearby.
   2. Pair using the code, then copy text on A and paste on B. It should appear in B's history, and A must not receive it back.
   3. A wrong code: pairing fails on both, and nothing is stored.
   4. Unpair, and turn sync off on B: A's copies stop arriving.
   5. Block UDP 5353 on one machine: Nearby stays empty, and Connect by IP still pairs and syncs.
   6. Change B's IP (reconnect Wi‑Fi): the next copy from A still arrives, and A logs `Sync now reaches … at <new IP>`.
   7. Run `avahi-browse -rt _quakboard._tcp` and check that only UUIDs appear, no hostname.
