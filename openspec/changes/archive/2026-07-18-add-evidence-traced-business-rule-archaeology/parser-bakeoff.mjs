#!/usr/bin/env node

import { createHash } from 'node:crypto';
import { copyFile, mkdir, mkdtemp, readFile, readdir, rm, stat, writeFile } from 'node:fs/promises';
import os from 'node:os';
import path from 'node:path';
import { spawn } from 'node:child_process';
import { fileURLToPath } from 'node:url';

const SCHEMA_VERSION = 1;
const TREE_SITTER_CLI_VERSION = '0.25.10';
const MAVEN_VERSION = '3.9.11';
const MAVEN_SHA512 =
  'bcfe4fe305c962ace56ac7b5fc7a08b87d5abd8b7e89027ab251069faebee516b0ded8961445d6d91ec1985dfe30f8153268843c89aa392733d1a3ec956c9978';
const changeRoot = path.dirname(fileURLToPath(import.meta.url));
const harnessPath = fileURLToPath(import.meta.url);
const repoRoot = path.resolve(changeRoot, '../../..');
const fixtureRoot = path.join(
  repoRoot,
  'apps/desktop/src-tauri/src/commands/business_rule_archaeology/fixtures'
);
const sourceRoot = path.join(fixtureRoot, 'sources');
const manifestPath = path.join(fixtureRoot, 'expected.json.fixture');
const outputPath = path.join(changeRoot, 'parser-bakeoff-results.json');
const keepTemp = process.argv.includes('--keep-temp');
const tempRoot = await mkdtemp(path.join(os.tmpdir(), 'codevetter-parser-bakeoff-'));
const checkoutsRoot = path.join(tempRoot, 'checkouts');
const toolsRoot = path.join(tempRoot, 'tools');
const artifactsRoot = path.join(tempRoot, 'artifacts');
await Promise.all([
  mkdir(checkoutsRoot, { recursive: true }),
  mkdir(toolsRoot, { recursive: true }),
  mkdir(artifactsRoot, { recursive: true }),
]);

const manifest = JSON.parse(await readFile(manifestPath, 'utf8'));
const sourceUnits = new Map(manifest.source_units.map((unit) => [unit.id, unit]));
const spansByPath = new Map();
for (const span of manifest.spans) {
  const unit = sourceUnits.get(span.source_unit_id);
  if (!unit || unit.protected) continue;
  const spans = spansByPath.get(unit.path) ?? [];
  spans.push(span);
  spansByPath.set(unit.path, spans);
}

const candidates = [
  {
    id: 'bloopai-tree-sitter-cobol',
    family: 'cobol',
    operation: 'self-contained-tree-sitter',
    repository: 'https://github.com/BloopAI/tree-sitter-cobol.git',
    commit: '8ba6692cc3c2bded0693d198936c6e26e6501230',
    license: 'MIT',
    licensePath: 'LICENSE',
    grammarPath: '.',
    files: cobolFiles(),
    buildDependencies: ['C compiler', `tree-sitter-cli ${TREE_SITTER_CLI_VERSION}`],
    runtimeDependencies: ['tree-sitter C runtime'],
    preprocessing: 'Recognizes COPY syntax; does not expand copybooks.',
  },
  {
    id: 'yutaro-tree-sitter-cobol',
    family: 'cobol',
    operation: 'self-contained-tree-sitter',
    repository: 'https://github.com/yutaro-sakamoto/tree-sitter-cobol.git',
    commit: 'e99dbdc3d800d5fa2796476efd60af91f6b43d93',
    license: 'MIT',
    licensePath: 'LICENSE',
    grammarPath: '.',
    files: cobolFiles(),
    buildDependencies: ['C compiler', `tree-sitter-cli ${TREE_SITTER_CLI_VERSION}`],
    runtimeDependencies: ['tree-sitter C runtime'],
    preprocessing: 'Recognizes COPY syntax; does not expand copybooks.',
  },
  {
    id: 'rush-rs-tree-sitter-asm',
    family: 'assembly-generic',
    operation: 'self-contained-tree-sitter',
    repository: 'https://github.com/rush-rs/tree-sitter-asm.git',
    commit: '839741fef4dab5128952334624905c82b40c7133',
    license: 'MIT',
    licensePath: 'LICENSE',
    grammarPath: '.',
    files: assemblyFiles(),
    buildDependencies: ['C compiler', `tree-sitter-cli ${TREE_SITTER_CLI_VERSION}`],
    runtimeDependencies: ['tree-sitter C runtime'],
    preprocessing: 'No macro expansion or dialect-specific assembler pass.',
  },
  {
    id: 'tape-z-tree-sitter-hlasm',
    family: 'assembly-hlasm',
    operation: 'self-contained-tree-sitter',
    repository: 'https://github.com/avishek-sen-gupta/tape-z.git',
    commit: 'df789ae2e8d0cb7e58971bea0655ba1694d4185e',
    license: 'MIT-subproject',
    licensePath: 'java/hlasm-parser/LICENSE',
    grammarPath: 'tree-sitter-hlasm',
    files: assemblyFiles(),
    buildDependencies: ['C compiler', `tree-sitter-cli ${TREE_SITTER_CLI_VERSION}`],
    runtimeDependencies: ['tree-sitter C runtime'],
    preprocessing: 'Grammar recognizes HLASM syntax; the evaluated grammar does not expand macros.',
  },
  {
    id: 'tree-sitter-nasm',
    family: 'assembly-nasm',
    operation: 'self-contained-tree-sitter',
    repository: 'https://github.com/naclsn/tree-sitter-nasm.git',
    commit: 'd1b3638d017f2a8585e26dcfc66fe1df94185e30',
    license: 'MIT',
    licensePath: 'LICENSE',
    grammarPath: '.',
    files: assemblyFiles(),
    buildDependencies: ['C compiler', `tree-sitter-cli ${TREE_SITTER_CLI_VERSION}`],
    runtimeDependencies: ['tree-sitter C runtime'],
    preprocessing:
      'NASM-specific grammar; recognizes macro tokens but does not run NASM expansion.',
  },
];

