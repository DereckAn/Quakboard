<script lang="ts">
  import { clipboardStore } from "$lib/stores/clipboard.svelte";
  import type { ClipboardItem } from "$lib/types";
  import { tick } from "svelte";
  import Icon from "../icons/Icon.svelte";
  import SidebarItem from "./SidebarItem.svelte";

  interface Props {
    items: ClipboardItem[];
    loadMore: () => Promise<void>;
  }

  let { items, loadMore }: Props = $props();
  let isCleaningDuplicates = $state(false);
  let groupElements = $state<Record<string, HTMLElement>>({});
  // "Older" is the last bucket, so nothing loaded later can follow it.
  const LAST_GROUP = "Older";

  // Función para agrupar items por fecha (usa updatedAt para agrupar)
  function groupItemsByDate(items: ClipboardItem[]) {
    const now = new Date();
    const today = new Date(now.getFullYear(), now.getMonth(), now.getDate());
    const yesterday = new Date(today);
    yesterday.setDate(yesterday.getDate() - 1);
    const thisWeek = new Date(today);
    thisWeek.setDate(thisWeek.getDate() - 7);
    const thisMonth = new Date(today);
    thisMonth.setMonth(thisMonth.getMonth() - 1);

    const groups: Record<string, ClipboardItem[]> = {
      Today: [],
      Yesterday: [],
      "This Week": [],
      "This Month": [],
      Older: [],
    };

    items.forEach((item) => {
      // Usar updatedAt en lugar de createdAt para agrupar
      // Así los items "bumped" aparecen en el grupo correcto
      const itemDate = new Date(item.updatedAt);
      const itemDay = new Date(
        itemDate.getFullYear(),
        itemDate.getMonth(),
        itemDate.getDate()
      );

      if (itemDay.getTime() === today.getTime()) {
        groups.Today.push(item);
      } else if (itemDay.getTime() === yesterday.getTime()) {
        groups.Yesterday.push(item);
      } else if (itemDay >= thisWeek) {
        groups["This Week"].push(item);
      } else if (itemDay >= thisMonth) {
        groups["This Month"].push(item);
      } else {
        groups.Older.push(item);
      }
    });

    // Filtrar grupos vacíos
    return Object.entries(groups).filter(([_, items]) => items.length > 0);
  }

  const handleCleanDuplicates = async () => {
    if (isCleaningDuplicates) return;

    isCleaningDuplicates = true;
    try {
      const deleted = await clipboardStore.removeDuplicates();
      console.log(`Removed ${deleted} duplicate items`);
    } catch (err) {
      console.error("Failed to clean duplicates:", err);
    } finally {
      isCleaningDuplicates = false;
    }
  };

  const groupedItems = $derived(groupItemsByDate(items));

  function scrollToGroup(index: number) {
    const group = groupedItems[index];
    if (!group) return;
    groupElements[group[0]]?.scrollIntoView({ behavior: "smooth", block: "start" });
  }

  function hasNextGroup(index: number) {
    return (
      index < groupedItems.length - 1 ||
      (clipboardStore.hasMore && groupedItems[index][0] !== LAST_GROUP)
    );
  }

  // The next separator may not be loaded yet: load pages until it shows up.
  // Stops when a page adds nothing (a load already running, or an error).
  async function jumpToNextGroup(index: number) {
    while (index === groupedItems.length - 1 && clipboardStore.hasMore) {
      const before = items.length;
      await loadMore();
      await tick();
      if (items.length === before) break;
    }
    scrollToGroup(index + 1);
  }
</script>

<div class="flex flex-col relative">
  {#each groupedItems as [groupName, groupItems], index}
    <div class="p-0" bind:this={groupElements[groupName]}>
      <!-- Group header -->
      <div
        class="px-3 py-1 sticky top-0 bg-transparent backdrop-blur-sm z-10 flex rounded-2xl items-center justify-between"
      >
        <div class="flex items-center gap-0.5">
          {#if index > 0}
            <button
              onclick={() => scrollToGroup(index - 1)}
              class="hover:bg-surface-hover rounded transition-colors text-text-muted"
              aria-label="Jump to {groupedItems[index - 1][0]}"
              title="Jump to {groupedItems[index - 1][0]}"
            >
              <Icon name="caretUp" size={16} fill="currentColor" strokeWidth={0} />
            </button>
          {/if}
          <h3
            class="text-xs font-semibold text-text-muted uppercase tracking-wide"
          >
            {groupName}
          </h3>
          {#if hasNextGroup(index)}
            <button
              onclick={() => jumpToNextGroup(index)}
              class="hover:bg-surface-hover rounded transition-colors text-text-muted"
              aria-label="Jump to the next section"
              title="Jump to the next section"
            >
              <Icon name="caretDown" size={16} fill="currentColor" strokeWidth={0} />
            </button>
          {/if}
        </div>
        <!-- Clean duplicates button -->
        <button
          onclick={handleCleanDuplicates}
          disabled={isCleaningDuplicates}
          class=" hover:bg-surface-hover rounded transition-colors disabled:opacity-50 px-1"
          title="Remove duplicate items"
        >
          <Icon name="trash" size={10} class="text-text-muted" />
        </button>
      </div>

      <!-- Group items -->
      <div class="">    
        {#each groupItems as item (item.id)}
          <SidebarItem {item} />
        {/each}
      </div>
    </div>
  {/each}
</div>