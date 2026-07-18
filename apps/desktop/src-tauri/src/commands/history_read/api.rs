use super::{
    contributors::HistoryContributorScope, contributors::HistoryContributorSummary,
    HistoryReadService,
};
use crate::{
    commands::history_graph::{
        canonical_repo_path, git_text, HistoryLandmarkCatalog, HistoryLandmarkKind,
        HistoryOpaqueCursor, HistoryReleaseCatalog, HistoryTimelineCenter, HistoryTimelineWindow,
    },
    DbState,
};
use rusqlite::Connection;
use std::{path::PathBuf, sync::Arc};
use tauri::State;

#[tauri::command]
pub async fn get_history_release_catalog(
    repo_path: String,
    limit: Option<usize>,
    cursor: Option<HistoryOpaqueCursor>,
    _current_revision: Option<String>,
    db: State<'_, DbState>,
) -> Result<HistoryReleaseCatalog, String> {
    let root = canonical_repo_path(&repo_path)?;
    let database = Arc::clone(&db.0);
    tokio::task::spawn_blocking(move || {
        let current_revision = live_current_revision(&root)?;
        let connection = database
            .lock()
            .map_err(|_| "History database is unavailable".to_string())?;
        release_catalog(&connection, root, current_revision, limit, cursor.as_ref())
    })
    .await
    .map_err(|error| format!("Release catalog worker failed: {error}"))?
}

#[tauri::command]
pub async fn get_history_landmark_catalog(
    repo_path: String,
    kind: Option<HistoryLandmarkKind>,
    limit: Option<usize>,
    cursor: Option<HistoryOpaqueCursor>,
    _current_revision: Option<String>,
    db: State<'_, DbState>,
) -> Result<HistoryLandmarkCatalog, String> {
    let root = canonical_repo_path(&repo_path)?;
    let database = Arc::clone(&db.0);
    tokio::task::spawn_blocking(move || {
        let current_revision = live_current_revision(&root)?;
        let connection = database
            .lock()
            .map_err(|_| "History database is unavailable".to_string())?;
        landmark_catalog(
            &connection,
            root,
            current_revision,
            kind,
            limit,
            cursor.as_ref(),
        )
    })
    .await
    .map_err(|error| format!("Landmark catalog worker failed: {error}"))?
}

#[tauri::command]
pub async fn get_history_contributor_summary(
    repo_path: String,
    scope: HistoryContributorScope,
    limit: Option<usize>,
    cursor: Option<HistoryOpaqueCursor>,
    _current_revision: Option<String>,
    db: State<'_, DbState>,
) -> Result<HistoryContributorSummary, String> {
    let root = canonical_repo_path(&repo_path)?;
    let database = Arc::clone(&db.0);
    tokio::task::spawn_blocking(move || {
        let current_revision = live_current_revision(&root)?;
        let connection = database
            .lock()
            .map_err(|_| "History database is unavailable".to_string())?;
        contributor_summary(
            &connection,
            root,
            current_revision,
            scope,
            limit,
            cursor.as_ref(),
        )
    })
    .await
    .map_err(|error| format!("Contributor summary worker failed: {error}"))?
}

#[tauri::command]
pub async fn get_history_timeline_window(
    repo_path: String,
    center: HistoryTimelineCenter,
    limit: Option<usize>,
    _current_revision: Option<String>,
    db: State<'_, DbState>,
) -> Result<HistoryTimelineWindow, String> {
    let root = canonical_repo_path(&repo_path)?;
    let database = Arc::clone(&db.0);
    tokio::task::spawn_blocking(move || {
        let current_revision = live_current_revision(&root)?;
        let connection = database
            .lock()
            .map_err(|_| "History database is unavailable".to_string())?;
        timeline_window(&connection, root, current_revision, center, limit)
    })
    .await
    .map_err(|error| format!("Timeline window worker failed: {error}"))?
}

fn release_catalog(
    connection: &Connection,
    root: PathBuf,
    current_revision: String,
    limit: Option<usize>,
    cursor: Option<&HistoryOpaqueCursor>,
) -> Result<HistoryReleaseCatalog, String> {
    HistoryReadService::new_with_current_head(connection, root, current_revision)?
        .release_catalog(limit, cursor)
}

fn landmark_catalog(
    connection: &Connection,
    root: PathBuf,
    current_revision: String,
    kind: Option<HistoryLandmarkKind>,
    limit: Option<usize>,
    cursor: Option<&HistoryOpaqueCursor>,
) -> Result<HistoryLandmarkCatalog, String> {
    HistoryReadService::new_with_current_head(connection, root, current_revision)?
        .landmark_catalog(kind, limit, cursor)
}

fn contributor_summary(
    connection: &Connection,
    root: PathBuf,
    current_revision: String,
    scope: HistoryContributorScope,
    limit: Option<usize>,
    cursor: Option<&HistoryOpaqueCursor>,
) -> Result<HistoryContributorSummary, String> {
    HistoryReadService::new_with_current_head(connection, root, current_revision)?
        .contributor_summary_page(scope, limit, cursor)
}

fn timeline_window(
    connection: &Connection,
    root: PathBuf,
    current_revision: String,
    center: HistoryTimelineCenter,
    limit: Option<usize>,
) -> Result<HistoryTimelineWindow, String> {
    HistoryReadService::new_with_current_head(connection, root, current_revision)?
        .timeline_window(center, limit)
}

