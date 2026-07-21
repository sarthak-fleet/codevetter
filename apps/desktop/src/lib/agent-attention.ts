import type { AgentProvider } from './tauri-ipc';

export type AgentAttentionKind = 'approval' | 'question' | 'confirmation' | 'setup';
export type AgentAttentionConfidence = 'confirmed' | 'possible';

export interface AgentAttention {
  kind: AgentAttentionKind;
  confidence: AgentAttentionConfidence;
  title: string;
  detail: string;
  evidence: string;
  primaryAction: 'focus-composer' | 'review-output';
}

export function attentionFromStructuredEvent(input: {
  provider: AgentProvider;
  event: string | null;
  detail?: string;
}): AgentAttention | null {
  const providerName = input.provider === 'claude' ? 'Claude' : 'Codex';
  const detail = input.detail?.trim();
  if (input.event === 'permission_request') {
    return {
      kind: 'approval',
      confidence: 'confirmed',
      title: `${providerName} needs your approval`,
      detail: detail || 'The provider is waiting before continuing.',
      evidence: 'Confirmed provider event',
      primaryAction: 'review-output',
    };
  }
  if (input.event === 'question_asked' || input.event === 'ask_user') {
    return {
      kind: 'question',
      confidence: 'confirmed',
      title: `${providerName} is waiting for your answer`,
      detail: detail || 'The provider asked a question.',
      evidence: 'Confirmed provider event',
      primaryAction: 'focus-composer',
    };
  }
  return null;
}

export function attentionFromOutput(input: {
  provider: AgentProvider;
  output: string;
}): AgentAttention | null {
  const plain = input.output
    // biome-ignore lint/suspicious/noControlCharactersInRegex: PTY sanitization intentionally matches CSI escape bytes.
    .replace(/\u001b\[[0-9;?]*[ -/]*[@-~]/g, '')
    .replace(/\s+/g, ' ')
    .trim();
  const lower = plain.toLowerCase();
  const providerName = input.provider === 'claude' ? 'Claude' : 'Codex';
  const match = [
    [
      'allow this command',
      'approval',
      'The provider may be waiting for permission.',
      'review-output',
    ],
    [
      'requires approval',
      'approval',
      'The provider may be waiting for permission.',
      'review-output',
    ],
    [
      'approval required',
      'approval',
      'The provider may be waiting for permission.',
      'review-output',
    ],
    ['allow command', 'approval', 'The provider may be waiting for permission.', 'review-output'],
    [
      'press enter',
      'confirmation',
      'The provider may be waiting for confirmation.',
      'review-output',
    ],
    ['(y/n)', 'confirmation', 'The provider may be waiting for confirmation.', 'review-output'],
    ['enter to review hooks', 'setup', 'The provider may need hook setup review.', 'review-output'],
    ['hooks need review', 'setup', 'The provider may need hook setup review.', 'review-output'],
    ['api key', 'setup', 'The provider may need credentials or setup.', 'review-output'],
    ['sign in', 'setup', 'The provider may need you to finish sign-in.', 'review-output'],
  ].find(([needle]) => lower.includes(needle));
  if (!match) return null;
  const kind = match[1] as AgentAttentionKind;
  return {
    kind,
    confidence: 'possible',
    title: `Possible prompt detected from ${providerName}`,
    detail: match[2],
    evidence: 'Detected in direct provider output',
    primaryAction: match[3] as AgentAttention['primaryAction'],
  };
}
