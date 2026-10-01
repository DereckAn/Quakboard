<script lang="ts">
  import Icon from "$lib/components/icons/Icon.svelte";
  import { clipboardStore } from "$lib/stores/clipboard.svelte";
  import { uiStore } from "$lib/stores/ui.svelte";
  import {
    tauriBumpItem,
    tauriEnsureThumbnail,
    tauriExtractDomain,
    tauriPastefromClipboard,
    tauriWriteImageToClipboard,
    tauriWriteToClipboard,
  } from "$lib/tauri/commands";
  import type { ClipboardItem } from "$lib/types";
  import { cn } from "$lib/utils/cn";
  import { getContentTypeIcon } from "$lib/utils/format";
  import { getLanguageInfo } from "$lib/utils/languages";
  import { convertFileSrc } from "@tauri-apps/api/core";

  interface Props {
    item: ClipboardItem;
  }

  let { item }: Props = $props();

  // Local state
  let isHoveringIcon = $state(false);
  let showDeleteConfirm = $state(false);
  let isDeleting = $state(false);
  let faviconUrl = $state<string>("");
  let rowElement = $state<HTMLDivElement>();

  // Computed
  const isSelected = $derived(uiStore.selectedItemId === item.id);
  const displayText = $derived(item.contentText?.slice(0, 50) || "Empty");
  const iconName = $derived(getContentTypeIcon(item.contentType));
  const isColor = $derived(item.contentType === "color");
  const isLink = $derived(item.contentType === "link");
  const isCode = $derived(item.contentType === "code");
  const isSvg = $derived(item.contentType === "svg");
  const isImage = $derived(item.contentType === "image");
  const isFile = $derived(item.contentType === "file");
  const languageInfo = $derived(
    isCode ? getLanguageInfo(item.codeLanguage) : null
  );

  // Parse metadata to get thumbnail path
  const parsedMetadata = $derived.by(() => {
    if (!item.contentMetadata) return null;
    try {
      return typeof item.contentMetadata === "string"
        ? JSON.parse(item.contentMetadata)
        : item.contentMetadata;
    } catch (e) {
      console.error("Failed to parse metadata:", e);
      return null;
    }
  });

  const thumbnailPath = $derived(parsedMetadata?.thumbnail_path || null);
  // Offered by another device and not fetched yet.
  const remoteOrigin = $derived<string | null>(
    parsedMetadata?.remote?.origin_name ?? null,
  );
  const originalName = $derived(
    parsedMetadata?.original_name || item.fileName || null
  );
  const previewType = $derived(parsedMetadata?.preview_type || null);
  const previewSource = $derived(parsedMetadata?.source || null);
  const textPreview = $derived(parsedMetadata?.text_preview || null);
  const textPreviewSnippet = $derived.by(() => {
    if (!textPreview) return null;
    return textPreview.trim().split("\n").slice(0, 6).join("\n");
  });
  let resolvedThumbnail = $state<string | null>(null);

  // Keep the keyboard-selected row visible while arrowing through the list.
  $effect(() => {
    if (isSelected) rowElement?.scrollIntoView({ block: "nearest" });
  });

  // Load favicon for links
  $effect(() => {
    if (isLink && item.contentText) {
      tauriExtractDomain(item.contentText)
        .then((domain) => {
          faviconUrl = `https://www.google.com/s2/favicons?domain=${domain}&sz=32`;
        })
        .catch(() => {
          faviconUrl = "";
        });
    } else {
      faviconUrl = "";
    }
  });

  // Handlers
  const handleClick = () => {
    uiStore.selectItem(item.id);
  };

  const handleDoubleClick = async () => {
    try {
      // Bump item to top (update timestamp)
      await tauriBumpItem(item.id);

      // Handle based on content type
      if (isImage && item.fileUrl) {
        // Copy image to clipboard
        await tauriWriteImageToClipboard(item.fileUrl);
        console.log("Image copied to clipboard");
        await clipboardStore.loadItems();
      } else if (item.contentText) {
        // Write to clipboard, hide window, and simulate Cmd+V in previous app
        await tauriPastefromClipboard(item.contentText);
        // Window is hidden by paste_item — skip loadItems(), monitor will handle the bump
      }
    } catch (err) {
      console.error("Failed to paste:", err);
    }
  };

  const handleKeyDown = (e: KeyboardEvent) => {
    if (e.key === "Enter" || e.key === " ") {
      e.preventDefault();
      handleClick();
    }
  };

  const handleToggleFavorite = async (e: MouseEvent) => {
    e.stopPropagation();
    await clipboardStore.toggleFavorite(item.id);
  };

  const handleFavoriteKeyDown = (e: KeyboardEvent) => {
    if (e.key === "Enter" || e.key === " ") {
      e.preventDefault();
      e.stopPropagation();
      clipboardStore.toggleFavorite(item.id);
    }
  };

  const handleDeleteClick = (e: MouseEvent) => {
    e.stopPropagation();
    showDeleteConfirm = true;
  };

  const handleConfirmDelete = async (e: MouseEvent) => {
    e.stopPropagation();
    isDeleting = true;
    try {
      await clipboardStore.deleteItem(item.id);
      if (isSelected) {
        uiStore.selectItem(null);
      }
    } catch (err) {
      console.error("Failed to delete:", err);
    } finally {
      isDeleting = false;
      showDeleteConfirm = false;
    }
  };

  const handleCancelDelete = (e: MouseEvent) => {
    e.stopPropagation();
    showDeleteConfirm = false;
  };

  async function computeResolvedThumbnail(): Promise<string | null> {
    if (typeof thumbnailPath === "string" && thumbnailPath.length > 0) {
      return thumbnailPath;
    }

    const canGenerateFromStore =
      isImage && item.fileUrl && previewSource !== "file";

    if (canGenerateFromStore) {
      const regenerated = await tauriEnsureThumbnail(item.fileUrl!);
      if (regenerated) {
        return regenerated;
      }
    }

    return null;
  }

  $effect(() => {
    // Re-run when dependencies change
    const _deps = [item.id, thumbnailPath, item.fileUrl, isImage, isFile];
    void _deps;

    let cancelled = false;
    (async () => {
      const candidate = await computeResolvedThumbnail();
      if (!cancelled) {
        resolvedThumbnail = candidate;
      }
    })();

    return () => {
      cancelled = true;
    };
  });
