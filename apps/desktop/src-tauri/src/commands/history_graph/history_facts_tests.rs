use super::*;
use std::io::Cursor;
use std::process::Command;
use tempfile::TempDir;

struct HistoryFixture {
    root: TempDir,
    initial_sha: String,
    divergent_sha: String,
}

#[test]
fn bounded_reader_stops_at_the_limit_and_notifies_the_git_supervisor() {
    let overflow = AtomicBool::new(false);
    let (retained, truncated) =
        read_bounded_notifying(Cursor::new(vec![7_u8; 32 * 1024]), 1024, &overflow)
            .expect("bounded read");
    assert_eq!(retained.len(), 1024);
    assert!(truncated);
    assert!(overflow.load(Ordering::Acquire));
}

impl HistoryFixture {
    fn build() -> Self {
        let root = tempfile::tempdir().expect("history fixture");
        git(root.path(), &["init", "-b", "main"]);
        git(root.path(), &["config", "user.name", "Fixture Dev"]);
        git(
            root.path(),
            &["config", "user.email", "fixture@example.test"],
        );
        fs::create_dir_all(root.path().join("src")).expect("src");
        fs::write(
            root.path().join(".mailmap"),
            "Canonical Dev <canonical@example.test> Alias Dev <alias@example.test>\n\
             <canonical@example.test> <second-alias@example.test>\n",
        )
        .expect("mailmap");
        fs::write(root.path().join("src/base.rs"), "fn base() {}\n").expect("base");
        commit_as(
            root.path(),
            "2001-01-01T00:00:00Z",
            "Alias Dev",
            "alias@example.test",
            "initial",
        );
        let initial_sha = git_output(root.path(), &["rev-parse", "HEAD"]);
        git(root.path(), &["tag", "v0.1.0-lite"]);
        git_at(
            root.path(),
            &["tag", "-a", "v0.1.0", "-m", "old release"],
            "2001-01-01T00:01:00Z",
        );

        fs::create_dir_all(root.path().join("generated")).expect("generated");
        fs::create_dir_all(root.path().join("vendor/pkg")).expect("vendor");
        fs::write(
            root.path().join("src/line\nbreak\tfile.rs"),
            "fn unusual() {}\n",
        )
        .expect("unusual path");
        fs::write(
            root.path().join("generated/client.generated.ts"),
            "export const generated = true;\n",
        )
        .expect("generated file");
        fs::write(root.path().join("vendor/pkg/lib.js"), "vendor();\n").expect("vendor file");
        fs::write(root.path().join("asset.bin"), [0, 159, 146, 150, 255]).expect("binary");
        commit_as(
            root.path(),
            "2001-01-02T00:00:00Z",
            "Fixture Dev",
            "fixture@example.test",
            "normal paths",
        );

        let extreme = (0..2_000)
            .map(|index| format!("pub fn generated_{index}() {{}}\n"))
            .collect::<String>();
        fs::write(root.path().join("src/extreme.rs"), extreme).expect("extreme");
        commit_as(
            root.path(),
            "2001-01-03T00:00:00Z",
            "Fixture Dev",
            "fixture@example.test",
            "extreme churn\n\nCo-authored-by: Alias Dev <alias@example.test>\nCo-authored-by: Build Bot <bot@automation.test>",
        );

        git(root.path(), &["mv", "src/base.rs", "src/renamed.rs"]);
        commit_as(
            root.path(),
            "2001-01-04T00:00:00Z",
            "Fixture Dev",
            "fixture@example.test",
            "rename base",
        );
        fs::copy(
            root.path().join("src/renamed.rs"),
            root.path().join("src/copied.rs"),
        )
        .expect("copy source");
        fs::write(
            root.path().join("src/renamed.rs"),
            "fn base() {}\nfn changed() {}\n",
        )
        .expect("modify copy source");
        commit_as(
            root.path(),
            "2001-01-05T00:00:00Z",
            "Fixture Dev",
            "fixture@example.test",
            "copy base",
        );
        fs::remove_file(root.path().join("src/copied.rs")).expect("delete copy");
        commit_as(
            root.path(),
            "2001-01-06T00:00:00Z",
            "Fixture Dev",
            "fixture@example.test",
            "delete copy",
        );

        git(root.path(), &["checkout", "-b", "divergent", &initial_sha]);
        fs::write(root.path().join("side.txt"), "side\n").expect("side");
        commit_as(
            root.path(),
            "2001-01-07T00:00:00Z",
            "Side Dev",
            "side@example.test",
            "divergent",
        );
        let divergent_sha = git_output(root.path(), &["rev-parse", "HEAD"]);
        git(root.path(), &["tag", "v9.9.9-divergent"]);
        git(root.path(), &["checkout", "main"]);

        git(root.path(), &["checkout", "-b", "feature"]);
        fs::write(root.path().join("bot.txt"), "automation\n").expect("bot");
        commit_as(
            root.path(),
            "2001-01-08T00:00:00Z",
            "Build Bot [bot]",
            "bot@automation.test",
            "automated update",
        );
        git(root.path(), &["checkout", "main"]);
        fs::write(root.path().join("main.txt"), "main\n").expect("main");
        commit_as(
            root.path(),
            "2001-01-09T00:00:00Z",
            "Fixture Dev",
            "fixture@example.test",
            "main update",
        );
        git_at(
            root.path(),
            &["merge", "--no-ff", "feature", "-m", "merge feature"],
            "2001-01-10T00:00:00Z",
        );
        Self {
            root,
            initial_sha,
            divergent_sha,
        }
    }
}

