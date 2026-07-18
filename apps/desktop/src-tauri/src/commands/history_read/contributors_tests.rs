use super::*;
use rusqlite::Connection;

const REPO: &str = "/fixture/contributors";

#[test]
fn release_cycle_separates_primary_churn_coauthors_and_automation() {
    let fixture = Fixture::new(false);
    let summary = fixture
        .service()
        .contributor_summary(
            HistoryContributorScope::ReleaseCycleThrough {
                tag: "v2.0.0".to_string(),
                to_inclusive: None,
            },
            Some(10),
            None,
        )
        .expect("release contributor summary");
    assert_eq!(summary.from_exclusive.as_deref(), Some(sha('B').as_str()));
    assert_eq!(summary.to_inclusive, sha('E'));
    assert_eq!(summary.totals.primary_commits, 3);
    assert_eq!(summary.totals.coauthor_participations, 2);
    assert_eq!(
        (summary.totals.additions, summary.totals.deletions),
        (13, 2)
    );
    assert_eq!(summary.automation_primary_commit_share, 1.0 / 3.0);
    assert_eq!(summary.human_primary_commit_share, 2.0 / 3.0);
    assert!(summary
        .caveats
        .contains(&"binary_churn_unavailable".to_string()));
    assert!(summary
        .caveats
        .contains(&"generated_paths_present".to_string()));
    assert!(summary
        .caveats
        .contains(&"vendored_paths_present".to_string()));
    assert!(summary
        .caveats
        .contains(&"merge_commits_present".to_string()));
    let alice = summary
        .contributors
        .iter()
        .find(|row| row.contributor_id == "contributor:alice")
        .expect("canonical Alice");
    assert_eq!(alice.display_name, "Alice Canonical");
    assert_eq!(alice.alias_count, 2);
    assert_eq!(alice.activity.primary_commits, 1);
    assert_eq!(alice.activity.coauthor_participations, 1);
    assert_eq!(
        alice.revisions,
        vec![
            HistoryContributorRevision {
                sha: sha('E'),
                role: "coauthor".to_string(),
            },
            HistoryContributorRevision {
                sha: sha('C'),
                role: "primary".to_string(),
            },
        ]
    );
    assert_eq!(
        (alice.activity.additions, alice.activity.deletions),
        (10, 0)
    );
    assert!(summary
        .contributors
        .iter()
        .any(|row| row.identity_kind == "automation"));
    assert!(summary.contributors.iter().all(|row| {
        !row.contributor_id.contains('@')
            && row.evidence_ids.len() <= 16
            && row.revisions.len() <= 16
            && row.areas.len() <= 8
    }));
    let serialized = serde_json::to_string(&summary).expect("summary json");
    assert!(!serialized.contains("ownership"));
    assert!(!serialized.contains("quality"));
}

#[test]
fn exact_ancestry_interval_and_bounded_pages_reconcile_with_other() {
    let fixture = Fixture::new(false);
    let scope = HistoryContributorScope::ExactInterval {
        from_exclusive: Some(sha('A')),
        to_inclusive: sha('E'),
    };
    let first = fixture
        .service()
        .contributor_summary(scope.clone(), Some(1), Some(0))
        .expect("first page");
    let second = fixture
        .service()
        .contributor_summary(scope, Some(1), first.next_offset)
        .expect("second page");
    assert_eq!(first.applied_limit, 1);
    assert_eq!(first.applied_offset, 0);
    assert!(first.truncated);
    assert_eq!(second.applied_offset, 1);
    assert_eq!(first.totals.primary_commits, 4);
    assert_eq!(first.totals.contributor_count, 3);
    assert_eq!(first.totals.coauthor_participations, 2);
    assert_eq!((first.totals.additions, first.totals.deletions), (18, 3));
    assert_eq!(
        first.contributors[0].activity.primary_commits + first.other.primary_commits,
        first.totals.primary_commits
    );
    assert_eq!(
        first.contributors[0].activity.contributor_count + first.other.contributor_count,
        first.totals.contributor_count
    );
    assert_eq!(
        first.contributors[0].activity.additions + first.other.additions,
        first.totals.additions
    );
    assert_eq!(
        second.contributors[0].activity.primary_commits + second.other.primary_commits,
        second.totals.primary_commits
    );
    assert_eq!(first.top_human_primary_concentration, 2.0 / 3.0);
    assert_eq!(first.automation_primary_commit_share, 0.25);
    assert_eq!(
        first,
        fixture
            .service()
            .contributor_summary(
                HistoryContributorScope::ExactInterval {
                    from_exclusive: Some(sha('A')),
                    to_inclusive: sha('E'),
                },
                Some(1),
                Some(0),
            )
            .expect("deterministic repeat")
    );
}

