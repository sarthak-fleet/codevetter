use super::*;
use std::fs;

#[test]
fn every_promised_language_extracts_a_named_symbol() {
    let fixtures = [
        ("a.ts", "export function alpha() { beta(); }", "alpha"),
        (
            "a.tsx",
            "export function Alpha() { beta(); return <div/>; }",
            "Alpha",
        ),
        ("a.js", "function alpha() { beta(); }", "alpha"),
        (
            "a.jsx",
            "function Alpha() { beta(); return <div/>; }",
            "Alpha",
        ),
        ("a.rs", "fn alpha() { beta(); }", "alpha"),
        ("a.py", "def alpha():\n    beta()\n", "alpha"),
        ("a.go", "package a\nfunc alpha() { beta() }", "alpha"),
        ("A.java", "class A { void alpha() { beta(); } }", "alpha"),
        ("a.c", "void alpha(void) { beta(); }", "alpha"),
        ("a.cpp", "class A { void alpha() { beta(); } };", "alpha"),
        ("A.cs", "class A { void Alpha() { Beta(); } }", "Alpha"),
        ("a.rb", "def alpha\n  beta()\nend", "alpha"),
        ("a.php", "<?php function alpha() { beta(); }", "alpha"),
        ("A.kt", "fun alpha() { beta() }", "alpha"),
        ("A.swift", "func alpha() { beta() }", "alpha"),
    ];
    for (path, source, symbol) in fixtures {
        let language = SupportedLanguage::from_path(Path::new(path)).expect("language");
        let contribution = extract_source(path, language, source);
        assert_eq!(contribution.disposition, FileDisposition::Indexed, "{path}");
        assert!(
            contribution.nodes.iter().any(|node| node.label == symbol),
            "{path} should contain {symbol}; nodes: {:?}; diagnostics: {:?}",
            contribution
                .nodes
                .iter()
                .map(|node| (&node.kind, &node.label))
                .collect::<Vec<_>>(),
            contribution.diagnostics
        );
        assert!(
            contribution.nodes.iter().any(|node| node.kind == "file"),
            "{path} should include its file/module anchor"
        );
        assert!(
            contribution.edges.iter().any(|edge| edge.kind == "defines"),
            "{path} should include a direct definition edge"
        );
        assert!(
            contribution.edges.iter().any(|edge| edge.kind == "calls"),
            "{path} should include a direct call edge"
        );
        let declaration = contribution
            .nodes
            .iter()
            .find(|node| node.label == symbol)
            .expect("declaration");
        assert_eq!(declaration.sources[0].path, path);
        assert!(declaration.sources[0].start_line.is_some());
        let metric = contribution
            .metrics
            .iter()
            .find(|fact| fact.node_id == declaration.id)
            .unwrap_or_else(|| panic!("{path} should publish metrics for {symbol}"));
        assert_eq!(metric.schema_version, 1);
        assert_eq!(metric.path, path);
        assert_eq!(metric.language, language.name());
        assert!(metric.metrics.cyclomatic_complexity >= 1);
        assert!(!metric.sources.is_empty());
    }
}

#[test]
fn modules_fields_and_nested_qualified_names_are_source_located() {
    let rust = extract_source(
        "src/model.rs",
        SupportedLanguage::Rust,
        "mod inner { struct User { name: String } impl User { fn save(&self) {} } }",
    );
    assert!(rust
        .nodes
        .iter()
        .any(|node| node.kind == "module" && node.label == "inner"));
    assert!(rust.nodes.iter().any(|node| {
        node.label == "User"
            && node
                .qualified_name
                .as_deref()
                .is_some_and(|name| name.contains("inner::User"))
    }));

    let typescript = extract_source(
        "src/model.ts",
        SupportedLanguage::TypeScript,
        "export class User { name: string; save(): void {} }",
    );
    assert!(typescript
        .nodes
        .iter()
        .any(|node| node.kind == "field" && node.label == "name"));
    assert!(typescript
        .edges
        .iter()
        .any(|edge| edge.kind == "has_type" && edge.trust == GraphTrust::Extracted));
    assert!(typescript.nodes.iter().any(|node| {
        node.kind == "method"
            && node
                .qualified_name
                .as_deref()
                .is_some_and(|name| name.contains("User::save"))
    }));
    assert!(typescript
        .edges
        .iter()
        .any(|edge| edge.kind == "exports" && edge.trust == GraphTrust::Extracted));
}

