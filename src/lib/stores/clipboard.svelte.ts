import {
  tauriCountItems, // AGREGAR
  tauriCountSearchResultsFTS,
  tauriGetItemsPaginated,
  tauriRemoveDuplicates,
  tauriSearchItemsFTS,
} from "$lib/tauri/commands";
import { clipboardRepository } from "$lib/tauri/storage";
import type {
  ClipboardItem,
  CreateClipboardItemDto,
  GetItemsOptions,
  UpdateClipboardItemDto,
} from "$lib/types";

class ClipboardStore {
  items = $state<ClipboardItem[]>([]);
  isLoading = $state(false);
  error = $state<string | null>(null);

  // Paginación
  totalItems = $state(0);
  currentPage = $state(0);
  pageSize = $state(100); // 100 items por página para móviles
  hasMore = $state(true);
  isLoadingMore = $state(false);
  // Applied in the database: null for everything, "favorites", or a content type.
  filter = $state<string | null>(null);

  private dedupe(items: ClipboardItem[]): ClipboardItem[] {
    const seen = new Set<string>();
    const result: ClipboardItem[] = [];

    for (const item of items) {
      if (!seen.has(item.id)) {
        seen.add(item.id);
        result.push(item);
      }
    }

    return result;
  }

  // Load items (primera página)
  async loadItems(options?: GetItemsOptions) {
    this.isLoading = true;
    this.error = null;
    this.currentPage = 0;

    try {
      // Cargar primera página
      const firstPage = await tauriGetItemsPaginated(this.pageSize, 0, this.filter);
      this.items = this.dedupe(firstPage);

      // Obtener total de items
      this.totalItems = await tauriCountItems(this.filter);

      // Verificar si hay más
      this.hasMore = this.items.length < this.totalItems;
    } catch (err) {
      this.error = err instanceof Error ? err.message : "Failed to load items";
      console.error("Failed to load items:", err);
    } finally {
      this.isLoading = false;
    }
  }

  // NUEVO: Cargar más items (siguiente página)
  async loadMore() {
    if (!this.hasMore || this.isLoadingMore) return;

    this.isLoadingMore = true;
    this.error = null;

    try {
      this.currentPage++;
      const offset = this.currentPage * this.pageSize;

      const moreItems = await tauriGetItemsPaginated(this.pageSize, offset, this.filter);

      const combined = [...this.items, ...moreItems];
      this.items = this.dedupe(combined);

      // Verificar si hay más
      this.hasMore = this.items.length < this.totalItems;
    } catch (err) {
      this.error =
        err instanceof Error ? err.message : "Failed to load more items";
      console.error("Failed to load more:", err);
      this.currentPage--; // Revertir el incremento
    } finally {
      this.isLoadingMore = false;
    }
  }

  // Create item
  async createItem(data: CreateClipboardItemDto) {
    try {
      const newItem = await clipboardRepository.createItem(data);
      this.items = [newItem, ...this.items];
      this.totalItems++;
      return newItem;
    } catch (err) {
      this.error = err instanceof Error ? err.message : "Failed to create item";
      throw err;
    }
  }

  // Update item
  async updateItem(id: string, data: UpdateClipboardItemDto) {
    try {
      const updatedItem = await clipboardRepository.updateItem(id, data);
      this.items = this.items.map((item) =>
        item.id === id ? updatedItem : item
      );
      return updatedItem;
    } catch (err) {
      this.error = err instanceof Error ? err.message : "Failed to update item";
      throw err;
    }
  }

  // Delete item
  async deleteItem(id: string) {
    try {
      await clipboardRepository.deleteItem(id);
      this.items = this.items.filter((item) => item.id !== id);
      this.totalItems--;
    } catch (err) {
      this.error = err instanceof Error ? err.message : "Failed to delete item";
      throw err;
    }
  }

  // Toggle favorite
  async toggleFavorite(id: string) {
    const item = this.items.find((i) => i.id === id);
    if (!item) return;

    const updated = await this.updateItem(id, { isFavorite: !item.isFavorite });
    const filtered = this.items.filter((i) => i.id !== id);
    this.items = [updated, ...filtered];
  }

  // Clear all
  async clearAll() {
    try {
      await clipboardRepository.clearAll();
      this.items = [];
      this.totalItems = 0;
      this.currentPage = 0;
      this.hasMore = false;
    } catch (err) {
      this.error = err instanceof Error ? err.message : "Failed to clear items";
      throw err;
    }
  }

  // Search (reemplaza los items con resultados de búsqueda)
  async search(query: string) {
    console.log("📦 Store.search() called with query:", query);

    if (!query.trim()) {
      // Si no hay query, volver a cargar items normales
      console.log("❌ Empty query, reloading all items");
      await this.loadItems();
      return;
    }

    this.isLoading = true;
    this.error = null;
    this.currentPage = 0;

    try {
      console.log("🔍 Calling tauriSearchItemsPaginated...");
      // Buscar en base de datos (primeros 100 resultados)
      this.items = await tauriSearchItemsFTS(query, this.pageSize, 0, this.filter);
      console.log("📊 Search returned", this.items.length, "items");

      // Contar total de resultados de búsqueda
      console.log("🔢 Counting total results...");
      this.totalItems = await tauriCountSearchResultsFTS(query, this.filter);
      console.log("📈 Total results:", this.totalItems);

      // Verificar si hay más resultados
      this.hasMore = this.items.length < this.totalItems;
      console.log("✅ Search complete. hasMore:", this.hasMore);
    } catch (err) {
      this.error = err instanceof Error ? err.message : "Failed to search";
      console.error("❌ Failed to search:", err);
    } finally {
      this.isLoading = false;
    }
  }

  // Cargar más resultados de búsqueda
  async loadMoreSearchResults(query: string) {
    if (!this.hasMore || this.isLoadingMore || !query.trim()) return;

    this.isLoadingMore = true;
    this.error = null;

    try {
      this.currentPage++;
      const offset = this.currentPage * this.pageSize;

      const moreItems = await tauriSearchItemsFTS(query, this.pageSize, offset, this.filter);

      const combined = [...this.items, ...moreItems];
      this.items = this.dedupe(combined);

      // Verificar si hay más
      this.hasMore = this.items.length < this.totalItems;
    } catch (err) {
      this.error =
        err instanceof Error
          ? err.message
          : "Failed to load more search results";
      console.error("Failed to load more search results:", err);
      this.currentPage--; // Revertir el incremento
    } finally {
      this.isLoadingMore = false;
    }
  }

  // Remove duplicates
  async removeDuplicates() {
    try {
      const deletedCount = await tauriRemoveDuplicates();
      await this.loadItems(); // Recargar items
      return deletedCount;
    } catch (err) {
      this.error =
        err instanceof Error ? err.message : "Failed to remove duplicates";
      throw err;
    }
  }
}

export const clipboardStore = new ClipboardStore();
