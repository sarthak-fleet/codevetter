## Why

CodeVetter's local repository graph and historical evidence are useful to people in the desktop UI, but coding agents cannot query the same bounded, cited context efficiently. A repository-scoped MCP surface makes that intelligence reusable without exposing the database, repository path, credentials, or unrestricted filesystem access.

## What Changes

- Add a local stdio MCP server that exposes read-only structural graph, release, temporal-history, causal-evidence, and annotation queries through strict bounded schemas.
- Add opaque repository scopes that are disabled by default, can be enabled or revoked live, and record bounded metadata-only access audit entries.
- Add desktop Settings controls that show index freshness, exposed data kinds, the exact credential-free client configuration, copy/enable/disable actions, and audit clearing.
- Package and qualify the MCP sidecar with the desktop release, including atomic preparation, protocol-boundary tests, safety checks, and a realistic local benchmark fixture.
- Keep agent output explicitly cited, freshness-aware, paginated, redacted, and non-authoritative where history evidence is inferred or incomplete.

## Capabilities

### New Capabilities

- `local-history-mcp`: Repository-scoped, read-only MCP access to CodeVetter's local structural and temporal history intelligence, including lifecycle, privacy, packaging, and performance contracts.

### Modified Capabilities

- None.

## Impact

- Rust MCP protocol, access-control, history-read, audit, sidecar, and packaging code under `apps/desktop/src-tauri`.
- Typed Tauri IPC and a dedicated MCP Settings panel in the desktop frontend.
- Desktop build scripts, Tauri bundle configuration, CI/release qualification, protocol tests, browser tests, and performance fixtures.
- One minimal pinned MCP SDK dependency in the Rust sidecar; no network service, cloud account, credential, or production database is introduced.