#[test]
fn source_locations_are_one_based_and_calls_are_source_backed() {
    let contribution = extract_source(
        "a.rs",
        SupportedLanguage::Rust,
        "fn alpha() {\n    beta();\n}\n",
    );
    let function = contribution
        .nodes
        .iter()
        .find(|node| node.kind == "function")
        .expect("function");
    assert_eq!(function.sources[0].start_line, Some(1));
    let call = contribution
        .edges
        .iter()
        .find(|edge| edge.kind == "calls")
        .expect("call edge");
    assert_eq!(call.sources[0].start_line, Some(2));
    assert_eq!(call.trust, GraphTrust::Extracted);
}

#[test]
fn source_metadata_extracts_product_boundaries_and_analytics() {
    let contribution = extract_source(
        "src/app.tsx",
        SupportedLanguage::Tsx,
        r#"
            <Route path="/settings" element={<Settings />} />
            trackCoreAction('settings_opened');
            test("opens settings", () => {});
            "#,
    );
    for (kind, label) in [
        ("route", "/settings"),
        ("analytics_event", "settings_opened"),
        ("test", "opens settings"),
    ] {
        let node = contribution
            .nodes
            .iter()
            .find(|node| node.kind == kind && node.label == label)
            .unwrap_or_else(|| panic!("missing {kind} {label}"));
        assert_eq!(node.origin, GraphOrigin::Metadata);
        assert_eq!(node.trust, GraphTrust::Extracted);
        assert!(node.sources[0].start_line.is_some());
    }
}

#[test]
fn source_metadata_extracts_tauri_commands_and_sql_objects() {
    let contribution = extract_source(
        "src/main.rs",
        SupportedLanguage::Rust,
        r#"
            #[tauri::command]
            async fn build_graph() {}
            const SQL: &str = "CREATE TABLE IF NOT EXISTS graph_nodes (id TEXT);";
            "#,
    );
    assert!(contribution
        .nodes
        .iter()
        .any(|node| node.kind == "tauri_command" && node.label == "build_graph"));
    assert!(contribution
        .nodes
        .iter()
        .any(|node| node.kind == "db_table" && node.label == "graph_nodes"));
}

#[test]
fn framework_routes_and_sql_lineage_resolve_to_exact_implementations() {
    let route = extract_source(
            "src/routes.ts",
            SupportedLanguage::TypeScript,
            "export function listUsers() { return db.query('SELECT * FROM users'); }\nrouter.get('/users', listUsers);\n",
        );
    let schema = extract_blob(
        "db/schema.sql",
        b"CREATE TABLE users (id INTEGER PRIMARY KEY);",
        1024,
    );
    let mut nodes = route
        .nodes
        .into_iter()
        .chain(schema.nodes)
        .collect::<Vec<_>>();
    let mut edges = route
        .edges
        .into_iter()
        .chain(schema.edges)
        .collect::<Vec<_>>();
    deduplicate_nodes(&mut nodes);
    deduplicate_edges(&mut edges);
    resolve_cross_file(&nodes, &mut edges);

    let list_users_id = nodes
        .iter()
        .find(|node| node.kind == "function" && node.label == "listUsers")
        .expect("route handler")
        .id
        .clone();
    let route_node_id = nodes
        .iter()
        .find(|node| node.kind == "route" && node.label == "/users")
        .expect("route")
        .id
        .clone();
    assert!(edges.iter().any(|edge| {
        edge.from == route_node_id
            && edge.to == list_users_id
            && edge.kind == "routes_to"
            && edge.trust == GraphTrust::Inferred
    }));

    let users_table_id = nodes
        .iter()
        .find(|node| node.kind == "db_table" && node.label == "users")
        .expect("users table")
        .id
        .clone();
    assert!(edges.iter().any(|edge| {
        edge.from == list_users_id
            && edge.to == users_table_id
            && edge.kind == "reads_from"
            && edge.trust == GraphTrust::Inferred
    }));

    let communities = analyze_graph(&mut nodes, &edges);
    let summary = crate::commands::structural_graph::analysis::summarize_graph_analysis(
        &nodes,
        &edges,
        &communities,
    );
    assert!(summary.algorithms.execution_flows.iter().any(|flow| {
        flow.node_ids
            .starts_with(&[route_node_id.clone(), list_users_id.clone()])
            && flow.node_ids.contains(&users_table_id)
    }));
}

