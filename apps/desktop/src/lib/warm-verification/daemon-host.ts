import type { Server } from 'node:net';

import { collectWorktreeChangeSet } from './change-set';
import { VerificationDaemon } from './daemon';
import { createDefaultDifferentialVerificationService } from './differential-composition';
import { closeServer, closeServerWithin, listenVerifyIpcServer } from './ipc';
import { resolveVerifyRuntimePaths } from './runtime-paths';
import { throwIfAborted } from './runtime-utils';
import {
  acquireVerifySingleton,
  releaseVerifySingleton,
  type VerifySingletonHandle,
} from './singleton';
import { AppServerSupervisor, WarmChromiumSupervisor, WarmRuntimeSupervisor } from './supervision';

export class VerificationDaemonHost {
  readonly #daemon: VerificationDaemon;
  readonly #server: Server;
  readonly #singleton: VerifySingletonHandle;
  #stopPromise: Promise<void> | undefined;

  private constructor(
    daemon: VerificationDaemon,
    server: Server,
    singleton: VerifySingletonHandle
  ) {
    this.#daemon = daemon;
    this.#server = server;
    this.#singleton = singleton;
  }

  static async start(repoPath: string, signal?: AbortSignal): Promise<VerificationDaemonHost> {
    throwIfAborted(signal);
    const collected = await collectWorktreeChangeSet(repoPath);
    throwIfAborted(signal);
    const paths = await resolveVerifyRuntimePaths(collected.repositoryRoot);
    const singleton = await acquireVerifySingleton(paths);
    let daemon: VerificationDaemon | undefined;
    let server: Server | undefined;

    try {
      let host: VerificationDaemonHost | undefined;
      daemon = await VerificationDaemon.create(
        collected.repositoryRoot,
        collected.changeSet.target_sha,
        singleton.lease,
        (repoRoot, config) =>
          new WarmRuntimeSupervisor(
            new AppServerSupervisor(repoRoot, config.config.target),
            new WarmChromiumSupervisor()
          ),
        {
          onShutdown: (graceMs) => void host?.stop(graceMs),
          differentialServiceFactory: (repoRoot, lease, runtime) =>
            createDefaultDifferentialVerificationService(repoRoot, lease, runtime.browser),
        }
      );
      await daemon.start();
      throwIfAborted(signal);
      const readyDaemon = daemon;
      server = await listenVerifyIpcServer(paths.socketPath, (request, connectionSignal) =>
        readyDaemon.handle(request, connectionSignal)
      );
      host = new VerificationDaemonHost(daemon, server, singleton);
      return host;
    } catch (error) {
      if (server) await closeServer(server).catch(() => undefined);
      let cleaned = true;
      if (daemon) {
        try {
          await daemon.stop();
        } catch {
          cleaned = false;
        }
      }
      if (cleaned) await releaseVerifySingleton(singleton).catch(() => undefined);
      throw error;
    }
  }

  stop(graceMs = 5_000): Promise<void> {
    this.#stopPromise ??= this.#stop(graceMs).catch((error) => {
      this.#stopPromise = undefined;
      throw error;
    });
    return this.#stopPromise;
  }

  async #stop(graceMs: number): Promise<void> {
    await this.#daemon.stop(graceMs);
    await closeServerWithin(this.#server, graceMs);
    await releaseVerifySingleton(this.#singleton);
  }
}
