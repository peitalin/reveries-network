use dotenv;
use r2d2_sqlite::SqliteConnectionManager;
use r2d2::Pool;
use rusqlite::params;
use std::path::Path;
use std::sync::Arc;
use tracing::{info, error, warn, trace};
use color_eyre::eyre::{Result, anyhow};

use llm_proxy::usage::UsageReportPayload;

pub type UsageDbPool = Arc<Pool<SqliteConnectionManager>>;

const DB_SCHEMA: &str = "
CREATE TABLE IF NOT EXISTS usage_reports (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    request_id TEXT UNIQUE,
    timestamp INTEGER NOT NULL,
    input_tokens INTEGER NOT NULL,
    output_tokens INTEGER NOT NULL,
    cache_creation_tokens INTEGER,
    cache_read_tokens INTEGER,
    tool_id TEXT,            -- The ID of the tool *if* this report is for the FIRST call
    tool_name TEXT,
    tool_input TEXT,
    tool_type TEXT,
    linked_tool_id TEXT,     -- The ID of the tool use this report is a *follow-up* to
    received_at INTEGER DEFAULT (strftime('%s', 'now'))
);

CREATE INDEX IF NOT EXISTS idx_usage_reports_request_id ON usage_reports(request_id);
CREATE INDEX IF NOT EXISTS idx_usage_reports_timestamp ON usage_reports(timestamp);
CREATE INDEX IF NOT EXISTS idx_usage_reports_tool_id ON usage_reports(tool_id);
CREATE INDEX IF NOT EXISTS idx_usage_reports_linked_tool_id ON usage_reports(linked_tool_id);
";

/// Initializes the SQLite database pool for usage reports.
/// Reads P2P_USAGE_DB_PATH env var, defaults to in-memory if not set.
pub fn init_usage_db() -> Result<UsageDbPool> {

    dotenv::dotenv().ok();
    let db_path_opt = std::env::var("P2P_USAGE_DB_PATH").ok();

    let manager = match db_path_opt {
        None => {
            info!("Initializing in-memory SQLite database pool for usage reports.");
            SqliteConnectionManager::memory()
        },
        Some(ref path) => {
            info!("Initializing file-based SQLite database pool for usage reports at: {}", path);
            let db_path_obj = Path::new(path);
            if let Some(parent) = db_path_obj.parent() {
                std::fs::create_dir_all(parent)
                    .map_err(|e| anyhow!("Failed to create usage DB directory '{}': {}", parent.display(), e))?;
            }
            SqliteConnectionManager::file(path)
        }
    };

    let pool = Pool::builder()
        .max_size(5) // Small pool size often sufficient for internal node DB
        .build(manager)
        .map_err(|e| anyhow!("Failed to create usage db pool: {}", e))?;

    // Initialize schema
    {
        let conn = pool.get()?;
        conn.execute_batch(DB_SCHEMA)
            .map_err(|e| anyhow!("Failed to initialize usage database schema: {}", e))?;
    }

    info!("Usage database pool initialized.");
    Ok(Arc::new(pool))
}

/// Stores a verified usage report payload in the database.
pub fn store_usage_payload(pool: &UsageDbPool, payload: &UsageReportPayload) -> Result<()> {
    trace!("Storing usage report payload to DB: request_id={}, timestamp={}", payload.request_id, payload.timestamp);
    let conn = pool.get()?;

    // Extract specific tool details if present in this payload (i.e., the first call)
    let tool_id = payload.usage.tool_use.as_ref().map(|tu| tu.id.clone());
    let tool_name = payload.usage.tool_use.as_ref().map(|tu| tu.name.clone());
    let tool_input_json = payload.usage.tool_use.as_ref()
        .map(|tu| serde_json::to_string(&tu.input).unwrap_or_else(|_| "null".to_string()));
    let tool_type = payload.usage.tool_use.as_ref().map(|tu| tu.tool_type.clone());

    // Get the linking ID from the payload using the correct name
    let linked_tool_id = payload.linked_tool_use_id.clone();

    conn.execute(
        "INSERT INTO usage_reports (
            request_id, timestamp, input_tokens, output_tokens, cache_creation_tokens, cache_read_tokens,
            tool_id, tool_name, tool_input, tool_type, linked_tool_id -- DB column names
        ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
        params![
            payload.request_id,
            payload.timestamp,
            payload.usage.input_tokens,
            payload.usage.output_tokens,
            payload.usage.cache_creation_input_tokens,
            payload.usage.cache_read_input_tokens,
            tool_id, // Value extracted from payload.usage.tool_use
            tool_name, // Value extracted from payload.usage.tool_use
            tool_input_json, // Value extracted from payload.usage.tool_use
            tool_type, // Value extracted from payload.usage.tool_use
            linked_tool_id, // Value extracted from payload.linked_tool_use_id
        ],
    )?;

    trace!("Successfully stored usage report payload for request_id: {}", payload.request_id);
    Ok(())
}