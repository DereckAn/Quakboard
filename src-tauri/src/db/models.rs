use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClipboardItem {
    pub id: String,
    pub content_type: String,
    pub content_text: Option<String>,
    pub content_metadata: String, // JSON string
    pub source_app: Option<String>,
    pub code_language: Option<String>,

    // Files (futuro)
    pub file_url: Option<String>,
    pub file_name: Option<String>,
    pub file_size_bytes: Option<i64>,
    pub file_mime_type: Option<String>,
    pub file_hash: Option<String>, // SHA256 hash for duplicate detection

    // Organization
    pub is_favorite: bool,
    pub is_snippet: bool,
    pub snippet_name: Option<String>,

    // Timestamps (stored as ISO 8601 strings)
    pub created_at: String,
    pub updated_at: String,

    // Sync (futuro)
    pub synced: bool,
    pub server_id: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct CreateClipboardItemDto {
    pub content_type: String,
    pub content_text: String,
    pub content_metadata: Option<String>,
    pub source_app: Option<String>,
    pub code_language: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct UpdateClipboardItemDto {
    pub content_text: Option<String>,
    pub is_favorite: Option<bool>,
    pub is_snippet: Option<bool>,
    pub snippet_name: Option<String>,
}

/// Narrows the history list. Every field is optional; all set ones apply.
#[derive(Debug, Default, Deserialize)]
pub struct ItemFilter {
    /// "favorites" or a content type ("link", "image", ...).
    pub kind: Option<String>,
    /// RFC 3339 bounds on `updated_at`: `since` inclusive, `until` exclusive.
    pub since: Option<String>,
    pub until: Option<String>,
}
