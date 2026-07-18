## 1. Graph Trust Contract

- [x] 1.1 Extend `RepoGraph` nodes and edges with schema-v2 trust, origin, source-location, and community fields using backward-compatible serde defaults.
- [x] 1.2 Classify every native fast/enriched graph edge as extracted or inferred and attach the narrowest available source anchors.
- [x] 1.3 Add Rust regression tests proving new scans emit schema v2 and schema-v1 snapshots load as legacy without being rewritten.

## 2. External Graph Import Boundary

- [x] 2.1 Add a size-bounded Tauri command that parses user-selected `nodes` plus `links`/`edges`, validates endpoints, and returns a transient normalized graph.
- [x] 2.2 Preserve supported imported relation, confidence, source file/location, and community metadata while mapping missing or unknown confidence to ambiguous.
- [x] 2.3 Add fixture-based tests for current generic node-link JSON, loose edge-key JSON, dangling endpoints, malformed JSON, and configured size/node/edge caps.

## 3. Trusted Path Query

- [x] 3.1 Implement deterministic endpoint ranking with exact ID/path/label precedence and near-equal ambiguity results.
- [x] 3.2 Implement trust-weighted bounded path search that preserves stored edge direction and returns hop evidence, anchors, trust summary, and bound metadata.
- [x] 3.3 Add unit tests for extracted-path preference, ambiguous endpoints, reverse-direction display, no-path results, and traversal caps.

## 4. Repo Graph Experience

- [x] 4.1 Add an explicit local graph import action and non-mutating imported-preview state to the Repo Graph surface.
- [x] 4.2 Add source/target path controls, endpoint candidate selection, and an accessible hop list with trust badges and source links.
- [x] 4.3 Add focused frontend tests for import errors, ambiguity handling, path rendering, and preservation of the saved native graph.

## 5. Review Evidence Integration

- [x] 5.1 Derive a small bounded set of native graph paths from changed files to routes, commands, persistence points, or tests and include them in review context.
- [x] 5.2 Render the same qualified path summaries in the Review graph panel and reviewer-proof Markdown without creating findings or upgrading evidence status.
- [x] 5.3 Add regression tests proving uncertain hops are labeled as leads and graph paths cannot independently create verified claims.

## 6. Verification and Documentation

- [x] 6.1 Run the smallest relevant Rust graph/import/path tests, then desktop unit tests, typecheck, lint, and build.
- [x] 6.2 Runtime-verify native path tracing and explicit generic graph import in the Tauri app against a local fixture or temp repo.
- [x] 6.3 Update the archived Review Memory Graph reference URL/status and `PROJECT_STATUS.md` only after the capability is implemented and verified.
