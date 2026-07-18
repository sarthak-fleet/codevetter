---
title: Signing keys and auto-update
description: One-time Tauri signing-key setup and how the desktop auto-updater works.
sidebar:
  order: 4
---

# Signing keys and auto-update

For the step-by-step release flow (version bump → CI build → publish), see
[runbooks/cut-a-release.md](./runbooks/cut-a-release.md) and
[release-pipeline.md](./release-pipeline.md). This page covers the one-time
signing-key setup and the auto-update mechanism.

## Prerequisites

1. Generate signing keys (one-time setup):
   ```bash
   tauri signer generate -w ~/.tauri/codevetter.key
   ```
   This creates `~/.tauri/codevetter.key` (private) and `~/.tauri/codevetter.key.pub` (public).

2. Add the **public key** to `src-tauri/tauri.conf.json` under `plugins.updater.pubkey`.

3. Set environment variables before building:
   ```bash
   export TAURI_SIGNING_PRIVATE_KEY=$(cat ~/.tauri/codevetter.key)
   export TAURI_SIGNING_PRIVATE_KEY_PASSWORD="your-password"
   ```

## Steps

1. Bump the version in `src-tauri/tauri.conf.json` (the `version` field).

2. Build:
   ```bash
   npm run tauri:build
   ```

3. The release workflow produces:
   - `CodeVetter.app.tar.gz` — compressed app bundle for auto-update
   - `CodeVetter.app.tar.gz.sig` — signature file
   - `latest.json` — update manifest (contains version, download URL, signature)

   Prefer the app archive for manual installs until the macOS bundle is Developer ID signed and notarized. Unsigned or ad-hoc signed DMGs fail Gatekeeper for new installs.

4. Create a GitHub release at `Codevetter/codevetter`:
   - Tag: `v{x.y.z}`
   - Upload all build artifacts including `latest.json`

5. Users with the desktop app installed receive an in-app notification automatically.

## How Auto-Update Works

- On launch (after 5s delay) and every 30 minutes, the app checks the `latest.json` endpoint.
- If a newer version is found, a banner appears: "Update available: vX.Y.Z [Install now] [Later]".
- "Install now" downloads the update, installs it, and relaunches the app.
- "Later" dismisses the banner for the current session.

## Notes

- The `pubkey` in `tauri.conf.json` must match the key pair used to sign builds. Without it, update verification will fail.
- The `TAURI_SIGNING_PRIVATE_KEY` env var is only needed at build time, never at runtime.
- Auto-update checks fail silently if the endpoint is unreachable or no update is available.
