import { AlertTriangle, CheckCircle2, ChevronDown, ChevronRight, Plus, Users } from 'lucide-react';
import { useEffect, useMemo, useState } from 'react';

import { Button } from '@/components/ui/button';
import { Input } from '@/components/ui/input';
import {
  audienceModeLabel,
  audienceValidationWarning,
  qualifyAudienceBundleWithWarmEvidence,
} from '@/lib/audience-validation';
import {
  addAudienceValidationResponse,
  type AudienceResponseProvenance,
  type AudienceValidationBundle,
  createAudienceValidationRun,
  getCurrentWarmVerificationIdentity,
  getAudienceValidation,
  isTauriAvailable,
  listWarmVerificationRuns,
  waiveAudienceValidation,
} from '@/lib/tauri-ipc';

export interface AudienceValidationPanelProps {
  reviewId: string;
  repoPath: string;
  defaultArtifact?: string;
  onBundleChange: (bundle: AudienceValidationBundle | null) => void;
}

const fieldClass =
  'h-8 border-[var(--cv-line)] bg-[#08090d] font-mono text-[11px] text-slate-200 placeholder:text-slate-700';
const textareaClass =
  'min-h-16 w-full rounded-md border border-[var(--cv-line)] bg-[#08090d] px-3 py-2 font-mono text-[11px] text-slate-200 outline-none placeholder:text-slate-700 focus:border-cyan-400/40';
const selectClass =
  'h-8 w-full rounded-md border border-[var(--cv-line)] bg-[#08090d] px-2 font-mono text-[11px] text-slate-200 outline-none focus:border-cyan-400/40';

function stageTone(status: string): string {
  if (status === 'passed' || status === 'completed' || status === 'verified') {
    return 'text-emerald-300';
  }
  if (status === 'failed' || status === 'blocked') return 'text-red-300';
  if (status === 'waived') return 'text-slate-400';
  return 'text-amber-300';
}

async function qualifyWithCurrentWarmEvidence(
  value: AudienceValidationBundle,
  repoPath: string
): Promise<AudienceValidationBundle> {
  const [runs, current] = await Promise.allSettled([
    listWarmVerificationRuns({ repoPath, limit: 1 }),
    getCurrentWarmVerificationIdentity(repoPath),
  ]);
  return qualifyAudienceBundleWithWarmEvidence(
    value,
    runs.status === 'fulfilled' ? (runs.value[0] ?? null) : null,
    current.status === 'fulfilled' ? current.value : null,
    [
      ...(runs.status === 'rejected' ? ['Warm verification history could not be read.'] : []),
      ...(current.status === 'rejected'
        ? ['Current verification identity lookup failed; prior evidence remains unverified.']
        : []),
    ]
  );
}