const limitations = [];
const selectionDecision = {
  id: 'day-one-local-fallback-v1',
  status: 'selected',
  production_dependencies_added: [],
  modern: 'existing-structural-parser',
  cobol: 'bounded-original-source-local-adapter',
  assembly: 'positive-dialect-gated-bounded-local-adapters',
  optional_validators: ['installed-gas-compatible-assembler-diagnostics-only'],
};
let report;
let cleanup;
let cleanupError;
try {
  const treeSitter = await installTreeSitterCli();
  const results = [];
  for (const candidate of candidates) {
    results.push(await evaluateTreeSitterCandidate(candidate, treeSitter));
  }
  results.push(await evaluateProLeap());
  results.push(await evaluateClangGas());
  report = {
    schema_version: SCHEMA_VERSION,
    evaluation_id: 'business-rule-parser-bakeoff-2026-07-16',
    generated_at: new Date().toISOString(),
    selection_decision: selectionDecision,
    corpus: {
      id: manifest.corpus_id,
      manifest_path: path.relative(repoRoot, manifestPath),
      manifest_sha256: sha256(await readFile(manifestPath)),
      source_unit_count: manifest.source_units.length,
      labeled_span_count: manifest.spans.length,
      evaluated_paths: [...new Set([...cobolFiles(), ...assemblyFiles()])].sort(),
      source_sha256: await sourceHashes([...new Set([...cobolFiles(), ...assemblyFiles()])]),
    },
    harness: {
      path: path.relative(repoRoot, harnessPath),
      sha256: sha256(await readFile(harnessPath)),
    },
    machine: await machineMetadata(),
    tools: {
      tree_sitter_cli: TREE_SITTER_CLI_VERSION,
      maven: MAVEN_VERSION,
      node: process.version,
    },
    candidates: results,
    reproduction: {
      run: `node ${path.relative(repoRoot, harnessPath)}`,
      keep_temp_debugging: `node ${path.relative(repoRoot, harnessPath)} --keep-temp`,
      clone:
        'git clone --quiet --filter=blob:none --no-checkout <repository> <temporary-checkout> && git checkout --quiet <commit>',
      tree_sitter_build: `${TREE_SITTER_CLI_VERSION}: tree-sitter build --output <temporary-dylib> <grammar>`,
      tree_sitter_measure:
        'tree-sitter parse --json --time [--paths <21-pass-path-list>] <fixtures>',
      proleap_build: `${MAVEN_VERSION}: mvn -DskipTests -Dskip.nist.tests package`,
      clang_validate:
        '/usr/bin/clang -target x86_64-apple-macos -c <fixture> -o <temporary-object>',
    },
    limitations,
  };
} finally {
  if (keepTemp) {
    cleanup = { automatic: false, status: 'retained-by-explicit-keep-temp' };
  } else {
    try {
      await rm(tempRoot, { recursive: true });
      cleanup = { automatic: true, status: 'removed-owned-mkdtemp-root' };
    } catch (error) {
      cleanupError = error;
      cleanup = { automatic: true, status: 'failed', error: boundedText(String(error), 500) };
    }
  }
}
report.temporary_storage = cleanup;
await writeFile(outputPath, `${JSON.stringify(report, null, 2)}\n`);
process.stdout.write(`${outputPath}${keepTemp ? `\n${tempRoot}` : ''}\n`);
if (cleanupError) throw cleanupError;

async function installTreeSitterCli() {
  const prefix = path.join(toolsRoot, 'tree-sitter');
  const args = [
    'install',
    '--ignore-scripts=false',
    '--no-audit',
    '--no-fund',
    '--prefix',
    prefix,
    `tree-sitter-cli@${TREE_SITTER_CLI_VERSION}`,
  ];
  await checked('npm', args, { cwd: tempRoot, timeoutMs: 120_000 });
  return path.join(prefix, 'node_modules/.bin/tree-sitter');
}

