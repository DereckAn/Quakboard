export type ContentType = "text" | "code" | "link" | "color" | "image" | "file" | "svg";

export interface ClipboardItem {
  id: string;
  contentType: ContentType;
  contentText: string | null;
  contentMetadata: Record<string, any>;

  // Source info - NUEVO: para saber de qué app se copió
  sourceApp: string | null;
  codeLanguage: string | null;

  // Files (futuro)
  fileUrl: string | null;
  fileName: string | null;
  fileSizeBytes: number | null;
  fileMimeType: string | null;

  // Organization
  isFavorite: boolean;
  isSnippet: boolean;
  snippetName: string | null;

  // Timestamps
  createdAt: Date;
  updatedAt: Date;

  // Sync (futuro - Pro)
  synced: boolean;
  serverId: string | null;

  fileHash: string | null;
}

export interface CreateClipboardItemDto {
  contentType: ContentType;
  contentText: string;
  contentMetadata?: Record<string, any>;
}

export interface UpdateClipboardItemDto {
  contentText?: string;
  isFavorite?: boolean;
  isSnippet?: boolean;
  snippetName?: string;
}

// Repository interface (contrato)
export interface ClipboardRepository {
  // Queries
  getItems(options?: GetItemsOptions): Promise<ClipboardItem[]>;
  getItem(id: string): Promise<ClipboardItem | null>;
  getFavorites(): Promise<ClipboardItem[]>;
  getSnippets(): Promise<ClipboardItem[]>;

  // Mutations
  createItem(data: CreateClipboardItemDto): Promise<ClipboardItem>;
  updateItem(id: string, data: UpdateClipboardItemDto): Promise<ClipboardItem>;
  deleteItem(id: string): Promise<void>;
  clearAll(): Promise<void>;
}

export interface GetItemsOptions {
  limit?: number;
  offset?: number;
  contentType?: ContentType;
  isFavorite?: boolean;
  isSnippet?: boolean;
}
// Narrows the history list in the database; null fields don't filter.
export interface ItemFilter {
  // "favorites" or a content type.
  kind: string | null;
  // ISO bounds on updatedAt: since inclusive, until exclusive.
  since: string | null;
  until: string | null;
}
