use crate::clipboard::asset_cleanup;
use crate::db::models::{ClipboardItem, CreateClipboardItemDto, UpdateClipboardItemDto};
use chrono::Utc;
use rusqlite::{params, Connection, Result};
use serde_json::Value;
use std::path::PathBuf;
use uuid::Uuid;

pub struct ClipboardRepository {
    pub conn: Connection,
}

impl ClipboardRepository {
    pub fn new(db_path: &str) -> Result<Self> {
        let conn = Connection::open(db_path)?;
        crate::db::schema::init_database(&conn)?;
        Ok(Self { conn })
    }

    pub fn get_item(&self, id: &str) -> Result<Option<ClipboardItem>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, content_type, content_text, content_metadata, source_app, code_language,
                      file_url, file_name, file_size_bytes, file_mime_type, file_hash,
                      is_favorite, is_snippet, snippet_name,
                      created_at, updated_at, synced, server_id
               FROM clipboard_items
               WHERE id = ?1",
        )?;

        let mut rows = stmt.query([id])?;

        if let Some(row) = rows.next()? {
            Ok(Some(ClipboardItem {
                id: row.get(0)?,
                content_type: row.get(1)?,
                content_text: row.get(2)?,
                content_metadata: row.get(3)?,
                source_app: row.get(4)?,
                code_language: row.get(5)?,
                file_url: row.get(6)?,
                file_name: row.get(7)?,
                file_size_bytes: row.get(8)?,
                file_mime_type: row.get(9)?,
                file_hash: row.get(10)?,
                is_favorite: row.get(11)?,
                is_snippet: row.get(12)?,
                snippet_name: row.get(13)?,
                created_at: row.get(14)?,
                updated_at: row.get(15)?,
                synced: row.get(16)?,
                server_id: row.get(17)?,
            }))
        } else {
            Ok(None)
        }
    }

    /// Find existing item by content_text to avoid duplicates
    pub fn find_by_content(&self, content_text: &str) -> Result<Option<ClipboardItem>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, content_type, content_text, content_metadata, source_app, code_language,
                      file_url, file_name, file_size_bytes, file_mime_type, file_hash,
                      is_favorite, is_snippet, snippet_name,
                      created_at, updated_at, synced, server_id
               FROM clipboard_items
               WHERE content_text = ?1
               ORDER BY updated_at DESC
               LIMIT 1",
        )?;

        let mut rows = stmt.query([content_text])?;

        if let Some(row) = rows.next()? {
            Ok(Some(ClipboardItem {
                id: row.get(0)?,
                content_type: row.get(1)?,
                content_text: row.get(2)?,
                content_metadata: row.get(3)?,
                source_app: row.get(4)?,
                code_language: row.get(5)?,
                file_url: row.get(6)?,
                file_name: row.get(7)?,
                file_size_bytes: row.get(8)?,
                file_mime_type: row.get(9)?,
                file_hash: row.get(10)?,
                is_favorite: row.get(11)?,
                is_snippet: row.get(12)?,
                snippet_name: row.get(13)?,
                created_at: row.get(14)?,
                updated_at: row.get(15)?,
                synced: row.get(16)?,
                server_id: row.get(17)?,
            }))
        } else {
            Ok(None)
        }
    }

    /// "Bump" an existing item to the top by updating its timestamp
    pub fn bump_item(&self, id: &str) -> Result<ClipboardItem> {
        let now = Utc::now().to_rfc3339();

        self.conn.execute(
            "UPDATE clipboard_items SET updated_at = ?1 WHERE id = ?2",
            params![now, id],
        )?;

        self.get_item(id).map(|opt| opt.unwrap())
    }

    pub fn create_item(&self, dto: CreateClipboardItemDto) -> Result<ClipboardItem> {
        let id = Uuid::new_v4().to_string();
        let now = Utc::now().to_rfc3339();
        let metadata = dto.content_metadata.unwrap_or_else(|| "{}".to_string());

        self.conn.execute(
            "INSERT INTO clipboard_items (
                  id, content_type, content_text, content_metadata, source_app, code_language,
                  is_favorite, is_snippet, created_at, updated_at, synced
              ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, 0, 0, ?7, ?8, 0)",
            params![
                id,
                dto.content_type,
                dto.content_text,
                metadata,
                dto.source_app,
                dto.code_language, // <- AGREGADO
                now,
                now
            ],
        )?;

        self.get_item(&id).map(|opt| opt.unwrap())
    }

    /// Create or update item - prevents duplicates
    pub fn upsert_item(&self, dto: CreateClipboardItemDto) -> Result<ClipboardItem> {
        // Check if item with same content already exists
        if let Some(existing) = self.find_by_content(&dto.content_text)? {
            // Item exists, just bump it to the top
            self.bump_item(&existing.id)
        } else {
            // New item, create it
            self.create_item(dto)
        }
    }

    pub fn update_item(&self, id: &str, dto: UpdateClipboardItemDto) -> Result<ClipboardItem> {
        let now = Utc::now().to_rfc3339();

        // Build dynamic update query
        let mut updates = Vec::new();
        let mut params: Vec<Box<dyn rusqlite::ToSql>> = Vec::new();

        if let Some(content_text) = dto.content_text {
            updates.push("content_text = ?");
            params.push(Box::new(content_text));
        }
        if let Some(is_favorite) = dto.is_favorite {
            updates.push("is_favorite = ?");
            params.push(Box::new(is_favorite));
        }
        if let Some(is_snippet) = dto.is_snippet {
            updates.push("is_snippet = ?");
            params.push(Box::new(is_snippet));
        }
        if let Some(snippet_name) = dto.snippet_name {
            updates.push("snippet_name = ?");
            params.push(Box::new(snippet_name));
        }

        updates.push("updated_at = ?");
        params.push(Box::new(now));

        params.push(Box::new(id.to_string()));

        let query = format!(
            "UPDATE clipboard_items SET {} WHERE id = ?",
            updates.join(", ")
        );

        let params_refs: Vec<&dyn rusqlite::ToSql> = params.iter().map(|p| p.as_ref()).collect();
        self.conn.execute(&query, params_refs.as_slice())?;

        self.get_item(id).map(|opt| opt.unwrap())
    }

    pub fn delete_item(&self, id: &str) -> Result<()> {
        self.conn
            .execute("DELETE FROM clipboard_items WHERE id = ?1", [id])?;
        Ok(())
    }

    pub fn clear_all(&self) -> Result<()> {
        self.conn.execute("DELETE FROM clipboard_items", [])?;
        Ok(())
    }

    // Removed: Use search_items_paginated() instead for better performance
    pub fn remove_duplicates(&self) -> Result<usize> {
        let mut stmt = self.conn.prepare(
            "SELECT id, file_url, content_type, content_metadata
             FROM clipboard_items
             WHERE id NOT IN (
                 SELECT MIN(id)
                 FROM clipboard_items
                 GROUP BY content_text
             )",
        )?;

        let targets: Vec<(String, Option<String>, String, Option<String>)> = stmt
            .query_map([], |row| {
                Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?))
            })
            .and_then(|mapped| mapped.collect())?;

        let deleted_count = targets.len();
        for (id, file_url, content_type, metadata) in targets {
            if let Some(url) = file_url {
                asset_cleanup::delete_file_url(
                    &content_type,
                    &url,
                    metadata.as_deref().unwrap_or("{}"),
                );
            }
            self.conn
                .execute("DELETE FROM clipboard_items WHERE id = ?1", [id.as_str()])?;
        }

        Ok(deleted_count)
    }

    // Get items con paginación
    pub fn get_items_paginated(&self, limit: i64, offset: i64) -> Result<Vec<ClipboardItem>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, content_type, content_text, content_metadata, source_app, code_language,
                    file_url, file_name, file_size_bytes, file_mime_type, file_hash,
                    is_favorite, is_snippet, snippet_name,
                    created_at, updated_at, synced, server_id
             FROM clipboard_items
             ORDER BY updated_at DESC
             LIMIT ?1 OFFSET ?2",
        )?;

        let mut items: Vec<ClipboardItem> = stmt
            .query_map([limit, offset], |row| {
                Ok(ClipboardItem {
                    id: row.get(0)?,
                    content_type: row.get(1)?,
                    content_text: row.get(2)?,
                    content_metadata: row.get(3)?,
                    source_app: row.get(4)?,
                    code_language: row.get(5)?,
                    file_url: row.get(6)?,
                    file_name: row.get(7)?,
                    file_size_bytes: row.get(8)?,
                    file_mime_type: row.get(9)?,
                    file_hash: row.get(10)?,
                    is_favorite: row.get(11)?,
                    is_snippet: row.get(12)?,
                    snippet_name: row.get(13)?,
                    created_at: row.get(14)?,
                    updated_at: row.get(15)?,
                    synced: row.get(16)?,
                    server_id: row.get(17)?,
                })
            })?
            .collect::<Result<Vec<_>, _>>()?;

        let _removed = self.prune_missing_file_items(&mut items)?;

        Ok(items)
    }
    // Contar total de items
    pub fn count_items(&self) -> Result<i64> {
        let mut stmt = self.conn.prepare("SELECT COUNT(*) FROM clipboard_items")?;
        let count: i64 = stmt.query_row([], |row| row.get(0))?;
        Ok(count)
    }

    pub fn search_items_fts(
        &self,
        query: &str,
        limit: i64,
        offset: i64,
    ) -> Result<Vec<ClipboardItem>> {
        // Preparar query para FTS5
        // Si tiene espacios, asumimos búsqueda de frase exacta
        let fts_query = if query.contains(' ') {
            format!("\"{}\"", query)
        } else {
            // Búsqueda de palabra con prefijo (para autocompletado)
            format!("{}*", query)
        };

        let mut stmt = self.conn.prepare(
            "SELECT ci.id, ci.content_type, ci.content_text, ci.content_metadata,
                  ci.source_app, ci.code_language,
                  ci.file_url, ci.file_name, ci.file_size_bytes, ci.file_mime_type, ci.file_hash,
                  ci.is_favorite, ci.is_snippet, ci.snippet_name,
                  ci.created_at, ci.updated_at, ci.synced, ci.server_id,
                  bm25(clipboard_items_fts) as rank
           FROM clipboard_items ci
           INNER JOIN clipboard_items_fts fts ON ci.id = fts.id
           WHERE clipboard_items_fts MATCH ?1
           ORDER BY rank
           LIMIT ?2 OFFSET ?3",
        )?;

        let mut items = stmt
            .query_map(params![&fts_query, limit, offset], |row| {
                Ok(ClipboardItem {
                    id: row.get(0)?,
                    content_type: row.get(1)?,
                    content_text: row.get(2)?,
                    content_metadata: row.get(3)?,
                    source_app: row.get(4)?,
                    code_language: row.get(5)?,
                    file_url: row.get(6)?,
                    file_name: row.get(7)?,
                    file_size_bytes: row.get(8)?,
                    file_mime_type: row.get(9)?,
                    file_hash: row.get(10)?,
                    is_favorite: row.get(11)?,
                    is_snippet: row.get(12)?,
                    snippet_name: row.get(13)?,
                    created_at: row.get(14)?,
                    updated_at: row.get(15)?,
                    synced: row.get(16)?,
                    server_id: row.get(17)?,
                    // Ignoramos rank (row 18) por ahora
                })
            })?
            .collect::<Result<Vec<_>, _>>()?;

        let _removed = self.prune_missing_file_items(&mut items)?;
        Ok(items)
    }

    // Contar resultados FTS
    pub fn count_search_results_fts(&self, query: &str) -> Result<i64> {
        let fts_query = if query.contains(' ') {
            format!("\"{}\"", query)
        } else {
            format!("{}*", query)
        };

        let mut stmt = self.conn.prepare(
            "SELECT COUNT(*) 
           FROM clipboard_items_fts 
           WHERE clipboard_items_fts MATCH ?1",
        )?;

        let count: i64 = stmt.query_row([&fts_query], |row| row.get(0))?;
        Ok(count)
    }

    // Update file information for an item
    pub fn update_file_info(
        &self,
        id: &str,
        file_url: &str,
        file_name: &str,
        file_size_bytes: i64,
        file_mime_type: &str,
        file_hash: Option<&str>,
    ) -> Result<()> {
        self.conn.execute(
            "UPDATE clipboard_items
            SET file_url = ?1, file_name = ?2, file_size_bytes = ?3, file_mime_type = ?4, file_hash = ?5, updated_at = ?6
            WHERE id = ?7",
            rusqlite::params![
                file_url,
                file_name,
                file_size_bytes,
                file_mime_type,
                file_hash,
                chrono::Utc::now().to_rfc3339(),
                id
            ],
        )?;
        Ok(())
    }

    pub fn update_metadata(&self, id: &str, metadata_json: &str) -> Result<()> {
        self.conn.execute(
            "UPDATE clipboard_items SET content_metadata = ?1 WHERE id = ?2",
            rusqlite::params![metadata_json, id],
        )?;
        Ok(())
    }

    /// Find existing item by file_hash to avoid duplicate files
    pub fn find_by_file_hash(&self, file_hash: &str) -> Result<Option<ClipboardItem>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, content_type, content_text, content_metadata, source_app, code_language,
                      file_url, file_name, file_size_bytes, file_mime_type, file_hash,
                      is_favorite, is_snippet, snippet_name,
                      created_at, updated_at, synced, server_id
               FROM clipboard_items
               WHERE file_hash = ?1
               ORDER BY updated_at DESC
               LIMIT 1",
        )?;

        let mut rows = stmt.query([file_hash])?;

        if let Some(row) = rows.next()? {
            Ok(Some(ClipboardItem {
                id: row.get(0)?,
                content_type: row.get(1)?,
                content_text: row.get(2)?,
                content_metadata: row.get(3)?,
                source_app: row.get(4)?,
                code_language: row.get(5)?,
                file_url: row.get(6)?,
                file_name: row.get(7)?,
                file_size_bytes: row.get(8)?,
                file_mime_type: row.get(9)?,
                file_hash: row.get(10)?,
                is_favorite: row.get(11)?,
                is_snippet: row.get(12)?,
                snippet_name: row.get(13)?,
                created_at: row.get(14)?,
                updated_at: row.get(15)?,
                synced: row.get(16)?,
                server_id: row.get(17)?,
            }))
        } else {
            Ok(None)
        }
    }
}