async function evaluateTreeSitterCandidate(candidate, treeSitterPath) {
  const checkout = path.join(checkoutsRoot, candidate.id);
  await clonePinned(candidate.repository, candidate.commit, checkout);
  const grammar = path.join(checkout, candidate.grammarPath);
  const artifact = path.join(artifactsRoot, `${candidate.id}.dylib`);
  const buildStarted = performance.now();
  const build = await checked(treeSitterPath, ['build', '--output', artifact, grammar], {
    cwd: grammar,
    timeoutMs: 180_000,
  });
  const buildMs = round(performance.now() - buildStarted);
  const fixtureAliases = await fixturePaths(candidate.files, candidate.id);
  const files = [];
  for (const fixture of fixtureAliases) {
    const xml = await run(treeSitterPath, ['parse', '--xml', fixture.parsePath], {
      cwd: grammar,
      timeoutMs: 30_000,
      allowFailure: true,
    });
    const summary = await run(treeSitterPath, ['parse', '--json', '--time', fixture.parsePath], {
      cwd: grammar,
      timeoutMs: 30_000,
      allowFailure: true,
    });
    files.push(
      summarizeTreeSitterFile(fixture, xml.stdout, parseCliJson(summary.stdout), xml.code)
    );
  }
  const timing = await treeSitterTiming(treeSitterPath, grammar, fixtureAliases);
  const memory = await measuredRss(
    treeSitterPath,
    ['parse', '--json', '--quiet', ...fixtureAliases.map((f) => f.parsePath)],
    grammar
  );
  const metadata = await repositoryMetadata(checkout, candidate);
  const artifactStat = await stat(artifact);
  return {
    id: candidate.id,
    family: candidate.family,
    operation: candidate.operation,
    repository: candidate.repository,
    commit: candidate.commit,
    ...metadata,
    license: candidate.license,
    license_path: candidate.licensePath,
    license_sha256: sha256(await readFile(path.join(checkout, candidate.licensePath))),
    build_dependencies: candidate.buildDependencies,
    runtime_dependencies: candidate.runtimeDependencies,
    preprocessing: candidate.preprocessing,
    build: {
      success: build.code === 0,
      duration_ms: buildMs,
      parser_binary_bytes: artifactStat.size,
      generated_grammar_bytes: await grammarBytes(grammar),
      checkout_bytes_without_git: await directoryBytes(checkout, new Set(['.git'])),
      ...(await grammarMetadata(grammar)),
      warnings: boundedText(build.stderr),
    },
    performance: { ...timing, maximum_resident_bytes: memory },
    files,
    aggregate: aggregateTreeSitter(files),
  };
}

function summarizeTreeSitterFile(fixture, xml, cliJson, exitCode) {
  const nodes = parseXmlNodes(xml, fixture.source);
  const errorNodes = nodes.filter((node) => node.type === 'ERROR' || node.type === 'MISSING');
  const labeled = spansByPath.get(fixture.relativePath) ?? [];
  const spanChecks = labeled.map((span) => {
    const exactBytes = nodes.filter(
      (node) =>
        node.type !== 'ERROR' && node.startByte === span.start[0] && node.endByte === span.end[0]
    );
    const exactLineColumns = nodes.filter(
      (node) =>
        node.type !== 'ERROR' &&
        node.start[0] + 1 === span.start[1] &&
        node.start[1] + 1 === span.start[2] &&
        node.end[0] + 1 === span.end[1] &&
        node.end[1] + 1 === span.end[2]
    );
    const covering = nodes
      .filter(
        (node) =>
          node.type !== 'ERROR' && node.startByte <= span.start[0] && node.endByte >= span.end[0]
      )
      .sort((left, right) => left.endByte - left.startByte - (right.endByte - right.startByte));
    const smallest = covering[0];
    const expectedBytes = span.end[0] - span.start[0];
    return {
      span_id: span.id,
      exact_byte_span: exactBytes.length > 0,
      exact_line_column_span: exactLineColumns.length > 0,
      tight_cover:
        smallest !== undefined &&
        smallest.endByte - smallest.startByte <= Math.max(expectedBytes + 8, expectedBytes * 1.5),
      smallest_cover_type: smallest?.type ?? null,
      smallest_cover_bytes: smallest ? smallest.endByte - smallest.startByte : null,
      overlaps_error_region: errorNodes.some((node) => overlap(node, span.start[0], span.end[0])),
    };
  });
  const parseSummary = cliJson?.parse_summaries?.[0];
  return {
    path: fixture.relativePath,
    dialect: fixture.dialect,
    parse_success: Boolean(parseSummary?.successful) && exitCode === 0,
    duration_us: parseSummary ? durationUs(parseSummary.duration) : null,
    bytes: fixture.bytes,
    root_byte_range: nodes[0] ? [nodes[0].startByte, nodes[0].endByte] : null,
    node_count: nodes.length,
    error_node_count: errorNodes.length,
    error_byte_ranges: errorNodes.map(({ type, startByte, endByte }) => [type, startByte, endByte]),
    exact_byte_span_ids: spanChecks
      .filter((entry) => entry.exact_byte_span)
      .map((entry) => entry.span_id),
    exact_line_column_span_ids: spanChecks
      .filter((entry) => entry.exact_line_column_span)
      .map((entry) => entry.span_id),
    tight_span_ids: spanChecks.filter((entry) => entry.tight_cover).map((entry) => entry.span_id),
    error_overlap_span_ids: spanChecks
      .filter((entry) => entry.overlaps_error_region)
      .map((entry) => entry.span_id),
    exact_labeled_span_count: spanChecks.filter((entry) => entry.exact_byte_span).length,
    exact_line_column_span_count: spanChecks.filter((entry) => entry.exact_line_column_span).length,
    tight_labeled_span_count: spanChecks.filter((entry) => entry.tight_cover).length,
    labeled_span_count: spanChecks.length,
  };
}

