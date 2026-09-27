<script lang="ts">
  import { goto } from "$app/navigation";
  import SettingsContent from "$lib/components/settings/SettingsContent.svelte";
  import SettingsSidebar from "$lib/components/settings/SettingsSidebar.svelte";
  import { settingsStore } from "$lib/stores/settings.svelte";
  import type { TabIcon, TabId } from "$lib/types";
  import { onMount } from "svelte";

  let globalHotkey = $state(settingsStore.hotkey);
  let activeTab = $state<TabId>("account");

  const tabs: { id: TabId; label: string; icon: TabIcon }[] = [
    { id: "account", label: "Account", icon: "user" },
    { id: "hotkeys", label: "Hotkeys", icon: "text" },
    { id: "items", label: "Item Management", icon: "trash" },
    { id: "memory", label: "Usage", icon: "database" },
    { id: "privacy", label: "Privacy", icon: "lock" },
    { id: "application", label: "Application", icon: "settings" },
    { id: "laboratory", label: "Laboratory", icon: "flask" },
    { id: "about", label: "About", icon: "info" },
  ];

  onMount(async () => {
    await settingsStore.loadSettings();
    await settingsStore.loadDatabaseStats();
    globalHotkey = settingsStore.hotkey;
  });

  const handleBack = () => {
    goto("/");
  };

  const handleWindowKeyDown = (e: KeyboardEvent) => {
    if (e.key === "Escape") {
      e.preventDefault();
      handleBack();
    }
  };
</script>

<svelte:window onkeydown={handleWindowKeyDown} />

<div
  class="h-screen w-screen overflow-hidden flex flex-col bg-surface rounded-3xl relative p-3"
>
  <div
    class="absolute top-0 left-0 right-0 bottom-0"
    data-tauri-drag-region
  ></div>

  <div class="flex-1 flex overflow-hidden gap-3">
    <!-- Sidebar -->
    <SettingsSidebar
      {tabs}
      {activeTab}
      onTabChange={(tabId) => (activeTab = tabId as TabId)}
      onBack={handleBack}
    />

    <!-- Content -->
    <SettingsContent {activeTab} onBack={handleBack} />
  </div>
</div>