#[test]
fn real_reader_captures_bounded_private_deterministic_history_facts() {
    let fixture = HistoryFixture::build();
    let cancellation = StructuralGraphCancellation::default();
    let first = read_all_history_facts(fixture.root.path(), &cancellation).expect("first read");
    let second = read_all_history_facts(fixture.root.path(), &cancellation).expect("second read");
    assert_eq!(first, second);
    assert_eq!(first.git_process_count, 1);
    assert_eq!(first.schema_version, HISTORY_FACTS_SCHEMA_VERSION);
    assert_eq!(
        first.classification_version,
        HISTORY_FACT_CLASSIFICATION_VERSION
    );
    assert!(first
        .revisions
        .iter()
        .all(|revision| revision.sha.len() == 40));
    assert!(!first
        .revisions
        .iter()
        .any(|revision| revision.sha == fixture.divergent_sha));

    let initial = first
        .revisions
        .iter()
        .find(|revision| revision.sha == fixture.initial_sha)
        .expect("initial fact");
    assert_eq!(initial.primary.display_name, "Canonical Dev");
    assert_eq!(initial.primary.alias_count, 2);
    assert_eq!(initial.tags, ["v0.1.0", "v0.1.0-lite"]);
    let tags = crate::commands::git_metadata::read_git_tags(fixture.root.path()).expect("tags");
    let annotated = tags
        .iter()
        .find(|tag| tag.name == "v0.1.0")
        .expect("annotated");
    let lightweight = tags
        .iter()
        .find(|tag| tag.name == "v0.1.0-lite")
        .expect("lightweight");
    assert_ne!(annotated.object_sha, annotated.commit_sha);
    assert_eq!(lightweight.object_sha, lightweight.commit_sha);

    let paths = first
        .revisions
        .iter()
        .flat_map(|revision| &revision.paths)
        .collect::<Vec<_>>();
    assert!(paths
        .iter()
        .any(|path| path.path == "src/line\nbreak\tfile.rs"));
    assert!(paths
        .iter()
        .any(|path| path.binary && path.path == "asset.bin"));
    assert!(paths.iter().any(|path| path.generated));
    assert!(paths.iter().any(|path| path.vendored));
    assert!(paths
        .iter()
        .any(|path| path.status == HistoryPathStatus::Renamed));
    assert!(paths
        .iter()
        .any(|path| path.status == HistoryPathStatus::Copied));
    assert!(paths
        .iter()
        .any(|path| path.status == HistoryPathStatus::Deleted));
    assert!(paths
        .iter()
        .any(|path| path.additions.unwrap_or_default() >= 2_000));
    assert!(first.revisions.iter().any(|revision| {
        revision.is_merge && revision.parents.len() == 2 && revision.subject == "merge feature"
    }));
    let coauthor_revision = first
        .revisions
        .iter()
        .find(|revision| revision.subject == "extreme churn")
        .expect("coauthor revision");
    assert!(coauthor_revision
        .coauthors
        .iter()
        .any(|identity| identity.display_name == "Canonical Dev" && identity.alias_count == 2));
    assert!(first
        .revisions
        .iter()
        .any(|revision| { revision.primary.automation == HistoryAutomationKind::Automation }));
    let debug = format!("{first:?}");
    for raw_email in [
        "alias@example.test",
        "canonical@example.test",
        "fixture@example.test",
        "bot@automation.test",
    ] {
        assert!(!debug.contains(raw_email));
    }
}

