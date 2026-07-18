import {
  Check,
  Download,
  GitCommitHorizontal,
  Link2,
  Loader2,
  MessageSquareText,
  X,
} from 'lucide-react';
import { type FormEvent, useEffect, useRef, useState } from 'react';

import { Button } from '@/components/ui/button';
import { Input } from '@/components/ui/input';
import { readableArchaeologyError } from '@/lib/business-rule-archaeology/catalog-view';
import type {
  ArchaeologyExportFormat,
  ArchaeologyReviewMutation,
  ArchaeologyRuleDetail,
  ArchaeologyRuleLifecycle,
} from '@/lib/business-rule-archaeology/contracts';
import {
  exportBusinessRuleArchaeology,
  mutateBusinessRuleArchaeologyReview,
} from '@/lib/tauri-ipc';

function requestId(): string {
  return globalThis.crypto?.randomUUID?.() ?? `review-${Date.now()}-${Math.random()}`;
}

export function ArchaeologyExportControls({ repositoryId }: { repositoryId: string }) {
  const [format, setFormat] = useState<ArchaeologyExportFormat>('json');
  const [busy, setBusy] = useState(false);
  const [message, setMessage] = useState<string | null>(null);
  const [cursor, setCursor] = useState<string | null>(null);
  const [chunk, setChunk] = useState(1);
  const requestEpoch = useRef(0);

  useEffect(() => {
    requestEpoch.current += 1;
    setMessage(null);
    setCursor(null);
    setChunk(1);
  }, [format, repositoryId]);

  const exportCatalog = async () => {
    const epoch = ++requestEpoch.current;
    setBusy(true);
    setMessage(null);
    try {
      const result = await exportBusinessRuleArchaeology({
        repository_id: repositoryId,
        format,
        limit: 1_000,
        cursor,
      });
      if (epoch !== requestEpoch.current) return;
      const blob = new Blob([result.content], { type: `${result.mime_type};charset=utf-8` });
      const url = URL.createObjectURL(blob);
      try {
        const anchor = document.createElement('a');
        anchor.href = url;
        anchor.download = `business-rules-${result.generation_id.slice(0, 12)}-part-${chunk}.${result.extension}`;
        anchor.click();
      } finally {
        URL.revokeObjectURL(url);
      }
      setMessage(
        result.truncated
          ? `${result.rule_count.toLocaleString()} rules exported; more remain in the bounded catalog.`
          : `${result.rule_count.toLocaleString()} rules exported.`
      );
      setCursor(result.next_cursor);
      setChunk((value) => (result.next_cursor ? value + 1 : 1));
    } catch (error) {
      if (epoch === requestEpoch.current) setMessage(readableArchaeologyError(error));
    } finally {
      if (epoch === requestEpoch.current) setBusy(false);
    }
  };

  return (
    <div className="flex flex-wrap items-center justify-end gap-2">
      <label className="sr-only" htmlFor="archaeology-export-format">
        Export format
      </label>
      <select
        id="archaeology-export-format"
        value={format}
        onChange={(event) => setFormat(event.target.value as ArchaeologyExportFormat)}
        className="h-8 rounded-md border border-white/10 bg-[var(--bg-raised)] px-2 text-xs text-[var(--text-secondary)]"
        disabled={busy}
      >
        <option value="json">JSON</option>
        <option value="markdown">Markdown</option>
        <option value="csv">CSV</option>
      </select>
      <Button
        type="button"
        variant="outline"
        size="sm"
        className="h-8 gap-1.5"
        onClick={() => void exportCatalog()}
        disabled={busy}
      >
        {busy ? <Loader2 size={13} className="animate-spin" /> : <Download size={13} />}
        {cursor ? 'Export next chunk' : 'Export'}
      </Button>
      {message ? (
        <span role="status" className="basis-full text-right text-[10px] text-[var(--text-muted)]">
          {message}
        </span>
      ) : null}
    </div>
  );
}

