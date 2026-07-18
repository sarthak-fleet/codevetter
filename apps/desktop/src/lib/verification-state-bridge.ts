export interface VerificationRequest {
  protocolVersion: 1;
  runId: string;
  scenarioId: string;
  stateName: string;
  frozenTime: string;
  flags: Readonly<Record<string, string | number | boolean>>;
}

export interface VerificationStatus {
  protocolVersion: 1;
  runId: string;
  scenarioId: string;
  status: 'requested' | 'ready' | 'error';
  message?: string;
}

export interface VerificationWindow {
  __CODEVETTER_VERIFY__?: VerificationRequest;
  __CODEVETTER_VERIFY_STATE__?: VerificationStatus;
}

type StateInstaller = (request: VerificationRequest) => void | Promise<void>;

const codevetterStates: Readonly<Record<string, StateInstaller>> = Object.freeze({
  'shell-navigation-ready': () => undefined,
});

export async function initializeVerificationStateBridge(
  host: VerificationWindow = window as unknown as VerificationWindow,
  installers: Readonly<Record<string, StateInstaller>> = codevetterStates
): Promise<boolean> {
  const request = host.__CODEVETTER_VERIFY__;
  if (!request) return false;

  const base: Omit<VerificationStatus, 'status'> = {
    protocolVersion: 1,
    runId: request.runId,
    scenarioId: request.scenarioId,
  };
  if (!validRequest(request)) {
    host.__CODEVETTER_VERIFY_STATE__ = {
      ...base,
      status: 'error',
      message: 'Verification request is invalid',
    };
    return true;
  }

  const install = installers[request.stateName];
  if (!install) {
    host.__CODEVETTER_VERIFY_STATE__ = {
      ...base,
      status: 'error',
      message: `Unsupported CodeVetter verification state: ${request.stateName}`,
    };
    return true;
  }

  try {
    await install(request);
    host.__CODEVETTER_VERIFY_STATE__ = { ...base, status: 'ready' };
  } catch {
    host.__CODEVETTER_VERIFY_STATE__ = {
      ...base,
      status: 'error',
      message: `Could not install CodeVetter verification state: ${request.stateName}`,
    };
  }
  return true;
}

function validRequest(request: VerificationRequest): boolean {
  return (
    request.protocolVersion === 1 &&
    stableId(request.runId) &&
    stableId(request.scenarioId) &&
    stableId(request.stateName) &&
    !Number.isNaN(Date.parse(request.frozenTime)) &&
    typeof request.flags === 'object' &&
    request.flags !== null
  );
}

function stableId(value: string): boolean {
  return /^[a-zA-Z0-9][a-zA-Z0-9._:-]{0,127}$/.test(value);
}
