export const WORK_ITEM_STATUSES = ['plan', 'build', 'review', 'verify', 'done'] as const;

export type WorkItemStatus = (typeof WORK_ITEM_STATUSES)[number];
export type WorkItemProvider = 'codex' | 'claude';
export type WorkItemVerificationStatus = 'missing' | 'running' | 'passed' | 'failed' | 'stale';
export type WorkItemCompletionDisposition = 'verified' | 'waived' | 'legacy';

export interface WorkItem {
  schema_version: 1;
  id: string;
  title: string;
  description: string | null;
  acceptance_criteria: string | null;
  project_path: string | null;
  workspace_id: string | null;
  status: WorkItemStatus;
  preferred_provider: WorkItemProvider;
  assigned_agent: string | null;
  agent_terminal_id: string | null;
  agent_session_id: string | null;
  change_identity: string | null;
  review_id: string | null;
  review_score: number | null;
  review_attempts: number;
  verification_run_id: string | null;
  verification_status: WorkItemVerificationStatus;
  completion_disposition: WorkItemCompletionDisposition | null;
  attention: boolean;
  created_at: string;
  updated_at: string;
}

export interface CreateWorkItemInput {
  title: string;
  description?: string | null;
  acceptance_criteria?: string | null;
  project_path?: string | null;
  workspace_id?: string | null;
  preferred_provider?: WorkItemProvider | null;
}

export interface UpdateWorkItemInput {
  title?: string;
  description?: string;
  acceptance_criteria?: string;
  project_path?: string;
  preferred_provider?: WorkItemProvider;
  assigned_agent?: string;
  agent_terminal_id?: string;
  agent_session_id?: string;
  change_identity?: string;
  review_id?: string;
  review_score?: number;
  verification_run_id?: string;
  verification_status?: WorkItemVerificationStatus;
  attention?: boolean;
}

export interface WorkSessionLink {
  key: string;
  label: string;
  detail: string;
  provider: WorkItemProvider;
  terminal_id: string | null;
  session_id: string | null;
  project_path: string | null;
  running: boolean;
}

export interface AttachWorkItemSessionInput {
  provider: WorkItemProvider;
  terminal_id?: string | null;
  session_id?: string | null;
  project_path?: string | null;
}

export function normalizeWorkItemStatus(status: string): WorkItemStatus {
  switch (status.trim().toLowerCase()) {
    case 'build':
    case 'in_progress':
    case 'in-progress':
      return 'build';
    case 'review':
    case 'in_review':
    case 'in-review':
      return 'review';
    case 'verify':
    case 'test':
    case 'in_test':
    case 'in-test':
      return 'verify';
    case 'done':
    case 'completed':
      return 'done';
    default:
      return 'plan';
  }
}

export function groupWorkItems(items: readonly WorkItem[]): Record<WorkItemStatus, WorkItem[]> {
  const grouped: Record<WorkItemStatus, WorkItem[]> = {
    plan: [],
    build: [],
    review: [],
    verify: [],
    done: [],
  };
  for (const item of items) grouped[normalizeWorkItemStatus(item.status)].push(item);
  return grouped;
}

export type WorkEvidenceTone = 'neutral' | 'active' | 'attention' | 'success';

export interface WorkEvidenceSummary {
  label: string;
  tone: WorkEvidenceTone;
  detail: string;
}

export function workItemEvidence(item: WorkItem): WorkEvidenceSummary {
  if (item.attention) {
    return { label: 'Needs attention', tone: 'attention', detail: 'The linked work needs input.' };
  }
  if (item.status === 'done') {
    if (item.completion_disposition === 'verified') {
      return {
        label: 'Verified',
        tone: 'success',
        detail: 'Review and exact-current verification are linked.',
      };
    }
    return {
      label: item.completion_disposition === 'waived' ? 'Completed · waived' : 'Completed · legacy',
      tone: 'neutral',
      detail: 'Completion is not qualified as verified.',
    };
  }
  if (item.verification_status === 'failed' || item.verification_status === 'stale') {
    return {
      label: item.verification_status === 'failed' ? 'Verification failed' : 'Evidence stale',
      tone: 'attention',
      detail: 'Run verification against the current change.',
    };
  }
  if (item.agent_terminal_id) {
    return {
      label: `${item.preferred_provider} active`,
      tone: 'active',
      detail: 'A conversation is attached.',
    };
  }
  if (item.agent_session_id) {
    return {
      label: `${item.preferred_provider} run linked`,
      tone: 'active',
      detail: 'A historical agent run is attached as evidence.',
    };
  }
  if (item.review_id) {
    return { label: 'Review linked', tone: 'active', detail: 'Review evidence is available.' };
  }
  return {
    label: 'No evidence yet',
    tone: 'neutral',
    detail: 'Start with the next workflow action.',
  };
}

export function nextWorkItemStatus(status: WorkItemStatus): WorkItemStatus | null {
  const index = WORK_ITEM_STATUSES.indexOf(status);
  return index >= 0 && index < WORK_ITEM_STATUSES.length - 1
    ? WORK_ITEM_STATUSES[index + 1]!
    : null;
}
