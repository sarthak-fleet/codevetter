import { setupWorker } from 'msw/browser';

import { installTargetOwnedBridge } from './bridge';
import { createFixtureHandlers } from './handlers';
import { FixtureStateRegistry } from './states';

export async function startQualificationBridge(target: Window = window) {
  const registry = new FixtureStateRegistry();
  const worker = setupWorker(...createFixtureHandlers(registry));
  return installTargetOwnedBridge(target, registry, worker);
}
