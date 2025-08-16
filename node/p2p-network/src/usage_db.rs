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
    request_id TEXT PRIMARY KEY,
    timestamp INTEGER NOT NULL,
    received_at TEXT DEFAULT (datetime('now')),
    input_tokens INTEGER NOT NULL,
    output_tokens INTEGER NOT NULL,
    cache_creation_tokens INTEGER,
    cache_read_tokens INTEGER,
    tool_id TEXT,
    tool_name TEXT,
    tool_input TEXT,
    tool_type TEXT,
    linked_tool_id TEXT,
    reverie_id TEXT,
    spender_address TEXT,
    spender_type TEXT
);

-- CREATE INDEX IF NOT EXISTS idx_usage_reports_request_id ON usage_reports(request_id);
CREATE INDEX IF NOT EXISTS idx_usage_reports_timestamp ON usage_reports(timestamp);
CREATE INDEX IF NOT EXISTS idx_usage_reports_tool_id ON usage_reports(tool_id);
CREATE INDEX IF NOT EXISTS idx_usage_reports_linked_tool_id ON usage_reports(linked_tool_id);
CREATE INDEX IF NOT EXISTS idx_usage_reports_reverie_id ON usage_reports(reverie_id);
CREATE INDEX IF NOT EXISTS idx_usage_reports_spender_address ON usage_reports(spender_address);
";

/// Initializes the SQLite database pool for usage reports.
/// Reads P2P_USAGE_DB_PATH env var, uses default filepath if not provided
pub fn init_usage_db() -> Result<UsageDbPool> {

    dotenv::dotenv().ok();
    let db_path_opt = std::env::var("P2P_USAGE_DB_PATH").ok();
    info!("P2P_USAGE_DB_PATH env var: {:?}", db_path_opt);

    let manager = match db_path_opt {
        None => {
            let default_path = "./temp-data/p2p_usage.db";
            let db_path_obj = Path::new(default_path);
            if let Some(parent) = db_path_obj.parent() {
                std::fs::create_dir_all(parent)
                    .map_err(|e| anyhow!("Failed to create usage DB directory '{}': {}", parent.display(), e))?;
            }
            SqliteConnectionManager::file(default_path)
            // info!("Initializing in-memory SQLite database pool for usage reports.");
            // SqliteConnectionManager::memory()
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

/// Reads usage data from the database for a specific reverie_id
pub fn read_usage_data_for_reverie(pool: &UsageDbPool, reverie_id: &str) -> Result<Vec<UsageReportPayload>> {
    info!("Reading usage data for reverie_id: {}", reverie_id);
    let conn = pool.get()?;

    let mut stmt = conn.prepare(
        "SELECT request_id, timestamp, input_tokens, output_tokens,
                cache_creation_tokens, cache_read_tokens, tool_id, tool_name,
                tool_input, tool_type, linked_tool_id, reverie_id,
                spender_address, spender_type
         FROM usage_reports
         WHERE reverie_id = ?
         ORDER BY timestamp DESC"
    )?;

    let rows = stmt.query_map(params![reverie_id], |row| {
        let tool_use = if let (Some(tool_id), Some(tool_name), Some(tool_input), Some(tool_type)) =
            (row.get::<_, Option<String>>(6)?,
             row.get::<_, Option<String>>(7)?,
             row.get::<_, Option<String>>(8)?,
             row.get::<_, Option<String>>(9)?) {
            let input_json = serde_json::from_str(&tool_input).unwrap_or(serde_json::Value::Null);
            Some(llm_proxy::usage::ToolUse {
                id: tool_id,
                name: tool_name,
                input: input_json,
                tool_type,
            })
        } else {
            None
        };

        let usage_data = llm_proxy::usage::UsageData {
            reverie_id: row.get(11)?,
            spender: row.get(12)?,
            spender_type: row.get(13)?,
            input_tokens: row.get(2)?,
            output_tokens: row.get(3)?,
            cache_creation_input_tokens: row.get(4)?,
            cache_read_input_tokens: row.get(5)?,
            tool_use,
        };

        Ok(llm_proxy::usage::UsageReportPayload {
            usage: usage_data,
            timestamp: row.get(1)?,
            linked_tool_use_id: row.get(10)?,
            request_id: row.get(0)?,
        })
    })?;

    let mut results = Vec::new();
    for row_result in rows {
        match row_result {
            Ok(record) => {
                info!("Successfully processed record: {}", record.request_id);
                results.push(record);
            },
            Err(e) => {
                error!("Failed to process row: {}", e);
                return Err(anyhow!("Failed to process database row: {}", e));
            }
        }
    }

    info!("Found {} usage records for reverie_id: {}", results.len(), reverie_id);
    Ok(results)
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

    conn.execute(
        "INSERT INTO usage_reports (
            request_id, timestamp, input_tokens, output_tokens, cache_creation_tokens, cache_read_tokens,
            tool_id, tool_name, tool_input, tool_type, linked_tool_id,
            reverie_id, spender_address, spender_type -- Added DB columns
        ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14)", // Added placeholders
        params![
            payload.request_id,
            payload.timestamp,
            payload.usage.input_tokens,
            payload.usage.output_tokens,
            payload.usage.cache_creation_input_tokens,
            payload.usage.cache_read_input_tokens,
            tool_id,
            tool_name,
            tool_input_json,
            tool_type,
            payload.linked_tool_use_id,
            payload.usage.reverie_id,
            payload.usage.spender,
            payload.usage.spender_type,
        ],
    )?;

    trace!("Successfully stored usage report payload for request_id: {}", payload.request_id);
    Ok(())
}