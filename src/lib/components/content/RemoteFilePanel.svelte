<script lang="ts">
  import Icon from "$lib/components/icons/Icon.svelte";
  import Button from "$lib/components/ui/Button.svelte";
  import { tauriSyncCancelFetch, tauriSyncFetchFile } from "$lib/tauri/commands";
  import type { FileDone, FileProgress, RemoteFileInfo } from "$lib/types";
  import { formatFileSize } from "$lib/utils/format";
  import { listen, type UnlistenFn } from "@tauri-apps/api/event";

  interface Props {
    itemId: string;
    fileName: string;
    remote: RemoteFileInfo;
  }

  let { itemId, fileName, remote }: Props = $props();

  let isFetching = $state(false);
  let received = $state(0);
  let errorMessage = $state<string | null>(null);

  const percent = $derived(
    remote.size > 0 ? Math.floor((received / remote.size) * 100) : 0,
  );

  function describe(err: unknown): string {
    return err instanceof Error ? err.message : String(err);
  }

  // A different item was selected: forget this one's state.
  $effect(() => {
    void itemId;
    isFetching = false;
    received = 0;
    errorMessage = null;
  });

  $effect(() => {
    const unlisteners: UnlistenFn[] = [];
    let isActive = true;
    const keep = (stop: UnlistenFn) => {
      if (isActive) unlisteners.push(stop);
      else stop();
    };

    listen<FileProgress>("sync-file-progress", (event) => {
      const progress = event.payload;
      if (progress.itemId !== itemId || progress.phase !== "fetching") return;
      isFetching = true;
      received = progress.done;
    }).then(keep);

    // On success the item becomes a local file and this panel goes away.
    listen<FileDone>("sync-file-done", (event) => {
      const done = event.payload;
      if (done.itemId !== itemId) return;
      isFetching = false;
      received = 0;
      if (!done.ok && !done.cancelled) {
        errorMessage = done.error ?? "The fetch failed";
      }
    }).then(keep);

    return () => {
      isActive = false;
      unlisteners.forEach((stop) => stop());
    };
  });

  async function handleFetch() {
    errorMessage = null;
    received = 0;
    isFetching = true;
    try {
      await tauriSyncFetchFile(itemId);
    } catch (err) {
      isFetching = false;
      errorMessage = describe(err);
    }
  }

  function handleCancel() {
    tauriSyncCancelFetch(itemId).catch((err) =>
      console.error("Failed to cancel fetch:", err),
    );
  }
</script>

<div class="w-full max-w-xl space-y-4 p-2">
  <div class="flex flex-col items-center gap-3 p-4">
    <Icon name="file" size={56} class="text-text-muted" />
    <p class="text-sm font-medium text-text break-all">{fileName}</p>
    <p class="text-xs text-text-muted">
      {formatFileSize(remote.size)} · on {remote.origin_name}
    </p>
  </div>

  {#if isFetching}
    <div class="space-y-2">
      <div
        class="h-2 w-full overflow-hidden rounded-full bg-border"
        role="progressbar"
        aria-label="Fetching {fileName}"
        aria-valuemin={0}
        aria-valuemax={100}
        aria-valuenow={percent}
      >
        <div class="h-full bg-primary transition-[width]" style="width: {percent}%"></div>
      </div>
      <div class="flex items-center justify-between text-xs text-text-muted">
        <span>
          Fetching from {remote.origin_name}… {formatFileSize(received)} of
          {formatFileSize(remote.size)}
        </span>
        <Button variant="ghost" size="sm" onclick={handleCancel}>Cancel</Button>
      </div>
    </div>
  {:else}
    {#if errorMessage}
      <p role="alert" class="text-xs text-danger text-center">
        Couldn't fetch it: {errorMessage}
      </p>
    {/if}
    <div class="flex justify-center">
      <Button onclick={handleFetch}>
        <Icon name="externalLink" size={16} />
        {errorMessage ? "Try again" : `Fetch from ${remote.origin_name}`}
      </Button>
    </div>
  {/if}
</div>
