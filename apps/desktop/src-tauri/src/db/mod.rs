pub(crate) mod archaeology_schema;
pub(crate) mod history_graph_schema;
pub(crate) mod mcp_schema;
pub mod queries;
pub mod schema;
pub(crate) mod structural_graph_schema;

use rusqlite::Connection;
use std::path::PathBuf;

/// Open (or create) the SQLite database in the app data directory and run
/// all migrations so that every table is guaranteed to exist.
pub fn init_db(app_data_dir: PathBuf) -> Result<Connection, rusqlite::Error> {
    std::fs::create_dir_all(&app_data_dir).ok();
    let db_path = app_data_dir.join("codevetter.db");
    let conn = Connection::open(db_path)?;

    // Performance pragmas ------------------------------------------------
    // mmap_size: up to 256 MB mapped I/O — big speedup for warm reads
    //   on the indexed message DB without actually using that much RAM
    //   (pages page in on demand).
    // temp_store=MEMORY: keeps sort/group temp tables in RAM — matters
    //   for the GROUP BY strftime() used by the token usage chart.
    // cache_size negative = KiB — 16 MB page cache (was 64 MB pre-1.1.84).
    //   The indexer churns whole tables, so a bigger cache was rarely a hot
    //   hit; trimming it cuts steady-state RSS by ~50 MB.
    // wal_autocheckpoint = 200 pages (~800 KB) — more aggressive than the
    //   1000-page default; the indexer commits frequently and we want the
    //   WAL to stay small in steady-state.
    conn.execute_batch(
        "PRAGMA journal_mode      = WAL;
         PRAGMA synchronous       = NORMAL;
         PRAGMA foreign_keys      = ON;
         PRAGMA busy_timeout      = 30000;
         PRAGMA mmap_size         = 268435456;
         PRAGMA temp_store        = MEMORY;
         PRAGMA cache_size        = -16384;
         PRAGMA wal_autocheckpoint = 200;",
    )?;

    schema::run_migrations(&conn)?;

    Ok(conn)
}

/// True when SQLite could not acquire a write lock (background indexer contention).
pub fn is_database_busy(err: &rusqlite::Error) -> bool {
    match err {
        rusqlite::Error::SqliteFailure(code, _) => {
            code.code == rusqlite::ErrorCode::DatabaseBusy
                || code.code == rusqlite::ErrorCode::DatabaseLocked
        }
        _ => false,
    }
}

/// Retry a DB operation when the shared DB file is busy (e.g. periodic indexer).
pub fn with_busy_retry<T, F>(mut op: F, max_attempts: u32) -> Result<T, rusqlite::Error>
where
    F: FnMut() -> Result<T, rusqlite::Error>,
{
    let mut attempt = 0u32;
    loop {
        match op() {
            Ok(value) => return Ok(value),
            Err(err) if is_database_busy(&err) && attempt + 1 < max_attempts => {
                attempt += 1;
                let delay_ms = (300u64 * attempt as u64).min(4000);
                std::thread::sleep(std::time::Duration::from_millis(delay_ms));
            }
            Err(err) => return Err(err),
        }
    }
}
