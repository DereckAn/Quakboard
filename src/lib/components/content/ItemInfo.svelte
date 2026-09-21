<script lang="ts">
  import Icon from "$lib/components/icons/Icon.svelte";
  import type { ClipboardItem } from "$lib/types";
  import { formatTimestamp } from "$lib/utils/date";
  import {
    detectColorFormat,
    formatFileSize,
    getCharacterCount,
    getContentTypeIcon,
    getWordCount,
  } from "$lib/utils/format";
  import { getLanguageInfo } from "$lib/utils/languages";

  interface Props {
    item: ClipboardItem | null;
  }

  let { item }: Props = $props();

  const characterCount = $derived(getCharacterCount(item?.contentText ?? null));
  const wordCount = $derived(getWordCount(item?.contentText ?? null));
  const colorFormat = $derived(
    item?.contentType === "color" && item.contentText
      ? detectColorFormat(item.contentText)
      : null
  );
  const languageInfo = $derived(
    item?.contentType === "code" && item.codeLanguage
      ? getLanguageInfo(item.codeLanguage)
      : null
  );
  const parsedMetadata = $derived.by(() => {
    if (!item?.contentMetadata) return null;
    try {
      return typeof item.contentMetadata === "string"
        ? JSON.parse(item.contentMetadata)
        : item.contentMetadata;
    } catch (err) {
      console.error("Failed to parse metadata:", err);
      return null;
    }
  });
  const isFile = $derived(item?.contentType === "file");
  const isImage = $derived(item?.contentType === "image");
  const imageDimensions = $derived(
    isImage && parsedMetadata?.width && parsedMetadata?.height
      ? `${parsedMetadata.width} × ${parsedMetadata.height} px`
      : null
  );
  const isExternalImage = $derived(isImage && parsedMetadata?.source === "file");
</script>

<div class="absolute bottom-0 left-0 right-0 h-44 mx-3 mb-3 border border-border bg-surface rounded-2xl flex flex-col ">
  {#if !item}
    <!-- Empty state -->
    <div class="flex-1 flex items-center justify-center px-6 text-center">
      <p class="text-sm text-text-muted">No information to display</p>
    </div>
  {:else}
    <!-- Info grid -->
    <div
      class="flex-1 overflow-y-auto px-3 py-3 text-xs divide-y divide-white/5"
    >
      <!-- Content Type -->
      <div class="flex items-center justify-between p-1.5">
        <dt class="text-text-muted">Content type</dt>
        <dd class="flex items-end text-text font-medium gap-2">
          <Icon name={getContentTypeIcon(item.contentType)} size={16} />
          <span class="capitalize">{item.contentType}</span>
        </dd>
      </div>

      <!-- Created At -->
      <div class="flex items-center justify-between p-1.5">
        <dt class="text-text-muted flex items-center gap-2">
          <Icon name="clock" size={14} />
          <span>Created</span>
        </dt>
        <dd class="text-text">{formatTimestamp(item.createdAt)}</dd>
      </div>

      <!-- Characters -->
      <div class="flex items-center justify-between p-1.5">
        <dt class="text-text-muted">Characters</dt>
        <dd class="text-text">{characterCount.toLocaleString()}</dd>
      </div>

      <!-- Words (only for text/code) -->
      {#if item.contentType === "text" || item.contentType === "code"}
        <div class="flex items-center justify-between p-1.5">
          <dt class="text-text-muted">Words</dt>
          <dd class="text-text">{wordCount.toLocaleString()}</dd>
        </div>
      {/if}

      <!-- Source App -->
      {#if item.sourceApp}
        <div class="flex items-center justify-between p-1.5">
          <dt class="text-text-muted">Source</dt>
          <dd class="text-text">{item.sourceApp}</dd>
        </div>
      {/if}

      <!-- File size (if available) -->
      {#if item.fileSizeBytes !== null}
        <div class="flex items-center justify-between p-1.5">
          <dt class="text-text-muted">Size</dt>
          <dd class="text-text">{formatFileSize(item.fileSizeBytes)}</dd>
        </div>
      {/if}

      {#if imageDimensions}
        <div class="flex items-center justify-between p-1.5">
          <dt class="text-text-muted">Dimensions</dt>
          <dd class="text-text">{imageDimensions}</dd>
        </div>
      {/if}

      {#if isExternalImage}
        <div class="flex items-center justify-between p-1.5">
          <dt class="text-text-muted">Preview</dt>
          <dd class="text-text">Quick Look cache</dd>
        </div>
      {/if}

      <!-- Color format (if color) -->
      {#if colorFormat}
        <div class="flex items-center justify-between p-1.5">
          <dt class="text-text-muted">Format</dt>
          <dd class="text-text">{colorFormat}</dd>
        </div>
      {/if}

      <!-- Code language (if code) -->
      {#if languageInfo}
        <div class="flex items-center justify-between p-1.5">
          <dt class="text-text-muted">Language</dt>
          <dd class="text-text flex items-center gap-1">
            <img
              src={languageInfo.svgPath}
              alt={languageInfo.name}
              class="w-4 h-4 object-contain"
            />
            <span>{languageInfo.name}</span>
          </dd>
        </div>
      {/if}

      <div class="flex justify-between p-1.5">
        <span class="text-text-muted">File name</span>
        <span class="font-medium"
          >{parsedMetadata?.original_name || item.fileName || "-"}</span
        >
      </div>
      {#if isFile}
        <li class="flex justify-between p-1.5">
          <span class="text-text-muted">Type</span>
          <span class="font-medium"
            >{item.fileMimeType ||
              parsedMetadata?.original_extension ||
              "Unknown"}</span
          >
        </li>
        <li class="flex justify-between p-1.5">
          <span class="text-text-muted">Size</span>
          <span class="font-medium">
            {item.fileSizeBytes
              ? (item.fileSizeBytes / 1024).toFixed(1) + " KB"
              : "—"}
          </span>
        </li>
      {/if}

      <!-- Favorite status -->
      <div class="flex items-center justify-between p-1.5">
        <dt class="text-text-muted">Favorite</dt>
        <dd class="text-text flex items-center gap-2">
          {#if item.isFavorite}
            <Icon
              name="starFilled"
              size={14}
              class="text-favorite"
              fill="currentColor"
              strokeWidth={0}
            />
            <span>Yes</span>
          {:else}
            <span>No</span>
          {/if}
        </dd>
      </div>
    </div>
  {/if}
</div>
