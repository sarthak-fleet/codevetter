---
title: Competitive landscape
description: How CodeVetter is positioned against hosted PR-review bots, agent orchestrators, and Claude Code /review.
sidebar:
  order: 2
---

# Competitive landscape

CodeVetter is a **local-first desktop workbench for verifying agent-generated
code** (no server, no auth, local SQLite). This page positions that against the
market. The exhaustive March 2026 competitor survey and the (now-obsolete)
April 2026 hosted-GitHub-App go-to-market plan are archived in
[stale-competitive-landscape-2026-03.md](https://github.com/Codevetter/codevetter/blob/main/docs/archive/stale-competitive-landscape-2026-03.md)
— kept for history, not current.

## Two market categories

**A. Hosted PR-review bots** — Greptile, CodeRabbit, Ellipsis, Qodo, Sourcery,
Bito. Install as a GitHub/GitLab App, review every PR automatically, compete on
accuracy / false-positive rate / learning-from-feedback / custom rules /
platform breadth. Pricing roughly $12–40/user/month. Greptile leads on catch
accuracy (~82% claimed, codebase graph); CodeRabbit leads on adoption (2M+
repos).

**B. Agent orchestrators** — Conductor, Superset, Claude Squad. Desktop apps
that run multiple AI agents in parallel with basic built-in diff viewers to
skim agent output. Free to ~$20/month. No deep AI review.

**The built-in baseline** — **Anthropic Claude Code `/review`**: multi-agent
parallel review with a verification step, deduped and posted as inline GitHub
comments. Cloud-run, ~20 min/review, ~$15–25 per review on Claude
Teams/Enterprise quota. This is the tool every other reviewer justifies itself
against, and it is CodeVetter's most direct comparison because CodeVetter also
runs multi-pass specialist review with a verification/proof step.

## Where CodeVetter sits

CodeVetter straddles both categories: a **desktop app** (like B) that does
**deep code review** (like A), specialized for **agent-generated** code.

- **Opportunity**: no single tool does both well. Orchestrators have shallow
  review; PR bots have no agent orchestration and are cloud-hosted.
- **Local-first is the wedge, not a limitation.** The archived plan proposed a
  hosted GitHub App to compete on distribution; the product deliberately went
  the other way — everything runs on the user's machine, offline, against local
  SQLite. That is the differentiator vs. every Category A tool, which requires
  granting a cloud service repo access.
- **Risk to manage**: being seen as "worse than an orchestrator at
  orchestration and worse than Greptile at review." The answer is the
  verification loop below, not feature parity.

## What CodeVetter does that the bots don't

CodeVetter's on-strategy angle is *trustworthy agent output*, not generic PR
review (see [product/overview.md](../product/overview.md)):

- **Risk-tiered multi-pass review** with security/product/agent specialists and
  coordinator dedup — see
  [architecture/review-pipeline.md](../architecture/review-pipeline.md).
- **Verification proof**: `review-proof` + `agent-fix-packet` with per-finding
  fixed/reproduced/unchecked tallies, not just static diff judgment.
- **Repo understanding + history**: evidence-backed repo briefs
  ([repo-unpacked.md](../architecture/repo-unpacked.md)) and a release-history
  workbench ([graph-and-history.md](../architecture/graph-and-history.md)).
- **Synthetic user QA** to exercise the changed behavior
  ([product/synthetic-user-qa.md](../product/synthetic-user-qa.md)).
- **Benchmarked catch-rate evidence** rather than marketing accuracy claims —
  see [development/benchmark.md](../development/benchmark.md).

## Table stakes (present in most Category A tools)

GitHub/GitLab bot integration, custom review rules (English or YAML), learning
from feedback, PR summaries, and one-click inline suggestions. CodeVetter's
local-first posture means it does not compete on cloud-bot distribution; it
competes on depth of verification and on keeping code on the user's machine.

## Open competitive questions

- What does CodeVetter's review catch that Claude Code `/review` does not, at a
  lower incremental cost? (This is the head-to-head the benchmark exists to
  answer; real agent-PR case curation is still pending — see
  [`../../STATUS.md`](https://github.com/Codevetter/codevetter/blob/main/STATUS.md).)
- Is the agent-generated-code specialization a durable moat, given that
  codebase-aware bots apply their graph to agent code too?

For the full per-competitor breakdown (pricing, features, unique angles as of
March 2026), see the
[archived survey](https://github.com/Codevetter/codevetter/blob/main/docs/archive/stale-competitive-landscape-2026-03.md).
