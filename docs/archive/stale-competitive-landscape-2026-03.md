---
title: AI code review competitive landscape (archived, 2026-03)
description: Archived March 2026 competitor survey plus an April OSS GitHub-App GTM plan. Kept for git history.
---

> **Archived.** Superseded by the lean current page at
> `docs/knowledge/competitive-landscape.md`. This document is a March 2026
> competitor survey plus an April 2026 "Free PR Review for OSS" GTM plan that
> assumed a hosted GitHub-App product; the product is now local-first with no
> server, so the monetization/GitHub-App sections here are historical only.
> Not current.

**Date**: 2026-03-22
**Purpose**: Understand the competitive landscape for CodeVetter

---

## Table of Contents
1. [Greptile](#1-greptile)
2. [CodeRabbit](#2-coderabbit)
3. [Superset](#3-superset)
4. [Conductor](#4-conductor)
5. [Ellipsis](#5-ellipsis)
6. [Qodo (formerly CodiumAI)](#6-qodo-formerly-codiumai)
7. [Sourcery](#7-sourcery)
8. [Bito](#8-bito)
9. [What The Diff](#9-what-the-diff)
10. [Other Notable Players](#10-other-notable-players)
11. [Anthropic Claude Code /review](#11-anthropic-claude-code-review)
12. [Competitive Summary Matrix](#12-competitive-summary-matrix)
13. [Key Takeaways for CodeVetter](#13-key-takeaways-for-codevetter)

---

## 1. Greptile

**Website**: https://www.greptile.com
**Category**: AI Code Review (codebase-aware)
**Funding**: Eyeing $180M valuation

### What They Do
Greptile indexes your entire repository to build a semantic code graph -- mapping structure, relationships, dependencies, and patterns. When a PR is opened, it performs multi-hop investigation: tracing dependencies, checking git history, and following leads across files. Built on Anthropic Claude Agent SDK (v3+).

### How It Works
- GitHub App (also GitLab)
- Installs as a bot that comments on PRs
- Also has Slack, Jira, Notion, Google Drive, Sentry, VS Code integrations
- Separate "Chat" product for codebase Q&A

### Pricing
| Tier | Price | Details |
|------|-------|---------|
| Cloud | $30/seat/month | 50 reviews included, $1/additional review |
| Enterprise | Custom | Self-hosted, SSO/SAML, GitHub Enterprise |
| Chat (separate) | $20/user/month | Unlimited codebase queries |
| API | $0.45/request | For custom integrations |
| Open Source | Free | 100% off |
| Startups | 50% off | |

14-day free trial, no credit card required. Up to 20% off on annual prepaid.

### Custom Rules / Learning
- Write rules in plain English or point to a markdown file
- Automatically absorbs .cursorrules, claude.md, agents.md
- Rules can target specific repos, file paths, or code patterns
- Learns from PR comments -- reads every engineer's PR comments to learn standards
- Tracks thumbs-up/thumbs-down reactions to adapt over time
- Infers new rules from team behavior

### Unique Differentiators
- Highest benchmark score: 82% bug catch rate (vs CodeRabbit 44%, Copilot 54%)
- Deep codebase graph -- understands cross-file impact
- Multi-hop agent investigation on every PR
- Actively ingests team context files (claude.md, cursorrules)

### Agent-Generated Code Review
Not specifically marketed for this, but the codebase-aware approach applies equally to agent-written and human-written code.

---

## 2. CodeRabbit

**Website**: https://www.coderabbit.ai
**Category**: AI Code Review (broadest platform support)

### What They Do
The most widely installed AI code review app on GitHub/GitLab. Provides line-by-line code suggestions, PR summaries, and real-time chat within PRs. Can generate unit tests, draft documentation, or open issues in Jira/Linear directly from PR context. 2M+ repos connected, 13M+ PRs processed.

### How It Works
- GitHub App, GitLab, Bitbucket, Azure DevOps
- Bot comments on PRs with inline suggestions
- Interactive: you can chat with @coderabbitai in PR comments
- IDE reviews (VS Code)

### Pricing
| Tier | Price | Details |
|------|-------|---------|
| Free | $0 | PR summarization, IDE reviews, unlimited repos (public + private) |
| Pro | $24/mo (annual) or $30/mo | Unlimited PR reviews, Jira/Linear, SAST, analytics, docstrings |
| Enterprise | Custom | Self-hosted, multi-org, SLA, RBAC |

14-day free trial of Pro. Only charged for developers who create PRs (seats are reassignable).

### Custom Rules / Learning
- `.coderabbit.yaml` config file for project-specific rules
- Path-based instructions: custom guidelines per file glob pattern
- AST-grep rules: precise code structure matching via abstract syntax tree
- Learns from chat: tell @coderabbitai what you do/don't want and it remembers
- Automatically closes conversations when suggested fixes are applied

### Unique Differentiators
- Broadest platform support (GitHub, GitLab, Bitbucket, Azure DevOps)
- Free tier that actually works (unlimited repos, PR summarization)
- Interactive chat within PRs -- can ask it to generate tests, docs, issues
- Code graph analysis + real-time web query for documentation context
- Massive scale (2M+ repos) -- the "default choice" for many teams

### Agent-Generated Code Review
Not specifically positioned for agent code, but interactive chat model works well for iterating on agent output.

---

## 3. Superset

**Website**: https://superset.sh
**Category**: Agent Orchestration / Parallel Development
**Note**: This is NOT a code review tool -- it's an agent orchestrator similar to Conductor.

### What They Do
Desktop app for running 10+ CLI-based coding agents in parallel (Claude Code, Codex, OpenCode, Aider), each in its own Git worktree with its own branch. Built-in diff viewer for reviewing agent changes before merging.

### How It Works
- Desktop app (Mac, with other platforms planned)
- Each agent gets an isolated Git worktree
- Unified dashboard to monitor all agents
- Built-in diff viewer with syntax highlighting
- Agent-agnostic (works with any terminal-based coding tool)

### Pricing
| Tier | Price | Details |
|------|-------|---------|
| Free | $0 | Open source (Apache 2.0) |
| Pro | $20/seat/month | Advanced features |

You still pay your agent provider (API keys, Claude Max, etc.) separately.

### Unique Differentiators
- Open source (Apache 2.0) -- Conductor is NOT open source
- Agent-agnostic (any CLI tool, not just Claude Code + Codex)
- Built by 3 ex-YC CTOs
- 512 upvotes on Product Hunt launch (Mar 2026)

### Relevance to CodeVetter
Superset is what CodeVetter's "Workspaces + Agents" direction could look like if taken further. The built-in diff viewer is a code review surface for agent output, which is directly comparable to what CodeVetter does.

---

## 4. Conductor

**Website**: https://www.conductor.build
**Category**: Agent Orchestration / Parallel Development
**YC Company**

### What They Do
Mac app for orchestrating teams of coding agents (Claude Code and Codex) working simultaneously in isolated Git worktrees. Dashboard view of all agents, review + merge interface.

### How It Works
- Native Mac app (Apple Silicon required; Intel planned, no Windows/Linux)
- Creates parallel agents in isolated Git worktrees
- Real-time visibility into what each agent is doing
- Code review and merge built in
- Uses your existing Claude Code / Codex auth (API key, Pro, or Max plan)

### Pricing
| Tier | Price | Details |
|------|-------|---------|
| Free | $0 | The app itself is free |

You pay for your Claude Code / Codex usage separately. Conductor adds no marginal cost.

### Unique Differentiators
- Completely free
- Trusted by builders at Linear, Vercel, Notion, Stripe
- Zero-friction setup -- uses your existing Claude login
- Purpose-built for Claude Code + Codex (not agent-agnostic like Superset)

### Relevance to CodeVetter
Conductor is the most direct comparison to CodeVetter's original vision. The key question: what does CodeVetter offer that Conductor doesn't? Conductor is free, polished, and YC-backed. CodeVetter needs to differentiate on review depth, rules, or targeting a different workflow.

---

## 5. Ellipsis

**Website**: https://www.ellipsis.dev
**Category**: AI Code Review + Code Generation
**YC Company**

### What They Do
AI code reviewer that also generates code fixes. Reviews every PR for bugs, anti-patterns, security vulnerabilities, and style guide violations. Can read a reviewer's comment and auto-generate a commit with the fix. Actually executes generated code to verify it works.

### How It Works
- GitHub App (also GitLab)
- Reviews every commit automatically
- Auto-generates fixes from reviewer comments
- PR summaries and weekly codebase change summaries

### Pricing
| Tier | Price | Details |
|------|-------|---------|
| Single tier | $20/user/month | Unlimited usage, reviews on every commit, PR summaries, code gen, Q&A |

7-day free trial. SOC 2 Type 1 certified. Does not persist source code.

### Custom Rules / Learning
- Write style guides in natural language
- Learns which comment types your team values over time
- Customizes reviews based on team feedback patterns

### Unique Differentiators
- Actually executes generated code before suggesting fixes
- Generates commits directly from reviewer comments (not just suggestions)
- Weekly codebase change summaries
- SOC 2 Type 1 certified
- Simple single-tier pricing

### Agent-Generated Code Review
Not specifically marketed, but the auto-fix capability is especially useful for iterating on agent output -- reviewer says "fix this," Ellipsis generates the commit.

---

## 6. Qodo (formerly CodiumAI)

**Website**: https://www.qodo.ai
**Category**: AI Code Integrity Platform (review + testing + code gen)
**Recognition**: Named Visionary in 2026 Gartner Magic Quadrant for AI Code Assistants

### What They Do
"Review-first" platform with 15+ specialized agentic workflows. Goes beyond PR review into test generation, security scanning, and documentation. Multi-repo context engine that indexes across repositories. Context Engine claims 80% accuracy in understanding codebases (vs competitors at 45-74%).

### How It Works
- GitHub App for PR reviews
- IDE plugin (VS Code, JetBrains) -- the core product
- 15+ agentic workflows (review, test gen, docstrings, etc.)
- CLI available

### Pricing
| Tier | Price | Details |
|------|-------|---------|
| Developer (Free) | $0 | 75 PRs/month, 250 LLM credits |
| Teams | $19-30/user/month | 2,500 credits, PR automation |
| Enterprise | $45+/user/month | SSO, on-prem, priority support |

### Custom Rules / Learning
- Customizable review rules per project
- 15+ specialized workflow templates
- Context Engine learns codebase patterns across repos

### Unique Differentiators
- Strongest test generation capability in the market
- 15+ specialized agentic workflows (not just review)
- Multi-repo context engine (80% accuracy claim)
- Gartner Magic Quadrant Visionary recognition
- IDE-first approach -- not just a GitHub bot

### Agent-Generated Code Review
Qodo's test generation is arguably more useful for agent code than pure review -- it can generate tests to verify agent output is correct.

---

## 7. Sourcery

**Website**: https://www.sourcery.ai
**Category**: AI Code Review (multi-reviewer approach)

### What They Do
Uses a series of specialized AI reviewers each with different focuses (complexity, security, style, etc.) to review code from multiple angles. Combines AI review with its own static analysis engine. Supports 30+ programming languages.

### How It Works
- GitHub App / GitLab integration
- IDE plugin (VS Code, PyCharm)
- Multiple specialized AI "reviewers" run in parallel
- Static analysis engine on top of AI
- Validation process to reduce false positives before presenting results

### Pricing
| Tier | Price | Details |
|------|-------|---------|
| Free | $0 | Public repos only |
| Pro | $12/user/month | Full reviews, private repos, limited security |
| Team | Custom | Full security reviews, repo analytics |

20% discount on annual billing.

### Custom Rules / Learning
- Define custom "Review Rules" -- what to look for in specific areas
- Add existing coding standards, style guides, contributing guidelines as rules
- Learns from developer feedback -- dismissing noise trains future reviews
- Custom rules via sourcery-rules repository/config

### Unique Differentiators
- Multiple specialized reviewers (not one monolithic pass)
- Hybrid approach: AI + static analysis engine
- Most affordable paid tier ($12/user/month)
- Open-source rules repository
- Validation step specifically to cut false positives

### Agent-Generated Code Review
No specific agent-code features.

---

## 8. Bito

**Website**: https://bito.ai
**Category**: AI Code Review + Codebase Intelligence
**Key Feature**: AI Architect (live knowledge graph)

### What They Do
AI code review with a unique "AI Architect" -- a live knowledge graph that maps APIs, modules, and dependencies across your codebase. Reviews are context-aware at the architectural level. Validates PRs against Jira tickets and Confluence docs.

### How It Works
- GitHub App, GitLab, Bitbucket
- IDE plugin (VS Code)
- AI Architect maintains live knowledge graph of your system
- Reviews validate against Jira tickets and Confluence docs
- CI/CD pipeline integration

### Pricing
| Tier | Price | Details |
|------|-------|---------|
| Team | $15/user/month | Custom guidelines, Jira integration, CI/CD reviews |
| Professional | $25/user/month | + self-hosted option, learning system |
| Enterprise | Custom | + Confluence, on-prem, multi-org, SLA |

14-day free trial, no credit card. Supports 50+ languages.

### Custom Rules / Learning
- Custom review guidelines you define
- **Learned Rules**: Auto-creates rules from negative feedback
  - 3 negative signals on a pattern -> rule auto-enables
  - GitHub: checkbox feedback; GitLab: emoji reactions; Bitbucket: comments
  - Dashboard to manage learned rules
  - Only negative feedback triggers learning (positive feedback does nothing)
- Validates PRs against Jira tickets to ensure alignment

### Unique Differentiators
- AI Architect: live knowledge graph of your entire system
- Jira + Confluence validation (ensures code matches requirements)
- Auto-learning from negative feedback (3-strike rule creation)
- CI/CD pipeline review integration
- Self-hosted option on Professional tier (unusual at that price point)

### Agent-Generated Code Review
The Jira validation feature is uniquely useful for agent code -- ensuring agent output actually matches the ticket requirements, not just generating plausible code.

---

## 9. What The Diff

**Website**: https://whatthediff.ai
**Category**: PR Description Generator (lightweight)

### What They Do
Primarily an automated PR description generator, not a full code reviewer. Reads the diff and writes a plain-English summary of what changed. Also generates changelogs and weekly progress reports.

### How It Works
- GitHub App / GitLab
- Reads PR diffs via API
- Generates plain-English descriptions
- No source code storage

### Pricing
| Tier | Price | Details |
|------|-------|---------|
| Free | $0 | 25K tokens |
| Starter | $19/month | 200K tokens |
| Unlimited | $199/month | Unlimited tokens |

Token-based pricing. Average PR uses ~2,300 tokens. No per-seat pricing.

### Features
- Automated PR descriptions
- Rich summary notifications for non-technical stakeholders
- Public changelogs (shareable) or JSON API
- Weekly progress reports
- Inline AI refactoring suggestions
- Multi-language support (translates summaries)
- Fine-grained settings: skip CI PRs, delay drafts, limit token consumption

### Custom Rules / Learning
None documented.

### Unique Differentiators
- Simplest tool in the space -- does one thing well (PR descriptions)
- Token-based pricing (not per-seat) -- good for small teams with many PRs
- Non-technical stakeholder features (translated summaries, changelogs)
- Privacy-focused: no code storage, no training on user code

### Agent-Generated Code Review
Not applicable. This is a description/summary tool, not a bug-finding reviewer.

---

## 10. Other Notable Players

### Graphite Agent
- Full code review platform built around stacked PRs
- Acquired in Dec 2025; unified Diamond (AI reviewer) + Chat into "Graphite Agent"
- Built on Claude; focuses on logic errors and edge cases
- Team plan: $40/month (includes AI reviews, stacking, merge queue)
- Unhelpful comment rate under 3%; developers change code 55% of the time it flags an issue
- Best for teams willing to adopt stacked PR workflow

### BugBot (Cursor)
- Built into Cursor IDE and GitHub workflow
- Launched July 2025; reviews 2M+ PRs monthly
- Runs 8 parallel review passes with randomized diff order per PR
- ~$40/user/month as add-on to Cursor plans
- Best for teams already using Cursor IDE

### Panto AI
- Unified AI code review + AppSec platform
- Combines static analysis, secrets detection, dependency scanning, IaC security
- Claims 14x more refactoring opportunities than Greptile, 5x more performance optimizations
- Newer entrant, less established

### CodeAnt AI
- Bundles PR reviews, SAST, secrets detection, IaC scanning, SCA, DORA metrics
- Supports all 4 major git platforms
- Zero false positives but fewer total comments
- More security-focused than review-focused

### Traycer AI
- Good categorization of issues (bug, performance, security, clarity)
- Simple setup, solid second-tier tool
- Less thorough than Qodo or Greptile

### Google Conductor (Gemini CLI extension)
- Different product from Conductor.build
- Gemini CLI extension for structured planning
- Added "Automated Reviews" -- checks AI-generated code against project standards
- Static + logic analysis, checks against specs/plans, style guides, security

---

## 11. Anthropic Claude Code /review

**The built-in competitor that all these tools must justify their existence against.**

### What It Does
Multi-agent code review system built into Claude Code. Multiple agents analyze the diff in parallel, each looking for a different class of issue. Verification step checks candidates against actual code behavior. Results are deduplicated, ranked by severity, posted as inline GitHub comments.

### How It Works
- Available for Claude Teams and Enterprise users
- Runs in the cloud on Anthropic infrastructure
- `/code-review` command in Claude Code CLI
- Also available as a plugin for CI/CD
- Reviews take ~20 minutes on average

### Pricing
- Included with Claude Teams/Enterprise subscription
- Individual review cost: $15-$25 per review (based on PR size/complexity)
- No separate subscription -- uses your Claude Code quota

### Performance
- PRs >1,000 lines: findings in 84% of cases, avg 7.5 issues per review
- PRs <50 lines: findings in 31% of cases, avg 0.5 issues per review

### Limitations
- Expensive per review ($15-$25) vs $1 incremental on Greptile
- No persistent learning or custom rules (yet)
- No team-level analytics or dashboards
- Requires Claude Teams/Enterprise (not available on Pro/free)
- 20-minute review time is slow vs competitors (seconds to minutes)

---

## 12. Competitive Summary Matrix

| Tool | Price (per user/mo) | Free Tier | Platform | Custom Rules | Learning | Unique Angle |
|------|---------------------|-----------|----------|--------------|----------|-------------|
| **Greptile** | $30 | Trial only | GitHub, GitLab | Yes (English/MD) | Yes (from PRs + reactions) | Deepest codebase graph, 82% catch rate |
| **CodeRabbit** | $24 | Yes (basic) | GH, GL, BB, Azure | Yes (YAML + AST) | Yes (from chat) | Broadest platform, interactive chat |
| **Superset** | $0-20 | Yes (OSS) | Desktop (agent orchestrator) | N/A | N/A | Open-source agent orchestrator |
| **Conductor** | Free | Yes | Desktop (Mac only) | N/A | N/A | Free agent orchestrator, YC-backed |
| **Ellipsis** | $20 | Trial only | GitHub, GitLab | Yes (natural language) | Yes (implicit) | Auto-generates fix commits |
| **Qodo** | $0-45 | Yes (250 credits) | GH + IDE | Yes | Yes (context engine) | Test gen + 15 workflows, Gartner Visionary |
| **Sourcery** | $12 | Yes (public) | GH, GL + IDE | Yes (rules) | Yes (from feedback) | Cheapest paid, multi-reviewer + static |
| **Bito** | $15-25 | Trial only | GH, GL, BB | Yes (guidelines) | Yes (3-strike auto-rules) | AI Architect knowledge graph, Jira validation |
| **What The Diff** | $19-199 (token) | Yes (25K) | GH, GL | No | No | PR descriptions, not review. Cheapest entry. |
| **Claude /review** | $15-25/review | No | CLI + GitHub | No (yet) | No | Multi-agent, deepest analysis per PR |
| **Graphite Agent** | $40 | No | GitHub | Unknown | Unknown | Stacked PRs + review + merge queue |
| **BugBot (Cursor)** | ~$40 | No | Cursor + GitHub | Unknown | Unknown | 8 parallel passes, 2M+ PRs/month |

---

## 13. Key Takeaways for CodeVetter

### The Market Has Split Into Two Categories

**Category A: PR Review Bots** (Greptile, CodeRabbit, Ellipsis, Qodo, Sourcery, Bito)
- Install as GitHub App, review every PR automatically
- Compete on: accuracy, false positive rate, learning, rules, platform breadth
- Pricing: $12-30/user/month
- **Greptile leads on accuracy (82%), CodeRabbit leads on adoption (2M repos)**

**Category B: Agent Orchestrators** (Conductor, Superset, Claude Squad)
- Desktop apps for running multiple AI agents in parallel
- Built-in diff viewers to review agent output
- Pricing: Free to $20/month
- **Conductor is free and YC-backed; Superset is open source**

### Where CodeVetter Sits
CodeVetter straddles both categories -- it's a desktop app (like Category B) that does code review (like Category A). This is both an opportunity and a risk:

**Opportunity**: No one tool does both well. Conductor/Superset have basic diff views but no deep AI review. Greptile/CodeRabbit have deep review but no agent orchestration.

**Risk**: CodeVetter could be seen as "worse than Conductor at orchestration AND worse than Greptile at review."

### Critical Questions
1. **Should CodeVetter be a review bot (GitHub App) instead of / in addition to a desktop app?** Every successful review tool is a GitHub App. Desktop-only limits adoption.
2. **What does CodeVetter's review catch that Claude Code /review doesn't?** The /review command costs $15-25 per review and takes 20 min. If CodeVetter can deliver 80% of that quality at $0 incremental cost, that's compelling.
3. **Does CodeVetter have a learning/rules feature?** Every serious competitor does. This is table stakes.
4. **Is the agent-code angle real?** 41% of commits are now AI-assisted. Reviewing agent output is a genuine pain point. But no tool has truly specialized in this -- it's an opportunity.

### Pricing Benchmarks
- Budget option: Sourcery at $12/user/month
- Mid-range: Bito $15, Qodo $19, Ellipsis $20, CodeRabbit $24
- Premium: Greptile $30, Graphite $40, BugBot ~$40
- Free: Conductor (orchestrator), CodeRabbit (basic), Qodo (250 credits)

### Features That Are Now Table Stakes
1. GitHub/GitLab integration (as a bot, not just CLI)
2. Custom review rules (in English or YAML)
3. Learning from feedback (thumbs up/down, chat corrections)
4. PR summaries and descriptions
5. Inline suggestions with one-click apply

### Features That Differentiate
1. Codebase-wide context graph (Greptile, Bito AI Architect)
2. Test generation alongside review (Qodo)
3. Auto-fix commits from reviewer comments (Ellipsis)
4. Jira/ticket validation (Bito)
5. Agent orchestration + review in one tool (nobody does this well yet)
6. Stacked PR workflow (Graphite)


---
---

# Free PR Review for Open Source — Strategy Document

**Date**: 2026-04-04
**Status**: Proposed
**Previous plans**: `plans/competitive-landscape-2026-03-22.md`, `plans/2026-03-28-roadmap-all-phases-complete.md`

---

## Thesis

Give every public repo free automated PR reviews. Every review comment is a public ad for CodeVetter. OSS maintainers get value immediately; CodeVetter gets distribution, SEO, and a pipeline to paid conversions on private repos.

This is how CodeRabbit got to 2M+ connected repos. Sourcery already gates their free tier to public repos only ($0 public, $12/user/month private). The playbook is proven. CodeVetter's edge: agent-aware reviews that no free competitor offers.

---

## 1. Free Tier for Public Repos

### What the user gets
- Install the CodeVetter GitHub App on any **public** repository
- Every PR opened or pushed to gets an automated review (same pipeline as paid)
- Agent detection included (`workers/api/src/agentDetection.ts` — bot accounts, PR body markers, branch prefixes, co-author patterns)
- Review includes: severity-ranked findings, inline comments with file/line anchors, composite score, suggested fixes for agent PRs
- Same review-core logic: `buildPrompt()`, `computeScore()`, `determineReviewAction()`, `buildOverallBody()` from `packages/review-core/src/`

### What changes vs. paid
- Rate-limited (see section 4)
- No custom review rules (paid only — `workspace_rule_defaults` and `repository_rule_overrides` tables stay locked)
- No custom review tone (defaults to `balanced`; paid users can set `strict` or `friendly` per `ReviewTone` type in `packages/shared-types/src/v1.ts`)
- No workspace dashboard or team management
- No Slack/webhook integrations
- "Reviewed by CodeVetter" badge on every comment (cannot be removed)

### Why this works
- The review worker (`workers/review/src/handlers.ts`) already runs the full pipeline: fetch diff via GitHub App token, call AI gateway, post PR review via `postPrReview()`. No new review logic needed for free tier — just gating.
- Public repos are readable by the GitHub App installation token, so no additional auth flow is required.
- The `repositories.is_private` column already exists in the schema (`packages/db/migrations/0001_init.sql`, line 104). This is the tier gate.

---

## 2. "Reviewed by CodeVetter" Badge

### Implementation
Every review comment posted via `postPrReview()` in `workers/review/src/github.ts` includes an `overallBody` string built by `buildOverallBody()` in `packages/review-core/src/formatting.ts`. Currently it ends with:

```
*Automated review by CodeVetter*
```

Change this to include a badge and link for free-tier repos:

```markdown
[![Reviewed by CodeVetter](https://codevetter.com/badge/reviewed.svg)](https://codevetter.com/reviews/{owner}/{repo}/pr/{prNumber})

**Score:** 87/100 | **Findings:** 1 medium, 2 low
*Free automated review for open source — [get CodeVetter for your repo](https://codevetter.com/install)*
```

### Badge variants
- `reviewed.svg` — default green shield
- `reviewed-score-{N}.svg` — score-colored (green 80+, yellow 60-79, red <60)
- `reviewed-agent.svg` — distinct badge for agent-authored PRs (differentiator)

### Where it appears
1. **PR review body** — the main review comment posted via GitHub API
2. **Inline comments** — each finding comment gets a small footer: `— [CodeVetter](https://codevetter.com)`
3. **README badge** — provide an embeddable badge for repos: `![CodeVetter](https://codevetter.com/badge/{owner}/{repo}.svg)` showing last review score

### Paid tier difference
Paid users get the review without the marketing footer. The `*Free automated review for open source*` line and the install CTA are stripped. The badge link still works (points to the dashboard instead of the public page).

### Files to modify
- `packages/review-core/src/formatting.ts` — `buildOverallBody()` accepts a new `tier: 'free' | 'paid'` param
- `workers/review/src/handlers.ts` — `handleReviewJob()` passes tier based on `repository.isPrivate`
- New: `apps/landing-page-astro/` — badge SVGs in `/public` (static Astro site)

---

## 3. GitHub App Changes

### Current state
The GitHub App already handles the full PR review lifecycle:
- **Webhook receiver**: `workers/api/src/index.ts` line 1990 — `POST /v1/webhooks/github`
- **Event handling**: `pull_request` events with `opened`, `reopened`, `synchronize` actions trigger reviews (line 2100: `actionCanTriggerReview()`)
- **@codevetter mentions**: `issue_comment` events with `@codevetter` in body trigger on-demand reviews (line 2140)
- **Review posting**: `workers/review/src/github.ts` — `postPrReview()` posts the review, `postPrComment()` for standalone comments
- **Installation sync**: `POST /v1/workspaces/:id/github/sync` syncs repos from GitHub installation (API worker line ~1350)

### What needs to change

#### 3a. Installation flow (new: no-auth free path)
Currently, installing the GitHub App requires:
1. GitHub OAuth login → user record in `users` table
2. Workspace creation → `workspaces` table
3. GitHub App installation linked to workspace → `github_installations` table
4. Repository sync → `repositories` table

For free/public tier, add a **lightweight path**:
1. Install GitHub App (no OAuth needed — GitHub handles app installation independently)
2. On `installation` webhook event: auto-create a workspace with `kind: 'oss_free'` and link the installation
3. Auto-sync public repos only (`is_private = false`)
4. Skip user/member creation until they visit the dashboard

This means adding a new webhook handler for the `installation` event type in the API worker. Currently only `pull_request` and `issue_comment` are handled.

#### 3b. Repository visibility gate
The webhook handler at line 2070 currently finds the repository by `fullName` and creates a review run unconditionally. Add a check:

```
if (repository.isPrivate && workspace.kind === 'oss_free') {
  // Skip review — private repo on free tier
  processingStatus = 'skipped_free_tier';
}
```

The `repositories.is_private` field is already populated during sync from the GitHub API (`workers/api/src/github.ts` line 83: `isPrivate` is parsed from the repo payload via `toGitHubRepository()`).

#### 3c. GitHub App permissions
Current required permissions (inferred from API calls):
- `pull_requests: read` — fetch PR diffs and files
- `pull_requests: write` — post review comments
- `contents: read` — fetch file trees and blobs for indexing
- `issues: read` — detect @codevetter mentions in issue comments

No additional permissions needed for free tier. Public repo content is readable with the installation token.

#### 3d. Workspace kind enum
Add `'oss_free'` to the workspace kind. Currently `WorkspaceKind = 'organization' | 'personal'` in `packages/shared-types/src/v1.ts` line 61. Extend to:

```typescript
export type WorkspaceKind = 'organization' | 'personal' | 'oss_free';
```

#### 3e. Schema changes
New migration `0003_free_tier.sql`:

```sql
-- Add tier tracking to workspaces
ALTER TABLE workspaces ADD COLUMN tier TEXT NOT NULL DEFAULT 'free';
ALTER TABLE workspaces ADD COLUMN tier_pr_limit INTEGER NOT NULL DEFAULT 10;
ALTER TABLE workspaces ADD COLUMN tier_period_start TEXT;

-- Track monthly review counts per repository for rate limiting
CREATE TABLE IF NOT EXISTS repository_usage (
  id TEXT PRIMARY KEY,
  repository_id TEXT NOT NULL REFERENCES repositories(id),
  period TEXT NOT NULL,          -- '2026-04' format
  review_count INTEGER NOT NULL DEFAULT 0,
  last_review_at TEXT,
  UNIQUE (repository_id, period)
);

CREATE INDEX IF NOT EXISTS idx_repository_usage_repo ON repository_usage(repository_id);
```

---

## 4. Rate Limiting

### Free tier limits
| Resource | Limit | Period |
|----------|-------|--------|
| PR reviews per repo | 10 | Monthly |
| Total reviews per installation | 50 | Monthly |
| Max files per PR | 50 | Per review |
| Max diff size | 60,000 chars | Per review (half of paid: `MAX_DIFF_CHARS` in `packages/review-core/src/prompt.ts` is 120,000) |
| @codevetter mention triggers | 3 | Per PR |

### Implementation
The rate limit check happens in the webhook handler before creating a review run. The `repository_usage` table (new) tracks monthly counts. Check flow:

1. Webhook arrives at `POST /v1/webhooks/github`
2. Parse the repository and PR
3. Query `repository_usage` for the current period
4. If `review_count >= tier_pr_limit`, skip the review and post a comment:
   > "This repo has reached its free tier limit of 10 reviews/month. [Upgrade to Pro](https://codevetter.com/pricing) for unlimited reviews."
5. Otherwise, increment `review_count` and proceed

The existing rate limiter in the API worker (line 770: IP-based, 120 req/min) stays as-is for API abuse prevention. The new tier-based limiter is separate and specific to review execution.

### Why these numbers
- 10 PRs/month per repo is generous enough to be useful for small-to-medium OSS projects (most repos get <10 PRs/month)
- 50 per installation covers orgs with many repos
- Low enough that heavy users feel the constraint and consider upgrading
- Comparable to Qodo free tier (75 PRs/month) and Greptile trial (50 reviews included)

---

## 5. Upgrade Path

### Free (Public repos only)
- Automated PR reviews on public repos
- Agent detection and agent-specific review rules
- 10 reviews/month per repo
- "Reviewed by CodeVetter" badge on every comment
- Default review tone (balanced)
- @codevetter mention for on-demand reviews

### Pro ($15/user/month)
- Everything in Free
- **Private repo support** — the `is_private` gate is lifted
- **Unlimited reviews** — no monthly cap
- **Custom review rules** — workspace-level (`workspace_rule_defaults`) and per-repo (`repository_rule_overrides`): severity thresholds, blocked patterns, required checks, review tone
- **Team dashboard** — workspace members, audit logs, review history with score trends
- **Full diff analysis** — 120,000 char limit (2x free)
- **Badge removal** — marketing footer stripped from review comments
- **Priority review queue** — paid jobs processed before free tier in the review worker
- **Semantic indexing** — repository codebase indexing for context-aware reviews (`indexing_runs`, `indexed_files`, `semantic_chunks` tables)

### Team ($30/user/month)
- Everything in Pro
- **Slack notifications** — review results posted to a Slack channel
- **Webhook callbacks** — POST review results to a custom URL
- **Custom AI provider** — bring your own API key (OpenAI, Anthropic, OpenRouter) via `workspace_secrets` table
- **Review analytics** — score trends over time, agent vs human comparison, per-developer breakdown
- **SSO/SAML** — enterprise auth
- **Priority support**

### Price rationale
- $15 undercuts CodeRabbit ($24), Ellipsis ($20), Qodo Teams ($19-30)
- $15 matches Bito Team tier
- $30 team tier competitive with Greptile ($30) but includes features Greptile charges extra for (codebase chat)
- CodeVetter's existing infrastructure runs on Cloudflare Workers + D1 — marginal cost per review is near-zero (AI gateway cost only)

### Upgrade triggers
The system creates natural upgrade moments:
1. **Rate limit hit** — "10/10 reviews used this month" comment on PR
2. **Private repo installed** — "CodeVetter free tier is for public repos. [Upgrade to review private repos](https://codevetter.com/pricing)"
3. **Custom rule attempt** — dashboard shows rules editor locked with upgrade CTA
4. **Team growth** — adding workspace members prompts Team tier
5. **Agent-heavy repos** — repos with >50% agent-authored PRs get a targeted upsell: "Your team ships a lot of agent code. Pro tier includes agent-specific review rules and suggestion generation."

---

## 6. SEO Play

### Public review pages on codevetter.com

Every free-tier review generates a public URL:
```
https://codevetter.com/reviews/{owner}/{repo}/pr/{prNumber}
```

This page shows:
- Review score and severity breakdown
- All findings with file paths and line numbers
- Whether the PR was agent-authored (and which agent)
- Link to the GitHub PR
- "Get CodeVetter for your repo" CTA

### Why this matters
- Every reviewed PR creates an indexable page
- Pages target long-tail keywords: `{repo-name} code review`, `{repo-name} PR #{N} review`
- Agent-authored PR reviews target: `claude code review`, `devin code review`, `copilot code review` (from `agentDetection.ts` patterns)
- At 1,000 reviewed PRs/month, that is 1,000 new indexable pages/month
- Review pages link back to GitHub (backlink authority) and GitHub PRs link back to codevetter.com (badge links)

### Implementation
- New route in `apps/landing-page-astro/` (Astro): `/reviews/[owner]/[repo]/pr/[prNumber]`
- Server-side rendered for SEO (Next.js SSR/ISR)
- Data source: query the API worker (`workers/api/`) for review run + findings by repo fullName and PR number
- Need a new public API endpoint: `GET /v1/public/reviews/{owner}/{repo}/{prNumber}` — returns review data without auth (only for repos on free tier with `is_private = false`)
- Structured data (JSON-LD) for rich search results

### Additional SEO content
- `/reviews/{owner}/{repo}` — repo overview page showing all reviewed PRs, average score, agent %
- `/reviews/{owner}` — org/user overview page
- `/leaderboard` — top-scored OSS repos reviewed by CodeVetter
- Blog-style "monthly OSS review roundup" auto-generated from review data

---

## 7. Distribution

### Phase 1: Foundation (before launch)
1. **GitHub Marketplace listing** — the GitHub App must be listed on marketplace.github.com with a free plan. This is the primary discovery channel.
2. **codevetter.com/install** — one-click install page with GitHub App installation link
3. **README badges** — provide embeddable badges that repos can add to their READMEs
4. **`.codevetter.yml` config** — optional repo-level config file (like `.coderabbit.yaml`) that free-tier users can use for basic settings (enable/disable, ignore paths)

### Phase 2: Launch (week 1-2)
1. **Show HN** — "CodeVetter: Free AI code review for open source repos" — lead with the agent-detection angle, which is genuinely novel
2. **Product Hunt** — launch with demo video showing a Devin/Claude Code PR getting reviewed
3. **Twitter/X threads** — demo reviewing real OSS PRs from popular repos (with permission)
4. **r/programming, r/opensource** — "We built a free AI code reviewer for OSS that detects agent-generated code"

### Phase 3: Organic growth (week 3+)
1. **awesome-code-review** lists — submit to curated GitHub lists
2. **awesome-github-apps** — submit to GitHub App directories
3. **Dev.to / Hashnode articles** — "How we review 1,000 OSS PRs/month for free"
4. **OSS maintainer outreach** — DM maintainers of repos with >100 stars that accept agent PRs, offer free review setup
5. **Conference talks** — "The state of AI-generated code: what we learned reviewing 10,000 agent PRs"

### Phase 4: Viral mechanics (ongoing)
1. **Every review is an ad** — the badge link on every PR comment drives traffic from any developer reading the PR
2. **PR review pages are indexable** — SEO compounds over time
3. **Contributor effect** — when a contributor sees CodeVetter on one repo, they install it on their own repos
4. **Agent tool integrations** — partner with Claude Code, Cursor, Devin to be the recommended reviewer. CodeVetter already detects these agents via `agentDetection.ts` — position as "the tool that validates agent output"

### Metrics to track
| Metric | Target (3 months) | Target (6 months) |
|--------|--------------------|--------------------|
| GitHub App installations | 500 | 5,000 |
| Public repos connected | 1,000 | 10,000 |
| PRs reviewed/month | 2,000 | 20,000 |
| Public review pages indexed | 5,000 | 50,000 |
| Free-to-paid conversion rate | 3% | 5% |
| MRR | $2,000 | $15,000 |

---

## Implementation Phases

### Phase A: Core free tier (2-3 weeks)
**Goal**: Public repos get free reviews with badge

- [ ] Add `installation` webhook handler to `workers/api/src/index.ts` for auto-provisioning
- [ ] Add `tier` column to `workspaces` table, `repository_usage` table (migration `0003_free_tier.sql`)
- [ ] Extend `WorkspaceKind` type in `packages/shared-types/src/v1.ts`
- [ ] Modify `buildOverallBody()` in `packages/review-core/src/formatting.ts` to accept tier and inject badge
- [ ] Add tier-based rate limiting in webhook handler before review run creation
- [ ] Add rate limit exceeded comment via `postPrComment()` in `workers/review/src/github.ts`
- [ ] Create badge SVGs and serve from `apps/landing-page-astro/public/`
- [ ] Test: install GitHub App on a public repo, open PR, verify review + badge

### Phase B: Public review pages + SEO (2 weeks)
**Goal**: Every review creates an indexable page

- [ ] Add `GET /v1/public/reviews/:owner/:repo/:prNumber` endpoint to API worker (no auth, public repos only)
- [ ] Build Astro pages in `apps/landing-page-astro/`: `/reviews/[owner]/[repo]/pr/[prNumber]`
- [ ] Add JSON-LD structured data to review pages
- [ ] Build repo overview page: `/reviews/[owner]/[repo]`
- [ ] Add sitemap generation for review pages
- [ ] Add OG meta tags for social sharing (score, findings count, agent badge)

### Phase C: GitHub Marketplace + distribution (1 week)
**Goal**: Listed and discoverable

- [ ] Prepare GitHub Marketplace listing (description, screenshots, pricing plans)
- [ ] Set up free plan in GitHub Marketplace
- [ ] Write install docs at `codevetter.com/docs/install`
- [ ] Create README badge generator at `codevetter.com/badge`
- [ ] Prepare Show HN and Product Hunt assets

### Phase D: Paid tier gating (1-2 weeks)
**Goal**: Upgrade path works end-to-end

- [ ] Build pricing page at `codevetter.com/pricing`
- [ ] Implement Stripe integration for Pro/Team billing (or GitHub Marketplace billing)
- [ ] Gate custom rules behind paid tier (check `workspace.tier` before allowing `upsertWorkspaceRuleDefaults`)
- [ ] Gate private repos behind paid tier (check in webhook handler)
- [ ] Build upgrade CTA components for rate limit, private repo, and rules editor
- [ ] Implement priority queue for paid reviews in `workers/review/src/queue.ts`

### Phase E: Growth and iteration (ongoing)
**Goal**: Compound distribution

- [ ] Launch Show HN, Product Hunt
- [ ] Begin OSS maintainer outreach
- [ ] Build review analytics for paid tier
- [ ] Auto-generate monthly OSS review roundup content
- [ ] Track conversion funnel: install -> review -> rate limit -> upgrade
- [ ] A/B test badge designs and CTA copy in review comments

---

## Cost Model

### Per-review cost (Cloudflare Workers + AI gateway)
| Component | Cost per review |
|-----------|----------------|
| Cloudflare Worker execution | ~$0.0001 (50ms avg, 10M free requests/month) |
| D1 database queries | ~$0.0001 (5 reads + 3 writes per review) |
| AI gateway (LLM call) | ~$0.02-0.08 (depends on diff size and model) |
| GitHub API calls | $0 (within rate limits) |
| **Total** | **~$0.03-0.10 per review** |

### Break-even math
At 10,000 free reviews/month:
- Cost: $300-1,000/month (AI gateway dominates)
- Revenue needed: 20-70 paid users at $15/month
- At 3% conversion from 5,000 installations: 150 paid users = $2,250/month

The unit economics work as long as AI gateway costs stay under $0.10/review. Use model routing (cheaper models for simple diffs, expensive models for complex ones) to optimize. The `AI_GATEWAY_MODEL` env var in `workers/review/src/index.ts` line 49 already supports model selection — extend to per-tier model routing.

---

## Risks and Mitigations

| Risk | Mitigation |
|------|------------|
| AI costs spiral on high-volume repos | Strict rate limits + diff size cap (60k chars free vs 120k paid). Model downgrade for free tier. |
| Low conversion rate | Badge + rate limit create natural upgrade moments. Track which triggers convert best. |
| GitHub App review fatigue (too noisy) | Ship `.codevetter.yml` config for ignore paths/files. Allow disabling via PR label. |
| Competitors copy the free-for-OSS play | CodeRabbit already has a free tier. Differentiate on agent detection (unique) and SEO page strategy (novel). |
| Review quality insufficient at launch | Use the existing review-core pipeline which is already tuned. Start with `balanced` tone. Iterate based on thumbs-up/down reactions on GitHub comments. |
| Abuse (spam repos, CI exploit) | Rate limits per installation. Flag repos with no stars and no contributors. Require at least 1 star or 1 contributor to activate free reviews. |

---

## Key Files Reference

| File | Role |
|------|------|
| `workers/api/src/index.ts` | API worker — webhook handler, auth, workspace CRUD |
| `workers/api/src/github.ts` | GitHub API client (org, repo, PR, tree, blob) |
| `workers/api/src/agentDetection.ts` | Detects agent-authored PRs (bot accounts, body markers, branch prefixes) |
| `workers/review/src/index.ts` | Review worker — processes queued review and indexing jobs |
| `workers/review/src/handlers.ts` | Review job handler — fetches diff, calls AI, posts review |
| `workers/review/src/github.ts` | GitHub App auth (JWT), diff fetching, review posting |
| `workers/review/src/queue.ts` | Job queue (D1 + Postgres adapters) |
| `packages/review-core/src/formatting.ts` | `buildOverallBody()` — generates the PR review comment body |
| `packages/review-core/src/scoring.ts` | `computeScore()`, `determineReviewAction()` |
| `packages/review-core/src/prompt.ts` | `buildPrompt()` — LLM prompt with agent-specific rules |
| `packages/db/src/schema.ts` | Table names enum |
| `packages/db/src/controlPlane.ts` | Database interface + in-memory implementation |
| `packages/db/migrations/0001_init.sql` | Base schema (includes `repositories.is_private`) |
| `packages/db/migrations/0002_agent_metadata.sql` | Agent detection columns on PRs and review runs |
| `packages/shared-types/src/v1.ts` | Core types: `WorkspaceKind`, `ReviewTone`, rule configs |
| `packages/shared-types/src/gateway.ts` | AI gateway request/response types including `AgentContext` |
| `apps/landing-page-astro/` | Astro marketing site (where review pages and badge routes go) |
| `plans/competitive-landscape-2026-03-22.md` | Competitor pricing and feature analysis |
