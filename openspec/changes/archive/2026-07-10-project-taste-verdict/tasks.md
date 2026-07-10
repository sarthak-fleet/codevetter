# Tasks — Project Taste Verdict

- [x] 1. Rust: `commands/taste.rs` with `TasteVerdict` struct + `get_project_taste_verdict` command (read-only SQL over local_reviews, local_review_findings, synthetic_qa_runs, audience_validation_runs/_responses, repo_unpacked_reports); thresholds in one const block. Verify: `cargo check`.
- [x] 2. Register module in `commands/mod.rs` + handler in `main.rs`. Verify: `cargo check`.
- [x] 3. IPC: types + `getProjectTasteVerdict` wrapper in `tauri-ipc.ts`. Verify: `tsc --noEmit`.
- [x] 4. UI: `TasteVerdictCard` component; mount on Unpack page for the selected repo. Verify: `tsc --noEmit` + `pnpm lint`.
- [x] 5. Runtime verify in dev app: codevetter repo shows a real verdict (has 1 scored review + audience run); a repo with no data shows unknown + gaps.
- [x] 6. Archive change, update PROJECT_STATUS.md (Features shipped + Timeline), commit + push.
