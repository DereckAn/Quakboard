<script lang="ts">
  import { settingsStore } from "$lib/stores/settings.svelte";
  import {
    tauriCaptureFullScreenshot,
    tauriCaptureRegionScreenshot,
  } from "$lib/tauri/commands";
  import type { ContentType } from "$lib/types";
  import { onMount } from "svelte";
  import FilterDropdown from "./FilterDropdown.svelte";
  import ProfileDropdown from "./ProfileDropdown.svelte";
  import SearchBar from "./SearchBar.svelte";

  interface Props {
    searchQuery?: string;
    filterType?: "all" | "favorites" | ContentType;
    isAuthenticated?: boolean;
    isSearching?: boolean;
    resultCount?: number;
    onSearchChange?: (query: string) => void;
    onFilterChange?: (filter: "all" | "favorites" | ContentType) => void;
  }

  let {
    searchQuery = $bindable(""),
    filterType = $bindable<"all" | "favorites" | ContentType>("all"),
    isAuthenticated = false,
    isSearching = false,
    resultCount,
    onSearchChange,
    onFilterChange,
  }: Props = $props();

  let isCleaningDuplicates = $state(false);
  let isCapturing = $state(false);
  let screenshotMenuOpen = $state(false);

  const screenshotFeature = $derived.by(() =>
    settingsStore.labFeatures.find((f) => f.id === "screenshot")
  );

  const showScreenshotButton = $derived.by(
    () =>
      !!(
        screenshotFeature &&
        screenshotFeature.status === "installed" &&
        screenshotFeature.enabled
      )
  );

  // Handle search changes
  $effect(() => {
    if (onSearchChange) {
      onSearchChange(searchQuery);
    }
  });

  const handleFilterSelect = (value: "all" | "favorites" | ContentType) => {
    filterType = value;
    if (onFilterChange) {
      onFilterChange(value);
    }
  };

  onMount(() => {
    ensureLabState();
  });

  const ensureLabState = async () => {
    if (!settingsStore.labFeatures.length) {
      try {
        await settingsStore.refreshLabFeatures();
      } catch (err) {
        console.error("Failed to refresh lab features:", err);
      }
    }
  };

  const runCapture = async (mode: "full" | "region") => {
    if (isCapturing) return;
    isCapturing = true;
    screenshotMenuOpen = false;
    await ensureLabState();
    try {
      if (mode === "full") {
        await tauriCaptureFullScreenshot();
      } else {
        await tauriCaptureRegionScreenshot();
      }
    } catch (err) {
      console.error("Capture failed:", err);
    } finally {
      isCapturing = false;
    }
  };
</script>

<header
  data-tauri-drag-region
  class="h-12 border-border flex items-center gap-2 px-3 z-50"
>
  <!-- Profile dropdown -->
  <ProfileDropdown {isAuthenticated} />

  <!-- Search bar -->
  <SearchBar bind:value={searchQuery} {isSearching} {resultCount} />

  {#if showScreenshotButton}
    <div class="relative bg-surface rounded-md">
      <div
        class="flex items-center bg-surface-200/30 border border-border/70 rounded-md overflow-hidden"
      >
        <button
          class="h-7 px-2 border-l border-border/60 hover:bg-surface-hover"
          onclick={() => runCapture("full")}
          disabled={isCapturing}
        >
          <svg
            xmlns="http://www.w3.org/2000/svg"
            width="1em"
            height="1em"
            viewBox="0 0 24 24"
            ><path
              fill="currentColor"
              fill-rule="evenodd"
              d="M11 17H4a3 3 0 0 1-3-3V6a3 3 0 0 1 3-3h16a3 3 0 0 1 3 3v8a3 3 0 0 1-3 3h-7v2h3a1 1 0 1 1 0 2H8a1 1 0 1 1 0-2h3zM4 5h16a1 1 0 0 1 1 1v8a1 1 0 0 1-1 1H4a1 1 0 0 1-1-1V6a1 1 0 0 1 1-1"
              clip-rule="evenodd"
            /></svg
          >
          <!-- {isCapturing ? "Capturing..." : "Screenshot"} -->
        </button>
        <button
          class="h-7 px-2 border-l border-border/60 hover:bg-surface-hover"
          onclick={() => runCapture("region")}
          aria-label="Screenshot options"
          disabled={isCapturing}
        >
          <svg
            xmlns="http://www.w3.org/2000/svg"
            width="1em"
            height="1em"
            viewBox="0 0 24 24"
            ><path
              fill="currentColor"
              d="M5 3H3v2h2zm4 0H7v2h2zM7 19h2v2H7zM5 7H3v2h2zm14 0h2v2h-2zM5 11H3v2h2zm14 0h2v2h-2zM5 15H3v2h2zm14 0h2v2h-2zM5 19H3v2h2zm6-16h2v2h-2zm2 16h-2v2h2zm2-16h2v2h-2zm2 16h-2v2h2zm2-16h2v2h-2zm2 16h-2v2h2z"
            /></svg
          >
        </button>
      </div>
    </div>
  {/if}

  <!-- Filter dropdown -->
  <FilterDropdown selected={filterType} onSelect={handleFilterSelect} />
</header>