#[test]
fn opaque_contributor_cursor_is_deterministic_and_rejects_scope_or_index_drift() {
    let fixture = Fixture::new(false);
    let scope = HistoryContributorScope::ExactInterval {
        from_exclusive: Some(sha('A')),
        to_inclusive: sha('E'),
    };
    let first = fixture
        .service()
        .contributor_summary_page(scope.clone(), Some(1), None)
        .expect("first cursor page");
    let cursor = first.next_cursor.as_ref().expect("opaque continuation");
    assert_eq!(first.next_offset, Some(1));
    let second = fixture
        .service()
        .contributor_summary_page(scope.clone(), Some(1), Some(cursor))
        .expect("second cursor page");
    assert_eq!(second.applied_offset, 1);
    assert_eq!(
        fixture
            .service()
            .contributor_summary_page(
                HistoryContributorScope::ExactInterval {
                    from_exclusive: Some(sha('B')),
                    to_inclusive: sha('E'),
                },
                Some(1),
                Some(cursor),
            )
            .unwrap_err(),
        "History cursor does not match this repository or query"
    );
    fixture
        .connection
        .execute(
            "UPDATE history_graph_fact_catalogs SET indexed_head = ?1 WHERE repo_path = ?2",
            params![sha('D'), REPO],
        )
        .expect("advance contributor index");
    assert_eq!(
        fixture
            .service()
            .contributor_summary_page(scope, Some(1), Some(cursor))
            .unwrap_err(),
        "History cursor is stale"
    );
}

#[test]
fn legacy_contributor_index_returns_an_explicit_empty_versioned_summary() {
    let connection = Connection::open_in_memory().expect("database");
    crate::db::schema::run_migrations(&connection).expect("schema");
    connection
        .execute_batch(&format!(
            "INSERT INTO history_graph_repositories (
                repo_path, repository_fingerprint, indexed_head, status, created_at, updated_at
             ) VALUES ('/fixture/legacy-contributors', 'legacy', '{}', 'ready', 'now', 'now');
             INSERT INTO history_graph_revisions (
                repo_path, sha, ordinal, committed_at, author_name, subject, parents_json, tags_json, is_head
             ) VALUES ('/fixture/legacy-contributors', '{}', 0, 'now', 'Legacy', 'legacy', '[]', '[]', 1);",
            sha('A'),
            sha('A')
        ))
        .expect("existing legacy timeline");
    let service = HistoryReadService::new_with_current_head(
        &connection,
        PathBuf::from("/fixture/legacy-contributors"),
        sha('A'),
    )
    .expect("service");
    let summary = service
        .contributor_summary_page(
            HistoryContributorScope::ExactInterval {
                from_exclusive: None,
                to_inclusive: sha('A'),
            },
            None,
            None,
        )
        .expect("empty legacy summary");
    assert_eq!(
        summary.schema_version,
        HISTORY_CONTRIBUTOR_SUMMARY_SCHEMA_VERSION
    );
    assert!(summary.contributors.is_empty());
    assert_eq!(summary.coverage, HistoryCoverageState::Unavailable);
    assert_eq!(summary.applied_limit, 20);
    assert!(summary.next_cursor.is_none());
}