#[test]
fn incremental_reader_reads_only_fast_forward_commits_with_the_same_fact_shape() {
    let fixture = HistoryFixture::build();
    let cancellation = StructuralGraphCancellation::default();
    let full = read_all_history_facts(fixture.root.path(), &cancellation).expect("full read");
    let incremental =
        read_history_facts_since(fixture.root.path(), &fixture.initial_sha, &cancellation)
            .expect("incremental read");

    assert_eq!(incremental.git_process_count, 1);
    assert!(incremental
        .revisions
        .iter()
        .all(|revision| revision.sha != fixture.initial_sha));
    let expected = full
        .revisions
        .iter()
        .filter(|revision| revision.sha != fixture.initial_sha)
        .cloned()
        .collect::<Vec<_>>();
    assert_eq!(incremental.revisions, expected);
    incremental
        .validate()
        .expect("incremental facts remain valid");
}

#[test]
fn reader_cancellation_and_parser_bounds_fail_closed() {
    let fixture = HistoryFixture::build();
    let cancellation = StructuralGraphCancellation::default();
    cancellation.cancel();
    assert!(read_all_history_facts(fixture.root.path(), &cancellation)
        .expect_err("cancelled")
        .contains("cancelled"));

    let active = StructuralGraphCancellation::default();
    let mailmap = Mailmap::default();
    let valid = header("0123456789012345678901234567890123456789");
    assert!(
        parse_history_facts(&valid, Limits::default(), &cancellation, &mailmap, "repo")
            .expect_err("parse cancellation")
            .contains("cancelled")
    );
    let short = header("01234567");
    assert!(
        parse_history_facts(&short, Limits::default(), &active, &mailmap, "repo")
            .expect_err("short SHA")
            .contains("non-full")
    );
    assert!(parse_history_facts(
        &valid[..valid.len() - 2],
        Limits::default(),
        &active,
        &mailmap,
        "repo"
    )
    .is_err());
    let mut repeated = valid.clone();
    repeated.extend_from_slice(&valid);
    assert!(parse_history_facts(
        &repeated,
        Limits {
            revisions: 1,
            ..Limits::default()
        },
        &active,
        &mailmap,
        "repo"
    )
    .expect_err("record bound")
    .contains("revision bound"));
    assert!(parse_history_facts(
        &valid,
        Limits {
            output_bytes: valid.len() - 1,
            ..Limits::default()
        },
        &active,
        &mailmap,
        "repo"
    )
    .expect_err("byte bound")
    .contains("byte bound"));

    let (mailmap, _) = read_mailmap(fixture.root.path()).expect("fixture mailmap");
    let output =
        run_git_once(fixture.root.path(), "HEAD", &active, MAX_OUTPUT_BYTES).expect("git output");
    assert!(parse_history_facts(
        &output[..output.len() - 3],
        Limits::default(),
        &active,
        &mailmap,
        "repo"
    )
    .is_err());
    assert!(parse_history_facts(
        &output,
        Limits {
            paths: 0,
            ..Limits::default()
        },
        &active,
        &mailmap,
        "repo"
    )
    .expect_err("path bound")
    .contains("path bound"));
}