async function treeSitterTiming(treeSitterPath, grammar, fixtures) {
  const pathsFile = path.join(tempRoot, `timing-${path.basename(grammar)}-${randomId()}.txt`);
  const repeated = [];
  for (let iteration = 0; iteration < 21; iteration += 1) {
    for (const fixture of fixtures) repeated.push(fixture.parsePath);
  }
  await writeFile(pathsFile, `${repeated.join('\n')}\n`);
  const measured = await run(treeSitterPath, ['parse', '--json', '--time', '--paths', pathsFile], {
    cwd: grammar,
    timeoutMs: 120_000,
    allowFailure: true,
  });
  const summaries = parseCliJson(measured.stdout).parse_summaries;
  const values = summaries.map((entry) => durationUs(entry.duration));
  const cold = values.slice(0, fixtures.length);
  const warm = values.slice(fixtures.length);
  return {
    cold_corpus_ms: round(cold.reduce((total, value) => total + value, 0) / 1_000),
    warm_file_p50_us: percentile(warm, 50),
    warm_file_p95_us: percentile(warm, 95),
    warm_sample_count: warm.length,
  };
}

async function evaluateProLeap() {
  const candidate = {
    id: 'proleap-cobol-parser',
    repository: 'https://github.com/uwol/proleap-cobol-parser.git',
    commit: 'd1bfe75bdd6d480f70c74c6345bcc02610ac30d3',
  };
  const checkout = path.join(checkoutsRoot, candidate.id);
  await clonePinned(candidate.repository, candidate.commit, checkout);
  const javaHome = await detectJavaHome();
  if (!javaHome) {
    limitations.push('ProLeap was not executed because no JDK 17+ was available.');
    return unavailableCandidate(candidate, 'JDK 17+ unavailable');
  }
  const mavenRoot = await installMaven();
  const maven = path.join(mavenRoot, 'bin/mvn');
  const m2 = path.join(tempRoot, 'm2');
  const buildStarted = performance.now();
  await checked(
    maven,
    [
      '-quiet',
      '-file',
      path.join(checkout, 'pom.xml'),
      `-Dmaven.repo.local=${m2}`,
      '-DskipTests',
      '-Dskip.nist.tests',
      'package',
    ],
    { cwd: checkout, timeoutMs: 600_000, env: { JAVA_HOME: javaHome } }
  );
  const buildMs = round(performance.now() - buildStarted);
  const classpathFile = path.join(tempRoot, 'proleap-classpath.txt');
  await checked(
    maven,
    [
      '-quiet',
      '-file',
      path.join(checkout, 'pom.xml'),
      `-Dmaven.repo.local=${m2}`,
      'dependency:build-classpath',
      `-Dmdep.outputFile=${classpathFile}`,
    ],
    { cwd: checkout, timeoutMs: 300_000, env: { JAVA_HOME: javaHome } }
  );
  const parserJar = path.join(checkout, 'target/proleap-cobol-parser-4.0.0.jar');
  const runnerRoot = path.join(tempRoot, 'proleap-runner');
  await mkdir(runnerRoot, { recursive: true });
  const runnerSource = path.join(runnerRoot, 'ProLeapBakeoff.java');
  await writeFile(runnerSource, proLeapRunnerSource());
  const dependencyClasspath = (await readFile(classpathFile, 'utf8')).trim();
  const classpath = `${parserJar}${path.delimiter}${dependencyClasspath}`;
  await checked(
    path.join(javaHome, 'bin/javac'),
    ['-cp', classpath, '-d', runnerRoot, runnerSource],
    { cwd: runnerRoot, timeoutMs: 120_000, env: { JAVA_HOME: javaHome } }
  );
  const runnerClasspath = `${runnerRoot}${path.delimiter}${classpath}`;
  const args = ['-cp', runnerClasspath, 'ProLeapBakeoff', sourceRoot, ...cobolFiles()];
  const executed = await checked(path.join(javaHome, 'bin/java'), args, {
    cwd: runnerRoot,
    timeoutMs: 300_000,
    env: { JAVA_HOME: javaHome },
  });
  const payload = JSON.parse(lastJsonLine(executed.stdout));
  const memory = await measuredRss(path.join(javaHome, 'bin/java'), args, runnerRoot, {
    JAVA_HOME: javaHome,
  });
  const metadata = await repositoryMetadata(checkout, {
    ...candidate,
    licensePath: 'LICENSE',
  });
  return {
    id: candidate.id,
    family: 'cobol',
    operation: 'self-contained-java-with-preprocessor-and-semantic-pass',
    repository: candidate.repository,
    commit: candidate.commit,
    ...metadata,
    license: 'MIT',
    license_path: 'LICENSE',
    license_sha256: sha256(await readFile(path.join(checkout, 'LICENSE'))),
    build_dependencies: [
      `JDK ${payload.java_version}`,
      `Apache Maven ${MAVEN_VERSION}`,
      'ANTLR 4.7.2',
    ],
    runtime_dependencies: ['JVM 17+', 'ANTLR runtime', 'SLF4J'],
    preprocessing:
      'COPY/REPLACE/CBL/PROCESS capable and semantic ASG-capable; preprocessing changes the parse coordinate space.',
    original_source_span_fidelity: 'not directly exposed after preprocessing',
    build: {
      success: true,
      duration_ms: buildMs,
      parser_binary_bytes: (await stat(parserJar)).size,
      generated_grammar_bytes: await matchingBytes(path.join(checkout, 'src/main/antlr4'), /\.g4$/),
      checkout_bytes_without_git: await directoryBytes(checkout, new Set(['.git', 'target'])),
    },
    performance: {
      cold_corpus_ms: payload.cold_corpus_ms,
      warm_file_p50_us: payload.warm_file_p50_us,
      warm_file_p95_us: payload.warm_file_p95_us,
      warm_sample_count: payload.warm_sample_count,
      maximum_resident_bytes: memory,
    },
    files: payload.files,
    aggregate: {
      strict_successes: payload.files.filter((entry) => entry.strict_success).length,
      tolerant_successes: payload.files.filter((entry) => entry.tolerant_success).length,
      file_count: payload.files.length,
      exact_original_span_measurement_available: false,
    },
  };
}