#[test]
fn divergent_and_non_ancestral_intervals_fail_while_shallow_is_partial() {
    let complete = Fixture::new(false);
    assert!(complete
        .service()
        .contributor_summary(
            HistoryContributorScope::ReleaseCycleThrough {
                tag: "v9.9.9".to_string(),
                to_inclusive: None,
            },
            None,
            None,
        )
        .expect_err("divergent release")
        .contains("Divergent"));
    assert!(complete
        .service()
        .contributor_summary(
            HistoryContributorScope::ExactInterval {
                from_exclusive: Some(sha('X')),
                to_inclusive: sha('E'),
            },
            None,
            None,
        )
        .expect_err("non ancestor")
        .contains("not an ancestor"));

    let shallow = Fixture::new(true);
    let summary = shallow
        .service()
        .contributor_summary(
            HistoryContributorScope::ReleaseCycleThrough {
                tag: "v2.0.0".to_string(),
                to_inclusive: None,
            },
            Some(200),
            None,
        )
        .expect("partial summary");
    assert_eq!(summary.applied_limit, MAX_CONTRIBUTOR_LIMIT);
    assert_eq!(summary.coverage, HistoryCoverageState::Partial);
    assert!(summary.caveats.contains(&"shallow".to_string()));
    assert!(summary
        .caveats
        .contains(&"ancestry_coverage_partial".to_string()));
}

#[test]
fn privacy_unsafe_persisted_identity_fails_closed() {
    let fixture = Fixture::new(false);
    fixture
        .connection
        .execute(
            "UPDATE history_graph_contributors SET display_name = 'raw@example.test'
             WHERE repo_path = ?1 AND contributor_id = 'contributor:alice'",
            [REPO],
        )
        .expect("unsafe fixture identity");
    assert!(fixture
        .service()
        .contributor_summary(
            HistoryContributorScope::ExactInterval {
                from_exclusive: Some(sha('A')),
                to_inclusive: sha('E'),
            },
            None,
            None,
        )
        .expect_err("privacy failure")
        .contains("privacy-safe"));
}

#[test]
fn large_repository_bounds_only_the_requested_ancestry_interval() {
    let connection = large_linear_database(6_002);
    let service = HistoryReadService::new_with_current_head(
        &connection,
        PathBuf::from(REPO),
        large_sha(6_001),
    )
    .expect("large service");

    let recent = service
        .contributor_summary(
            HistoryContributorScope::ExactInterval {
                from_exclusive: Some(large_sha(5_991)),
                to_inclusive: large_sha(6_001),
            },
            Some(5),
            None,
        )
        .expect("small interval in large repository");
    assert_eq!(recent.totals.primary_commits, 10);

    assert_eq!(
        service
            .interval_revisions(Some(&large_sha(1_001)), &large_sha(6_001), false)
            .expect("exact 5000")
            .len(),
        MAX_INTERVAL_REVISIONS
    );
    assert!(service
        .interval_revisions(Some(&large_sha(1_000)), &large_sha(6_001), false)
        .expect_err("5001 rejected")
        .contains("revision bound"));

    let plan = connection
        .prepare(
            "EXPLAIN QUERY PLAN WITH RECURSIVE ancestry(sha, parents_json) AS (
                SELECT sha, parents_json FROM history_graph_revisions
                 WHERE repo_path = ?1 AND sha = ?2
                UNION
                SELECT parent.sha, parent.parents_json FROM ancestry child
                 JOIN json_each(child.parents_json) edge
                 JOIN history_graph_revisions parent
                   ON parent.repo_path = ?1 AND parent.sha = edge.value
             ) SELECT sha FROM ancestry",
        )
        .expect("query plan")
        .query_map(params![REPO, large_sha(6_001)], |row| {
            row.get::<_, String>(3)
        })
        .expect("plan rows")
        .collect::<Result<Vec<_>, _>>()
        .expect("plan")
        .join(" ");
    assert!(plan.contains("repo_path") && plan.contains("sha"), "{plan}");
}

struct Fixture {
    connection: Connection,
}

impl Fixture {
    fn new(partial: bool) -> Self {
        let connection = Connection::open_in_memory().expect("database");
        crate::db::schema::run_migrations(&connection).expect("schema");
        connection
            .execute_batch(&fixture_sql(partial))
            .expect("contributor fixture");
        Self { connection }
    }