#[test]
fn ambiguous_framework_handlers_retain_candidates() {
    let route = extract_source(
        "src/routes.ts",
        SupportedLanguage::TypeScript,
        "router.get('/users', listUsers);",
    );
    let first = extract_source(
        "src/admin.ts",
        SupportedLanguage::TypeScript,
        "export function listUsers() {}",
    );
    let second = extract_source(
        "src/public.ts",
        SupportedLanguage::TypeScript,
        "export function listUsers() {}",
    );
    let mut nodes = route
        .nodes
        .into_iter()
        .chain(first.nodes)
        .chain(second.nodes)
        .collect::<Vec<_>>();
    let mut edges = route
        .edges
        .into_iter()
        .chain(first.edges)
        .chain(second.edges)
        .collect::<Vec<_>>();
    deduplicate_nodes(&mut nodes);
    deduplicate_edges(&mut edges);
    resolve_cross_file(&nodes, &mut edges);

    assert!(edges.iter().any(|edge| {
        edge.kind == "candidate_for"
            && edge.trust == GraphTrust::Ambiguous
            && edge.candidates.len() == 2
    }));
    assert!(!edges.iter().any(|edge| {
        edge.kind == "routes_to"
            && edge.origin == GraphOrigin::Resolution
            && edge.trust == GraphTrust::Inferred
    }));
}

#[test]
fn dynamic_references_remain_escape_hatches_even_with_one_named_candidate() {
    let contribution = extract_source(
            "src/runtime.ts",
            SupportedLanguage::TypeScript,
            "export function UserService() {}\nexport function resolve() { return container.resolve('UserService'); }",
        );
    let mut nodes = contribution.nodes;
    let mut edges = contribution.edges;
    deduplicate_nodes(&mut nodes);
    deduplicate_edges(&mut edges);
    resolve_cross_file(&nodes, &mut edges);

    let dynamic = nodes
        .iter()
        .find(|node| node.kind == "dynamic_reference")
        .expect("dynamic reference");
    assert_eq!(dynamic.trust, GraphTrust::Ambiguous);
    let candidate = edges
        .iter()
        .find(|edge| edge.kind == "candidate_for" && edge.to == dynamic.id)
        .expect("dynamic candidate edge");
    assert_eq!(candidate.trust, GraphTrust::Ambiguous);
    assert_eq!(candidate.candidates.len(), 1);
    assert!(!edges.iter().any(|edge| {
        edge.origin == GraphOrigin::Resolution
            && edge.trust == GraphTrust::Inferred
            && edge.kind == "may_reference"
    }));

    let communities = analyze_graph(&mut nodes, &edges);
    let summary = crate::commands::structural_graph::analysis::summarize_graph_analysis(
        &nodes,
        &edges,
        &communities,
    );
    assert!(summary
        .coverage
        .gaps
        .contains(&"dynamic_references:1".to_string()));
    assert!(!summary.coverage.reachability_complete);
}