async function evaluateClangGas() {
  const clang = '/usr/bin/clang';
  const version = await checked(clang, ['--version'], { cwd: tempRoot });
  const fixture = path.join(sourceRoot, 'asm/route_x86.s');
  const output = path.join(artifactsRoot, 'route_x86.o');
  const command = ['-target', 'x86_64-apple-macos', '-c', fixture, '-o', output];
  const samples = [];
  for (let index = 0; index < 21; index += 1) {
    const started = performance.now();
    await checked(clang, command, { cwd: tempRoot, timeoutMs: 30_000 });
    samples.push(round((performance.now() - started) * 1_000));
  }
  const memory = await measuredRss(clang, command, tempRoot);
  return {
    id: 'apple-clang-integrated-gas-x86_64',
    family: 'assembly-x86-64-gas-att',
    operation: 'compiler-assisted-validation',
    repository: 'https://github.com/llvm/llvm-project',
    commit: null,
    installed_version: version.stdout.split('\n')[0],
    maintenance_evidence:
      'Installed Apple Clang distribution; upstream LLVM is actively maintained.',
    license: 'Apache-2.0 WITH LLVM-exception',
    license_path: 'https://github.com/llvm/llvm-project/blob/main/LICENSE.TXT',
    build_dependencies: [],
    runtime_dependencies: ['installed clang toolchain'],
    preprocessing: 'Compiler/assembler performs real target validation and emits an object file.',
    original_source_span_fidelity: 'diagnostic locations only; no reusable syntax tree',
    build: {
      success: true,
      parser_binary_bytes: (await stat(clang)).size,
      emitted_object_bytes: (await stat(output)).size,
    },
    performance: {
      cold_corpus_ms: round(samples[0] / 1_000),
      warm_file_p50_us: percentile(samples.slice(1), 50),
      warm_file_p95_us: percentile(samples.slice(1), 95),
      warm_sample_count: samples.length - 1,
      maximum_resident_bytes: memory,
    },
    files: [
      {
        path: 'asm/route_x86.s',
        dialect: 'x86-64-gas-att',
        parse_success: true,
        emitted_object: true,
        node_count: null,
        error_node_count: null,
        labeled_span_count: (spansByPath.get('asm/route_x86.s') ?? []).length,
        exact_labeled_span_count: null,
      },
    ],
    aggregate: {
      strict_successes: 1,
      file_count: 1,
      exact_original_span_measurement_available: false,
    },
  };
}

