import type { UnlistenFn } from "@tauri-apps/api/event";
import { Bot, ChevronRight, Loader2, Play, Square } from "lucide-react";
import { useCallback, useEffect, useRef, useState } from "react";

import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Card } from "@/components/ui/card";
import { Separator } from "@/components/ui/separator";
import { actionReasoning, describeAction } from "@/lib/agent-action-format";
import {
  type AgentRunInput,
  type AgentRunResult,
  agentRunTask,
  type AgentStep,
  isTauriAvailable,
  listenToAgentSteps,
} from "@/lib/tauri-ipc";

type RunState = "idle" | "running" | "completed" | "gave_up" | "errored";

const PROVIDERS = [
  { value: "claude" as const, label: "Claude CLI", note: "DOM only — no screenshots" },
  { value: "codex" as const, label: "Codex CLI", note: "Vision via attached screenshots" },
];

interface Props {
  defaultUrl?: string;
  defaultGoal?: string;
}

export function LiveAgentRunner({
  defaultUrl = "https://codevetter.com",
  defaultGoal = "Figure out what this product does and find the download link",
}: Props) {
  const [url, setUrl] = useState(defaultUrl);
  const [goal, setGoal] = useState(defaultGoal);
  const [persona, setPersona] = useState("");
  const [projectDir, setProjectDir] = useState("");
  const [provider, setProvider] = useState<"claude" | "codex">("claude");
  const [maxSteps, setMaxSteps] = useState(20);
  const [state, setState] = useState<RunState>("idle");
  const [steps, setSteps] = useState<AgentStep[]>([]);
  const [result, setResult] = useState<AgentRunResult | null>(null);
  const [error, setError] = useState<string | null>(null);

  const unlistenRef = useRef<UnlistenFn | null>(null);

  // Cleanup any active listener on unmount.
  useEffect(() => {
    return () => {
      unlistenRef.current?.();
      unlistenRef.current = null;
    };
  }, []);

  const handleRun = useCallback(async () => {
    if (!isTauriAvailable()) {
      setError("Run inside the CodeVetter desktop app — live agent needs the Tauri backend.");
      return;
    }
    if (!url.trim() || !goal.trim()) {
      setError("URL and goal are required.");
      return;
    }

    setSteps([]);
    setResult(null);
    setError(null);
    setState("running");

    unlistenRef.current?.();
    unlistenRef.current = await listenToAgentSteps((step) => {
      setSteps((prev) => [...prev, step]);
    });

    const input: AgentRunInput = {
      url: url.trim(),
      goal: goal.trim(),
      persona: persona.trim() || null,
      provider,
      model: null,
      max_steps: maxSteps,
      project_dir: projectDir.trim() || null,
    };

    try {
      const run = await agentRunTask(input);
      setResult(run);
      setState(run.completed ? "completed" : run.gave_up ? "gave_up" : "errored");
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
      setState("errored");
    } finally {
      unlistenRef.current?.();
      unlistenRef.current = null;
    }
  }, [url, goal, persona, projectDir, provider, maxSteps]);

  const running = state === "running";

  return (
    <div className="grid gap-5 lg:grid-cols-[1fr_1.4fr]">
      <Card className="border-[#1a1a1a] bg-[#0f1117] p-5 space-y-4">
        <div className="flex items-center gap-3">
          <div className="flex h-9 w-9 items-center justify-center rounded-2xl border border-violet-400/25 bg-violet-400/10 text-violet-200">
            <Bot size={16} />
          </div>
          <div>
            <p className="text-xs font-semibold uppercase tracking-[0.22em] text-violet-200">
              Live agent
            </p>
            <p className="text-xs text-slate-500">
              Real Chrome · spawns claude/codex CLI directly
            </p>
          </div>
        </div>

        <Field label="Target URL">
          <input
            value={url}
            onChange={(e) => setUrl(e.target.value)}
            placeholder="https://example.com or http://localhost:5173"
            disabled={running}
            className="w-full rounded-md border border-[#1a1a1a] bg-[#08090d] px-3 py-2 font-mono text-xs text-slate-100 outline-none focus:border-cyan-400/30"
          />
        </Field>

        <Field label="Goal">
          <textarea
            value={goal}
            onChange={(e) => setGoal(e.target.value)}
            rows={2}
            disabled={running}
            className="w-full resize-none rounded-md border border-[#1a1a1a] bg-[#08090d] px-3 py-2 text-xs text-slate-100 outline-none focus:border-cyan-400/30"
          />
        </Field>

        <Field label="Persona (optional)">
          <textarea
            value={persona}
            onChange={(e) => setPersona(e.target.value)}
            rows={2}
            placeholder="A 30-year-old frontend dev evaluating this for personal use…"
            disabled={running}
            className="w-full resize-none rounded-md border border-[#1a1a1a] bg-[#08090d] px-3 py-2 text-xs text-slate-100 outline-none focus:border-cyan-400/30"
          />
        </Field>

        <Field label="Project directory (optional)">
          <input
            value={projectDir}
            onChange={(e) => setProjectDir(e.target.value)}
            placeholder="/path/to/your/project — auto-runs npm run dev"
            disabled={running}
            className="w-full rounded-md border border-[#1a1a1a] bg-[#08090d] px-3 py-2 font-mono text-xs text-slate-100 outline-none focus:border-cyan-400/30"
          />
        </Field>

        <Field label="Brain">
          <div className="grid grid-cols-2 gap-2">
            {PROVIDERS.map((p) => (
              <button
                key={p.value}
                type="button"
                onClick={() => setProvider(p.value)}
                disabled={running}
                className={`rounded-md border p-2.5 text-left text-xs transition-colors ${
                  provider === p.value
                    ? "border-violet-400/40 bg-violet-400/10 text-slate-100"
                    : "border-[#1a1a1a] bg-[#08090d] text-slate-400 hover:border-violet-400/20"
                }`}
              >
                <div className="font-semibold">{p.label}</div>
                <div className="mt-0.5 text-[10px] text-slate-500">{p.note}</div>
              </button>
            ))}
          </div>
        </Field>

        <Field label={`Step budget: ${maxSteps}`}>
          <input
            type="range"
            min={5}
            max={40}
            step={1}
            value={maxSteps}
            onChange={(e) => setMaxSteps(Number(e.target.value))}
            disabled={running}
            className="w-full"
          />
        </Field>

        <Button
          type="button"
          onClick={handleRun}
          disabled={running}
          className="w-full"
        >
          {running ? (
            <>
              <Loader2 size={14} className="mr-2 animate-spin" />
              Running…
            </>
          ) : (
            <>
              <Play size={14} className="mr-2" />
              Run agent
            </>
          )}
        </Button>

        {error && (
          <div className="rounded-md border border-red-500/30 bg-red-500/10 p-3 text-xs text-red-200">
            {error}
          </div>
        )}
      </Card>

      <Card className="border-[#1a1a1a] bg-[#0f1117] p-6">
        <header className="flex items-start justify-between gap-3">
          <div>
            <h2 className="text-lg font-semibold text-slate-100">
              {result ? result.goal : "Trace"}
            </h2>
            <p className="mt-1 text-xs text-slate-500 font-mono">
              {steps.length} step{steps.length === 1 ? "" : "s"}
              {result && ` · ${(result.duration_ms / 1000).toFixed(1)}s`}
            </p>
          </div>
          {result && (
            <Badge
              variant={
                result.completed ? "secondary" : result.gave_up ? "destructive" : "outline"
              }
            >
              {result.completed ? "DONE" : result.gave_up ? "GAVE UP" : "INCOMPLETE"}
            </Badge>
          )}
          {state === "running" && !result && (
            <Badge variant="outline" className="border-violet-400/30 text-violet-200">
              <Square size={10} className="mr-1.5" />
              Live
            </Badge>
          )}
        </header>

        <Separator className="my-5 bg-[#1a1a1a]" />

        {steps.length === 0 ? (
          <div className="py-10 text-center text-xs text-slate-500">
            {state === "running"
              ? "Waiting for the first step…"
              : "Run the agent to see the step trace stream in."}
          </div>
        ) : (
          <ol className="space-y-2 text-sm text-slate-300">
            {steps.map((step) => (
              <li
                key={step.index}
                className="rounded-md border border-[#1a1a1a] bg-[#08090d] p-3"
              >
                <div className="flex items-start gap-2">
                  <span className="mt-0.5 font-mono text-[10px] text-slate-500">
                    {String(step.index + 1).padStart(2, "0")}
                  </span>
                  <ChevronRight size={12} className="mt-1 shrink-0 text-slate-600" />
                  <div className="min-w-0 flex-1">
                    <div className="flex items-center gap-2">
                      <span className="font-mono text-xs text-violet-200">
                        {describeAction(step)}
                      </span>
                      {step.error && (
                        <Badge variant="destructive" className="text-[9px]">
                          err
                        </Badge>
                      )}
                    </div>
                    {actionReasoning(step) && (
                      <p className="mt-1 text-xs text-slate-400">
                        {actionReasoning(step)}
                      </p>
                    )}
                    <p className="mt-1 text-[10px] font-mono text-slate-500">
                      {step.url} · {step.elapsed_ms}ms
                    </p>
                    {step.error && (
                      <p className="mt-1 text-[10px] text-red-300">{step.error}</p>
                    )}
                    {step.screenshot_data_url && (
                      <StepThumbnail
                        src={step.screenshot_data_url}
                        alt={`screenshot for step ${step.index + 1}`}
                      />
                    )}
                  </div>
                </div>
              </li>
            ))}
          </ol>
        )}
      </Card>
    </div>
  );
}

function StepThumbnail({ src, alt }: { src: string; alt: string }) {
  const [expanded, setExpanded] = useState(false);
  return (
    <button
      type="button"
      onClick={() => setExpanded((v) => !v)}
      className="mt-2 block w-full overflow-hidden rounded-md border border-[#1a1a1a] bg-black/40 transition-colors hover:border-violet-400/30"
    >
      <img
        src={src}
        alt={alt}
        loading="lazy"
        className={`block ${expanded ? "max-h-none" : "max-h-32"} w-full object-cover object-top`}
      />
    </button>
  );
}

function Field({ label, children }: { label: string; children: React.ReactNode }) {
  return (
    <label className="block">
      <span className="text-[10px] font-semibold uppercase tracking-[0.18em] text-slate-500">
        {label}
      </span>
      <div className="mt-1.5">{children}</div>
    </label>
  );
}