#[test]
fn contract_file_extensions_are_indexed_with_source_backed_facts() {
    for (path, source, kind) in [
        (
            "api/user.proto",
            "message User {}\nservice Users { rpc Get (User) returns (User); }",
            "protobuf_message",
        ),
        (
            "api/schema.graphql",
            "type User { id: ID! }",
            "graphql_type",
        ),
        (
            "api/schema.gql",
            "input UserInput { id: ID! }",
            "graphql_input",
        ),
    ] {
        let contribution = extract_blob(path, source.as_bytes(), 4096);
        assert_eq!(contribution.disposition, FileDisposition::Indexed, "{path}");
        let fact = contribution
            .nodes
            .iter()
            .find(|node| node.kind == kind)
            .unwrap_or_else(|| panic!("missing {kind} in {path}"));
        assert_eq!(fact.trust, GraphTrust::Extracted);
        assert_eq!(fact.sources[0].path, path);
        assert!(fact.sources[0].start_line.is_some());
    }
}

#[test]
fn metadata_text_files_extract_docs_links_rationale_and_configuration() {
    let root = std::env::temp_dir().join(format!(
        "codevetter-structural-metadata-{}",
        uuid::Uuid::new_v4()
    ));
    fs::create_dir_all(&root).expect("fixture root");
    fs::write(
        root.join("README.md"),
        "# Notes\nDecision: keep parsing local\n[Architecture](docs/architecture.md)\n",
    )
    .expect("readme");
    fs::write(root.join("package.json"), "{\"name\":\"fixture\"}\n").expect("package config");
    let docs = extract_metadata_path(&root, Path::new("README.md"), "README.md", 1024);
    assert_eq!(docs.disposition, FileDisposition::Indexed);
    assert!(docs.nodes.iter().any(|node| node.kind == "decision"));
    assert!(docs
        .nodes
        .iter()
        .any(|node| { node.kind == "documentation_link" && node.label == "docs/architecture.md" }));
    let config = extract_metadata_path(&root, Path::new("package.json"), "package.json", 1024);
    assert!(config.nodes.iter().any(|node| node.kind == "configuration"));
    fs::remove_dir_all(root).expect("remove fixture root");
}

#[test]
fn duplicate_overloads_have_distinct_stable_ids() {
    let source = "function parse(value: string): string;\nfunction parse(value: number): number;\nfunction parse(value: string | number) { return value; }\n";
    let first = extract_source("parse.ts", SupportedLanguage::TypeScript, source);
    let second = extract_source("parse.ts", SupportedLanguage::TypeScript, source);
    let ids = |contribution: &FileContribution| {
        contribution
            .nodes
            .iter()
            .filter(|node| node.label == "parse")
            .map(|node| node.id.clone())
            .collect::<Vec<_>>()
    };
    let first_ids = ids(&first);
    assert!(first_ids.len() >= 2);
    assert_eq!(first_ids, ids(&second));
    assert_eq!(
        first_ids.iter().collect::<HashSet<_>>().len(),
        first_ids.len()
    );
}

#[test]
fn malformed_unicode_and_generated_files_preserve_honest_coverage() {
    let malformed = extract_source(
        "broken.py",
        SupportedLanguage::Python,
        "def résumé(:\n    pass\n",
    );
    assert!(malformed
        .diagnostics
        .iter()
        .any(|diagnostic| diagnostic.code == "syntax_error"));

    let unicode = extract_source(
        "unicode.py",
        SupportedLanguage::Python,
        "def résumé():\n    return 1\n",
    );
    assert!(unicode
        .nodes
        .iter()
        .any(|node| node.label == "résumé" && node.sources[0].start_line == Some(1)));

    let generated = extract_path(
        Path::new("/repo"),
        Path::new("src/client.generated.ts"),
        1_024,
    );
    assert_eq!(generated.disposition, FileDisposition::Generated);
    assert!(generated.nodes.iter().all(|node| node.kind == "file"));
    assert!(generated
        .diagnostics
        .iter()
        .any(|diagnostic| diagnostic.code == "generated_file_skipped"));
}