function proLeapRunnerSource() {
  return String.raw`
import java.io.File;
import java.util.*;
import io.proleap.cobol.asg.metamodel.*;
import io.proleap.cobol.asg.params.impl.CobolParserParamsImpl;
import io.proleap.cobol.asg.runner.impl.CobolParserRunnerImpl;
import io.proleap.cobol.preprocessor.CobolPreprocessor.CobolSourceFormatEnum;
import org.antlr.v4.runtime.tree.ParseTree;

public class ProLeapBakeoff {
  static record Outcome(boolean success, int units, int nodes, String error) {}
  static CobolSourceFormatEnum format(String path) {
    return path.contains("free_route") ? CobolSourceFormatEnum.TANDEM : CobolSourceFormatEnum.FIXED;
  }
  static Outcome parse(File root, String relative, boolean tolerant) {
    try {
      var params = new CobolParserParamsImpl();
      params.setFormat(format(relative));
      params.setCopyBookDirectories(List.of(new File(root, "cobol")));
      params.setCopyBookExtensions(List.of("cpy", "CPY"));
      params.setIgnoreSyntaxErrors(tolerant);
      Program program = new CobolParserRunnerImpl().analyzeFile(new File(root, relative), params);
      int nodes = 0;
      for (CompilationUnit unit : program.getCompilationUnits()) {
        if (unit.getProgramUnit() != null && unit.getProgramUnit().getCtx() != null) {
          nodes += count(unit.getProgramUnit().getCtx());
        }
      }
      return new Outcome(true, program.getCompilationUnits().size(), nodes, "");
    } catch (Throwable error) {
      String message = error.getClass().getSimpleName() + ":" + String.valueOf(error.getMessage());
      return new Outcome(false, 0, 0, message.substring(0, Math.min(240, message.length())));
    }
  }
  static int count(ParseTree node) {
    int total = 1;
    for (int index = 0; index < node.getChildCount(); index++) total += count(node.getChild(index));
    return total;
  }
  static String q(String value) {
    return "\"" + value.replace("\\", "\\\\").replace("\"", "\\\"").replace("\n", "\\n").replace("\r", "") + "\"";
  }
  static long percentile(List<Long> values, double p) {
    if (values.isEmpty()) return 0;
    Collections.sort(values);
    return values.get(Math.min(values.size() - 1, (int)Math.ceil(values.size() * p) - 1));
  }
  public static void main(String[] args) {
    File root = new File(args[0]);
    List<String> files = Arrays.asList(args).subList(1, args.length);
    long coldStart = System.nanoTime();
    List<Outcome> strict = new ArrayList<>();
    List<Outcome> tolerant = new ArrayList<>();
    for (String file : files) {
      strict.add(parse(root, file, false));
      tolerant.add(parse(root, file, true));
    }
    double coldMs = (System.nanoTime() - coldStart) / 1_000_000.0;
    List<Long> warm = new ArrayList<>();
    for (int iteration = 0; iteration < 20; iteration++) {
      for (String file : files) {
        long started = System.nanoTime();
        parse(root, file, false);
        warm.add((System.nanoTime() - started) / 1_000);
      }
    }
    StringBuilder out = new StringBuilder();
    out.append("{\"java_version\":").append(q(System.getProperty("java.version")));
    out.append(",\"cold_corpus_ms\":").append(String.format(Locale.ROOT, "%.3f", coldMs));
    out.append(",\"warm_file_p50_us\":").append(percentile(warm, .50));
    out.append(",\"warm_file_p95_us\":").append(percentile(warm, .95));
    out.append(",\"warm_sample_count\":").append(warm.size()).append(",\"files\":[");
    for (int index = 0; index < files.size(); index++) {
      if (index > 0) out.append(',');
      Outcome s = strict.get(index), t = tolerant.get(index);
      out.append("{\"path\":").append(q(files.get(index)))
        .append(",\"strict_success\":").append(s.success())
        .append(",\"tolerant_success\":").append(t.success())
        .append(",\"compilation_units\":").append(Math.max(s.units(), t.units()))
        .append(",\"parse_tree_nodes\":").append(Math.max(s.nodes(), t.nodes()))
        .append(",\"strict_error\":").append(q(s.error()))
        .append(",\"tolerant_error\":").append(q(t.error()))
        .append(",\"span_basis\":\"preprocessed-not-original\"}");
    }
    out.append("]}");
    System.out.println(out);
  }
}
`;
}

async function fixturePaths(relativePaths, candidateId) {
  const values = [];
  for (const relativePath of relativePaths) {
    const sourcePath = path.join(sourceRoot, relativePath);
    let parsePath = sourcePath;
    if (relativePath.endsWith('.lst')) {
      parsePath = path.join(tempRoot, `${candidateId}-generated.cbl`);
      await copyFile(sourcePath, parsePath);
    }
    const unit = manifest.source_units.find((entry) => entry.path === relativePath);
    const source = await readFile(sourcePath);
    values.push({
      relativePath,
      parsePath,
      dialect: unit?.dialect ?? 'unknown',
      bytes: source.byteLength,
      source,
    });
  }
  return values;
}

async function clonePinned(repository, commit, destination) {
  await checked(
    'git',
    ['clone', '--quiet', '--filter=blob:none', '--no-checkout', repository, destination],
    {
      cwd: tempRoot,
      timeoutMs: 180_000,
    }
  );
  await checked('git', ['checkout', '--quiet', commit], { cwd: destination, timeoutMs: 180_000 });
  const actual = (await checked('git', ['rev-parse', 'HEAD'], { cwd: destination })).stdout.trim();
  if (actual !== commit) throw new Error(`Pinned checkout mismatch for ${repository}`);
}

async function repositoryMetadata(checkout, candidate) {
  const commit = await checked('git', ['show', '-s', '--format=%cI%n%s', candidate.commit], {
    cwd: checkout,
  });
  const [committedAt, ...subject] = commit.stdout.trim().split('\n');
  return {
    committed_at: committedAt,
    commit_subject: subject.join('\n'),
    maintenance_source: `${candidate.repository.replace(/\.git$/, '')}/commit/${candidate.commit}`,
  };
}

