<script lang="ts">
  import Button from "$lib/components/ui/Button.svelte";
  import Modal from "$lib/components/ui/Modal.svelte";
  import {
    tauriSyncGetStatus,
    tauriSyncListPeers,
    tauriSyncSendFile,
  } from "$lib/tauri/commands";
  import type { FileProgress, SyncPeer } from "$lib/types";
  import { listen, type UnlistenFn } from "@tauri-apps/api/event";

  interface Props {
    isOpen: boolean;
    itemId: string;
    fileName: string;
    onClose: () => void;
  }

  let { isOpen, itemId, fileName, onClose }: Props = $props();

  let peers = $state<SyncPeer[]>([]);
  let chosen = $state<string[]>([]);
  let isSyncOn = $state(true);
  let isSending = $state(false);
  let preparedPercent = $state<number | null>(null);
  let result = $state<string | null>(null);
  let errorMessage = $state<string | null>(null);

  function describe(err: unknown): string {
    return err instanceof Error ? err.message : String(err);
  }

  async function load() {
    result = null;
    errorMessage = null;
    preparedPercent = null;
    try {
      const status = await tauriSyncGetStatus();
      isSyncOn = status.running;
      peers = isSyncOn ? await tauriSyncListPeers() : [];
      // Online devices are the likely targets; offline ones would miss it.
      chosen = peers.filter((peer) => peer.isOnline).map((peer) => peer.id);
    } catch (err) {
      errorMessage = describe(err);
    }
  }

  $effect(() => {
    if (isOpen) load();
  });

  // Hashing progress for big files, while "Send" is preparing the offer.
  $effect(() => {
    if (!isOpen) return;
    let unlisten: UnlistenFn | undefined;
    let isActive = true;
    listen<FileProgress>("sync-file-progress", (event) => {
      const progress = event.payload;
      if (progress.itemId !== itemId || progress.phase !== "hashing") return;
      preparedPercent =
        progress.total > 0
          ? Math.floor((progress.done / progress.total) * 100)
          : 100;
    }).then((stop) => {
      if (isActive) unlisten = stop;
      else stop();
    });
    return () => {
      isActive = false;
      unlisten?.();
    };
  });

  function toggle(peerId: string) {
    chosen = chosen.includes(peerId)
      ? chosen.filter((id) => id !== peerId)
      : [...chosen, peerId];
  }

  async function handleSend(e: SubmitEvent) {
    e.preventDefault();
    if (chosen.length === 0 || isSending) return;
    isSending = true;
    errorMessage = null;
    result = null;
    try {
      const sent = await tauriSyncSendFile(itemId, chosen);
      result =
        sent.reached === sent.chosen
          ? `Sent to ${sent.reached} ${sent.reached === 1 ? "device" : "devices"}`
          : `Sent to ${sent.reached} of ${sent.chosen} devices; the others were unreachable`;
    } catch (err) {
      errorMessage = describe(err);
    } finally {
      isSending = false;
      preparedPercent = null;
    }
  }
</script>

<Modal {isOpen} {onClose} title="Send to devices">
  <form class="p-6 space-y-4" onsubmit={handleSend}>
    <p class="text-sm text-text-muted truncate">{fileName}</p>

    {#if !isSyncOn}
      <p class="text-sm text-text-muted">
        Turn on sync in Settings → Account → Devices to send files.
      </p>
    {:else if peers.length === 0}
      <p class="text-sm text-text-muted">
        No paired devices yet. Pair one in Settings → Account → Devices.
      </p>
    {:else}
      <fieldset class="space-y-2">
        <legend class="text-sm font-medium text-text mb-2">
          Choose who can fetch it
        </legend>
        {#each peers as peer (peer.id)}
          <label
            class="flex items-center justify-between gap-3 rounded-xl border border-border/60 px-3 py-2 cursor-pointer"
          >
            <span class="flex items-center gap-2 min-w-0">
              <input
                type="checkbox"
                checked={chosen.includes(peer.id)}
                onchange={() => toggle(peer.id)}
                disabled={isSending}
              />
              <span class="text-sm text-text truncate">{peer.name}</span>
            </span>
            <span class="text-xs {peer.isOnline ? 'text-primary' : 'text-text-muted'}">
              {peer.isOnline ? "Online" : "Offline"}
            </span>
          </label>
        {/each}
      </fieldset>
    {/if}

    <div aria-live="polite" class="text-xs">
      {#if isSending}
        <p class="text-text-muted">
          {preparedPercent !== null && preparedPercent < 100
            ? `Preparing… ${preparedPercent}%`
            : "Sending…"}
        </p>
      {:else if result}
        <p class="text-primary">{result}</p>
      {/if}
      {#if errorMessage}
        <p role="alert" class="text-danger">{errorMessage}</p>
      {/if}
    </div>

    <div class="flex justify-end gap-2">
      <Button variant="ghost" onclick={onClose}>
        {result ? "Done" : "Cancel"}
      </Button>
      {#if isSyncOn && peers.length > 0 && !result}
        <Button type="submit" disabled={chosen.length === 0 || isSending}>
          Send
        </Button>
      {/if}
    </div>
  </form>
</Modal>
