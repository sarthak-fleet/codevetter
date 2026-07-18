import { createHash } from 'node:crypto';
import { readdir, readFile, writeFile } from 'node:fs/promises';
import { fileURLToPath } from 'node:url';
import path from 'node:path';

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..');
const fixtureRoot = path.join(root, 'tests/fixtures/business-rule-archaeology');
const archaeologyRoot = path.join(root, 'src-tauri/src/commands/business_rule_archaeology');
const inputs = {
  corpus: path.join(archaeologyRoot, 'fixtures/expected.json.fixture'),
  comparison: path.join(fixtureRoot, 'model-comparison-report-v1.json'),
  policy: path.join(fixtureRoot, 'qualification-policy-v1.json'),
};
const output = path.join(fixtureRoot, 'correctness-report-v1.json');

function digest(bytes) {
  return `sha256:${createHash('sha256').update(bytes).digest('hex')}`;
}

function ratio(numerator, denominator) {
  return denominator === 0 ? null : numerator / denominator;
}

function sortedRecord(entries) {
  return Object.fromEntries([...entries].sort(([left], [right]) => left.localeCompare(right)));
}

async function sourceFiles(directory, rootDirectory = directory) {
  const files = [];
  const entries = (await readdir(directory, { withFileTypes: true })).sort((left, right) =>
    left.name.localeCompare(right.name)
  );
  for (const entry of entries) {
    const filename = path.join(directory, entry.name);
    if (entry.isDirectory()) {
      files.push(...(await sourceFiles(filename, rootDirectory)));
    } else if (entry.isFile()) {
      files.push([
        path.relative(rootDirectory, filename).split(path.sep).join('/'),
        await readFile(filename),
      ]);
    }
  }
  return files;
}

async function sourceBundleIdentity(directory) {
  const hash = createHash('sha256');
  for (const [relativePath, bytes] of await sourceFiles(directory)) {
    hash.update(relativePath);
    hash.update('\0');
    hash.update(bytes);
    hash.update('\0');
  }
  return `sha256:${hash.digest('hex')}`;
}