#[test]
fn shallow_clone_remains_bounded_and_divergent_history_stays_outside_head_walk() {
    let fixture = HistoryFixture::build();
    let shallow = tempfile::tempdir().expect("shallow target");
    fs::remove_dir(shallow.path()).expect("empty target removal");
    let source = format!("file://{}", fixture.root.path().display());
    let status = Command::new("git")
        .args(["clone", "--depth", "2", &source])
        .arg(shallow.path())
        .status()
        .expect("shallow clone");
    assert!(status.success());
    let batch = read_all_history_facts(shallow.path(), &StructuralGraphCancellation::default())
        .expect("shallow facts");
    assert_eq!(batch.git_process_count, 1);
    // Depth two retains the merge tip plus both direct parents.
    assert_eq!(batch.revisions.len(), 3);
    assert_eq!(
        git_output(shallow.path(), &["rev-parse", "--is-shallow-repository"]),
        "true"
    );
    assert!(!batch
        .revisions
        .iter()
        .any(|revision| revision.sha == fixture.divergent_sha));
}

#[test]
fn contributor_ids_are_repository_scoped_and_mailmap_name_fallback_is_preserved() {
    let map = Mailmap {
        entries: vec![
            parse_mailmap_entry("<proper@example.test> <alias@example.test>")
                .expect("email-only mailmap"),
        ],
    };
    let (name, email) = map.resolve("Visible Alias", "alias@example.test");
    assert_eq!((name, email), ("Visible Alias", "proper@example.test"));
    assert_ne!(
        identity_fact("repo-a", name, email, 1).contributor_id,
        identity_fact("repo-b", name, email, 1).contributor_id
    );
    assert_eq!(classify_history_path("dist/client.min.js"), (true, false));
    assert_eq!(classify_history_path("third_party/lib.rs"), (false, true));
}

fn header(sha: &str) -> Vec<u8> {
    let mut output = Vec::new();
    for field in [
        MARKER,
        sha.as_bytes(),
        b"",
        b"2001-01-01T00:00:00Z",
        b"Fixture",
        b"fixture@example.test",
        b"HEAD -> refs/heads/main",
        b"subject",
        b"",
    ] {
        output.extend_from_slice(field);
        output.push(0);
    }
    output
}

fn commit_as(root: &Path, timestamp: &str, name: &str, email: &str, message: &str) {
    git(root, &["add", "-A"]);
    let status = Command::new("git")
        .arg("-C")
        .arg(root)
        .args(["commit", "-m", message])
        .env("GIT_AUTHOR_DATE", timestamp)
        .env("GIT_COMMITTER_DATE", timestamp)
        .env("GIT_AUTHOR_NAME", name)
        .env("GIT_AUTHOR_EMAIL", email)
        .env("GIT_COMMITTER_NAME", name)
        .env("GIT_COMMITTER_EMAIL", email)
        .status()
        .expect("git commit");
    assert!(status.success());
}

fn git_at(root: &Path, arguments: &[&str], timestamp: &str) {
    let status = Command::new("git")
        .arg("-C")
        .arg(root)
        .args(arguments)
        .env("GIT_AUTHOR_DATE", timestamp)
        .env("GIT_COMMITTER_DATE", timestamp)
        .status()
        .expect("dated git");
    assert!(status.success(), "git {arguments:?}");
}

fn git(root: &Path, arguments: &[&str]) {
    let status = Command::new("git")
        .arg("-C")
        .arg(root)
        .args(arguments)
        .status()
        .expect("git");
    assert!(status.success(), "git {arguments:?}");
}

fn git_output(root: &Path, arguments: &[&str]) -> String {
    let output = Command::new("git")
        .arg("-C")
        .arg(root)
        .args(arguments)
        .output()
        .expect("git output");
    assert!(output.status.success(), "git {arguments:?}");
    String::from_utf8(output.stdout)
        .expect("utf8 git output")
        .trim()
        .to_string()
}
