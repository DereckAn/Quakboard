<script lang="ts">
  import { clipboardStore } from "$lib/stores/clipboard.svelte";
  import type { ClipboardItem } from "$lib/types";
  import SidebarGroup from "./SidebarGroup.svelte";

  interface Props {
    items: ClipboardItem[];
    isLoading?: boolean;
    searchQuery?: string;
  }

  let { items, isLoading = false, searchQuery = "" }: Props = $props();

  let scrollContainer: HTMLDivElement;

  // Next page of whatever is listed: search results or the plain history.
  const loadMore = () =>
    searchQuery.trim()
      ? clipboardStore.loadMoreSearchResults(searchQuery.trim())
      : clipboardStore.loadMore();

  const handleScroll = (e: Event) => {
    const target = e.target as HTMLDivElement;
    const scrollTop = target.scrollTop;
    const scrollHeight = target.scrollHeight;
    const clientHeight = target.clientHeight;

    // Si está cerca del fondo (100px antes), cargar más
    if (scrollHeight - scrollTop - clientHeight < 100) {
      if (clipboardStore.hasMore && !clipboardStore.isLoadingMore) {
        loadMore();
      }
    }
  };
</script>

<div
  bind:this={scrollContainer}
  onscroll={handleScroll}
  class="min-w-sidebar max-w-sidebar rounded-2xl mb-3 ml-3 border border-border flex flex-col overflow-y-auto bg-surface"
>
  {#if isLoading && items.length === 0}
    <div class="flex-1 flex items-center justify-center">
      <div class="text-text-muted">Loading...</div>
    </div>
  {:else if items.length === 0}
    <div class="flex-1 flex items-center justify-center px-6 text-center">
      <div>
        {#if searchQuery.trim()}
          <!-- No search results -->
          <p class="text-text-muted">No results found</p>
          <p class="text-xs text-text-muted mt-2">
            Try a different search term
          </p>
        {:else}
          <!-- No items at all -->
          <p class="text-text-muted">No items yet</p>
          <p class="text-xs text-text-muted mt-2">
            Copy something to get started
          </p>
        {/if}
      </div>
    </div>
  {:else}
    <div class="flex-1">
      <SidebarGroup {items} {loadMore} />
    </div>

    <!-- Loading more indicator -->
    {#if clipboardStore.isLoadingMore}
      <div class="p-4 text-center">
        <div class="text-xs text-text-muted">Loading more...</div>
      </div>
    {/if}

    <!-- No more items indicator -->
    {#if !clipboardStore.hasMore && items.length > 0}
      <div class="p-4 text-center">
        <div class="text-xs text-text-muted">
          All items loaded ({clipboardStore.totalItems})
        </div>
      </div>
    {/if}
  {/if}
</div>
