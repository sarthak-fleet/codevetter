use super::*;

#[test]
fn cursor_is_opaque_and_request_bound() {
    let encoded = McpCursor::new("repo", "history_search", 25, "abc")
        .encode()
        .expect("cursor");
    assert!(!encoded.contains("history_search"));
    assert_eq!(
        McpCursor::decode(&encoded, "repo", "history_search", "abc")
            .expect("decode")
            .offset(),
        25
    );
    assert!(McpCursor::decode(&encoded, "other", "history_search", "abc").is_err());
}
