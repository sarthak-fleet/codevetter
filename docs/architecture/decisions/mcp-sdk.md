---
title: "Decision: MCP Rust SDK"
description: Chose the official modelcontextprotocol/rust-sdk crate rmcp pinned to 2.2.0 for the local history sidecar.
---

Evaluated 2026-07-13 for the local CodeVetter history sidecar.

## Decision

Use the official `modelcontextprotocol/rust-sdk` crate `rmcp` pinned to `2.2.0`.
The production dependency disables default features and enables only `server`,
`transport-io`, and `macros`. HTTP/SSE, OAuth, client, child-process, reqwest,
and network-server features are absent. The test-only dependency additionally
enables `client` for real lifecycle/framing tests.

The current stable protocol remains `2025-11-25`. The SDK also contains later
revision support, but CodeVetter declares the stable revision and lets the SDK's
negotiation layer handle compatible clients. The 2026-07-28 protocol was still a
release candidate on the evaluation date and is not the advertised baseline.

## Why this dependency

The official SDK owns JSON-RPC framing, initialization/version negotiation,
capability routing, cancellation plumbing, MCP error types, schema models, stdio
concurrency, and backward-compatible resource-not-found behavior. Reimplementing
those protocol mechanics would add more security and interoperability risk than
the dependency saves. CodeVetter still owns authorization, canonical queries,
redaction, limits, cursor validation, and product semantics.

## License, security, and dependency surface

- License: Apache-2.0.
- Upstream: <https://github.com/modelcontextprotocol/rust-sdk>
- Production feature closure: server + async read/write codec + stdio + schema
  support and macros. Direct runtime families are Tokio/Tokio-util, Serde,
  Schemars, Futures, Tracing, Chrono, Thiserror, and the SDK macros. Most were
  already present in the desktop dependency graph.
- Explicitly absent from the selected production features: reqwest, HTTP server,
  SSE, OAuth, JWT, URL-based elicitation, child-process client, and network
  transports.
- The upstream GitHub security/advisory review and GitHub Advisory Database search
  on 2026-07-13 found no published advisory specifically for `rmcp 2.2.0`. The
  local environment did not have `cargo-audit`, so this is a dated review, not a
  claim that future advisories cannot exist. CI should add an ecosystem advisory
  scanner when the fleet standard provides one.

## Measured distribution cost

The Apple Silicon release sidecar is 7.04 MiB and idles at 12.31 MiB RSS. That
binary size includes CodeVetter's shared graph/history/SQLite code and linked
desktop crate surface, so it is an upper bound—not an attribution of 6.80 MiB to
`rmcp`. Tauri packages the target-suffixed executable beside the app and applies
the same app signing/notarization pipeline. A local build is linker ad-hoc signed;
the signed release artifact remains the distribution proof.

The final release-binary benchmark on 2026-07-14 used a 10,000-event fixture and
measured cold initialize at 5.28 ms p50 / 7.90 ms p95. Steady compact queries were
2.16–2.34 ms p50 and 2.43–2.56 ms p95, with 1.4–2.0 KiB responses; resource listing
was 2.15 ms. The 25-launch sample contained one 442.81 ms cold outlier. An earlier
small fixture measured 5.30 ms / 6.18 ms cold p50/p95 and 1.37–1.40 ms warm p50.
Both runs verified zero TCP listeners and no target-repository mutation. The
benchmark should be rerun after any SDK, linking, schema, audit, or packaging
change.

## Rust versus Go

Keep the sidecar in Rust. Its canonical graph/history services and SQLite types
already live in Rust, so a Go process would either duplicate product semantics or
add another IPC boundary back into Rust. With warm queries around 2.2 ms, 12 MiB
RSS, a single 7.0 MiB packaged binary, and no runtime download, there is no measured
latency or distribution gap for Go to recover. A Go prototype is justified only
if a future isolated workload has an end-to-end benchmark—including process
startup, serialization, memory, signing, artifact size, and maintenance—that
beats this baseline materially.
