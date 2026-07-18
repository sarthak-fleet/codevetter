import type { SetupWorker } from 'msw/browser';

import { FixtureStateRegistry, type VerificationStateRequest } from './states';

export interface VerificationStateStatus {
  protocolVersion: 1;
  runId: string;
  scenarioId: string;
  status: 'requested' | 'ready' | 'error';
  message?: string;
}

export interface VerificationTarget {
  __CODEVETTER_VERIFY__?: VerificationStateRequest;
  __CODEVETTER_VERIFY_STATE__?: VerificationStateStatus;
}

export type FixtureWorker = Pick<SetupWorker, 'start'>;

export interface InstalledVerificationState {
  clientId: string;
  request: VerificationStateRequest;
}

function exactStatus(
  request: Pick<VerificationStateRequest, 'runId' | 'scenarioId'>,
  status: 'ready' | 'error',
  message?: string
): VerificationStateStatus {
  return message === undefined
    ? {
        protocolVersion: 1,
        runId: request.runId,
        scenarioId: request.scenarioId,
        status,
      }
    : {
        protocolVersion: 1,
        runId: request.runId,
        scenarioId: request.scenarioId,
        status,
        message,
      };
}

function isRequest(value: unknown): value is VerificationStateRequest {
  if (typeof value !== 'object' || value === null) return false;
  const candidate = value as Partial<VerificationStateRequest>;
  return (
    candidate.protocolVersion === 1 &&
    typeof candidate.runId === 'string' &&
    candidate.runId.length > 0 &&
    typeof candidate.scenarioId === 'string' &&
    candidate.scenarioId.length > 0 &&
    typeof candidate.stateName === 'string' &&
    candidate.stateName.length > 0 &&
    typeof candidate.frozenTime === 'string' &&
    typeof candidate.flags === 'object' &&
    candidate.flags !== null
  );
}

export async function installTargetOwnedBridge(
  target: VerificationTarget,
  registry: FixtureStateRegistry,
  worker: FixtureWorker
): Promise<InstalledVerificationState | null> {
  const request = target.__CODEVETTER_VERIFY__;
  if (!isRequest(request)) return null;

  let clientId: string;
  try {
    clientId = registry.install(request);
  } catch (error) {
    target.__CODEVETTER_VERIFY_STATE__ = exactStatus(
      request,
      'error',
      error instanceof Error ? error.message : 'Unknown verification state'
    );
    return null;
  }

  try {
    await worker.start({ onUnhandledRequest: 'error', quiet: true });
  } catch {
    registry.remove(clientId);
    target.__CODEVETTER_VERIFY_STATE__ = exactStatus(
      request,
      'error',
      'MSW verification worker failed to start'
    );
    return null;
  }

  target.__CODEVETTER_VERIFY_STATE__ = exactStatus(request, 'ready');
  return { clientId, request };
}
