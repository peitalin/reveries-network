use r2d2_sqlite::SqliteConnectionManager;
use r2d2::Pool;
use rusqlite::params;
use std::path::Path;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};
use tracing::{info, error, warn, trace};
use color_eyre::eyre::{Result, anyhow};

pub type DbPool = Arc<Pool<SqliteConnectionManager>>;

const DB_INIT_SQL: &str = "
CREATE TABLE IF NOT EXISTS request_context (
    request_id TEXT PRIMARY KEY,
    request_url TEXT,
    linked_tool_use_id TEXT,
    created_at INTEGER NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_request_context_created_at ON request_context(created_at);
";

/// Initializes the SQLite database pool.
/// Uses in-memory mode if db_path is None, otherwise uses the specified file.
pub fn init_db(db_path: Option<&str>) -> Result<DbPool> {
    let manager = match db_path {
        None => {
            info!("Initializing in-memory SQLite database pool.");
            SqliteConnectionManager::memory()
        },
        Some(path) => {
            info!("Initializing file-based SQLite database pool at: {}", path);
            let db_path_obj = Path::new(path);
            if let Some(parent) = db_path_obj.parent() {
                // Ensure directory exists for file-based DB
                std::fs::create_dir_all(parent)
                    .map_err(|e| anyhow!("Failed to create DB directory '{}': {}", parent.display(), e))?;
            }
            SqliteConnectionManager::file(path)
        }
    };

    let pool = Pool::builder()
        .max_size(5) // Adjust pool size as needed
        .build(manager)
        .map_err(|e| anyhow!("Failed to create db pool: {}", e))?;

    // Get a connection to initialize the schema
    {
        let conn = pool.get()?;
        conn.execute_batch(DB_INIT_SQL)
            .map_err(|e| anyhow!("Failed to initialize database schema: {}", e))?;
    }

    info!("Database pool initialized and schema ensured.");
    Ok(Arc::new(pool))
}

/// Stores the request URL associated with a request ID.
pub fn store_request_context(pool: &DbPool, request_id: &str, url: &str) -> Result<()> {
    info!("DB: Storing context for request_id: {}, url: {}", request_id, url);
    let conn = pool.get()?;
    let now = SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs();
    conn.execute(
        "
        INSERT INTO request_context (request_id, request_url, created_at)
        VALUES (?1, ?2, ?3)
        ON CONFLICT(request_id) DO UPDATE SET request_url=excluded.request_url, created_at=excluded.created_at
        ",
        params![request_id, url, now],
    )?;
    info!("DB: Stored context for request_id: {}", request_id);
    Ok(())
}

/// Stores the linked_tool_use_id for a given request ID.
pub fn store_linked_tool_use_id(pool: &DbPool, request_id: &str, linked_tool_use_id: &str) -> Result<()> {
    info!("DB: Storing linked_tool_use_id {} for request_id: {}", linked_tool_use_id, request_id);
    let conn = pool.get()?;
    let now = SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs();
    conn.execute(
        "
        INSERT INTO request_context (request_id, linked_tool_use_id, created_at)
        VALUES (?1, ?2, ?3)
        ON CONFLICT(request_id) DO UPDATE SET linked_tool_use_id=excluded.linked_tool_use_id, created_at=excluded.created_at
        ",
        params![request_id, linked_tool_use_id, now],
    )?;
    info!("DB: Stored linked_tool_use_id for request_id: {}", request_id);
    Ok(())
}

/// Retrieves the URL and linked tool ID for a request ID, then deletes the entry.
pub fn retrieve_and_clear_context(pool: &DbPool, request_id: &str) -> Result<(Option<String>, Option<String>)> {
    trace!("Retrieving and clearing context for request_id: {}", request_id);
    let mut conn = pool.get()?;
    let tx = conn.transaction()?;

    let context_result: Result<(Option<String>, Option<String>), rusqlite::Error> = tx.query_row(
        "
        SELECT request_url, linked_tool_use_id
        FROM request_context
        WHERE request_id = ?1
        ",
        params![request_id],
        |row| Ok((row.get(0)?, row.get(1)?))
    );

    let context = match context_result {
        Ok(data) => data,
        Err(rusqlite::Error::QueryReturnedNoRows) => {
            warn!("No context found in DB for request_id: {}", request_id);
            (None, None)
        }
        Err(e) => {
            return Err(anyhow!("DB query error for request_id {}: {}", request_id, e));
        }
    };

    // Delete the entry
    match tx.execute(
        "DELETE FROM request_context WHERE request_id = ?1",
        params![request_id]
    ) {
        Ok(_) => trace!("Cleared context for request_id: {}", request_id),
        Err(e) => error!("Failed to clear context for request_id {}: {}", request_id, e),
    }

    // Commit the transaction
    tx.commit()?;

    Ok(context)
}

/// Cleans up old entries from the request_context table.
pub fn cleanup_old_contexts(pool: &DbPool, max_age_seconds: u64) -> Result<usize> {
    let conn = pool.get()?;
    let cutoff_time = SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs().saturating_sub(max_age_seconds);
    let rows_affected = conn.execute(
        "DELETE FROM request_context WHERE created_at < ?1",
        params![cutoff_time],
    )?;
    if rows_affected > 0 {
        trace!("Cleaned up {} old request context entries older than {} seconds.", rows_affected, max_age_seconds);
    }
    Ok(rows_affected)
}