async function generate() {
  const loaded = Object.fromEntries(
    await Promise.all(
      Object.entries(inputs).map(async ([name, filename]) => {
        const bytes = await readFile(filename);
        return [name, { bytes, value: JSON.parse(bytes.toString('utf8')) }];
      })
    )
  );
  const { corpus, comparison, policy } = Object.fromEntries(
    Object.entries(loaded).map(([name, item]) => [name, item.value])
  );
  if (corpus.corpus_id !== comparison.scope.corpus_id) {
    throw new Error('Correctness inputs do not name the same labeled corpus');
  }
  if (comparison.policy.policy_id !== policy.policy_id) {
    throw new Error('Comparison report and qualification policy differ');
  }
  if (
    comparison.input_identities.corpus !== digest(loaded.corpus.bytes) ||
    comparison.input_identities.qualification_policy !== digest(loaded.policy.bytes)
  ) {
    throw new Error('Comparison report is not bound to the supplied corpus and policy');
  }
  const sourceFixtures = await sourceBundleIdentity(path.join(archaeologyRoot, 'fixtures/sources'));

  const sourceUnits = new Map(corpus.source_units.map((unit) => [unit.id, unit]));
  const spans = new Map(corpus.spans.map((span) => [span.id, span]));
  const matrix = new Map();
  for (const fact of corpus.facts) {
    const units = new Set(
      fact.span_ids.map((spanId) => sourceUnits.get(spans.get(spanId)?.source_unit_id))
    );
    if (units.size !== 1 || units.has(undefined)) {
      throw new Error(`Fact ${fact.id} has invalid labeled source provenance`);
    }
    const unit = [...units][0];
    const key = `${unit.language}/${unit.dialect}`;
    const entry = matrix.get(key) ?? { labeled_fact_count: 0, constructs: new Map() };
    entry.labeled_fact_count += 1;
    entry.constructs.set(fact.kind, (entry.constructs.get(fact.kind) ?? 0) + 1);
    matrix.set(key, entry);
  }

  const dialects = sortedRecord(
    [...matrix].map(([key, entry]) => [
      key,
      {
        labeled_fact_count: entry.labeled_fact_count,
        labeled_span_reference_count: corpus.facts
          .filter((fact) =>
            fact.span_ids.some((spanId) => {
              const unit = sourceUnits.get(spans.get(spanId)?.source_unit_id);
              return unit && `${unit.language}/${unit.dialect}` === key;
            })
          )
          .reduce((total, fact) => total + fact.span_ids.length, 0),
        constructs: sortedRecord(
          [...entry.constructs].map(([construct, count]) => [
            construct,
            {
              labeled_positive_count: count,
              exact_span_precision: null,
              exact_span_recall: null,
              fact_precision: null,
              fact_recall: null,
              status: 'not_measured_against_adapter_output',
            },
          ])
        ),
      },
    ])
  );

  const variants = Object.fromEntries(
    comparison.variants.map((variant) => [
      variant.variant,
      {
        case_count: variant.case_count,
        clause_count: variant.clause_count,
        supported_clause_count: variant.supported_clause_count,
        supported_clause_rate: ratio(variant.supported_clause_count, variant.clause_count),
        unsupported_clause_count: variant.unsupported_clause_count,
        unsupported_clause_rate: ratio(variant.unsupported_clause_count, variant.clause_count),
        text_edit_distance: variant.text_edit_distance,
        external_model_calls: variant.external_model_calls,
        input_tokens: variant.input_tokens,
        output_tokens: variant.output_tokens,
        reported_cost_microusd: variant.reported_cost_microusd,
      },
    ])
  );

  const report = {
    schema_version: 1,
    report_id: 'business-rule-archaeology-correctness-v1',
    corpus_id: corpus.corpus_id,
    input_identities: sortedRecord([
      ...Object.entries(loaded).map(([name, item]) => [name, digest(item.bytes)]),
      ['adapter_source_fixtures', sourceFixtures],
    ]),
    labeled_fixture_inventory: {
      source_unit_count: corpus.source_units.length,
      span_count: corpus.spans.length,
      fact_count: corpus.facts.length,
      edge_count: corpus.edges.length,
      rule_count: corpus.rules.length,
      dialects,
    },
    observed_rule_synthesis: {
      variants,
      clause_shapes_covered: comparison.scope.covered_clause_shapes,
      clause_shapes_not_measured: comparison.scope.missing_clause_shapes,
      rule_kind_matches: comparison.cases.filter((item) => item.rule_kind_match).length,
      rule_kind_cases: comparison.cases.length,
    },
    catalog_checks: {
      contradictions: {
        labeled_cases: corpus.conflicts.length,
        precision: null,
        recall: null,
        status: 'not_measured_by_the_comparison_fixture',
      },
      duplicate_reconciliation: {
        labeled_groups: corpus.duplicate_groups.length,
        reconciled_alias_cases: comparison.scope.generated_alias_cases,
        exact_scope_match:
          corpus.duplicate_groups.length === comparison.scope.generated_alias_cases,
        precision: null,
        recall: null,
        status: 'scope_reconciliation_only_not_clustering_accuracy',
      },
      retrieval: {
        precision: null,
        recall: null,
        status: 'not_measured_by_the_comparison_fixture',
      },
      reverse_lookup: {
        precision: null,
        recall: null,
        status: 'not_measured_by_the_comparison_fixture',
      },
      dependency_paths: {
        labeled_edges: corpus.edges.length,
        correct_paths: null,
        status: 'not_measured_by_the_comparison_fixture',
      },
      temporal_diffs: {
        labeled_changes: corpus.history_changes.length,
        correct_changes: null,
        status: 'label_integrity_only_not_canonical_read_accuracy',
      },
    },
    reviewer_correction_effort: {
      human_reviewers: 0,
      measured_minutes: null,
      measured_edits: null,
      deterministic_template_text_edit_distance: variants.deterministic_template.text_edit_distance,
      mock_structured_synthesis_text_edit_distance:
        variants.mock_structured_synthesis.text_edit_distance,
      status: 'not_human_measured_text_distance_is_only_a_reproducible_proxy',
    },
    qualification: {
      policy_id: policy.policy_id,
      policy_version: policy.policy_version,
      full_correctness_qualification: false,
      passing_claim: null,
    },
    limitations: [
      'The labeled inventory is not adapter-output precision or recall; null metrics are intentional.',
      'Clause support and edit distance come from a deterministic no-network mock comparison, not a live model.',
      'Text edit distance is not measured human reviewer effort.',
      'Contradiction, clustering, retrieval, reverse lookup, dependency-path, and temporal accuracy remain unmeasured by this artifact.',
      'This report is correctness evidence only and makes no repository-size or performance claim.',
    ],
  };
  return `${JSON.stringify(report, null, 2)}\n`;
}

const generated = await generate();
if (process.argv.includes('--write')) {
  await writeFile(output, generated);
} else {
  const checked = await readFile(output, 'utf8');
  if (checked !== generated) {
    throw new Error('Correctness report is stale; regenerate with --write');
  }
}