async function installMaven() {
  const archive = path.join(toolsRoot, `apache-maven-${MAVEN_VERSION}.tar.gz`);
  const response = await fetch(
    `https://archive.apache.org/dist/maven/maven-3/${MAVEN_VERSION}/binaries/apache-maven-${MAVEN_VERSION}-bin.tar.gz`
  );
  if (!response.ok) throw new Error(`Maven download failed: ${response.status}`);
  const bytes = Buffer.from(await response.arrayBuffer());
  if (sha512(bytes) !== MAVEN_SHA512) throw new Error('Maven SHA-512 did not match');
  await writeFile(archive, bytes);
  await checked('tar', ['-xzf', archive, '-C', toolsRoot], { cwd: toolsRoot, timeoutMs: 120_000 });
  return path.join(toolsRoot, `apache-maven-${MAVEN_VERSION}`);
}

async function detectJavaHome() {
  const candidates = [
    process.env.JAVA_HOME,
    '/opt/homebrew/opt/openjdk@21/libexec/openjdk.jdk/Contents/Home',
    '/opt/homebrew/opt/openjdk@17/libexec/openjdk.jdk/Contents/Home',
  ].filter(Boolean);
  for (const candidate of candidates) {
    try {
      await stat(path.join(candidate, 'bin/java'));
      return candidate;
    } catch {
      // Continue to the next explicit local JDK candidate.
    }
  }
  return null;
}

async function measuredRss(command, args, cwd, env = {}) {
  if (process.platform === 'darwin') {
    const measured = await run('/usr/bin/time', ['-l', command, ...args], {
      cwd,
      env,
      timeoutMs: 300_000,
      allowFailure: true,
    });
    const match = measured.stderr.match(/(\d+)\s+maximum resident set size/);
    return match ? Number(match[1]) : null;
  }
  limitations.push(`Maximum RSS was not measured on unsupported platform ${process.platform}.`);
  return null;
}

async function machineMetadata() {
  const kernel = await checked('uname', ['-srvmp'], { cwd: tempRoot });
  let osVersion = os.release();
  if (process.platform === 'darwin') {
    osVersion = (await checked('sw_vers', ['-productVersion'], { cwd: tempRoot })).stdout.trim();
  }
  return {
    platform: process.platform,
    architecture: process.arch,
    os_version: osVersion,
    cpu: os.cpus()[0]?.model ?? 'unknown',
    logical_cpu_count: os.cpus().length,
    memory_bytes: os.totalmem(),
    kernel: kernel.stdout.trim(),
  };
}

async function grammarBytes(grammar) {
  return matchingBytes(
    path.join(grammar, 'src'),
    /^(?:parser\.c|scanner\.(?:c|cc)|node-types\.json)$/
  );
}

