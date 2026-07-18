import { pathToFileURL } from 'node:url';

import { runVerifyCli } from './warm-verification/cli';

export async function runCodeVetterVerifyCli(argv: readonly string[]): Promise<number> {
  if (argv[0] === 'differential') {
    const { runDifferentialCli } = await import('./warm-verification/differential-cli');
    return runDifferentialCli(argv.slice(1));
  }
  if (argv[0] !== 'scenario') return runVerifyCli(argv);
  const { runScenarioCompilerCli } = await import('./scenario-compiler/cli');
  return runScenarioCompilerCli(argv.slice(1));
}

if (process.argv[1] && import.meta.url === pathToFileURL(process.argv[1]).href) {
  void runCodeVetterVerifyCli(process.argv.slice(2)).then((code) => {
    process.exitCode = code;
  });
}