    fn service(&self) -> HistoryReadService<'_> {
        HistoryReadService::new_with_current_head(&self.connection, PathBuf::from(REPO), sha('E'))
            .expect("service")
    }
}

fn fixture_sql(partial: bool) -> String {
    let catalog_status = if partial { "partial" } else { "ready" };
    let interval_kind = if partial { "shallow" } else { "complete" };
    format!(
        "INSERT INTO history_graph_repositories (
            repo_path, repository_fingerprint, indexed_head, status, created_at, updated_at
         ) VALUES ('{REPO}', 'repo', '{e}', 'ready', 'now', 'now');
         INSERT INTO history_graph_fact_catalogs (
            repo_path, schema_version, classification_version, index_identity, indexed_head,
            tags_fingerprint, mailmap_fingerprint, facts_fingerprint, status, updated_at
         ) VALUES ('{REPO}', 1, 1, 'facts', '{e}', 'tags', 'mailmap:canonical', 'facts', 'ready', 'now');
         INSERT INTO history_graph_release_catalogs (
            repo_path, index_identity, indexed_head, tags_fingerprint, status, coverage_json, updated_at
         ) VALUES ('{REPO}', 'releases', '{e}', 'tags', '{catalog_status}', '{{}}', 'now');
         INSERT INTO history_graph_revisions (
            repo_path, sha, ordinal, committed_at, author_name, subject, parents_json, tags_json, is_head
         ) VALUES
            ('{REPO}', '{a}', 0, '2026-01-01T00:00:00Z', 'Alice Canonical', 'A', '[]', '[]', 0),
            ('{REPO}', '{b}', 1, '2026-01-02T00:00:00Z', 'Alice Canonical', 'B', '[\"{a}\"]', '[]', 0),
            ('{REPO}', '{c}', 2, '2026-01-03T00:00:00Z', 'Alice Canonical', 'C', '[\"{b}\"]', '[]', 0),
            ('{REPO}', '{d}', 3, '2026-01-03T00:00:00Z', 'Build Bot', 'D', '[\"{b}\"]', '[]', 0),
            ('{REPO}', '{e}', 4, '2026-01-04T00:00:00Z', 'Bob', 'E', '[\"{c}\",\"{d}\"]', '[]', 1),
            ('{REPO}', '{x}', 5, '2026-01-05T00:00:00Z', 'Other', 'X', '[]', '[]', 0);
         INSERT INTO history_graph_fact_tags (
            repo_path, tag, revision_sha, tag_object_sha, tag_kind, tagged_at
         ) VALUES
            ('{REPO}', 'v2.0.0', '{e}', '{e}', 'lightweight', 1),
            ('{REPO}', 'v9.9.9', '{x}', '{x}', 'lightweight', 2);
         INSERT INTO history_graph_release_intervals (
            repo_path, tag, revision_sha, from_exclusive_sha, commit_count,
            observed_commit_count, coverage_kind
         ) VALUES
            ('{REPO}', 'v2.0.0', '{e}', '{b}', {count}, 3, '{interval_kind}'),
            ('{REPO}', 'v9.9.9', '{x}', NULL, NULL, 0, 'divergent');
         INSERT INTO history_graph_contributors (
            repo_path, contributor_id, display_name, identity_kind, alias_count) VALUES
            ('{REPO}', 'contributor:alice', 'Alice Canonical', 'human', 2),
            ('{REPO}', 'contributor:bob', 'Bob', 'human', 0),
            ('{REPO}', 'contributor:bot', 'Build Bot', 'automation', 0),
            ('{REPO}', 'contributor:other', 'Other', 'unknown', 0);
         INSERT INTO history_graph_revision_contributors (repo_path, revision_sha, contributor_id, role) VALUES
            ('{REPO}', '{a}', 'contributor:alice', 'primary'),
            ('{REPO}', '{b}', 'contributor:alice', 'primary'),
            ('{REPO}', '{c}', 'contributor:alice', 'primary'),
            ('{REPO}', '{c}', 'contributor:bob', 'coauthor'),
            ('{REPO}', '{d}', 'contributor:bot', 'primary'),
            ('{REPO}', '{e}', 'contributor:bob', 'primary'),
            ('{REPO}', '{e}', 'contributor:alice', 'coauthor'),
            ('{REPO}', '{x}', 'contributor:other', 'primary');
         INSERT INTO history_graph_revision_paths (
            repo_path, revision_sha, path, change_kind, additions, deletions, binary, generated, vendored
         ) VALUES
            ('{REPO}', '{a}', 'src/a.rs', 'added', 1, 0, 0, 0, 0),
            ('{REPO}', '{b}', 'src/b.rs', 'added', 5, 1, 0, 0, 0),
            ('{REPO}', '{c}', 'generated/client.ts', 'added', 10, 0, 0, 1, 0),
            ('{REPO}', '{d}', 'vendor/blob.bin', 'added', NULL, NULL, 1, 0, 1),
            ('{REPO}', '{e}', 'app/main.rs', 'modified', 3, 2, 0, 0, 0);",
        a = sha('A'),
        b = sha('B'),
        c = sha('C'),
        d = sha('D'),
        e = sha('E'),
        x = sha('X'),
        count = if partial { "NULL" } else { "3" },
    )
}

