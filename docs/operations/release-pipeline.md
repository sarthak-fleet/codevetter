---
title: Release pipeline
description: How a desktop version bump becomes a signed GitHub Release with auto-update.
sidebar:
  order: 1
---

# Release pipeline

The desktop app ships as a Tauri 2 macOS binary distributed through GitHub
Releases with `@tauri-apps/plugin-updater` auto-update. Three workflows
cooperate.

## The chain

```
tauri.conf.json version bumped on main
        │
        ▼
auto-release.yml  (on push to main, paths: tauri.conf.json)
   ├─ read version → tag v<version>
   ├─ if release exists → no-op (idempotent)
   ├─ gh release create v<version> --generate-notes
   └─ gh workflow run release.yml -f tag=v<version>   ← explicit dispatch
        │
        ▼
release.yml  (on release.created OR workflow_dispatch with tag)
   ├─ checkout the tag (not main head)
   ├─ pnpm install --frozen-lockfile
   ├─ prepare:mcp-sidecar:release + universal Agent Island helper + vite build
   ├─ tauri build (macos-latest) → DMG + signed updater archive
   ├─ upload assets to the release
   └─ upload latest.json manifest (consumed by the updater)
        │
        ▼
installed apps poll latest.json → auto-update
```

## Why the explicit dispatch

`auto-release.yml` creates the release using `GITHUB_TOKEN`. Workflows
triggered by `GITHUB_TOKEN` do **not** cascade to other workflows — this is a
documented GitHub anti-recursion safeguard. `workflow_dispatch` is the one
event that fires even from `GITHUB_TOKEN`, so `auto-release.yml` dispatches
`release.yml` explicitly with `gh workflow run release.yml -f tag=…`.

## Idempotency

`auto-release.yml` checks `gh release view v<version>` first; if it exists,
the whole job no-ops. Re-pushing the same version is safe.

## What triggers a release

- **Only** a change to `apps/desktop/src-tauri/tauri.conf.json`'s `version`
  field on `main`. The `paths:` filter in `auto-release.yml` enforces this.
- Manual `workflow_dispatch` of `auto-release.yml` is also possible.

## Cut a release

See [runbooks/cut-a-release.md](./runbooks/cut-a-release.md).

## Signing

`release.yml` uses Apple signing/notarization secrets (`APPLE_*`) stored as
GitHub Actions secrets. Signed release publication is the last gate; the
graph + MCP budget qualification runs before the build (see
[development/performance.md](../development/performance.md)).

The optional [Native Agent Island](../architecture/native-agent-island.md) is
bundled as a nested universal sidecar. Release verification checks arm64 and
x86_64 slices plus its nested code signature before assets are uploaded.

## Auto-updater

- Endpoint: `https://github.com/Codevetter/codevetter/releases/latest/download/latest.json`
- Pubkey: pinned in `tauri.conf.json` (`plugins.updater.pubkey`).
- `dialog: false` — the app applies updates without a prompt dialog.

## Key files

- `.github/workflows/auto-release.yml`
- `.github/workflows/release.yml`
- `apps/desktop/src-tauri/tauri.conf.json` (version + updater config)
- `apps/desktop/scripts/prepare-mcp-sidecar.mjs` (sidecar bundling)
- `apps/desktop/scripts/prepare-agent-island.mjs` (universal native helper)
- `scripts/verify-release-manifest.mjs` (post-upload manifest linkage check;
  see [automation-contract.md](./automation-contract.md#updater-manifest-validation))