export function ArchaeologyReviewActions({
  repositoryId,
  generationId,
  rule,
  onChanged,
}: {
  repositoryId: string;
  generationId: string;
  rule: ArchaeologyRuleDetail;
  onChanged: () => void;
}) {
  const [busy, setBusy] = useState(false);
  const [message, setMessage] = useState<string | null>(null);
  const [annotation, setAnnotation] = useState('');
  const [rejectReason, setRejectReason] = useState('');
  const [aliasRuleId, setAliasRuleId] = useState('');
  const [predecessorGenerationId, setPredecessorGenerationId] = useState('');
  const [predecessorRuleId, setPredecessorRuleId] = useState('');
  const [predecessorLifecycle, setPredecessorLifecycle] =
    useState<ArchaeologyRuleLifecycle>('accepted');
  const epoch = useRef(0);

  useEffect(() => {
    epoch.current += 1;
    setBusy(false);
    setMessage(null);
    setAnnotation('');
    setRejectReason('');
    setAliasRuleId('');
    setPredecessorGenerationId('');
    setPredecessorRuleId('');
    setPredecessorLifecycle('accepted');
  }, [generationId, repositoryId, rule.rule_id]);

  const mutate = async (mutation: ArchaeologyReviewMutation) => {
    const current = ++epoch.current;
    setBusy(true);
    setMessage(null);
    try {
      const result = await mutateBusinessRuleArchaeologyReview({
        request_id: requestId(),
        repository_id: repositoryId,
        generation_id: generationId,
        rule_id: rule.rule_id,
        expected_lifecycle: rule.lifecycle,
        mutation,
      });
      if (current !== epoch.current) return;
      setMessage(`Saved · ${result.lifecycle.replaceAll('_', ' ')}`);
      setAnnotation('');
      setRejectReason('');
      onChanged();
    } catch (error) {
      if (current === epoch.current) setMessage(readableArchaeologyError(error));
    } finally {
      if (current === epoch.current) setBusy(false);
    }
  };

  const annotate = (event: FormEvent) => {
    event.preventDefault();
    const value = annotation.trim();
    if (value) void mutate({ kind: 'annotate', annotation: value });
  };

  const updateAlias = (mutation: 'link' | 'unlink') => {
    const value = aliasRuleId.trim();
    if (value) void mutate({ kind: 'alias', alias_rule_id: value, mutation });
  };

  return (
    <section
      className="space-y-3 rounded-xl border border-white/[0.07] bg-white/[0.02] p-3"
      aria-label="Review this rule"
    >
      <div>
        <div className="cv-label">Human review</div>
        <p className="mt-1 text-[11px] text-[var(--text-muted)]">
          Every change is appended to the local audit stream. Stale writes are rejected.
        </p>
      </div>
      <div className="grid gap-2 sm:grid-cols-2">
        <Button
          type="button"
          variant="outline"
          size="sm"
          className="h-8 gap-1.5 border-emerald-400/20 text-emerald-200"
          disabled={busy}
          onClick={() => void mutate({ kind: 'review', decision: 'accept' })}
        >
          <Check size={13} /> Accept
        </Button>
        <div className="flex gap-1.5">
          <Input
            value={rejectReason}
            onChange={(event) => setRejectReason(event.target.value)}
            placeholder="Rejection reason"
            aria-label="Rejection reason"
            className="h-8 text-xs"
            disabled={busy}
          />
          <Button
            type="button"
            variant="outline"
            size="sm"
            className="h-8 gap-1.5 border-red-400/20 text-red-200"
            disabled={busy || !rejectReason.trim()}
            onClick={() =>
              void mutate({ kind: 'review', decision: 'reject', reason: rejectReason.trim() })
            }
          >
            <X size={13} /> Reject
          </Button>
        </div>
      </div>
      <form onSubmit={annotate} className="flex gap-1.5">
        <Input
          value={annotation}
          onChange={(event) => setAnnotation(event.target.value)}
          placeholder="Add an evidence or domain note"
          aria-label="Rule annotation"
          className="h-8 text-xs"
          disabled={busy}
        />
        <Button
          type="submit"
          variant="outline"
          size="sm"
          className="h-8 gap-1.5"
          disabled={busy || !annotation.trim()}
        >
          <MessageSquareText size={13} /> Note
        </Button>
      </form>
      <div className="flex gap-1.5">
        <Input
          value={aliasRuleId}
          onChange={(event) => setAliasRuleId(event.target.value)}
          placeholder="Stable alias rule identity"
          aria-label="Alias rule identity"
          className="h-8 font-mono text-xs"
          disabled={busy}
        />
        <Button
          type="button"
          variant="outline"
          size="sm"
          className="h-8 gap-1.5"
          disabled={busy || !aliasRuleId.trim()}
          onClick={() => updateAlias('link')}
        >
          <Link2 size={13} /> Link
        </Button>
        <Button
          type="button"
          variant="ghost"
          size="sm"
          className="h-8"
          disabled={busy || !aliasRuleId.trim()}
          onClick={() => updateAlias('unlink')}
        >
          Unlink
        </Button>
      </div>
      <details className="rounded-lg border border-white/[0.06] px-2.5 py-2">
        <summary className="cursor-pointer text-[11px] text-[var(--text-secondary)]">
          Link an exact predecessor
        </summary>
        <p className="mt-2 text-[10px] text-[var(--text-muted)]">
          Use generation-bound identities from temporal comparison. The selected ready rule becomes
          the reviewed successor.
        </p>
        <div className="mt-2 grid gap-2 sm:grid-cols-2">
          <Input
            value={predecessorGenerationId}
            onChange={(event) => setPredecessorGenerationId(event.target.value)}
            placeholder="Prior generation identity"
            aria-label="Predecessor generation identity"
            className="h-8 font-mono text-xs"
            disabled={busy}
          />
          <Input
            value={predecessorRuleId}
            onChange={(event) => setPredecessorRuleId(event.target.value)}
            placeholder="Prior stable rule identity"
            aria-label="Predecessor rule identity"
            className="h-8 font-mono text-xs"
            disabled={busy}
          />
          <select
            value={predecessorLifecycle}
            onChange={(event) =>
              setPredecessorLifecycle(event.target.value as ArchaeologyRuleLifecycle)
            }
            aria-label="Expected predecessor lifecycle"
            className="h-8 rounded-md border border-white/10 bg-[var(--bg-raised)] px-2 text-xs text-[var(--text-secondary)]"
            disabled={busy}
          >
            {['candidate', 'review_needed', 'accepted', 'rejected', 'conflicted'].map((value) => (
              <option key={value} value={value}>
                {value.replaceAll('_', ' ')}
              </option>
            ))}
          </select>
          <Button
            type="button"
            variant="outline"
            size="sm"
            className="h-8 gap-1.5"
            disabled={busy || !predecessorGenerationId.trim() || !predecessorRuleId.trim()}
            onClick={() =>
              void mutate({
                kind: 'supersede',
                predecessor_generation_id: predecessorGenerationId.trim(),
                predecessor_rule_id: predecessorRuleId.trim(),
                expected_predecessor_lifecycle: predecessorLifecycle,
              })
            }
          >
            <GitCommitHorizontal size={13} /> Mark successor
          </Button>
        </div>
      </details>
      {message ? (
        <p role="status" className="text-[11px] text-[var(--text-secondary)]">
          {busy ? <Loader2 size={12} className="mr-1 inline animate-spin" /> : null}
          {message}
        </p>
      ) : null}
    </section>
  );
}