async function grammarMetadata(grammar) {
  const parserPath = path.join(grammar, 'src/parser.c');
  const source = await readFile(parserPath, 'utf8');
  const languageVersion = source.match(/#define LANGUAGE_VERSION (\d+)/)?.[1];
  let externalScanner = false;
  for (const name of ['scanner.c', 'scanner.cc']) {
    try {
      await stat(path.join(grammar, 'src', name));
      externalScanner = true;
    } catch {
      // This scanner variant is absent.
    }
  }
  return {
    tree_sitter_language_version: languageVersion ? Number(languageVersion) : null,
    external_scanner: externalScanner,
  };
}

async function sourceHashes(relativePaths) {
  const entries = [];
  for (const relativePath of [...relativePaths].sort()) {
    entries.push([relativePath, sha256(await readFile(path.join(sourceRoot, relativePath)))]);
  }
  return Object.fromEntries(entries);
}

async function matchingBytes(root, pattern) {
  let total = 0;
  for await (const entry of walk(root)) {
    if (pattern.test(path.basename(entry))) total += (await stat(entry)).size;
  }
  return total;
}

async function directoryBytes(root, excludedNames = new Set()) {
  let total = 0;
  for await (const entry of walk(root, excludedNames)) total += (await stat(entry)).size;
  return total;
}

async function* walk(root, excludedNames = new Set()) {
  let entries;
  try {
    entries = await readdir(root, { withFileTypes: true });
  } catch {
    return;
  }
  for (const entry of entries) {
    if (excludedNames.has(entry.name) || entry.isSymbolicLink()) continue;
    const absolute = path.join(root, entry.name);
    if (entry.isDirectory()) yield* walk(absolute, excludedNames);
    else if (entry.isFile()) yield absolute;
  }
}

function parseXmlNodes(xml, source) {
  const nodes = [];
  const pattern =
    /<([A-Za-z_][A-Za-z0-9_.-]*)(?:\s+field="[^"]+")?\s+srow="(\d+)"\s+scol="(\d+)"\s+erow="(\d+)"\s+ecol="(\d+)"/g;
  for (const match of xml.matchAll(pattern)) {
    const start = [Number(match[2]), Number(match[3])];
    const end = [Number(match[4]), Number(match[5])];
    nodes.push({
      type: match[1],
      start,
      end,
      startByte: pointToByte(source, start),
      endByte: pointToByte(source, end),
    });
  }
  return nodes;
}

function pointToByte(source, [row, column]) {
  const text = source.toString('utf8');
  const lines = text.split('\n');
  let bytes = 0;
  for (let index = 0; index < row && index < lines.length; index += 1) {
    bytes += Buffer.byteLength(lines[index]) + 1;
  }
  return bytes + Buffer.byteLength((lines[row] ?? '').slice(0, column));
}

function parseCliJson(stdout) {
  const start = stdout.indexOf('{');
  if (start < 0) throw new Error(`Tree-sitter JSON output was missing: ${stdout.slice(0, 200)}`);
  return JSON.parse(stdout.slice(start));
}

function aggregateTreeSitter(files) {
  const labeled = files.reduce((total, file) => total + file.labeled_span_count, 0);
  const exact = files.reduce((total, file) => total + file.exact_labeled_span_count, 0);
  const tight = files.reduce((total, file) => total + file.tight_labeled_span_count, 0);
  const exactLineColumns = files.reduce(
    (total, file) => total + file.exact_line_column_span_count,
    0
  );
  return {
    parse_successes: files.filter((file) => file.parse_success).length,
    file_count: files.length,
    error_node_count: files.reduce((total, file) => total + file.error_node_count, 0),
    labeled_span_count: labeled,
    exact_labeled_span_count: exact,
    exact_labeled_span_rate: labeled === 0 ? null : round(exact / labeled, 4),
    exact_line_column_span_count: exactLineColumns,
    exact_line_column_span_rate: labeled === 0 ? null : round(exactLineColumns / labeled, 4),
    tight_labeled_span_count: tight,
    tight_labeled_span_rate: labeled === 0 ? null : round(tight / labeled, 4),
    exact_original_span_measurement_available: true,
  };
}

function overlap(node, startByte, endByte) {
  return node.startByte < endByte && node.endByte > startByte;
}

function durationUs(duration) {
  return round(duration.secs * 1_000_000 + duration.nanos / 1_000, 3);
}

function percentile(values, p) {
  if (values.length === 0) return null;
  const sorted = [...values].sort((left, right) => left - right);
  return sorted[Math.min(sorted.length - 1, Math.ceil((p / 100) * sorted.length) - 1)];
}

function cobolFiles() {
  return [
    'cobol/fixed_claim.cbl',
    'cobol/CLAIMREC.cpy',
    'cobol/free_route.cbl',
    'recovery/broken_claim.cbl',
    'conflict/override.cbl',
    'generated/claim_listing.lst',
  ];
}

function assemblyFiles() {
  return ['asm/billing_hlasm.asm', 'asm/route_x86.s', 'asm/ambiguous.asm'];
}

function unavailableCandidate(candidate, reason) {
  return {
    id: candidate.id,
    repository: candidate.repository,
    commit: candidate.commit,
    available: false,
    reason,
  };
}

function lastJsonLine(stdout) {
  return stdout
    .split('\n')
    .reverse()
    .find((line) => line.trim().startsWith('{'));
}

function sha256(bytes) {
  return createHash('sha256').update(bytes).digest('hex');
}

function sha512(bytes) {
  return createHash('sha512').update(bytes).digest('hex');
}

function randomId() {
  return Math.random().toString(16).slice(2);
}

function round(value, digits = 3) {
  const factor = 10 ** digits;
  return Math.round(value * factor) / factor;
}

function boundedText(value, max = 2_000) {
  return value.trim().slice(0, max);
}

async function checked(command, args, options = {}) {
  return run(command, args, { ...options, allowFailure: false });
}

function run(command, args, options = {}) {
  return new Promise((resolve, reject) => {
    const child = spawn(command, args, {
      cwd: options.cwd ?? tempRoot,
      env: { ...process.env, ...options.env },
      shell: false,
      stdio: ['ignore', 'pipe', 'pipe'],
    });
    const stdout = [];
    const stderr = [];
    let stdoutBytes = 0;
    let stderrBytes = 0;
    const maxBytes = options.maxBytes ?? 32 * 1024 * 1024;
    const timer = setTimeout(() => child.kill('SIGTERM'), options.timeoutMs ?? 60_000);
    child.stdout.on('data', (chunk) => {
      stdoutBytes += chunk.byteLength;
      if (stdoutBytes <= maxBytes) stdout.push(chunk);
    });
    child.stderr.on('data', (chunk) => {
      stderrBytes += chunk.byteLength;
      if (stderrBytes <= maxBytes) stderr.push(chunk);
    });
    child.once('error', reject);
    child.once('close', (code, signal) => {
      clearTimeout(timer);
      const result = {
        code: code ?? -1,
        signal,
        stdout: Buffer.concat(stdout).toString('utf8'),
        stderr: Buffer.concat(stderr).toString('utf8'),
      };
      if (stdoutBytes > maxBytes || stderrBytes > maxBytes) {
        reject(new Error(`${command} output exceeded ${maxBytes} bytes`));
      } else if (!options.allowFailure && code !== 0) {
        reject(
          new Error(
            `${command} ${args.join(' ')} failed (${code ?? signal}): ${result.stderr.slice(-2_000)}`
          )
        );
      } else resolve(result);
    });
  });
}
