import type {
  ClipboardItem,
  CreateClipboardItemDto,
  LabFeatureId,
  LabFeatureWithMeta,
  NearbyDevice,
  SendFileResult,
  SyncPeer,
  SyncStatus,
  UpdateClipboardItemDto,
} from "$lib/types";
import { invoke } from "@tauri-apps/api/core";

// Helper to deserialize dates from strings
function deserializeClipboardItem(item: any): ClipboardItem {
  return {
    ...item,
    createdAt: new Date(item.created_at),
    updatedAt: new Date(item.updated_at),
    // Map snake_case from Rust to camelCase for TypeScript
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

// Removed: Use tauriGetItemsPaginated() instead for better performance

export async function tauriGetItem(id: string): Promise<ClipboardItem | null> {
  const item = await invoke<any | null>("get_clipboard_item", { id });
  return item ? deserializeClipboardItem(item) : null;
}

export async function tauriCreateItem(
  dto: CreateClipboardItemDto
): Promise<ClipboardItem> {
  // Convert to snake_case for Rust
  const rustDto = {
    content_type: dto.contentType,
    content_text: dto.contentText,
    content_metadata: dto.contentMetadata
      ? JSON.stringify(dto.contentMetadata)
      : undefined,
    source_app: undefined, // Will be detected by backend
  };

  const item = await invoke<any>("create_clipboard_item", { dto: rustDto });
  return deserializeClipboardItem(item);
}

/**
 * Upsert item - creates new or bumps existing to top (prevents duplicates)
 */
export async function tauriUpsertItem(
  dto: CreateClipboardItemDto
): Promise<ClipboardItem> {
  // Convert to snake_case for Rust
  const rustDto = {
    content_type: dto.contentType,
    content_text: dto.contentText,
    content_metadata: dto.contentMetadata
      ? JSON.stringify(dto.contentMetadata)
      : undefined,
    source_app: undefined, // Will be detected by backend
  };

  const item = await invoke<any>("upsert_clipboard_item", { dto: rustDto });
  return deserializeClipboardItem(item);
}

/**
 * Bump item to top (update timestamp)
 */
export async function tauriBumpItem(id: string): Promise<ClipboardItem> {
  const item = await invoke<any>("bump_clipboard_item", { id });
  return deserializeClipboardItem(item);
}

export async function tauriUpdateItem(
  id: string,
  dto: UpdateClipboardItemDto
): Promise<ClipboardItem> {
  // Convert to snake_case for Rust
  const rustDto = {
    content_text: dto.contentText,
    is_favorite: dto.isFavorite,
    is_snippet: dto.isSnippet,
    snippet_name: dto.snippetName,
  };

  const item = await invoke<any>("update_clipboard_item", { id, dto: rustDto });
  return deserializeClipboardItem(item);
}

export async function tauriDeleteItem(id: string): Promise<void> {
  await invoke("delete_clipboard_item", { id });
}

export async function tauriClearAllItems(): Promise<void> {
  await invoke("clear_all_clipboard_items");
}

export async function tauriWriteToClipboard(text: string): Promise<void> {
  await invoke("write_to_clipboard", { text });
}

export async function tauriPastefromClipboard(text: string): Promise<void> {
  await invoke("paste_item", { text });
}

export async function tauriReadFromClipboard(): Promise<string> {
  return await invoke<string>("read_from_clipboard");
}

// Color conversion types
export interface ColorFormat {
  name: string;
  value: string;
  original: boolean;
}

export interface ColorConversion {
  formats: ColorFormat[];
  rgb_preview: string;
}

export interface LinkMetadata {
  title: string | null;
  description: string | null;
  image: string | null;
  site_name: string | null;
}

export interface CleanupSettings {
  maxItemsEnabled: boolean;
  maxLocalItems: number;
  retentionEnabled: boolean;
  retentionDays: number;
}

// Color conversion
export async function tauriConvertColor(
  color: string
): Promise<ColorConversion> {
  return await invoke("convert_color_formats", { color });
}

// Extract domain from URL
export async function tauriExtractDomain(url: string): Promise<string> {
  return await invoke("extract_domain_from_url", { url });
}

// Fetch link metadata
export async function tauriFetchLinkMetadata(
  url: string
): Promise<LinkMetadata> {
  return await invoke("fetch_link_metadata", { url });
}

// Remove duplicate items
export async function tauriRemoveDuplicates(): Promise<number> {
  return await invoke("remove_duplicate_items");
}

export async function tauriCleanupMissingFiles(): Promise<string[]> {
  return await invoke("cleanup_missing_clipboard_files");
}

export async function tauriGetCleanupSettings(): Promise<CleanupSettings> {
  const result = await invoke<{
    max_items_enabled: boolean;
    max_local_items: number;
    retention_enabled: boolean;
    retention_days: number;
  }>("get_cleanup_settings");

  return {
    maxItemsEnabled: result.max_items_enabled,
    maxLocalItems: result.max_local_items,
    retentionEnabled: result.retention_enabled,
    retentionDays: result.retention_days,
  };
}

// Settings
export async function tauriGetSetting(key: string): Promise<string> {
  return await invoke("get_setting", { key });
}

export async function tauriSaveSetting(
  key: string,
  value: string
): Promise<void> {
  await invoke("save_setting", { key, value });
}

export async function tauriSetFolderWatcher(enabled: boolean): Promise<void> {
  await invoke("set_folder_watcher", { enabled });
}

export async function tauriUpdateGlobalHotkey(
  newHotkey: string
): Promise<void> {
  await invoke("update_global_hotkey", { newHotkey });
}

export async function tauriGetCurrentShortcut(): Promise<string> {
  return await invoke("get_current_shortcut");
}

export async function tauriUnregisterShortcut(): Promise<void> {
  await invoke("unregister_shortcut");
}

export async function tauriSaveCleanupSettings(
  maxItemsEnabled: boolean,
  maxLocalItems: number,
  retentionEnabled: boolean,
  retentionDays: number
): Promise<void> {
  await invoke("save_cleanup_settings", {
    maxItemsEnabled,
    maxLocalItems,
    retentionEnabled,
    retentionDays,
  });
}

// Pagination
export async function tauriGetItemsPaginated(
  limit: number,
  offset: number
): Promise<ClipboardItem[]> {
  const items = await invoke<any[]>("get_clipboard_items_paginated", {
    limit,
    offset,
  });
  return items.map(deserializeClipboardItem);
}

export async function tauriCountItems(): Promise<number> {
  return await invoke<number>("count_clipboard_items");
}

export async function tauriSearchItemsFTS(
  query: string,
  limit: number,
  offset: number
): Promise<ClipboardItem[]> {
  console.log("🔌 tauriSearchItemsFTS called:", { query, limit, offset });
  const items = await invoke<any[]>("search_clipboard_items_fts", {
    query,
    limit,
    offset,
  });
  console.log("✅ tauriSearchItemsFTS returned", items.length, "items");
  return items.map(deserializeClipboardItem);
}

export async function tauriCountSearchResultsFTS(
  query: string
): Promise<number> {
  console.log("🔌 tauriCountSearchResultsFTS called:", query);
  const count = await invoke<number>("count_search_results_fts", {
    query,
  });
  console.log("✅ tauriCountSearchResultsFTS returned:", count);
  return count;
}

// Settings y limpieza
export async function tauriCleanupOldItems(
  retentionDays: number | null
): Promise<number> {
  return await invoke<number>("cleanup_old_items", {
    retentionDays,
  });
}

export async function tauriCleanupExcessItems(
  maxItems: number | null
): Promise<number> {
  return await invoke<number>("cleanup_excess_items", {
    maxItems,
  });
}

export async function tauriGetDatabaseSize(): Promise<number> {
  return await invoke<number>("get_database_size");
}

export async function tauriOptimizeDatabase(): Promise<void> {
  await invoke("optimize_database");
}

export interface DatabaseStats {
  total_items: number;
  favorites: number;
  snippets: number;
  database_size_bytes: number;
  database_size_mb: number;
}

export async function tauriGetDatabaseStats(): Promise<DatabaseStats> {
  return await invoke<DatabaseStats>("get_database_stats");
}

// Test commands for cleanup verification
export interface CleanupPreview {
  retention?: {
    would_delete: number;
    cutoff_date: string;
    retention_days: number;
  };
  excess?: {
    current_count: number;
    max_items: number;
    would_delete: number;
  };
}

export interface CleanupResult {
  retention_deleted?: number;
  excess_deleted?: number;
}

export async function tauriTestCleanupPreview(
  retentionDays: number | null,
  maxItems: number | null
): Promise<CleanupPreview> {
  return await invoke<CleanupPreview>("test_cleanup_preview", {
    retentionDays,
    maxItems,
  });
}

export async function tauriTestForceCleanup(): Promise<CleanupResult> {
  return await invoke<CleanupResult>("test_force_cleanup");
}

//Auto start commands
export async function tauriEnableAutoStart(): Promise<void> {
  await invoke("enable_autostart");
}

export async function tauriDisableAutoStart(): Promise<void> {
  await invoke("disable_autostart");
}

export async function tauriIsAutoStartEnabled(): Promise<boolean> {
  return await invoke("is_autostart_enabled");
}

export async function tauriQuitApp(): Promise<void> {
  console.log("🔵 tauriQuitApp() - calling invoke('quit_app')");
  try {
    await invoke("quit_app");
    console.log("✅ invoke('quit_app') completed");
  } catch (error) {
    console.error("❌ invoke('quit_app') failed:", error);
    throw error;
  }
}

// tray visibility commands
export async function tauriSetTrayVisible(visible: boolean): Promise<void> {
  await invoke<void>("set_tray_visible", { visible });
}

export async function tauriIsTrayVisible(): Promise<boolean> {
  return await invoke<boolean>("is_tray_visible");
}

export async function tauriWriteImageToClipboard(imagePath: string): Promise<void> {
  await invoke("write_image_to_clipboard", { imagePath });
}

export async function tauriEnsureThumbnail(imagePath: string): Promise<string | null> {
  try {
    const result = await invoke<string>("ensure_thumbnail", { filePath: imagePath });
    return result;
  } catch (err) {
    console.error("Failed to ensure thumbnail:", err);
    return null;
  }
}

export async function tauriWriteFileToClipboard(filePath: string): Promise<void> {
    await invoke("write_file_to_clipboard", { path: filePath });
}

// Laboratory features
export async function tauriGetLabFeatures(): Promise<LabFeatureWithMeta[]> {
  return await invoke<LabFeatureWithMeta[]>("get_lab_features");
}

export async function tauriInstallFeature(
  id: LabFeatureId
): Promise<LabFeatureWithMeta> {
  return await invoke<LabFeatureWithMeta>("install_feature", { id });
}

export async function tauriUninstallFeature(
  id: LabFeatureId
): Promise<LabFeatureWithMeta> {
  return await invoke<LabFeatureWithMeta>("uninstall_feature", { id });
}

export async function tauriEnableFeature(
  id: LabFeatureId,
  enabled: boolean
): Promise<LabFeatureWithMeta> {
  return await invoke<LabFeatureWithMeta>("enable_feature", { id, enabled });
}

export async function tauriCaptureFullScreenshot(): Promise<void> {
  await invoke("capture_full_screenshot");
}

export async function tauriCaptureRegionScreenshot(): Promise<void> {
  await invoke("capture_region_screenshot");
}

export async function tauriUpdateScreenshotHotkeys(
  fullHotkey: string,
  regionHotkey: string
): Promise<void> {
  await invoke("update_screenshot_hotkeys", { fullHotkey, regionHotkey });
}

export async function tauriUnregisterScreenshotHotkeys(): Promise<void> {
  await invoke("unregister_screenshot_hotkeys");
}
// LAN sync
export async function tauriSyncGetStatus(): Promise<SyncStatus> {
  return await invoke<SyncStatus>("sync_get_status");
}

export async function tauriSyncSetEnabled(enabled: boolean): Promise<SyncStatus> {
  return await invoke<SyncStatus>("sync_set_enabled", { enabled });
}

export async function tauriSyncSetImagesEnabled(
  enabled: boolean,
): Promise<SyncStatus> {
  return await invoke<SyncStatus>("sync_set_images_enabled", { enabled });
}

export async function tauriSyncListPeers(): Promise<SyncPeer[]> {
  return await invoke<SyncPeer[]>("sync_list_peers");
}

export async function tauriSyncListNearby(): Promise<NearbyDevice[]> {
  return await invoke<NearbyDevice[]>("sync_list_nearby");
}

export async function tauriSyncSetAcceptingPairing(accepting: boolean): Promise<void> {
  await invoke("sync_set_accepting_pairing", { accepting });
}

export async function tauriSyncPairStart(deviceId: string): Promise<void> {
  await invoke("sync_pair_start", { deviceId });
}

export async function tauriSyncPairByAddress(address: string): Promise<void> {
  await invoke("sync_pair_by_address", { address });
}

export async function tauriSyncPairSubmitCode(code: string): Promise<void> {
  await invoke("sync_pair_submit_code", { code });
}

export async function tauriSyncPairCancel(): Promise<void> {
  await invoke("sync_pair_cancel");
}

export async function tauriSyncUnpair(peerId: string): Promise<void> {
  await invoke("sync_unpair", { peerId });
}

// File sharing
export async function tauriSyncSendFile(
  itemId: string,
  deviceIds: string[],
): Promise<SendFileResult> {
  return await invoke<SendFileResult>("sync_send_file", { itemId, deviceIds });
}

export async function tauriSyncFetchFile(itemId: string): Promise<void> {
  await invoke("sync_fetch_file", { itemId });
}

export async function tauriSyncCancelFetch(itemId: string): Promise<void> {
  await invoke("sync_cancel_fetch", { itemId });
}

export async function tauriSyncGetDownloadDir(): Promise<string> {
  return await invoke<string>("sync_get_download_dir");
}

/** Shows a folder picker; resolves to the folder saved, or null if closed. */
export async function tauriSyncChooseDownloadDir(): Promise<string | null> {
  return await invoke<string | null>("sync_choose_download_dir");
}

/**
 * Shows a folder picker from the backend, which keeps the window from
 * auto-hiding behind it. Resolves to the chosen folder, or null if closed.
 */
export async function tauriChooseFolder(
  title: string,
  startIn?: string | null,
): Promise<string | null> {
  return await invoke<string | null>("choose_folder", {
    title,
    startIn: startIn || null,
  });
}