fn live_current_revision(root: &std::path::Path) -> Result<String, String> {
    git_text(root, &["rev-parse", "HEAD"])
}

#[cfg(test)]
mod tests {
    use super::*;
    use rusqlite::params;
    use std::fs;

    #[test]
    fn current_revision_is_derived_from_the_repository() {
        let root = std::env::temp_dir().join(format!("cv-history-head-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&root).expect("fixture");
        std::process::Command::new("git")
            .args(["init", "-q"])
            .current_dir(&root)
            .status()
            .expect("git init");
        fs::write(root.join("README.md"), "fixture").expect("source");
        for args in [
            vec!["add", "README.md"],
            vec![
                "-c",
                "user.name=CodeVetter",
                "-c",
                "user.email=codevetter@example.invalid",
                "commit",
                "-qm",
                "fixture",
            ],
        ] {
            assert!(std::process::Command::new("git")
                .args(args)
                .current_dir(&root)
                .status()
                .expect("git command")
                .success());
        }
        let revision = live_current_revision(&root).expect("live head");
        assert_eq!(revision.len(), 40);
        assert!(revision.bytes().all(|byte| byte.is_ascii_hexdigit()));
        fs::remove_dir_all(root).expect("cleanup");
    }

    #[test]
    fn api_helper_canonicalizes_by_filesystem_and_preserves_bounded_defaults() {
        let root = std::env::temp_dir().join(format!("cv-history-api-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&root).expect("fixture");
        let connection = Connection::open_in_memory().expect("database");
        crate::db::schema::run_migrations(&connection).expect("schema");
        let canonical = root.canonicalize().expect("canonical");
        let catalog = release_catalog(&connection, canonical.clone(), String::new(), None, None)
            .expect("legacy empty catalog");
        assert_eq!(catalog.schema_version, 1);
        assert_eq!(catalog.applied_limit, 100);
        assert!(catalog.releases.is_empty());
        assert!(catalog.next_cursor.is_none());
        let landmarks = landmark_catalog(&connection, canonical, String::new(), None, None, None)
            .expect("legacy empty landmarks");
        assert_eq!(landmarks.schema_version, 1);
        assert_eq!(landmarks.applied_limit, 100);
        assert!(landmarks.landmarks.is_empty());
        fs::remove_dir_all(root).expect("cleanup");
    }

    #[test]
    fn api_helpers_propagate_cursor_release_center_and_current_freshness() {
        let root = std::env::temp_dir().join(format!("cv-history-api-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&root).expect("fixture");
        let root = root.canonicalize().expect("canonical");
        let repo = root.to_string_lossy().to_string();
        let connection = Connection::open_in_memory().expect("database");
        crate::db::schema::run_migrations(&connection).expect("schema");
        let shas = ["1".repeat(40), "2".repeat(40), "3".repeat(40)];
        connection
            .execute(
                "INSERT INTO history_graph_repositories (
                repo_path, repository_fingerprint, indexed_head, indexed_tags_fingerprint,
                status, coverage_json, created_at, updated_at
             ) VALUES (?1, 'fixture', ?2, 'tags', 'ready', '{}', 'now', 'now')",
                params![repo, shas[2]],
            )
            .expect("repository");
        for (ordinal, sha) in shas.iter().enumerate() {
            connection
                .execute(
                    "INSERT INTO history_graph_revisions (
                    repo_path, sha, ordinal, committed_at, author_name, subject,
                    parents_json, tags_json, is_release, is_head
                 ) VALUES (?1, ?2, ?3, '2026-01-01T00:00:00Z', 'Fixture', 'commit',
                    '[]', '[]', 0, 0)",
                    params![repo, sha, ordinal as i64],
                )
                .expect("revision");
        }
        connection
            .execute(
                "INSERT INTO history_graph_release_catalogs (
                repo_path, index_identity, indexed_head, tags_fingerprint, status,
                coverage_json, updated_at
             ) VALUES (?1, 'index', ?2, 'tags', 'ready',
                '{\"ancestry_complete\":true}', 'now')",
                params![repo, shas[2]],
            )
            .expect("catalog");
        for (tag, revision) in [("v1", &shas[0]), ("v2", &shas[2])] {
            connection
                .execute(
                    "INSERT INTO history_graph_release_tags (
                    repo_path, tag, revision_sha, tag_object_sha, tag_kind
                 ) VALUES (?1, ?2, ?3, ?3, 'lightweight')",
                    params![repo, tag, revision],
                )
                .expect("tag");
        }

        let first = release_catalog(&connection, root.clone(), shas[2].clone(), Some(1), None)
            .expect("first page");
        let second = release_catalog(
            &connection,
            root.clone(),
            shas[2].clone(),
            Some(1),
            first.next_cursor.as_ref(),
        )
        .expect("cursor page");
        assert_eq!(second.releases[0].tag, "v1");
        let window = timeline_window(
            &connection,
            root,
            shas[2].clone(),
            HistoryTimelineCenter::Release { tag: "v1".into() },
            Some(1),
        )
        .expect("release window");
        assert_eq!(window.center_revision.as_ref(), Some(&shas[0]));
        assert!(!window.freshness.stale);
        fs::remove_dir_all(repo).expect("cleanup");
    }
}
