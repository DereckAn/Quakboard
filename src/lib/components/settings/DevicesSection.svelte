<script lang="ts">
  import Icon from "$lib/components/icons/Icon.svelte";
  import Button from "$lib/components/ui/Button.svelte";
  import Modal from "$lib/components/ui/Modal.svelte";
  import {
    tauriSyncChooseDownloadDir,
    tauriSyncGetDownloadDir,
    tauriSyncGetStatus,
    tauriSyncListNearby,
    tauriSyncListPeers,
    tauriSyncPairByAddress,
    tauriSyncPairCancel,
    tauriSyncPairStart,
    tauriSyncPairSubmitCode,
    tauriSyncSetAcceptingPairing,
    tauriSyncSetEnabled,
    tauriSyncSetImagesEnabled,
    tauriSyncUnpair,
  } from "$lib/tauri/commands";
  import type {
    NearbyDevice,
    PairingEvent,
    SyncPeer,
    SyncStatus,
  } from "$lib/types";
  import { listen, type UnlistenFn } from "@tauri-apps/api/event";
  import { onDestroy, onMount } from "svelte";

  const REFRESH_INTERVAL_MS = 2000;

  let status = $state<SyncStatus | null>(null);
  let peers = $state<SyncPeer[]>([]);
  let nearby = $state<NearbyDevice[]>([]);
  let isToggling = $state(false);
  let isTogglingImages = $state(false);
  let downloadDir = $state<string | null>(null);
  let errorMessage = $state<string | null>(null);
  let notice = $state<string | null>(null);
  let confirmingUnpairId = $state<string | null>(null);

  // This device started pairing and waits for the code the other one shows.
  // Holds how to name that device in the dialog.
  let pairingWith = $state<string | null>(null);
  let manualAddress = $state("");
  let typedCode = $state("");
  let isSubmittingCode = $state(false);
  let codeInput = $state<HTMLInputElement>();

  // Another device asked to pair; this is the code to type over there.
  let shownCode = $state<string | null>(null);

  let refreshTimer: ReturnType<typeof setInterval> | undefined;
  let unlistenPairing: UnlistenFn | undefined;
  let isDestroyed = false;

  const isRunning = $derived(status?.running ?? false);
  const syncsImages = $derived(status?.syncImages ?? true);
  const maxImageMb = $derived(
    Math.round((status?.maxImageBytes ?? 0) / (1024 * 1024)),
  );
  const isCodeComplete = $derived(/^\d{6}$/.test(typedCode.trim()));

  $effect(() => {
    if (pairingWith) codeInput?.focus();
  });

  function describe(err: unknown): string {
    return err instanceof Error ? err.message : String(err);
  }

  async function refresh() {
    try {
      status = await tauriSyncGetStatus();
      if (status.running) {
        [peers, nearby] = await Promise.all([
          tauriSyncListPeers(),
          tauriSyncListNearby(),
        ]);
      } else {
        peers = [];
        nearby = [];
      }
    } catch (err) {
      errorMessage = describe(err);
    }
  }

  // Pairing requests are only answered while this screen is open.
  async function acceptPairingWhileOpen() {
    if (!isDestroyed && status?.running) {
      await tauriSyncSetAcceptingPairing(true);
    }
  }

  async function handleToggle() {
    isToggling = true;
    errorMessage = null;
    notice = null;
    try {
      status = await tauriSyncSetEnabled(!isRunning);
      await acceptPairingWhileOpen();
      await refresh();
    } catch (err) {
      errorMessage = describe(err);
    } finally {
      isToggling = false;
    }
  }

  async function handleToggleImages() {
    isTogglingImages = true;
    errorMessage = null;
    try {
      status = await tauriSyncSetImagesEnabled(!syncsImages);
    } catch (err) {
      errorMessage = describe(err);
    } finally {
      isTogglingImages = false;
    }
  }

  async function handleChangeDownloadDir() {
    errorMessage = null;
    try {
      // The picker runs in the backend, which keeps the window from
      // auto-hiding while the dialog has the focus.
      const chosen = await tauriSyncChooseDownloadDir();
      if (chosen) downloadDir = chosen;
    } catch (err) {
      errorMessage = describe(err);
    }
  }

  async function handlePair(device: NearbyDevice) {
    errorMessage = null;
    notice = null;
    typedCode = "";
    try {
      await tauriSyncPairStart(device.id);
      pairingWith = `Quakboard device #${device.shortId}`;
    } catch (err) {
      errorMessage = describe(err);
    }
  }

  // For networks that block mDNS, so the device never shows up as nearby.
  async function handlePairByAddress(e: SubmitEvent) {
    e.preventDefault();
    const address = manualAddress.trim();
    if (!address || pairingWith) return;
    errorMessage = null;
    notice = null;
    typedCode = "";
    try {
      await tauriSyncPairByAddress(address);
      pairingWith = address;
    } catch (err) {
      errorMessage = describe(err);
    }
  }

  async function handleSubmitCode(e: SubmitEvent) {
    e.preventDefault();
    if (!isCodeComplete || isSubmittingCode) return;
    isSubmittingCode = true;
    try {
      // The outcome arrives as a sync-pairing event.
      await tauriSyncPairSubmitCode(typedCode.trim());
    } catch (err) {
      errorMessage = describe(err);
      isSubmittingCode = false;
      pairingWith = null;
    }
  }

  function handleCancelPairing() {
    pairingWith = null;
    isSubmittingCode = false;
    tauriSyncPairCancel().catch((err) =>
      console.error("Failed to cancel pairing:", err),
    );
  }

  function handlePairingEvent(event: PairingEvent) {
    if (event.kind === "codeShown") {
      shownCode = event.code;
      return;
    }

    // Both "paired" and "failed" end whichever dialog is open.
    const wasPairing = pairingWith !== null || shownCode !== null;
    pairingWith = null;
    shownCode = null;
    isSubmittingCode = false;

    if (event.kind === "paired") {
      notice = `Paired with ${event.name}`;
      refresh();
    } else if (wasPairing || event.needsUpdate) {
      // A failure after the user closed the dialog (a cancel) isn't news, but
      // a device that needs updating is, even if it never showed a code.
      errorMessage = `Pairing failed: ${event.reason}`;
    }
  }

  async function handleUnpair(peer: SyncPeer) {
    errorMessage = null;
    try {
      await tauriSyncUnpair(peer.id);
      confirmingUnpairId = null;
      notice = `Unpaired ${peer.name}`;
      await refresh();
    } catch (err) {
      errorMessage = describe(err);
    }
  }

  onMount(async () => {
    const unlisten = await listen<PairingEvent>("sync-pairing", (event) =>
      handlePairingEvent(event.payload),
    );
    if (isDestroyed) {
      unlisten();
      return;
    }
    unlistenPairing = unlisten;

    await refresh();
    await acceptPairingWhileOpen();
    downloadDir = await tauriSyncGetDownloadDir().catch(() => null);
    refreshTimer = setInterval(refresh, REFRESH_INTERVAL_MS);
  });

  onDestroy(() => {
    isDestroyed = true;
    clearInterval(refreshTimer);
    unlistenPairing?.();
    if (pairingWith) handleCancelPairing();
    tauriSyncSetAcceptingPairing(false).catch((err) =>
      console.error("Failed to stop accepting pairing:", err),
    );
  });
