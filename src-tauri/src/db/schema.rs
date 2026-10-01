use rusqlite::{Connection, Result};

pub fn init_database(conn: &Connection) -> Result<()> {
    conn.execute(
        "CREATE TABLE IF NOT EXISTS clipboard_items (
              id TEXT PRIMARY KEY,
              content_type TEXT NOT NULL,
              content_text TEXT,
              content_metadata TEXT NOT NULL DEFAULT '{}',
              source_app TEXT,
              code_language TEXT,

              file_url TEXT,
              file_name TEXT,
              file_size_bytes INTEGER,
              file_mime_type TEXT,
              file_hash TEXT,

              is_favorite BOOLEAN NOT NULL DEFAULT 0,
              is_snippet BOOLEAN NOT NULL DEFAULT 0,
              snippet_name TEXT,

              created_at TEXT NOT NULL,
              updated_at TEXT NOT NULL,

              synced BOOLEAN NOT NULL DEFAULT 0,
              server_id TEXT
          )",
        [],
    )?;

    // Migration: Add file_hash column if it doesn't exist
    let _ = conn.execute("ALTER TABLE clipboard_items ADD COLUMN file_hash TEXT", []);

    // Create indexes for better performance
    conn.execute(
        "CREATE INDEX IF NOT EXISTS idx_created_at ON 
  clipboard_items(created_at DESC)",
        [],
    )?;

    conn.execute(
        "CREATE INDEX IF NOT EXISTS idx_content_type ON 
  clipboard_items(content_type)",
        [],
    )?;

    conn.execute(
        "CREATE INDEX IF NOT EXISTS idx_is_favorite ON
  clipboard_items(is_favorite)",
        [],
    )?;

    conn.execute(
        "CREATE INDEX IF NOT EXISTS idx_file_hash ON
  clipboard_items(file_hash)",
        [],
    )?;

    crate::db::fts_migration::init_fts_table(conn)?;
    crate::db::fts_migration::create_fts_triggers(conn)?;

    // After clipboard_items: its trigger revokes offers when items are deleted.
    crate::sync::offers::init_offers_table(conn)?;

    Ok(())
}