export default function AudienceValidationPanel({
  reviewId,
  repoPath,
  defaultArtifact = '',
  onBundleChange,
}: AudienceValidationPanelProps) {
  const [open, setOpen] = useState(true);
  const [bundle, setBundle] = useState<AudienceValidationBundle | null>(null);
  const [loading, setLoading] = useState(false);
  const [saving, setSaving] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [audience, setAudience] = useState('Target users affected by this change');
  const [task, setTask] = useState('Complete the changed user flow and explain any friction.');
  const [candidateA, setCandidateA] = useState('Changed build');
  const [candidateAArtifact, setCandidateAArtifact] = useState(defaultArtifact);
  const [candidateB, setCandidateB] = useState('');
  const [candidateBArtifact, setCandidateBArtifact] = useState('');
  const [criteriaText, setCriteriaText] = useState('task completion, clarity, trust');
  const [minResponses, setMinResponses] = useState(3);
  const [required, setRequired] = useState(true);
  const [provenance, setProvenance] = useState<AudienceResponseProvenance>('agent');
  const [criterion, setCriterion] = useState('task completion');
  const [preferred, setPreferred] = useState('');
  const [reversePreferred, setReversePreferred] = useState('');
  const [confidence, setConfidence] = useState(0.7);
  const [taskPassed, setTaskPassed] = useState<'unknown' | 'yes' | 'no'>('unknown');
  const [feedback, setFeedback] = useState('');
  const [evidenceRef, setEvidenceRef] = useState('');
  const [waiverReason, setWaiverReason] = useState('');

  useEffect(() => {
    if (!candidateAArtifact && defaultArtifact) setCandidateAArtifact(defaultArtifact);
  }, [candidateAArtifact, defaultArtifact]);

  useEffect(() => {
    let canceled = false;
    if (!reviewId || !isTauriAvailable()) {
      setBundle(null);
      onBundleChange(null);
      return;
    }
    setLoading(true);
    setError(null);
    void getAudienceValidation(reviewId)
      .then((value) => qualifyWithCurrentWarmEvidence(value, repoPath))
      .then((value) => {
        if (canceled) return;
        setBundle(value);
        onBundleChange(value);
        if (value.run) {
          setCriterion(value.run.criteria[0] ?? 'task completion');
          setPreferred(value.run.candidate_a);
        }
      })
      .catch((cause) => {
        if (!canceled) setError(String(cause));
      })
      .finally(() => {
        if (!canceled) setLoading(false);
      });
    return () => {
      canceled = true;
    };
  }, [onBundleChange, repoPath, reviewId]);

  const warning = bundle ? audienceValidationWarning(bundle) : null;
  const stageRows = useMemo(
    () =>
      bundle
        ? [
            bundle.verification.review,
            bundle.verification.executable_test,
            bundle.verification.audience,
          ]
        : [],
    [bundle]
  );

  function acceptBundle(value: AudienceValidationBundle) {
    setBundle(value);
    onBundleChange(value);
    setError(null);
  }

  async function acceptQualifiedBundle(
    value: AudienceValidationBundle
  ): Promise<AudienceValidationBundle> {
    const qualified = await qualifyWithCurrentWarmEvidence(value, repoPath);
    acceptBundle(qualified);
    return qualified;
  }

  async function handleCreateRun() {
    setSaving(true);
    setError(null);
    try {
      const value = await createAudienceValidationRun({
        reviewId,
        repoPath,
        audience,
        task,
        candidateA,
        candidateAArtifact,
        candidateB: candidateB || null,
        candidateBArtifact: candidateBArtifact || null,
        criteria: criteriaText
          .split(',')
          .map((item) => item.trim())
          .filter(Boolean),
        minResponses,
        required,
      });
      const qualified = await acceptQualifiedBundle(value);
      setCriterion(qualified.run?.criteria[0] ?? 'task completion');
      setPreferred(qualified.run?.candidate_a ?? candidateA);
    } catch (cause) {
      setError(String(cause));
    } finally {
      setSaving(false);
    }
  }

  async function handleAddResponse() {
    if (!bundle?.run) return;
    setSaving(true);
    setError(null);
    try {
      const value = await addAudienceValidationResponse({
        runId: bundle.run.id,
        provenance,
        criterion,
        candidateA: bundle.run.candidate_a,
        candidateB: bundle.run.candidate_b,
        preferredCandidate: preferred || null,
        reversePreferredCandidate: reversePreferred || null,
        confidence,
        taskPassed: taskPassed === 'unknown' ? null : taskPassed === 'yes',
        feedback,
        evidenceRef,
      });
      await acceptQualifiedBundle(value);
      setFeedback('');
      setEvidenceRef('');
      setReversePreferred('');
    } catch (cause) {
      setError(String(cause));
    } finally {
      setSaving(false);
    }
  }

  async function handleWaive() {
    setSaving(true);
    setError(null);
    try {
      await acceptQualifiedBundle(await waiveAudienceValidation(reviewId, waiverReason));
      setWaiverReason('');
    } catch (cause) {
      setError(String(cause));
    } finally {
      setSaving(false);
    }
  }

  return (
    <section
      className="shrink-0 border-t border-[var(--cv-line)] bg-[#07080a]"
      data-testid="audience-validation-panel"
    >
      <button
        type="button"
        onClick={() => setOpen((value) => !value)}
        className="flex w-full items-center gap-2 px-3 py-2 text-left"
      >
        {open ? (
          <ChevronDown size={12} className="text-slate-500" />
        ) : (
          <ChevronRight size={12} className="text-slate-500" />
        )}
        <Users size={12} className="text-cyan-300" />
        <span className="cv-label flex-1 text-slate-300">Audience validation</span>
        {bundle && (
          <span
            className={`font-mono text-[10px] ${stageTone(bundle.verification.audience.status)}`}
          >
            {bundle.verification.audience.status}
          </span>
        )}
      </button>

      {open && (
        <div className="space-y-3 border-t border-[var(--cv-line)] px-3 py-3">
          {loading && (
            <p className="font-mono text-[11px] text-slate-500">Loading staged verification…</p>
          )}
          {error && <p className="font-mono text-[11px] text-red-300">{error}</p>}

          {bundle && (
            <div className="grid grid-cols-3 gap-1.5">
              {stageRows.map((stage) => (
                <div key={stage.label} className="border border-[var(--cv-line)] bg-[#090a0d] p-2">
                  <div className="font-mono text-[9px] uppercase tracking-[0.1em] text-slate-600">
                    {stage.label}
                  </div>
                  <div className={`mt-1 font-mono text-[10px] ${stageTone(stage.status)}`}>
                    {stage.status}
                  </div>
                </div>
              ))}
            </div>
          )}

          {!bundle?.run && !loading && (
            <div className="space-y-2">
              <p className="text-[11px] leading-5 text-slate-500">
                After review and executable QA, define who should exercise the changed behavior.
                Agent simulations and human evidence stay visibly separate.
              </p>
              <Input
                className={fieldClass}
                value={audience}
                onChange={(event) => setAudience(event.target.value)}
                placeholder="Target audience"
              />
              <textarea
                className={textareaClass}
                value={task}
                onChange={(event) => setTask(event.target.value)}
                placeholder="Audience task"
              />
              <div className="grid grid-cols-2 gap-2">
                <Input
                  className={fieldClass}
                  value={candidateA}
                  onChange={(event) => setCandidateA(event.target.value)}
                  placeholder="Candidate A"
                />
                <Input
                  className={fieldClass}
                  value={candidateB}
                  onChange={(event) => setCandidateB(event.target.value)}
                  placeholder="Candidate B (optional)"
                />
                <Input
                  className={fieldClass}
                  value={candidateAArtifact}
                  onChange={(event) => setCandidateAArtifact(event.target.value)}
                  placeholder="A route/artifact"
                />
                <Input
                  className={fieldClass}
                  value={candidateBArtifact}
                  onChange={(event) => setCandidateBArtifact(event.target.value)}
                  placeholder="B route/artifact"
                />
              </div>
              <Input
                className={fieldClass}
                value={criteriaText}
                onChange={(event) => setCriteriaText(event.target.value)}
                placeholder="Comma-separated criteria"
              />
              <div className="flex items-center gap-2">
                <Input
                  className={`${fieldClass} w-20`}
                  type="number"
                  min={1}
                  max={1000}
                  value={minResponses}
                  onChange={(event) =>
                    setMinResponses(Math.max(1, Number(event.target.value) || 1))
                  }
                  aria-label="Minimum audience responses"
                />
                <label className="flex items-center gap-2 font-mono text-[10px] text-slate-500">
                  <input
                    type="checkbox"
                    checked={required}
                    onChange={(event) => setRequired(event.target.checked)}
                  />
                  Required
                </label>
                <Button
                  size="sm"
                  className="ml-auto h-8 gap-1 text-[10px]"
                  onClick={handleCreateRun}
                  disabled={saving || !audience.trim() || !task.trim()}
                >
                  <Plus size={11} /> Start
                </Button>
              </div>
            </div>
          )}

          {bundle?.run && (
            <>
              <div className="space-y-1 border border-[var(--cv-line)] bg-[#090a0d] p-2">
                <div className="flex items-center gap-2">
                  <span className="font-mono text-[10px] text-slate-300">
                    {audienceModeLabel(bundle)}
                  </span>
                  <span className="ml-auto font-mono text-[10px] text-cyan-300">
                    {bundle.diagnostics.signal_strength} signal
                  </span>
                </div>
                <p className="text-[11px] text-slate-500">
                  {bundle.run.audience} · {bundle.run.task}
                </p>
                <p className="font-mono text-[10px] text-slate-600">
                  {bundle.responses.length}/{bundle.run.min_responses} responses · human{' '}
                  {bundle.diagnostics.human_response_count} · agent{' '}
                  {bundle.diagnostics.agent_response_count} · imported{' '}
                  {bundle.diagnostics.imported_response_count}
                </p>
                {warning && (
                  <p className="flex items-start gap-1.5 font-mono text-[10px] text-amber-300">
                    <AlertTriangle size={11} className="mt-0.5 shrink-0" /> {warning}
                  </p>
                )}
                {bundle.verification.human_validation_fulfilled && (
                  <p className="flex items-center gap-1.5 font-mono text-[10px] text-emerald-300">
                    <CheckCircle2 size={11} /> Human validation fulfilled
                  </p>
                )}
              </div>

              {!bundle.run.waived_reason && (
                <div className="space-y-2 border border-[var(--cv-line)] bg-[#090a0d] p-2">
                  <div className="grid grid-cols-2 gap-2">
                    <select
                      className={selectClass}
                      value={provenance}
                      onChange={(event) =>
                        setProvenance(event.target.value as AudienceResponseProvenance)
                      }
                      aria-label="Response provenance"
                    >
                      <option value="agent">Agent simulation</option>
                      <option value="human">Human participant</option>
                      <option value="imported">Imported evidence</option>
                    </select>
                    <select
                      className={selectClass}
                      value={criterion}
                      onChange={(event) => setCriterion(event.target.value)}
                      aria-label="Audience criterion"
                    >
                      {bundle.run.criteria.map((item) => (
                        <option key={item} value={item}>
                          {item}
                        </option>
                      ))}
                    </select>
                    <select
                      className={selectClass}
                      value={preferred}
                      onChange={(event) => setPreferred(event.target.value)}
                      aria-label="Preferred candidate"
                    >
                      <option value="">No preference</option>
                      <option value={bundle.run.candidate_a}>{bundle.run.candidate_a}</option>
                      {bundle.run.candidate_b && (
                        <option value={bundle.run.candidate_b}>{bundle.run.candidate_b}</option>
                      )}
                    </select>
                    <select
                      className={selectClass}
                      value={reversePreferred}
                      onChange={(event) => setReversePreferred(event.target.value)}
                      aria-label="Reverse-order preference"
                    >
                      <option value="">Reverse not run</option>
                      <option value={bundle.run.candidate_a}>{bundle.run.candidate_a}</option>
                      {bundle.run.candidate_b && (
                        <option value={bundle.run.candidate_b}>{bundle.run.candidate_b}</option>
                      )}
                    </select>
                    <select
                      className={selectClass}
                      value={taskPassed}
                      onChange={(event) => setTaskPassed(event.target.value as typeof taskPassed)}
                      aria-label="Task result"
                    >
                      <option value="unknown">Task result unknown</option>
                      <option value="yes">Task passed</option>
                      <option value="no">Task failed</option>
                    </select>
                    <label className="flex items-center gap-2 font-mono text-[10px] text-slate-500">
                      Confidence
                      <input
                        type="range"
                        min={0}
                        max={1}
                        step={0.05}
                        value={confidence}
                        onChange={(event) => setConfidence(Number(event.target.value))}
                        className="min-w-0 flex-1"
                      />
                      {Math.round(confidence * 100)}%
                    </label>
                  </div>
                  <textarea
                    className={textareaClass}
                    value={feedback}
                    onChange={(event) => setFeedback(event.target.value)}
                    placeholder="Feedback or observation"
                  />
                  <Input
                    className={fieldClass}
                    value={evidenceRef}
                    onChange={(event) => setEvidenceRef(event.target.value)}
                    placeholder="Screenshot, trace, recording, or external evidence reference"
                  />
                  <Button
                    size="sm"
                    className="h-7 gap-1 text-[10px]"
                    onClick={handleAddResponse}
                    disabled={saving}
                  >
                    <Plus size={11} /> Add response
                  </Button>
                </div>
              )}

              {!bundle.run.waived_reason && (
                <div className="flex gap-2">
                  <Input
                    className={fieldClass}
                    value={waiverReason}
                    onChange={(event) => setWaiverReason(event.target.value)}
                    placeholder="Why audience validation is not applicable"
                  />
                  <Button
                    size="sm"
                    variant="ghost"
                    className="h-8 shrink-0 text-[10px] text-slate-500"
                    onClick={handleWaive}
                    disabled={saving || !waiverReason.trim()}
                  >
                    Waive
                  </Button>
                </div>
              )}

              {bundle.diagnostics.order_inconsistent_count > 0 && (
                <p className="font-mono text-[10px] text-amber-300">
                  {bundle.diagnostics.order_inconsistent_count} order-sensitive judgment(s) were
                  excluded from the winner signal.
                </p>
              )}
              {bundle.diagnostics.criteria_with_cycles.length > 0 && (
                <p className="font-mono text-[10px] text-amber-300">
                  Preference cycle: {bundle.diagnostics.criteria_with_cycles.join(', ')}.
                </p>
              )}
            </>
          )}
        </div>
      )}
    </section>
  );
}