</script>

<section
  class="relative bg-surface rounded-2xl border border-border/60 p-6 space-y-5"
>
  <div class="flex items-start justify-between gap-4">
    <div>
      <h2 class="text-lg font-semibold flex items-center gap-2">
        <Icon name="link" size={20} />
        Devices
      </h2>
      <p class="text-xs text-text-muted mt-1">
        Sync copied text with your other computers on the same Wi‑Fi. It stays
        on your network, encrypted end to end.
      </p>
    </div>
    <button
      onclick={handleToggle}
      disabled={isToggling || status === null}
      class={`relative inline-flex h-6 w-11 shrink-0 items-center rounded-full transition-colors disabled:opacity-60 ${isRunning ? "bg-primary" : "bg-border"}`}
      role="switch"
      aria-label="Sync with nearby devices"
      aria-checked={isRunning}
    >
      <span
        class={`inline-block h-4 w-4 transform rounded-full bg-white transition-transform ${
          isRunning ? "translate-x-6" : "translate-x-1"
        }`}
      ></span>
    </button>
  </div>

  {#if errorMessage}
    <p role="alert" class="text-xs text-danger">{errorMessage}</p>
  {/if}
  <p aria-live="polite" class="text-xs text-primary empty:hidden">{notice ?? ""}</p>

  {#if isRunning && status}
    <p class="text-xs text-text-muted">
      This device:
      <span class="text-text">{status.deviceName}</span>
      · #{status.shortId}
      {#if status.addresses.length > 0}
        · IP <span class="text-text">{status.addresses.join(", ")}</span>
      {/if}
    </p>

    <div class="flex items-start justify-between gap-4">
      <div>
        <p class="text-sm font-medium text-text">Also sync images</p>
        <p class="text-xs text-text-muted">
          Screenshots and copied pictures up to {maxImageMb} MB. Turn off if
          you often copy screenshots with private information.
        </p>
      </div>
      <button
        onclick={handleToggleImages}
        disabled={isTogglingImages}
        class={`relative inline-flex h-6 w-11 shrink-0 items-center rounded-full transition-colors disabled:opacity-60 ${syncsImages ? "bg-primary" : "bg-border"}`}
        role="switch"
        aria-label="Also sync images"
        aria-checked={syncsImages}
      >
        <span
          class={`inline-block h-4 w-4 transform rounded-full bg-white transition-transform ${
            syncsImages ? "translate-x-6" : "translate-x-1"
          }`}
        ></span>
      </button>
    </div>

    <div class="flex items-start justify-between gap-4">
      <div class="min-w-0">
        <p class="text-sm font-medium text-text">Fetched files go to</p>
        <p class="text-xs text-text-muted break-all">
          {downloadDir ?? "…"}
        </p>
      </div>
      <Button variant="outline" size="sm" onclick={handleChangeDownloadDir}>
        Change…
      </Button>
    </div>

    <div class="space-y-2">
      <h3 class="text-sm font-medium text-text">Paired devices</h3>
      {#if peers.length === 0}
        <p class="text-xs text-text-muted">No paired devices yet.</p>
      {:else}
        <ul class="space-y-2">
          {#each peers as peer (peer.id)}
            <li
              class="flex items-center justify-between gap-3 rounded-xl border border-border/60 px-3 py-2"
            >
              <div class="min-w-0">
                <p class="text-sm text-text truncate">{peer.name}</p>
                <p class="text-xs text-text-muted">
                  {peer.lastAddr ?? "Address unknown"}
                </p>
                {#if peer.updateNeeded}
                  <p class="text-xs text-danger">
                    {peer.updateNeeded === "peerTooOld"
                      ? `Not syncing: update Quakboard on ${peer.name}`
                      : "Not syncing: update Quakboard on this device"}
                  </p>
                {/if}
              </div>
              {#if confirmingUnpairId === peer.id}
                <div class="flex gap-2 shrink-0">
                  <Button
                    variant="destructive"
                    size="sm"
                    onclick={() => handleUnpair(peer)}
                  >
                    Unpair
                  </Button>
                  <Button
                    variant="ghost"
                    size="sm"
                    onclick={() => (confirmingUnpairId = null)}
                  >
                    Keep
                  </Button>
                </div>
              {:else}
                <Button
                  variant="outline"
                  size="sm"
                  onclick={() => (confirmingUnpairId = peer.id)}
                >
                  Unpair
                </Button>
              {/if}
            </li>
          {/each}
        </ul>
      {/if}
    </div>

    <div class="space-y-2">
      <h3 class="text-sm font-medium text-text">Nearby</h3>
      {#if nearby.length === 0}
        <div class="text-xs text-text-muted space-y-1">
          <p>
            Looking for devices… Turn sync on on the other computer and open
            this screen there too.
          </p>
          <p>
            Nothing showing up? Both need the same Wi‑Fi, and the firewall must
            allow TCP port 47823 and UDP port 5353.
          </p>
        </div>
      {:else}
        <ul class="space-y-2">
          {#each nearby as device (device.id)}
            <li
              class="flex items-center justify-between gap-3 rounded-xl border border-border/60 px-3 py-2"
            >
              <div class="min-w-0">
                <p class="text-sm text-text">Quakboard device #{device.shortId}</p>
                <p class="text-xs text-text-muted">{device.addr}</p>
              </div>
              <Button
                size="sm"
                disabled={pairingWith !== null}
                onclick={() => handlePair(device)}
              >
                Pair
              </Button>
            </li>
          {/each}
        </ul>
      {/if}
    </div>

    <form class="space-y-2" onsubmit={handlePairByAddress}>
      <label for="pair-address" class="block text-sm font-medium text-text">
        Connect by IP
      </label>
      <p class="text-xs text-text-muted">
        Other device not showing up? Type the IP it shows under "This device".
      </p>
      <div class="flex gap-2">
        <input
          id="pair-address"
          bind:value={manualAddress}
          placeholder="192.168.1.20"
          inputmode="decimal"
          autocomplete="off"
          spellcheck="false"
          class="flex-1 rounded-md border border-border bg-surface px-3 py-1.5 text-sm text-text placeholder:text-text-muted"
        />
        <Button
          type="submit"
          size="sm"
          disabled={!manualAddress.trim() || pairingWith !== null}
        >
          Pair
        </Button>
      </div>
    </form>
  {/if}
</section>

<Modal
  isOpen={pairingWith !== null}
  onClose={handleCancelPairing}
  title="Enter pairing code"
>
  <form class="p-6 space-y-4" onsubmit={handleSubmitCode}>
    <label for="pairing-code" class="block text-sm text-text-muted">
      Type the 6-digit code shown on {pairingWith}.
    </label>
    <input
      id="pairing-code"
      bind:this={codeInput}
      bind:value={typedCode}
      inputmode="numeric"
      autocomplete="one-time-code"
      maxlength="6"
      disabled={isSubmittingCode}
      class="w-full rounded-md border border-border bg-surface px-3 py-2 text-center font-mono text-2xl tracking-[0.3em] text-text"
    />
    <div class="flex justify-end gap-2">
      <Button variant="ghost" onclick={handleCancelPairing}>Cancel</Button>
      <Button type="submit" disabled={!isCodeComplete || isSubmittingCode}>
        {isSubmittingCode ? "Pairing…" : "Pair"}
      </Button>
    </div>
  </form>
</Modal>

<Modal
  isOpen={shownCode !== null}
  onClose={() => (shownCode = null)}
  title="Pairing request"
>
  <div class="p-6 space-y-4 text-center">
    <p class="text-sm text-text-muted">
      Another device wants to pair. Type this code on it:
    </p>
    <p class="font-mono text-4xl tracking-[0.3em] text-text">{shownCode}</p>
    <p class="text-xs text-text-muted">
      Didn't start this? Close this window. Nothing gets paired without the
      code.
    </p>
  </div>
</Modal>
