<script lang="ts">
  import ContentViewer from "$lib/components/content/ContentViewer.svelte";
  import ItemInfo from "$lib/components/content/ItemInfo.svelte";
  import Header from "$lib/components/header/Header.svelte";
  import Sidebar from "$lib/components/sidebar/Sidebar.svelte";
  import { clipboardStore } from "$lib/stores/clipboard.svelte";
  import { uiStore } from "$lib/stores/ui.svelte";
  import { tauriPastefromClipboard } from "$lib/tauri/commands";
  import type { ClipboardItem, ContentType } from "$lib/types";
  import { listen, type UnlistenFn } from "@tauri-apps/api/event";
  import { getCurrentWindow } from "@tauri-apps/api/window";
  import { onDestroy, onMount } from "svelte";

  // Local state
  let searchQuery = $state("");
  let filterType = $state<"all" | "favorites" | ContentType>("all");
  let debouncedSearchQuery = $state("");
  let searchTimeoutId: number | null = null;
  let unlistenClipboard: UnlistenFn | null = null;
  let unlistenRemoved: UnlistenFn | null = null;
  let unlistenFocus: UnlistenFn | null = null;
  let isSearching = $state(false);

  // Debounced search - Ahora busca en la base de datos
  $effect(() => {
    console.log("🔍 Search effect triggered. Query:", searchQuery);

    if (searchTimeoutId !== null) {
      clearTimeout(searchTimeoutId);
    }

    searchTimeoutId = window.setTimeout(async () => {
      debouncedSearchQuery = searchQuery;
      console.log("⏱️ Debounce complete. Searching for:", searchQuery);

      // Si hay query, buscar en base de datos
      if (searchQuery.trim()) {
        console.log("🔎 Starting database search for:", searchQuery.trim());
        isSearching = true;
        await clipboardStore.search(searchQuery.trim());
        isSearching = false;
        console.log(
          "✅ Search complete. Results:",
          clipboardStore.items.length,
        );
      } else {
        // Si no hay query, volver a cargar items normales
        console.log("🔄 No query, loading all items");
        await clipboardStore.loadItems();
      }
      // New results: start keyboard navigation (and Enter) at the top.
      selectFirstItem();
    }, 300);

    return () => {
      if (searchTimeoutId !== null) {
        clearTimeout(searchTimeoutId);
      }
    };
  });

  // Filtered items based on filter type only (search is handled by store)
  const filteredItems = $derived.by(() => {
    let items = clipboardStore.items;

    // Apply filter type (favorites, content type)
    if (filterType === "favorites") {
      items = items.filter((item) => item.isFavorite);
    } else if (filterType !== "all") {
      items = items.filter((item) => item.contentType === filterType);
    }

    return items;
  });

  // Selected item
  const selectedItem = $derived(
    clipboardStore.items.find((item) => item.id === uiStore.selectedItemId) ||
      null,
  );

  // Helper to deserialize clipboard item from event
  function deserializeClipboardItem(item: any): ClipboardItem {
    return {
      ...item,
      createdAt: new Date(item.created_at),
      updatedAt: new Date(item.updated_at),
      contentType: item.content_type,
      contentText: item.content_text,
      contentMetadata: item.content_metadata
        ? JSON.parse(item.content_metadata)
        : {},
      sourceApp: item.source_app,
      codeLanguage: item.code_language,
      fileUrl: item.file_url,
      fileName: item.file_name,
      fileSizeBytes: item.file_size_bytes,
      fileMimeType: item.file_mime_type,
      isFavorite: item.is_favorite,
      isSnippet: item.is_snippet,
      snippetName: item.snippet_name,
      synced: item.synced,
      serverId: item.server_id,
      fileHash: item.file_hash,
    };
  }

  function selectFirstItem() {
    uiStore.selectItem(filteredItems[0]?.id ?? null);
  }

  async function handlePaste() {
    const itemToPaste =
      filteredItems.find((item) => item.id === uiStore.selectedItemId) ??
      filteredItems[0];
    if (!itemToPaste?.contentText) return;

    await tauriPastefromClipboard(itemToPaste.contentText);
  }

  function moveSelection(delta: number) {
    if (filteredItems.length === 0) return;

    const current = filteredItems.findIndex(
      (item) => item.id === uiStore.selectedItemId,
    );
    const next =
      current === -1
        ? 0
        : Math.min(Math.max(current + delta, 0), filteredItems.length - 1);
    uiStore.selectItem(filteredItems[next].id);

    // Same infinite-scroll rule as Sidebar, so arrows can reach older items.
    const isNearEnd = next >= filteredItems.length - 5;
    if (isNearEnd && clipboardStore.hasMore && !clipboardStore.isLoadingMore) {
      if (debouncedSearchQuery.trim()) {
        clipboardStore.loadMoreSearchResults(debouncedSearchQuery.trim());
      } else {
        clipboardStore.loadMore();
      }
    }
  }

  // Esc undoes one thing at a time: clear the search, then the filter, then
  // hide the window. Components that own Esc (HotkeyRecorder) stop it first.
  function handleEscape() {
    if (searchQuery) {
      searchQuery = "";
    } else if (filterType !== "all") {
      filterType = "all";
    } else {
      getCurrentWindow()
        .hide()
        .catch((err) => console.error("Failed to hide window:", err));
    }
  }

  function handleWindowKeyDown(e: KeyboardEvent) {
    if (e.key === "Escape") {
      e.preventDefault();
      handleEscape();
      return;
    }

    // Buttons and other controls keep their own Enter/arrow behavior.
    const target = e.target as HTMLElement | null;
    if (target?.closest("button, select, textarea, [contenteditable='true']")) {
      return;
    }

    if (e.key === "ArrowDown" || e.key === "ArrowUp") {
      e.preventDefault();
      moveSelection(e.key === "ArrowDown" ? 1 : -1);
    } else if (e.key === "Enter") {
      e.preventDefault();
      handlePaste().catch((err) => console.error("Failed to paste:", err));
    }
  }

  // Load items on mount
  onMount(async () => {
    // Load initial clipboard items
    await clipboardStore.loadItems();

    // Listening for new clipboard items
    unlistenClipboard = await listen<any>("clipboard-item-added", (event) => {
      console.log("New clipboard item received:", event.payload);
      const newItem = deserializeClipboardItem(event.payload);

      // Check if item already exists (prevent duplicates)
      const existingIndex = clipboardStore.items.findIndex(
        (item) => item.id === newItem.id,
      );

      if (existingIndex !== -1) {
        // Item exists - replace it and move to top
        const updatedItems = [...clipboardStore.items];
        updatedItems.splice(existingIndex, 1); // Remove from old position
        updatedItems.unshift(newItem); // Add to beginning
        clipboardStore.items = updatedItems;
        console.log("Item bumped to top:", newItem.id);
      } else {
        // New item - add to beginning
        clipboardStore.items = [newItem, ...clipboardStore.items];
        console.log("New item added:", newItem.id);
      }
    });

    unlistenRemoved = await listen<string[]>(
      "clipboard-items-removed",
      (event) => {
        const removedIds = event.payload || [];
        if (!removedIds.length) return;

        console.log("🧹 Removing clipboard items:", removedIds);
        const toRemove = new Set(removedIds);
        clipboardStore.items = clipboardStore.items.filter(
          (item) => !toRemove.has(item.id),
        );
        clipboardStore.totalItems = Math.max(
          0,
          clipboardStore.totalItems - removedIds.length,
        );
        clipboardStore.hasMore =
          clipboardStore.items.length < clipboardStore.totalItems;
      },
    );

    // The window auto-hides on blur, so gaining focus means it was just opened:
    // start at the newest item so Enter pastes it, like before.
    unlistenFocus = await getCurrentWindow().onFocusChanged(
      ({ payload: focused }) => {
        if (focused) selectFirstItem();
      },
    );
  });

  // Cleanup on destroy
  onDestroy(() => {
    if (unlistenClipboard) {
      unlistenClipboard();
    }
    if (unlistenRemoved) {
      unlistenRemoved();
    }
    unlistenFocus?.();
  });
</script>

<svelte:window onkeydown={handleWindowKeyDown} />

<div
  class="h-screen w-[750px] overflow-hidden flex flex-col rounded-3xl bg-[rgba(18,18,20,0.71)]"
>
  <!-- Header -->
  <Header
    bind:searchQuery
    bind:filterType
    isSearching={isSearching || clipboardStore.isLoading}
    resultCount={debouncedSearchQuery.trim()
      ? clipboardStore.totalItems
      : undefined}
    isAuthenticated={false}
  />

  <!-- Main content area -->
  <div class="flex-1 flex overflow-hidden w-[750px]">
    <!-- Sidebar (40%) -->
    <Sidebar
      items={filteredItems}
      isLoading={clipboardStore.isLoading}
      searchQuery={debouncedSearchQuery}
    />

    <!-- Right panel (60%) -->
    <div class="flex flex-col w-[65%] relative">
      <!-- Content viewer (top) -->
      <ContentViewer item={selectedItem} searchQuery={debouncedSearchQuery} />

      <!-- Item info (bottom) -->
      <ItemInfo item={selectedItem} />
    </div>
  </div>
</div>