</script>

<div
  bind:this={rowElement}
  class={cn(
    "group relative flex items-center gap-3 px-1.5 py-0.5 text-sm cursor-pointer transition-colors mx-2",
    isSelected
      ? "bg-surface-hover rounded-xl"
      : " border-transparent hover:bg-surface-hover rounded-xl"
  )}
  onclick={handleClick}
  ondblclick={handleDoubleClick}
  onkeydown={handleKeyDown}
  role="button"
  tabindex="0"
>
  <!-- Icon with favorite toggle -->
  <div
    class="relative shrink-0 w-8 h-8 flex items-center justify-center transition-colors"
    onmouseenter={() => (isHoveringIcon = true)}
    onmouseleave={() => (isHoveringIcon = false)}
    onclick={handleToggleFavorite}
    onkeydown={handleFavoriteKeyDown}
    role="button"
    tabindex="0"
    aria-label={item.isFavorite ? "Remove from favorites" : "Add to favorites"}
  >
    {#if (isHoveringIcon || isSelected) && !item.isFavorite}
      <!-- Show white star on hover, or on the keyboard-selected row -->
      <Icon name="star" size={20} class="text-white" />
    {:else if item.isFavorite}
      <!-- Show filled yellow star if favorite -->
      <Icon
        name="starFilled"
        size={20}
        class="text-favorite"
        fill="currentColor"
      />
    {:else if isColor && item.contentText}
      <!-- Show color circle for color type -->
      <div
        class="w-6 h-6 rounded-full border-2 border-border"
        style="background-color: {item.contentText};"
      ></div>
    {:else if isLink && faviconUrl}
      <!-- Show favicon for link type -->
      <img
        src={faviconUrl}
        alt="Site icon"
        class="w-5 h-5 rounded"
        onerror={(e) => {
          (e.currentTarget as HTMLImageElement).style.display = "none";
          (e.currentTarget as HTMLImageElement).parentElement
            ?.querySelector(".fallback-icon")
            ?.classList.remove("hidden");
        }}
      />
      <Icon name="link" size={20} class="text-text hidden fallback-icon" />
    {:else if isCode && languageInfo}
      <!-- Show language SVG icon for code -->
      <div
        class="w-7 h-7 rounded flex items-center justify-center p-1"
        style="background-color: transparent;"
        title={languageInfo.name}
      >
        <img
          src={languageInfo.svgPath}
          alt={languageInfo.name}
          class="w-full h-full object-contain"
        />
      </div>
    {:else if isFile && previewType === "text" && textPreviewSnippet}
      <div
        class="size-7 rounded border border-border bg-surface-high overflow-hidden flex items-center justify-center"
        title="Text preview"
      >
        <span
          class="text-[8px] leading-tight font-mono text-text-muted px-0.5 overflow-hidden"
          style="-webkit-box-orient: vertical; display: -webkit-box; -webkit-line-clamp: 4;"
        >
          {textPreviewSnippet}
        </span>
      </div>
    {:else if resolvedThumbnail && (isImage || isFile)}
      <!-- Show actual thumbnail image -->
      <img
        src={convertFileSrc(resolvedThumbnail)}
        alt={isFile ? "File preview" : "Thumbnail"}
        class="w-7 h-7 rounded object-cover border border-border"
        onerror={(e) => {
          console.error("Failed to load thumbnail:", resolvedThumbnail);
          if (isImage && item.fileUrl && resolvedThumbnail !== item.fileUrl) {
            // Fallback to original image path when cached thumb is missing
            resolvedThumbnail = item.fileUrl;
            return;
          }
          resolvedThumbnail = null;
          (e.currentTarget as HTMLImageElement).style.display = "none";
        }}
      />
    {:else if isImage && !resolvedThumbnail}
      <!-- Fallback icon if no thumbnail available -->
      <div class="size-7 flex items-center justify-center">
        <svg
          xmlns="http://www.w3.org/2000/svg"
          width="1.5em"
          height="1.5em"
          viewBox="0 0 448 512"
          ><path
            fill="#db2777"
            d="M64 32C28.7 32 0 60.7 0 96v320c0 35.3 28.7 64 64 64h320c35.3 0 64-28.7 64-64V96c0-35.3-28.7-64-64-64zm64 80a48 48 0 1 1 0 96a48 48 0 1 1 0-96m144 112c8.4 0 16.1 4.4 20.5 11.5l88 144c4.5 7.4 4.7 16.7.5 24.3S368.7 416 360 416H88c-8.9 0-17.2-5-21.3-12.9s-3.5-17.5 1.6-24.8l56-80c4.5-6.4 11.8-10.2 19.7-10.2s15.2 3.8 19.7 10.2l26.4 37.8l61.4-100.5c4.4-7.1 12.1-11.5 20.5-11.5z"
          /></svg
        >
      </div>
    {:else if isSvg}
      <!-- Show SVG icon with badge -->
      <div class="relative size-7 flex items-center justify-center" title="SVG">
        <svg
          xmlns="http://www.w3.org/2000/svg"
          width="1.5em"
          height="1.5em"
          viewBox="0 0 32 32"
          ><path
            fill="#ffb300"
            d="M29.168 14.03a2.7 2.7 0 0 0-1.968-.83a2.51 2.51 0 0 0-1.929.8h-4.443l3.078-3.078a2.835 2.835 0 0 0 2.857-2.842a2.6 2.6 0 0 0-.831-1.969a2.82 2.82 0 0 0-2.014-.788a2.67 2.67 0 0 0-1.968.788a2.36 2.36 0 0 0-.812 1.922L18 11.17V6.726a2.51 2.51 0 0 0 .8-1.929a2.7 2.7 0 0 0-.832-1.968a2.745 2.745 0 0 0-3.936 0a2.7 2.7 0 0 0-.832 1.968a2.51 2.51 0 0 0 .8 1.93v4.443l-3.138-3.138a2.36 2.36 0 0 0-.812-1.922a2.66 2.66 0 0 0-1.968-.788a2.83 2.83 0 0 0-2.014.788a2.6 2.6 0 0 0-.831 1.969a2.74 2.74 0 0 0 .831 2.013a2.8 2.8 0 0 0 2.026.829l3.078 3.078H6.729a2.51 2.51 0 0 0-1.929-.8a2.7 2.7 0 0 0-1.968.831a2.745 2.745 0 0 0 0 3.937a2.7 2.7 0 0 0 1.968.832a2.51 2.51 0 0 0 1.929-.8h4.443l-3.078 3.077a2.835 2.835 0 0 0-2.857 2.842a2.6 2.6 0 0 0 .831 1.969a2.82 2.82 0 0 0 2.014.788a2.67 2.67 0 0 0 1.968-.788a2.36 2.36 0 0 0 .812-1.922L14 20.827v4.444a2.51 2.51 0 0 0-.8 1.929a2.784 2.784 0 0 0 4.768 1.968A2.7 2.7 0 0 0 18.8 27.2a2.51 2.51 0 0 0-.8-1.929v-4.444l3.138 3.138a2.36 2.36 0 0 0 .812 1.922a2.66 2.66 0 0 0 1.968.788a2.83 2.83 0 0 0 2.014-.788a2.6 2.6 0 0 0 .831-1.969a2.74 2.74 0 0 0-.831-2.013a2.8 2.8 0 0 0-2.026-.829L20.828 18h4.443a2.51 2.51 0 0 0 1.93.8a2.784 2.784 0 0 0 1.967-4.769Z"
          /></svg
        >
      </div>
    {:else if isFile}
      <div
        class="size-7 flex items-center justify-center rounded border border-border bg-surface"
      >
        <Icon name="file" size={18} class="text-text-muted" />
      </div>
    {:else}
      <!-- Show type icon -->
      <Icon name={iconName} size={20} class="text-text" />
    {/if}
  </div>

  <!-- Content text -->
  <div class="flex-1 min-w-0">
    {#if (isFile || isImage) && originalName}
      <p class="text-sm text-text truncate">
        {originalName}
        {#if remoteOrigin}
          <span class="text-xs text-text-muted">· on {remoteOrigin}</span>
        {/if}
      </p>
    {:else}
      <p class="text-sm text-text truncate">
        {displayText}
      </p>
    {/if}
  </div>

  <!-- Delete button or confirmation -->
  {#if !showDeleteConfirm}
    <button
      onclick={handleDeleteClick}
      class={cn(
        "shrink-0 opacity-0 group-hover:opacity-100 transition-opacity p-1 hover:bg-danger/20 rounded",
        isSelected && "opacity-100"
      )}
      aria-label="Delete item"
    >
      <Icon name="trash" size={16} class="text-text-muted hover:text-danger" />
    </button>
  {:else}
    <!-- Confirmation buttons -->
    <div class="flex items-center gap-1 shrink-0">
      <button
        onclick={handleConfirmDelete}
        disabled={isDeleting}
        class="p-1 hover:bg-danger/20 rounded transition-colors disabled:opacity-50"
        aria-label="Confirm delete"
      >
        <Icon name="check" size={16} class="text-danger" />
      </button>
      <button
        onclick={handleCancelDelete}
        disabled={isDeleting}
        class="p-1 hover:bg-surface rounded transition-colors disabled:opacity-50"
        aria-label="Cancel delete"
      >
        <Icon name="x" size={16} class="text-text-muted" />
      </button>
    </div>
  {/if}
</div>