#[test]
fn sensitive_files_are_not_named_as_graph_nodes() {
    let contribution = extract_path(Path::new("/repo"), Path::new("config/.env.local"), 100);
    assert_eq!(contribution.disposition, FileDisposition::Sensitive);
    assert!(contribution.nodes.is_empty());
}

#[test]
fn incremental_build_reuses_untouched_files_and_removes_deleted_files() {
    let root = std::env::temp_dir().join(format!(
        "codevetter-structural-graph-{}",
        uuid::Uuid::new_v4()
    ));
    fs::create_dir_all(&root).expect("create fixture repo");
    run_git(&root, &["init"]);
    fs::write(root.join("a.rs"), "fn alpha() {}\n").expect("a");
    fs::write(root.join("b.rs"), "fn beta() {}\n").expect("b");
    fs::write(root.join("c.rs"), "fn removed() {}\n").expect("c");
    run_git(&root, &["add", "a.rs", "b.rs", "c.rs"]);

    let engine = BundledTreeSitterEngine;
    let cancellation = StructuralGraphCancellation::default();
    let progress = |_: StructuralGraphProgress| {};
    let first = engine
        .build(
            &StructuralGraphBuildInput::full(root.clone(), None),
            &cancellation,
            &progress,
        )
        .expect("full build");
    let beta_id = first
        .nodes
        .iter()
        .find(|node| node.label == "beta")
        .expect("beta")
        .id
        .clone();
    let beta_metric_id = first
        .metrics
        .iter()
        .find(|fact| fact.node_id == beta_id)
        .expect("beta metric")
        .id
        .clone();

    fs::write(root.join("a.rs"), "fn gamma() {}\n").expect("change a");
    fs::remove_file(root.join("c.rs")).expect("delete c");
    let second = engine
        .build(
            &StructuralGraphBuildInput {
                repo_root: root.clone(),
                repo_head: None,
                changed_files: vec!["a.rs".to_string()],
                deleted_files: vec!["c.rs".to_string()],
                previous_cursor: first.cursor.clone(),
                previous_snapshot: Some(Box::new(first)),
                max_files: 25_000,
                max_bytes_per_file: 2 * 1024 * 1024,
            },
            &cancellation,
            &progress,
        )
        .expect("incremental build");

    assert!(second.nodes.iter().any(|node| node.label == "gamma"));
    assert!(!second.nodes.iter().any(|node| node.label == "alpha"));
    assert!(!second.nodes.iter().any(|node| node.label == "removed"));
    assert!(second.nodes.iter().any(|node| node.id == beta_id));
    assert!(second
        .metrics
        .iter()
        .any(|fact| fact.id == beta_metric_id && fact.node_id == beta_id));
    assert!(second.metrics.iter().any(|fact| {
        second
            .nodes
            .iter()
            .any(|node| node.id == fact.node_id && node.label == "gamma")
    }));
    assert!(!second.metrics.iter().any(|fact| fact.path == "c.rs"));
    assert_eq!(second.coverage.indexed_files, 2);
    fs::remove_dir_all(root).expect("remove fixture repo");
}

