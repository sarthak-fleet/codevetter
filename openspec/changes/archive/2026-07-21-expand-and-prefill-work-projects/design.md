## Context

Work currently groups only saved/live `AgentTerminal` records. The same page already loads up to forty recent indexed Codex and Claude `SessionRow` records, including provider identity, title evidence, session id, working directory, and recency. Indexed paths may be stale, and the frontend has no trustworthy filesystem API for checking them.

## Goals / Non-Goals

**Goals:**

- Populate Work from verified local indexed history without duplicating live/saved threads.
- Make every project group independently keyboard-expandable.
- Never launch a provider merely by loading or expanding the sidebar.
- Fail closed for stale or unverifiable indexed directories.
- Keep the filesystem check bounded to one local IPC call per distinct path set.

**Non-Goals:**

- Restoring deleted repositories, relocating stale paths, or deleting indexed history.
- Treating indexed transcript recency as proof that an agent process is live.
- Automatically resuming historical sessions.
- Persisting collapsed group state across app restarts.

## Decisions

1. **Add a bounded batch directory check.** A read-only Rust command accepts at most 256 distinct non-empty paths, returns the original path plus `exists: path.is_dir()`, and performs no traversal or content reads. One batch avoids an IPC call per session.
2. **Use indexed history only after successful verification.** The frontend deduplicates candidate paths, checks them when indexed sessions change, and derives `availableIndexedSessions`. Missing paths and a failed verification request contribute no historical rows.
3. **Reuse the verified set across Work consumers.** The grouped sidebar, Recent runs, and Board session links receive the same filtered indexed sessions so stale directories do not reappear through another Work entry point.
4. **Represent history separately from active terminals.** A sidebar union keeps terminal rows interactive through existing Open/Archive behavior and indexed rows labelled `Previous` with an explicit Resume action. Session ids already represented by a provider-matched terminal are removed.
5. **Use accessible disclosures.** Each project header is a button with `aria-expanded` and `aria-controls`; groups default expanded, toggle independently, and search temporarily expands matching groups without mutating the collapsed-key set.

## Risks / Trade-offs

- **Filesystem state can change after verification** → Recheck whenever the indexed path signature changes; provider launch still performs its existing authoritative cwd validation.
- **Forty recent sessions can share many directories** → Deduplicate before the bounded batch call; the 256-path cap stays well above the current 40-session fetch.
- **An indexed session can lack a working directory** → Exclude it from project-prefill because ownership cannot be verified.
- **Resume is a side effect** → Use explicit Resume naming and never bind it to project expansion.