fn large_linear_database(revision_count: usize) -> Connection {
    let mut connection = Connection::open_in_memory().expect("large database");
    crate::db::schema::run_migrations(&connection).expect("schema");
    connection
        .execute(
            "INSERT INTO history_graph_repositories (
                repo_path, repository_fingerprint, indexed_head, status, created_at, updated_at
             ) VALUES (?1, 'large', ?2, 'ready', 'now', 'now')",
            params![REPO, large_sha(revision_count - 1)],
        )
        .expect("large repository");
    connection
        .execute(
            "INSERT INTO history_graph_fact_catalogs (
                repo_path, schema_version, classification_version, index_identity,
                indexed_head, tags_fingerprint, mailmap_fingerprint, facts_fingerprint,
                status, updated_at
             ) VALUES (?1, 1, 1, 'large-facts', ?2, 'tags', 'mailmap', 'facts', 'ready', 'now')",
            params![REPO, large_sha(revision_count - 1)],
        )
        .expect("large facts");
    connection
        .execute(
            "INSERT INTO history_graph_contributors (
                repo_path, contributor_id, display_name, identity_kind, alias_count
             ) VALUES (?1, 'contributor:large', 'Large Fixture', 'human', 0)",
            [REPO],
        )
        .expect("large contributor");
    let transaction = connection.transaction().expect("large transaction");
    {
        let mut revision_statement = transaction
            .prepare(
                "INSERT INTO history_graph_revisions (
                    repo_path, sha, ordinal, committed_at, author_name, subject,
                    parents_json, tags_json, is_head
                 ) VALUES (?1, ?2, ?3, '2026-01-01T00:00:00Z', 'Large Fixture',
                    'commit', ?4, '[]', ?5)",
            )
            .expect("large revisions");
        let mut role_statement = transaction
            .prepare(
                "INSERT INTO history_graph_revision_contributors (
                    repo_path, revision_sha, contributor_id, role
                 ) VALUES (?1, ?2, 'contributor:large', 'primary')",
            )
            .expect("large roles");
        for ordinal in 0..revision_count {
            let revision = large_sha(ordinal);
            let parents = if ordinal == 0 {
                "[]".to_string()
            } else {
                serde_json::to_string(&[large_sha(ordinal - 1)]).expect("parents")
            };
            revision_statement
                .execute(params![
                    REPO,
                    revision,
                    ordinal as i64,
                    parents,
                    i64::from(ordinal + 1 == revision_count),
                ])
                .expect("revision");
            if ordinal + 10 >= revision_count {
                role_statement
                    .execute(params![REPO, large_sha(ordinal)])
                    .expect("role");
            }
        }
    }
    transaction.commit().expect("large commit");
    connection
}

fn large_sha(ordinal: usize) -> String {
    format!("{:040x}", ordinal + 1)
}

fn sha(character: char) -> String {
    let character = if character == 'X' {
        '9'
    } else {
        character.to_ascii_lowercase()
    };
    character.to_string().repeat(40)
}