impl ClipboardRepository {
    fn prune_missing_file_items(&self, items: &mut Vec<ClipboardItem>) -> Result<Vec<String>> {
        let mut stale_items: Vec<(String, String, String, String)> = Vec::new();

        items.retain(|item| {
            if let Some(file_url) = &item.file_url {
                let candidate_path = Self::resolve_external_path(&item.content_metadata, file_url);
                if !candidate_path.exists() {
                    stale_items.push((
                        item.id.clone(),
                        item.content_type.clone(),
                        file_url.clone(),
                        item.content_metadata.clone(),
                    ));
                    return false;
                }
            }
            true
        });

        let mut removed_ids = Vec::new();

        for (id, content_type, file_url, metadata) in stale_items {
            asset_cleanup::delete_file_url(&content_type, &file_url, &metadata);
            if let Err(err) = self.delete_item(&id) {
                eprintln!("Failed to delete missing file item {}: {}", id, err);
            } else {
                removed_ids.push(id);
            }
        }

        Ok(removed_ids)
    }

    fn resolve_external_path(metadata_json: &str, fallback_url: &str) -> PathBuf {
        let path_string = serde_json::from_str::<Value>(metadata_json)
            .ok()
            .and_then(|value| {
                value
                    .get("external_path")
                    .and_then(|v| v.as_str())
                    .map(|s| s.to_string())
            });

        path_string
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from(fallback_url))
    }

    pub fn cleanup_missing_file_records(&self) -> Result<Vec<String>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, content_type, content_text, content_metadata, source_app, code_language,
                    file_url, file_name, file_size_bytes, file_mime_type, file_hash,
                    is_favorite, is_snippet, snippet_name,
                    created_at, updated_at, synced, server_id
             FROM clipboard_items
             WHERE file_url IS NOT NULL",
        )?;

        let mut items: Vec<ClipboardItem> = stmt
            .query_map([], |row| {
                Ok(ClipboardItem {
                    id: row.get(0)?,
                    content_type: row.get(1)?,
                    content_text: row.get(2)?,
                    content_metadata: row.get(3)?,
                    source_app: row.get(4)?,
                    code_language: row.get(5)?,
                    file_url: row.get(6)?,
                    file_name: row.get(7)?,
                    file_size_bytes: row.get(8)?,
                    file_mime_type: row.get(9)?,
                    file_hash: row.get(10)?,
                    is_favorite: row.get(11)?,
                    is_snippet: row.get(12)?,
                    snippet_name: row.get(13)?,
                    created_at: row.get(14)?,
                    updated_at: row.get(15)?,
                    synced: row.get(16)?,
                    server_id: row.get(17)?,
                })
            })?
            .collect::<Result<Vec<_>, _>>()?;

        self.prune_missing_file_items(&mut items)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::{fs, time::Duration};
    use tempfile::TempDir;

    fn setup_repo() -> (ClipboardRepository, TempDir) {
        let dir = TempDir::new().expect("temp dir");
        let path = dir.path().join("clipboard.db");
        let repo = ClipboardRepository::new(path.to_str().unwrap()).expect("repo init");
        (repo, dir)
    }

    fn sample_dto(text: &str) -> CreateClipboardItemDto {
        CreateClipboardItemDto {
            content_type: "text".to_string(),
            content_text: text.to_string(),
            content_metadata: Some("{}".to_string()),
            source_app: Some("test-suite".to_string()),
            code_language: None,
        }
    }

    #[test]
    fn create_and_get_item_returns_persisted_record() {
        let (repo, _dir) = setup_repo();
        let created = repo.create_item(sample_dto("hello world")).unwrap();
        let fetched = repo.get_item(&created.id).unwrap().unwrap();
        assert_eq!(fetched.id, created.id);
        assert_eq!(fetched.content_text.as_deref(), Some("hello world"));
        assert_eq!(fetched.content_type, "text");
    }

    #[test]
    fn upsert_item_bumps_existing_record() {
        let (repo, _dir) = setup_repo();
        let first = repo.upsert_item(sample_dto("dedupe me")).unwrap();
        std::thread::sleep(Duration::from_millis(10));
        let second = repo.upsert_item(sample_dto("dedupe me")).unwrap();

        assert_eq!(first.id, second.id, "upsert should not create duplicates");
        assert!(
            second.updated_at >= first.updated_at,
            "timestamp should be refreshed"
        );
    }

    #[test]
    fn pagination_marks_missing_external_files() {
        let (repo, _dir) = setup_repo();
        let metadata = json!({
            "external_path": "/tmp/i/do/not/exist"
        })
        .to_string();

        let dto = CreateClipboardItemDto {
            content_type: "file".into(),
            content_text: "orphan".into(),
            content_metadata: Some(metadata),
            source_app: Some("tester".into()),
            code_language: None,
        };

        let created = repo.create_item(dto).unwrap();
        repo.update_file_info(
            &created.id,
            "/tmp/i/do/not/exist",
            "ghost.png",
            123,
            "image/png",
            Some("hash123"),
        )
        .unwrap();

        let items = repo.get_items_paginated(10, 0).unwrap();
        assert!(items.is_empty(), "Missing files should be removed entirely");
        assert_eq!(repo.count_items().unwrap(), 0);
    }

    #[test]
    fn fts_search_returns_matching_results() {
        let (repo, _dir) = setup_repo();
        repo.create_item(sample_dto("Rust testing is fun")).unwrap();
        repo.create_item(sample_dto("Another entry")).unwrap();

        let results = repo.search_items_fts("Rust", 10, 0).unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(
            results[0].content_text.as_deref(),
            Some("Rust testing is fun")
        );

        let count = repo.count_search_results_fts("Rust").unwrap();
        assert_eq!(count, 1);
    }

    #[test]
    fn fts_search_prunes_missing_files() {
        let (repo, _dir) = setup_repo();
        let file_dir = TempDir::new().unwrap();
        let file_path = file_dir.path().join("missing2.png");
        fs::write(&file_path, b"fake").unwrap();

        let metadata = json!({ "external_path": file_path.to_string_lossy() }).to_string();
        let dto = CreateClipboardItemDto {
            content_type: "file".into(),
            content_text: "search orphan".into(),
            content_metadata: Some(metadata),
            source_app: Some("tester".into()),
            code_language: None,
        };

        let created = repo.create_item(dto).unwrap();
        repo.update_file_info(
            &created.id,
            file_path.to_str().unwrap(),
            "missing2.png",
            4,
            "image/png",
            Some("hash999"),
        )
        .unwrap();

        fs::remove_file(&file_path).unwrap();

        let results = repo.search_items_fts("search", 10, 0).unwrap();
        assert!(
            results.is_empty(),
            "search results should skip missing files and delete the row"
        );
        assert_eq!(repo.count_items().unwrap(), 0);
    }
}
