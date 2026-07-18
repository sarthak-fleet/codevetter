// Tauri command handlers intentionally keep their stable, flat IPC argument
// contracts. Bundling those parameters into Rust-only structs would complicate
// serialization and silently change the frontend command ABI.
#![allow(clippy::too_many_arguments)]

//! Shared CodeVetter backend library.
//!
//! Transport adapters share typed services instead of duplicating SQL or
//! repository interpretation.

pub mod agent;
pub mod commands;
pub mod db;
pub mod mcp;
pub mod talk;
pub mod timeutil;

use std::sync::{Arc, Mutex};

/// Shared database state accessible from transport command handlers.
#[derive(Clone)]
pub struct DbState(pub Arc<Mutex<rusqlite::Connection>>);