#[test]
fn incremental_build_repairs_a_renamed_file_without_stale_nodes() {
    let root = std::env::temp_dir().join(format!(
        "codevetter-structural-graph-rename-{}",
        uuid::Uuid::new_v4()
    ));
    fs::create_dir_all(root.join("src")).expect("create fixture repo");
    run_git(&root, &["init"]);
    fs::write(root.join("src/old.rs"), "fn carried() {}\n").expect("old");
    run_git(&root, &["add", "src/old.rs"]);

    let engine = BundledTreeSitterEngine;
    let cancellation = StructuralGraphCancellation::default();
    let progress = |_: StructuralGraphProgress| {};
    let first = engine
        .build(
            &StructuralGraphBuildInput::full(root.clone(), None),
            &cancellation,
            &progress,
        )
        .expect("full build");
    fs::rename(root.join("src/old.rs"), root.join("src/new.rs")).expect("rename");
    let second = engine
        .build(
            &StructuralGraphBuildInput {
                repo_root: root.clone(),
                repo_head: None,
                changed_files: vec!["src/new.rs".to_string()],
                deleted_files: vec!["src/old.rs".to_string()],
                previous_cursor: first.cursor.clone(),
                previous_snapshot: Some(Box::new(first)),
                max_files: 25_000,
                max_bytes_per_file: 2 * 1024 * 1024,
            },
            &cancellation,
            &progress,
        )
        .expect("rename refresh");

    assert!(second
        .nodes
        .iter()
        .any(|node| { node.label == "carried" && node.path.as_deref() == Some("src/new.rs") }));
    assert!(!second
        .nodes
        .iter()
        .any(|node| node.path.as_deref() == Some("src/old.rs")));
    assert_eq!(second.coverage.indexed_files, 1);
    fs::remove_dir_all(root).expect("remove fixture repo");
}

#[test]
fn incremental_graph_facts_are_identical_to_a_clean_rebuild() {
    let root = std::env::temp_dir().join(format!(
        "codevetter-structural-equivalence-{}",
        uuid::Uuid::new_v4()
    ));
    fs::create_dir_all(&root).expect("create fixture repo");
    run_git(&root, &["init"]);
    let clone_a = "export function alpha(items: number[]) { let total = 0; for (const item of items) { if (item > 1) { total += item; } } return total; }\n";
    let clone_b = "export function beta(values: number[]) { let sum = 9; for (const value of values) { if (value > 4) { sum += value; } } return sum; }\n";
    fs::write(root.join("a.ts"), clone_a).expect("a");
    fs::write(root.join("b.ts"), clone_b).expect("b");
    run_git(&root, &["add", "a.ts", "b.ts"]);

    let engine = BundledTreeSitterEngine;
    let cancellation = StructuralGraphCancellation::default();
    let progress = |_: StructuralGraphProgress| {};
    let first = engine
        .build(
            &StructuralGraphBuildInput::full(root.clone(), None),
            &cancellation,
            &progress,
        )
        .expect("initial build");
    assert_eq!(first.clone_groups.len(), 1);

    let changed_a = clone_a.replace("item > 1", "item > 2");
    fs::write(root.join("a.ts"), changed_a).expect("change a");
    fs::remove_file(root.join("b.ts")).expect("delete b");
    fs::write(root.join("c.ts"), clone_b.replace("beta", "gamma")).expect("add c");
    run_git(&root, &["add", "-A"]);
    let incremental = engine
        .build(
            &StructuralGraphBuildInput {
                repo_root: root.clone(),
                repo_head: None,
                changed_files: vec!["a.ts".to_string(), "c.ts".to_string()],
                deleted_files: vec!["b.ts".to_string()],
                previous_cursor: first.cursor.clone(),
                previous_snapshot: Some(Box::new(first)),
                max_files: 25_000,
                max_bytes_per_file: 2 * 1024 * 1024,
            },
            &cancellation,
            &progress,
        )
        .expect("incremental build");
    let clean = engine
        .build(
            &StructuralGraphBuildInput::full(root.clone(), None),
            &cancellation,
            &progress,
        )
        .expect("clean rebuild");

    assert_eq!(incremental.files, clean.files);
    assert_eq!(incremental.coverage, clean.coverage);
    assert_eq!(incremental.nodes, clean.nodes);
    assert_eq!(incremental.edges, clean.edges);
    assert_eq!(incremental.metrics, clean.metrics);
    assert_eq!(incremental.clone_groups, clean.clone_groups);
    assert_eq!(incremental.cursor, clean.cursor);
    fs::remove_dir_all(root).expect("remove fixture repo");
}

fn run_git(root: &Path, arguments: &[&str]) {
    let status = Command::new("git")
        .arg("-C")
        .arg(root)
        .args(arguments)
        .status()
        .expect("run git");
    assert!(status.success(), "git {arguments:?}");
}
