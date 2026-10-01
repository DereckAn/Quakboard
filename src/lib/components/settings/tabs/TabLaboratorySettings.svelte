<script lang="ts">
  import Icon from "$lib/components/icons/Icon.svelte";
  import Button from "$lib/components/ui/Button.svelte";
  import { settingsStore } from "$lib/stores/settings.svelte";
  import type { LabFeatureMeta, LabFeatureWithMeta, TabIcon } from "$lib/types";
  import { tauriChooseFolder } from "$lib/tauri/commands";

  const chooseFolder = async (which: "screenshots" | "recordings") => {
    const selected = await tauriChooseFolder(
      which === "screenshots"
        ? "Choose the screenshots folder"
        : "Choose the recordings folder",
      which === "screenshots"
        ? settingsStore.screenshotsDir
        : settingsStore.recordingsDir,
    );
    if (!selected) return;
    if (which === "screenshots") {
      await settingsStore.updateScreenshotsDir(selected);
    } else {
      await settingsStore.updateRecordingsDir(selected);
    }
    // If the watcher is running, restart it so the new path takes effect now.
    await settingsStore.reapplyWatcherIfActive();
  };

  const featureMeta: LabFeatureMeta[] = [
    {
      id: "screenshot",
      title: "Screenshot capture",
      description: "Take full or region screenshots and save to clipboard.",
      icon: "image",
      needsDownload: false,
    },
    {
      id: "ocr",
      title: "Image OCR",
      description: "Extract text from images. Downloads helper when installed.",
      icon: "search",
      needsDownload: true,
    },
    {
      id: "translator",
      title: "Text Translator",
      description: "Translate copied text to any language.",
      icon: "text",
      needsDownload: true,
    },
    {
      id: "pickColor",
      title: "Color Picker",
      description: "Pick colors from the screen and copy HEX/RGB.",
      icon: "color",
      needsDownload: true,
    },
  ];

  const mergeFeatures = (): LabFeatureWithMeta[] => {
    return featureMeta.map((meta) => {
      const state = settingsStore.labFeatures.find((f) => f.id === meta.id);
      return {
        ...meta,
        status: state?.status ?? "available",
        progress: state?.progress,
        error: state?.error,
        enabled: state?.enabled ?? false,
        installedVersion: state?.installedVersion,
      };
    });
  };

  let features = $state<LabFeatureWithMeta[]>(mergeFeatures());
  let busyId = $state<string | null>(null);

  $effect(() => {
    features = mergeFeatures();
  });

  const actionLabel = (feature: LabFeatureWithMeta) => {
    if (feature.progress !== undefined)
      return `Installing ${feature.progress}%`;
    if (feature.status === "installed") return "Uninstall";
    return "Install";
  };

  const isInstalling = (feature: LabFeatureWithMeta) =>
    feature.progress !== undefined || busyId === feature.id;

  const handleInstall = async (id: LabFeatureWithMeta["id"]) => {
    busyId = id;
    try {
      await settingsStore.installFeature(id);
    } catch (err) {
      console.error("Failed to install feature:", err);
    } finally {
      await settingsStore.refreshLabFeatures();
      busyId = null;
    }
  };

  const handleUninstall = async (id: LabFeatureWithMeta["id"]) => {
    busyId = id;
    try {
      await settingsStore.uninstallFeature(id);
    } catch (err) {
      console.error("Failed to uninstall feature:", err);
    } finally {
      await settingsStore.refreshLabFeatures();
      busyId = null;
    }
  };

  const handleToggle = async (feature: LabFeatureWithMeta) => {
    busyId = feature.id;
    try {
      await settingsStore.enableFeature(feature.id, !feature.enabled);
    } catch (err) {
      console.error("Failed to toggle feature:", err);
    } finally {
      await settingsStore.refreshLabFeatures();
      busyId = null;
    }
  };
</script>

<section
  class="relative bg-surface rounded-2xl border border-border/60 p-6 space-y-6"
