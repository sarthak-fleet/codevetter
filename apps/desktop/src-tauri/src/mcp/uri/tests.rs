use super::*;

const REPO: &str = "repo_0123456789abcdef";

#[test]
fn resource_uri_round_trips_opaque_identifiers() {
    let uri = HistoryResourceUri::new(REPO, "commit", "feature/a b#c")
        .expect("uri")
        .to_string();
    assert!(!uri.contains("feature"));
    assert_eq!(
        HistoryResourceUri::parse(&uri, REPO).expect("parse").id,
        "feature/a b#c"
    );
}

#[test]
fn resource_uri_rejects_scope_changes_and_traversal() {
    let uri = HistoryResourceUri::new(REPO, "release", "v1").expect("uri");
    assert!(HistoryResourceUri::parse(&uri.to_string(), "repo_different123456").is_err());
    assert!(
        HistoryResourceUri::parse(&format!("{SCHEME}://{REPO}/release/../evidence"), REPO).is_err()
    );
}

#[test]
fn resource_uri_rejects_malformed_and_oversized_inputs() {
    let oversized_id = URL_SAFE_NO_PAD.encode("x".repeat(4_097));
    let invalid = [
        "https://repo_0123456789abcdef/release/djE=".to_string(),
        format!("{SCHEME}://{REPO}/release/"),
        format!("{SCHEME}://{REPO}/unknown/djE"),
        format!("{SCHEME}://{REPO}/release/%%%"),
        format!("{SCHEME}://{REPO}/release/djE?cursor=1"),
        format!("{SCHEME}://{REPO}/release/djE#fragment"),
        format!(r"{SCHEME}://{REPO}\release\djE"),
        format!("{SCHEME}://{REPO}/release/djE/extra"),
        format!("{SCHEME}://{REPO}/release/{oversized_id}"),
    ];

    for raw in invalid {
        assert!(
            HistoryResourceUri::parse(&raw, REPO).is_err(),
            "accepted malformed URI: {raw}"
        );
    }
}
