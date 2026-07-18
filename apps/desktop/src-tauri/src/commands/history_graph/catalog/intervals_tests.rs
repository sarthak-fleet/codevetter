use super::*;
use crate::commands::history_graph::history_facts::{
    HistoryAutomationKind, HistoryIdentityFact, HistoryRevisionFact,
};
use rusqlite::Connection;

const REPO: &str = "/release-interval-fixture";

#[test]
fn intervals_are_exact_ancestry_aware_and_independent_of_loaded_window() {
    let connection = database();
    let build = fixture_build(false);
    let tags = fixture_tags();
    let first_identity = publish(&connection, &build, &tags).expect("publish intervals");
    let second_identity = publish(&connection, &build, &tags).expect("repeat intervals");
    assert_eq!(first_identity, second_identity);

    let rows = interval_rows(&connection);
    assert_eq!(rows.len(), 4);
    assert_eq!(
        rows.iter().find(|row| row.0 == "v1.0.0").unwrap(),
        &(
            "v1.0.0".to_string(),
            sha('B'),
            None,
            Some(2),
            2,
            "complete".to_string()
        )
    );
    let stable = rows.iter().find(|row| row.0 == "v2.0.0").unwrap();
    let lts = rows.iter().find(|row| row.0 == "v2.0.0-lts").unwrap();
    assert_eq!(stable.1, sha('E'));
    assert_eq!(stable.2.as_deref(), Some(sha('B').as_str()));
    assert_eq!(
        (stable.3, stable.4, stable.5.as_str()),
        (Some(3), 3, "complete")
    );
    assert_eq!(
        (lts.1.as_str(), &lts.2, lts.3, lts.4),
        (stable.1.as_str(), &stable.2, stable.3, stable.4)
    );
    let divergent = rows.iter().find(|row| row.0 == "v9.9.9").unwrap();
    assert_eq!(
        (divergent.3, divergent.4, divergent.5.as_str()),
        (None, 0, "divergent")
    );

    // The old release is absent from the one-row UI window but remains indexed.
    assert_eq!(build.timeline.revisions.len(), 1);
    assert_eq!(build.timeline.revisions[0].sha, sha('F'));
    let (status, coverage): (String, String) = connection
        .query_row(
            "SELECT status, coverage_json FROM history_graph_release_catalogs WHERE repo_path = ?1",
            [REPO],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .expect("catalog coverage");
    assert_eq!(status, "partial");
    let coverage: serde_json::Value = serde_json::from_str(&coverage).expect("coverage json");
    assert_eq!(coverage["release_interval_count"], 4);
    assert_eq!(coverage["divergent_release_count"], 1);
    assert_eq!(coverage["intervals_complete"], false);
}

#[test]
fn shallow_intervals_publish_observed_counts_without_claiming_exact_counts() {
    let connection = database();
    let build = fixture_build(true);
    let tags = fixture_tags()
        .into_iter()
        .filter(|tag| tag.name != "v9.9.9")
        .collect::<Vec<_>>();
    publish(&connection, &build, &tags).expect("shallow intervals");
    let rows = interval_rows(&connection);
    assert!(rows.iter().all(|row| row.3.is_none() && row.5 == "shallow"));
    assert_eq!(rows.iter().find(|row| row.0 == "v2.0.0").unwrap().4, 3);
}

type IntervalRow = (String, String, Option<String>, Option<i64>, i64, String);

fn interval_rows(connection: &Connection) -> Vec<IntervalRow> {
    let mut statement = connection
        .prepare(
            "SELECT tag, revision_sha, from_exclusive_sha, commit_count,
                    observed_commit_count, coverage_kind
             FROM history_graph_release_intervals WHERE repo_path = ?1 ORDER BY tag",
        )
        .expect("interval query");
    statement
        .query_map([REPO], |row| {
            Ok((
                row.get(0)?,
                row.get(1)?,
                row.get(2)?,
                row.get(3)?,
                row.get(4)?,
                row.get(5)?,
            ))
        })
        .expect("interval rows")
        .collect::<Result<_, _>>()
        .expect("interval values")
}

fn publish(
    connection: &Connection,
    build: &HistoryTimelineBuild,
    tags: &[GitTagRecord],
) -> Result<String, String> {
    let transaction = connection
        .unchecked_transaction()
        .map_err(|error| error.to_string())?;
    publish_history_facts(
        &transaction,
        build,
        tags,
        "2026-01-01T00:00:00Z",
        &StructuralGraphCancellation::default(),
    )?;
    let reachable = tags
        .iter()
        .filter(|tag| build.facts_by_revision.contains_key(&tag.commit_sha))
        .cloned()
        .collect::<Vec<_>>();
    publish_release_catalog(
        &transaction,
        &build.timeline,
        &reachable,
        "release-tags",
        !build.timeline.is_shallow && reachable.len() == tags.len(),
    )?;
    let identity = publish_release_intervals(&transaction, build, tags)?;
    transaction.commit().map_err(|error| error.to_string())?;
    Ok(identity)
}

fn fixture_build(shallow: bool) -> HistoryTimelineBuild {
    let parents = [
        ('A', vec![]),
        ('B', vec!['A']),
        ('C', vec!['B']),
        ('D', vec!['B']),
        ('E', vec!['C', 'D']),
        ('F', vec!['E']),
    ];
    let primary = HistoryIdentityFact {
        contributor_id: "contributor:fixture".to_string(),
        display_name: "Fixture".to_string(),
        automation: HistoryAutomationKind::Human,
        alias_count: 0,
    };
    let facts_by_revision = parents
        .iter()
        .map(|(revision, parents)| {
            let revision_sha = sha(*revision);
            (
                revision_sha.clone(),
                HistoryRevisionFact {
                    sha: revision_sha,
                    parents: parents.iter().map(|parent| sha(*parent)).collect(),
                    committed_at: "2026-01-01T00:00:00Z".to_string(),
                    subject: format!("commit {revision}"),
                    primary: primary.clone(),
                    coauthors: Vec::new(),
                    malformed_coauthor_count: 0,
                    tags: Vec::new(),
                    paths: Vec::new(),
                    is_merge: parents.len() > 1,
                    is_head: *revision == 'F',
                },
            )
        })
        .collect();
    HistoryTimelineBuild {
        timeline: HistoryTimeline {
            schema_version: 1,
            repo_path: REPO.to_string(),
            head: sha('F'),
            generated_at: "2026-01-01T00:00:00Z".to_string(),
            revisions: vec![HistoryRevision {
                sha: sha('F'),
                short_sha: sha('F')[..8].to_string(),
                parents: vec![sha('E')],
                committed_at: "2026-01-01T00:00:00Z".to_string(),
                author: "Fixture".to_string(),
                subject: "commit F".to_string(),
                tags: Vec::new(),
                is_release: false,
                is_head: true,
                ordinal: 5,
            }],
            total_commits: 6,
            truncated: true,
            is_shallow: shallow,
            coverage_complete: !shallow,
            release_ranges: Vec::new(),
            reachable_revisions: parents.iter().map(|(revision, _)| sha(*revision)).collect(),
        },
        fact_git_process_count: 1,
        facts_by_revision,
        mailmap_fingerprint: "mailmap:fixture".to_string(),
        facts_fingerprint: "facts:fixture".to_string(),
    }
}

fn fixture_tags() -> Vec<GitTagRecord> {
    vec![
        tag("v1.0.0", 'B', 'X'),
        tag("v2.0.0", 'E', 'E'),
        tag("v2.0.0-lts", 'E', 'Y'),
        tag("v9.9.9", 'G', 'G'),
    ]
}

fn tag(name: &str, commit: char, object: char) -> GitTagRecord {
    GitTagRecord {
        name: name.to_string(),
        object_sha: sha(object),
        commit_sha: sha(commit),
        created_ts: 1,
    }
}

fn sha(character: char) -> String {
    character.to_ascii_lowercase().to_string().repeat(40)
}

fn database() -> Connection {
    let connection = Connection::open_in_memory().expect("database");
    connection
        .execute_batch("PRAGMA foreign_keys = ON;")
        .expect("foreign keys");
    crate::db::schema::run_migrations(&connection).expect("migrations");
    connection
        .execute(
            "INSERT INTO history_graph_repositories (
                repo_path, repository_fingerprint, status, created_at, updated_at
             ) VALUES (?1, 'repo', 'ready', 'now', 'now')",
            [REPO],
        )
        .expect("repository");
    connection
}