>
  <header class="flex items-center justify-between">
    <div class="flex items-center gap-2">
      <Icon name="flask" size={20} />
      <div>
        <h2 class="text-lg font-semibold">Laboratory</h2>
        <p class="text-xs text-text-muted">
          Experimental add-ons you can install or remove anytime.
        </p>
      </div>
    </div>
  </header>

  <div class="border-b border-border/70">
    <div class="py-4 flex items-start justify-between gap-4">
      <div class="flex items-start gap-3">
        <div
          class="h-11 w-11 rounded-xl bg-surface-200/30 border border-border/60 flex items-center justify-center shrink-0"
        >
          <Icon name="image" size={20} />
        </div>
        <div>
          <div class="flex items-center gap-2">
            <p class="text-sm font-semibold text-text">Capture folders</p>
            {#if settingsStore.watchFoldersEnabled}
              <span
                class="text-[10px] px-2 py-0.5 rounded-full bg-success/10 text-success uppercase tracking-wide"
              >
                Active
              </span>
            {/if}
          </div>
          <p class="text-[10px] text-text-muted leading-relaxed">
            Watch these folders and store new screenshots/recordings as pointers
            (no byte copy). When on, raw clipboard screenshots aren't saved
            separately. Takes effect immediately.
          </p>

          <div class="mt-3 space-y-2">
            <div class="flex items-center gap-2">
              <span class="text-[11px] text-text-muted w-24 shrink-0">
                Screenshots
              </span>
              <span
                class="text-[11px] text-text truncate flex-1"
                title={settingsStore.screenshotsDir || "~/Pictures"}
              >
                {settingsStore.screenshotsDir || "~/Pictures"}
              </span>
              <Button
                variant="other"
                class="h-fit text-xs bg-white/5 hover:bg-blue-500/20 rounded-md px-2 py-1"
                onclick={() => chooseFolder("screenshots")}
              >
                <Icon name="image" size={14} />
                Choose
              </Button>
            </div>
            <div class="flex items-center gap-2">
              <span class="text-[11px] text-text-muted w-24 shrink-0">
                Recordings
              </span>
              <span
                class="text-[11px] text-text truncate flex-1"
                title={settingsStore.recordingsDir || "~/Videos"}
              >
                {settingsStore.recordingsDir || "~/Videos"}
              </span>
              <Button
                variant="other"
                class="h-fit text-xs bg-white/5 hover:bg-blue-500/20 rounded-md px-2 py-1"
                onclick={() => chooseFolder("recordings")}
              >
                <Icon name="image" size={14} />
                Choose
              </Button>
            </div>
          </div>
        </div>
      </div>

      <button
        class={`relative inline-flex h-5 w-9 shrink-0 items-center rounded-full transition-colors ${
          settingsStore.watchFoldersEnabled ? "bg-primary" : "bg-border"
        }`}
        onclick={() => settingsStore.toggleWatchFolders()}
        aria-label="Toggle capture-folder watching"
      >
        <span
          class={`inline-block h-4 w-4 transform rounded-full bg-white transition-transform ${
            settingsStore.watchFoldersEnabled ? "translate-x-4" : "translate-x-1"
          }`}
        ></span>
      </button>
    </div>
  </div>

  <div class="divide-y divide-border/70">
    {#each features as feature (feature.id)}
      <div class="py-4 flex items-start justify-between gap-4">
        <div class="flex items-start gap-3">
          <div
            class="h-11 w-11 rounded-xl bg-surface-200/30 border border-border/60 flex items-center justify-center shrink-0"
          >
            <Icon name={feature.icon as TabIcon} size={20} />
          </div>
          <div>
            <div class="flex items-center gap-2">
              <p class="text-sm font-semibold text-text">{feature.title}</p>
              {#if feature.needsDownload}
                <span
                  class="text-[10px] px-2 py-0.5 rounded-full border border-border/60 text-text-muted uppercase tracking-wide"
                >
                  Download
                </span>
              {/if}
              {#if feature.status === "installed"}
                <span
                  class="text-[10px] px-2 py-0.5 rounded-full bg-success/10 text-success uppercase tracking-wide"
                >
                  Installed
                </span>
              {/if}
            </div>
            <p class="text-[10px] text-text-muted leading-relaxed">
              {feature.description}
            </p>
            {#if feature.error}
              <p class="text-xs text-danger mt-2">Error: {feature.error}</p>
            {/if}
            {#if feature.status === "installed"}
              <div class="mt-3 flex items-center gap-2">
                <span class="text-[11px] text-text-muted">Enabled</span>
                <button
                  class={`relative inline-flex h-5 w-9 items-center rounded-full transition-colors ${
                    feature.enabled ? "bg-primary" : "bg-border"
                  }`}
                  onclick={() => handleToggle(feature)}
                  disabled={isInstalling(feature)}
                  aria-label={`Toggle ${feature.title}`}
                >
                  <span
                    class={`inline-block h-4 w-4 transform rounded-full bg-white transition-transform ${
                      feature.enabled ? "translate-x-4" : "translate-x-1"
                    }`}
                  ></span>
                </button>
              </div>
            {/if}
          </div>
        </div>
        <div class="flex items-center gap-2">
          {#if feature.status === "installed"}
            <Button
              variant="destructive"
              onclick={() => handleUninstall(feature.id)}
              disabled={isInstalling(feature)}
            >
              <Icon name="trash" size={16} />
              {isInstalling(feature) ? "Working..." : "Uninstall"}
            </Button>
          {:else}
            <Button
              onclick={() => handleInstall(feature.id)}
              disabled={isInstalling(feature)}
              variant="other"
              class="h-fit text-sm bg-white/5 hover:bg-blue-500/20 rounded-md z-10"
            >
              {actionLabel(feature)}
            </Button>
          {/if}
        </div>
      </div>
    {/each}
  </div>
</section>
