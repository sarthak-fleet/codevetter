# Curated Benchmark Cases

Store one public agent-generated PR benchmark case per JSON file in this directory.

Create a starter case:

```bash
npm run bench:new-case -- --id=owner-repo-pr-123 --title="Agent regresses checkout state" --repo=owner/repo --pr-url=https://github.com/owner/repo/pull/123
```

Fill every `TODO` field before using `--require-rationales` or making any external claim.

To measure deterministic evidence-search impact, store two reviewer outputs for
the same case, for example `codevetter` and `codevetter_no_evidence`, then run:

```bash
npm run bench:catch-rate -- --reviewer=codevetter --evidence-comparison=codevetter:codevetter_no_evidence --format=markdown
```

Treat this as an internal delta report until the cases are public, hand-labeled,
and include real review artifacts for both variants.
