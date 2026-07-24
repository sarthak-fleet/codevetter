import { CheckCircle2, Download, Eye, Loader2, ShieldAlert } from 'lucide-react';
import { useMemo, useState } from 'react';

import { Button } from '@/components/ui/button';
import {
  buildAgentPrXray,
  type CliReviewFinding,
  pickXrayExportPath,
  saveAgentPrXray,
  type XrayBuildResult,
  type XrayFormat,
  type XrayRequest,
} from '@/lib/tauri-ipc';

interface XrayExportPanelProps {
  reviewId: string;
  findings: CliReviewFinding[];
}

export default function XrayExportPanel({ reviewId, findings }: XrayExportPanelProps) {
  const [source, setSource] = useState('');
  const [confirmed, setConfirmed] = useState(false);
  const [format, setFormat] = useState<XrayFormat>('html');
  const [approved, setApproved] = useState<Set<string>>(new Set());
  const [result, setResult] = useState<XrayBuildResult | null>(null);
  const [busy, setBusy] = useState<'preview' | 'save' | null>(null);
  const [error, setError] = useState<string | null>(null);

  const approvable = useMemo(
    () => findings.filter((finding) => finding.id && finding.suggestion),
    [findings]
  );
  const request = (): XrayRequest => ({
    review_id: reviewId,
    public_source_confirmed: confirmed,
    public_source: source.trim() || null,
    approved_excerpt_finding_ids: [...approved],
  });

  const preview = async () => {
    setBusy('preview');
    setError(null);
    try {
      setResult(await buildAgentPrXray(request()));
    } catch (cause) {
      setError(cause instanceof Error ? cause.message : String(cause));
    } finally {
      setBusy(null);
    }
  };

  const save = async () => {
    const latest = await buildAgentPrXray(request());
    setResult(latest);
    if (!latest.eligible) return;
    const path = await pickXrayExportPath(format, latest.payload.xray_id);
    if (!path) return;
    setBusy('save');
    setError(null);
    try {
      await saveAgentPrXray(request(), format, path);
    } catch (cause) {
      setError(cause instanceof Error ? cause.message : String(cause));
    } finally {
      setBusy(null);
    }
  };

  const toggleExcerpt = (id: string) => {
    setApproved((current) => {
      const next = new Set(current);
      if (next.has(id)) next.delete(id);
      else next.add(id);
      return next;
    });
  };

  return (
    <details className="group shrink-0 border-b border-[var(--cv-line)]">
      <summary className="flex cursor-pointer list-none items-center justify-between px-6 py-4 text-sm text-slate-300 hover:bg-white/[0.02]">
        <span className="flex items-center gap-2 font-medium">
          <Eye size={15} className="text-[var(--cv-accent)]" />
          Agent PR X-Ray
        </span>
        <span className="text-xs text-slate-600">Safe, local evidence export</span>
      </summary>
      <div className="space-y-4 px-6 pb-6">
        <p className="max-w-2xl text-xs leading-5 text-slate-500">
          Build JSON, Markdown, and a self-contained HTML card from this completed review. The
          export never reruns a model and blocks local paths, secrets, prompts, and raw provider
          output.
        </p>

        <label className="block space-y-1.5">
          <span className="text-xs font-medium text-slate-400">Public source</span>
          <input
            value={source}
            onChange={(event) => setSource(event.target.value)}
            placeholder="owner/repository#123"
            className="h-9 w-full rounded-lg border border-[var(--cv-line)] bg-black/20 px-3 text-sm text-slate-200 outline-none placeholder:text-slate-700 focus:border-amber-300/30"
          />
        </label>

        <label className="flex items-start gap-2 text-xs leading-5 text-slate-400">
          <input
            type="checkbox"
            checked={confirmed}
            onChange={(event) => setConfirmed(event.target.checked)}
            className="mt-1 accent-amber-400"
          />
          I confirm this repository/change and the finding summaries are safe to publish.
        </label>

        {approvable.length > 0 && (
          <div className="rounded-xl border border-[var(--cv-line)] bg-black/10 p-3">
            <div className="mb-2 text-xs font-medium text-slate-400">
              Optional suggestion excerpts
            </div>
            <div className="space-y-2">
              {approvable.map((finding) => (
                <label key={finding.id} className="flex items-start gap-2 text-xs text-slate-500">
                  <input
                    type="checkbox"
                    checked={approved.has(finding.id ?? '')}
                    onChange={() => toggleExcerpt(finding.id ?? '')}
                    className="mt-0.5 accent-amber-400"
                  />
                  <span>{finding.title}</span>
                </label>
              ))}
            </div>
          </div>
        )}

        <div className="flex flex-wrap items-center gap-2">
          <select
            value={format}
            onChange={(event) => setFormat(event.target.value as XrayFormat)}
            className="h-9 rounded-lg border border-[var(--cv-line)] bg-[#0d0f12] px-3 text-xs text-slate-300 outline-none"
          >
            <option value="html">Static HTML</option>
            <option value="markdown">Markdown</option>
            <option value="json">JSON</option>
          </select>
          <Button
            variant="outline"
            size="sm"
            onClick={() => void preview()}
            disabled={busy !== null}
          >
            {busy === 'preview' ? (
              <Loader2 size={14} className="animate-spin" />
            ) : (
              <Eye size={14} />
            )}
            Preview
          </Button>
          <Button size="sm" onClick={() => void save()} disabled={busy !== null}>
            {busy === 'save' ? (
              <Loader2 size={14} className="animate-spin" />
            ) : (
              <Download size={14} />
            )}
            Save export
          </Button>
        </div>

        {error && <div className="text-xs text-red-300">{error}</div>}
        {result && (
          <div className="space-y-3">
            <div
              className={`flex items-start gap-2 rounded-xl border p-3 text-xs ${
                result.eligible
                  ? 'border-emerald-400/15 bg-emerald-400/[0.035] text-emerald-200'
                  : 'border-amber-400/20 bg-amber-400/[0.04] text-amber-100'
              }`}
            >
              {result.eligible ? <CheckCircle2 size={15} /> : <ShieldAlert size={15} />}
              <div>
                <div className="font-medium">
                  {result.eligible ? 'Ready to publish' : 'Export blocked'}
                </div>
                {[...result.missing_requirements, ...result.sanitizer_issues].map((issue) => (
                  <div key={issue} className="mt-1 text-slate-500">
                    {issue}
                  </div>
                ))}
              </div>
            </div>
            <iframe
              title="Agent PR X-Ray preview"
              sandbox=""
              srcDoc={result.html}
              className="h-[420px] w-full rounded-xl border border-[var(--cv-line)] bg-[#08090b]"
            />
          </div>
        )}
      </div>
    </details>
  );
}
