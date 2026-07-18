import { VerificationDaemonHost } from './daemon-host';

function repositoryArgument(argv: readonly string[]): string {
  if (argv.length !== 2 || argv[0] !== '--repo' || !argv[1]) {
    throw new Error('Usage: verifyd --repo <repository>');
  }
  return argv[1];
}

async function main(): Promise<void> {
  const startup = new AbortController();
  let host: VerificationDaemonHost | undefined;
  const stop = () => {
    startup.abort(new DOMException('verifyd interrupted', 'AbortError'));
    if (host) {
      void host
        .stop()
        .catch((error) => process.stderr.write(`verifyd shutdown failed: ${safeMessage(error)}\n`));
    }
  };
  process.once('SIGINT', stop);
  process.once('SIGTERM', stop);
  host = await VerificationDaemonHost.start(
    repositoryArgument(process.argv.slice(2)),
    startup.signal
  );
  if (startup.signal.aborted) await host.stop();
}

function safeMessage(error: unknown): string {
  const message = error instanceof Error ? error.message : String(error);
  return message.replace(/[\r\n]+/g, ' ').slice(0, 1_000);
}

void main().catch((error) => {
  process.stderr.write(`verifyd failed: ${safeMessage(error)}\n`);
  process.exitCode = 3;
});
