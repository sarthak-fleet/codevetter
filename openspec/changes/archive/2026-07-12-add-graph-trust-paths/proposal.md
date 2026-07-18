## Why

CodeVetter can already build and visualize repo graphs, but its persisted graph contract flattens relationship trust and its UI cannot answer the verification-critical question “how does this changed thing reach that behavior?” Source-backed edge provenance plus path/explain queries can strengthen CodeVetter’s review evidence without turning it into a generic knowledge-graph product.

## What Changes

- Add explicit provenance and confidence metadata to persisted repo-graph edges, with source anchors and backward-compatible handling of existing schema-v1 snapshots.
- Restore explicit graph JSON import and normalize generic `graph.json` output into CodeVetter’s local preview contract without mutating the target repo or requiring another runtime.
- Add a bounded path query that resolves two graph concepts and returns an evidence-bearing hop-by-hop route, including ambiguity when endpoint matching is not decisive.
- Surface graph paths as verification leads in Repo and as bounded context in Review/proof export; inferred or ambiguous hops remain leads, never claims of fact.
- Keep broad document/media ingestion, assistant hooks, wikis, hosted integrations, and semantic-learning surfaces out of scope.

## Capabilities

### New Capabilities
- `trusted-graph-context`: Provenance-preserving graph import, evidence-bearing path queries, and bounded Review integration for CodeVetter’s local repo graph.

### Modified Capabilities

- None.

## Impact

- Rust graph types/builders and persisted Repo Unpacked inventory schema.
- Tauri IPC types and commands for import normalization and path lookup.
- Repo Graph UI, Review graph context, and reviewer-proof Markdown.
- Backward compatibility for existing saved snapshots and loose graph-shaped JSON.
- No new production dependency, no network requirement, and no automatic target-repo writes or assistant-hook installation.
