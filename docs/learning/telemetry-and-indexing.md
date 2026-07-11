# Learning — telemetry and indexing

The pipeline that turns raw agent transcripts into the Home usage numbers.
Format matches [new-things.md](new-things.md); roadmap in [README.md](README.md).

## Claude JSONL transcript format (and duplicate usage lines)
- What: Claude Code logs each session as JSONL; an assistant message is written as one line PER CONTENT BLOCK, each repeating the same final `usage` object.
- Why here: the indexer summed every line — 50%+ of usage lines are byte-identical repeats, so ALL Claude token/cost numbers ran ~2.2× inflated until v1.2.17.
- Gotcha (from code): dedup key is `(message.id, requestId)`; duplicates are always adjacent among usage lines but flush up to ~40s apart, so the last key persists per session (`cc_sessions.last_usage_key`) to survive incremental-read boundaries. (`session_adapters.rs` claude parse; `history.rs::fix_claude_usage_dedup`)
- Source: https://github.com/ryoppippi/ccusage (dedups the same way)

## Cumulative vs delta token counters
- What: some CLIs report per-message token deltas (Claude), others a session-cumulative running total in every event (Codex `total_token_usage`).
- Why here: adding a cumulative total on each incremental pass compounds — one Codex session reached 61.5B tokens / $35k before the v1.1.99 fix.
- Gotcha (from code): `SessionAppendDelta.tokens_absolute` switches the UPDATE between SET and ADD per adapter. (`db/queries.rs::apply_session_append_delta`)
- Source: — (project-specific; see the field's doc comment)

## Incremental indexing with byte-offset cursors
- What: re-reading only the appended tail of a growing file, from a persisted byte offset, instead of re-parsing the whole file.
- Why here: transcripts reach 200+ MB and are tailed every ~15s; whole-file re-parsing pegged a core at ~95% (v1.1.98 incident).
- Gotcha (from code): the skip decision must key on byte offset == file size, never mtime strings — mtime nanoseconds drift between reads of the same inode and silently disable the skip. Cursors only advance past complete lines (`complete_lines_prefix`). (`history.rs`; regression test `eval_skip_keys_on_byte_offset_not_mtime`)
- Source: https://man7.org/linux/man-pages/man2/lseek.2.html (concept) + `docs/PERFORMANCE.md`

## Local-day bucketing and window boundaries
- What: attributing usage to calendar days in the user's timezone, and converting local midnight to UTC instants for window queries.
- Why here: "today/this week" panels; comparing local-date strings with `Z`-suffix timestamps started weeks 5.5h early in IST (fixed v1.2.9).
- Gotcha (from code): `cc_session_days` holds per-day message counts; the headline day/dollar numbers PRORATE whole-session cost by message share — close, but a midnight-spanning session smears. `timeutil::local_day_start_utc` is the one boundary helper. (`db/queries.rs` day_map query)
- Source: https://docs.rs/chrono/latest/chrono/

## API-equivalent pricing tables (pricing revs)
- What: pricing subscription usage at per-token list prices to get a comparable workload measure, versioned so stored costs refresh when prices change.
- Why here: all $ figures are API-equivalents, not bills; a stale or mis-matched table silently distorts everything (o3-priced Codex, sonnet-priced Fable, GPT-5.6-sol at 1/4 price — all real incidents).
- Gotcha (from code): `estimate_cost` match arms are ORDER-SENSITIVE (specific ids before family fallbacks); bump `PRICING_REV` on any change or already-indexed sessions keep old costs. (`history.rs::estimate_cost`)
- Source: https://docs.anthropic.com/en/docs/about-claude/pricing + https://developers.openai.com/api/docs/pricing

## Rolling quota windows (provider quota APIs)
- What: providers meter subscription usage in trailing windows (5h/7d) that re-anchor with activity; used% falls as bursts age out.
- Why here: the Codex/Claude cards mirror the provider's own endpoints — a dropping percentage and re-arming countdown is correct behavior, not a telemetry bug ("codex keeps resetting").
- Gotcha (from code): ChatGPT's `wham/usage` also carries manual rate-limit reset credits and per-model quota pools (`additional_rate_limits`) that the main window numbers don't include. (`accounts.rs::check_live_usage_openai`)
- Source: https://platform.openai.com/docs/guides/rate-limits (concept)

## macOS background QoS for indexer threads
- What: dropping a thread to `QOS_CLASS_BACKGROUND` so the OS schedules it on efficiency cores and throttles it whenever the user is active.
- Why here: the multi-GB catch-up index must "feel like it isn't running" on a daily-driver laptop.
- Gotcha (from code): set per-thread via `pthread_set_qos_class_self_np(0x09, 0)` at the top of the indexer thread; no-op off macOS. (`main.rs::set_thread_background_qos`)
- Source: https://developer.apple.com/documentation/dispatch/dispatchqos
