use rusqlite::Connection;

const MIGRATION_SQL: &str = include_str!("schema/mcp.sql");

pub fn run_migration(conn: &Connection) -> Result<(), rusqlite::Error> {
    conn.execute_batch(MIGRATION_SQL)
}

#[cfg(test)]
mod tests;
