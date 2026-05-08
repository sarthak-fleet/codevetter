import {
  AlertTriangle,
  ArrowLeft,
  CheckCircle,
  CheckSquare2,
  ChevronDown,
  ChevronRight,
  ExternalLink,
  FileCode,
  FolderOpen,
  GitBranch,
  GitCommitHorizontal,
  GitMerge,
  GitPullRequest,
  ListOrdered,
  Loader2,
  Plus,
  RefreshCw,
  Square,
  Trash2,
  Undo2,
  Zap,
} from "lucide-react";
import { type ReactNode,useCallback, useEffect, useMemo, useRef, useState } from "react";
import { Group as PanelGroup, Panel, Separator as PanelResizeHandle } from "react-resizable-panels";

import BlastRadiusPanel from "@/components/blast-radius-panel";
import ScoreBadge from "@/components/score-badge";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Separator } from "@/components/ui/separator";
import type { BlastRadiusReport,CliReviewFinding, CliReviewResult, FileLineData,FixChangedFile, FixFindingsResult , LocalReviewRow, PullRequest } from "@/lib/tauri-ipc";
import {
  analyzeBlastRadius,
  discardFix,
  fixFindings,
  getPreference,
  getReview,
  isTauriAvailable,
  listGitBranches,
  listPullRequests,
  listReviews,
  mergeFix,
  pickDirectory,
  readFileAroundLine,
  revertFiles,
  runCliReview,
  setPreference,
} from "@/lib/tauri-ipc";
import { cn } from "@/lib/utils";

// ─── Helpers ──────────────────────────────────────────────────────────────────

const severityOrder: Record<string, number> = {
  critical: 0,
  high: 1,
  medium: 2,
  warning: 3,
  low: 4,
  suggestion: 5,
  info: 6,
  nitpick: 7,
};

function severityColor(s: string): string {
  switch (s) {
    case "critical":
      return "text-red-400 bg-red-500/10 border-red-500/20";
    case "high":
      return "text-orange-400 bg-orange-500/10 border-orange-500/20";
    case "medium":
      return "text-yellow-400 bg-yellow-500/10 border-yellow-500/20";
    case "warning":
      return "text-yellow-400 bg-yellow-500/10 border-yellow-500/20";
    case "low":
      return "text-blue-400 bg-blue-500/10 border-blue-500/20";
    case "suggestion":
      return "text-cyan-400 bg-cyan-500/10 border-cyan-500/20";
    case "info":
      return "text-slate-400 bg-slate-500/10 border-slate-500/20";
    default:
      return "text-slate-400 bg-slate-500/10 border-slate-500/20";
  }
}

function severityIcon(s: string) {
  switch (s) {
    case "critical":
    case "high":
      return <AlertTriangle size={14} className="text-red-400" />;
    case "medium":
    case "warning":
      return <AlertTriangle size={14} className="text-yellow-400" />;
    default:
      return <CheckCircle size={14} className="text-slate-400" />;
  }
}

function formatRelativeTime(dateStr: string | null): string {
  if (!dateStr) return "";
  const now = Date.now();
  const then = new Date(dateStr).getTime();
  if (isNaN(then)) return "";
  const diffMs = now - then;
  const diffMin = Math.floor(diffMs / 60000);
  if (diffMin < 1) return "just now";
  if (diffMin < 60) return `${diffMin}m ago`;
  const diffHr = Math.floor(diffMin / 60);
  if (diffHr < 24) return `${diffHr}h ago`;
  const diffDay = Math.floor(diffHr / 24);
  return `${diffDay}d ago`;
}

function formatDuration(ms: number): string {
  if (ms < 1000) return `${ms}ms`;
  const secs = Math.round(ms / 1000);
  if (secs < 60) return `${secs}s`;
  const mins = Math.floor(secs / 60);
  const remSecs = secs % 60;
  return `${mins}m ${remSecs}s`;
}

function queueGuidance(findings: CliReviewFinding[]): string {
  if (findings.length === 0) return "Select findings to build a patch queue.";
  const blocking = findings.filter((finding) =>
    ["critical", "high", "medium", "warning"].includes(finding.severity),
  ).length;
  if (blocking > 0) {
    return `Start with ${blocking} blocking finding${blocking !== 1 ? "s" : ""}; keep unrelated cleanup out of this patch.`;
  }
  return "Low-risk queue. Patch together only if the files overlap.";
}

interface DiffFile {
  path: string;
  hunks: string[];
  additions: number;
  deletions: number;
}

function parseDiffIntoFiles(diff: string): DiffFile[] {
  if (!diff.trim()) return [];
  const files: DiffFile[] = [];
  const fileSections = diff.split(/^diff --git /m).filter(Boolean);

  for (const section of fileSections) {
    const lines = section.split("\n");
    // Extract file path from "a/path b/path"
    const headerMatch = lines[0]?.match(/a\/(.*?) b\/(.*)/);
    const path = headerMatch?.[2] ?? lines[0] ?? "unknown";

    let additions = 0;
    let deletions = 0;
    const hunks: string[] = [];
    let currentHunk: string[] = [];

    for (const line of lines.slice(1)) {
      if (line.startsWith("@@")) {
        if (currentHunk.length > 0) hunks.push(currentHunk.join("\n"));
        currentHunk = [line];
      } else if (currentHunk.length > 0 || line.startsWith("+") || line.startsWith("-")) {
        currentHunk.push(line);
        if (line.startsWith("+") && !line.startsWith("+++")) additions++;
        if (line.startsWith("-") && !line.startsWith("---")) deletions++;
      }
    }
    if (currentHunk.length > 0) hunks.push(currentHunk.join("\n"));

    files.push({ path, hunks, additions, deletions });
  }
  return files;
}

function shortenPath(path: string): string {
  const home = "/Users/";
  if (path.startsWith(home)) {
    const afterHome = path.slice(home.length);
    const slashIdx = afterHome.indexOf("/");
    if (slashIdx >= 0) return "~" + afterHome.slice(slashIdx);
  }
  return path;
}

const CODE_KEYWORDS = new Set([
  "as",
  "async",
  "await",
  "break",
  "case",
  "catch",
  "class",
  "const",
  "continue",
  "def",
  "default",
  "do",
  "else",
  "enum",
  "export",
  "extends",
  "false",
  "finally",
  "fn",
  "for",
  "from",
  "function",
  "if",
  "impl",
  "import",
  "in",
  "interface",
  "let",
  "match",
  "mod",
  "mut",
  "new",
  "null",
  "pub",
  "return",
  "self",
  "static",
  "struct",
  "switch",
  "this",
  "throw",
  "true",
  "try",
  "type",
  "undefined",
  "use",
  "var",
  "while",
]);

const CODE_BUILTINS = new Set([
  "Array",
  "Boolean",
  "Date",
  "Error",
  "Map",
  "Number",
  "Object",
  "Promise",
  "Record",
  "Result",
  "Set",
  "String",
  "Vec",
  "console",
  "fs",
  "JSON",
]);

const CODE_TOKEN_RE =
  /(\/\/.*$|#.*$|\/\*.*?\*\/|"(?:\\.|[^"\\])*"|'(?:\\.|[^'\\])*'|`(?:\\.|[^`\\])*`|\b\d+(?:\.\d+)?\b|\b[A-Za-z_$][\w$]*\b)/g;

function getCodeTokenClass(token: string, language: string): string {
  const lowerLanguage = language.toLowerCase();
  if (
    token.startsWith("//") ||
    token.startsWith("/*") ||
    (token.startsWith("#") && !["typescript", "javascript", "tsx", "jsx"].includes(lowerLanguage))
  ) {
    return "text-slate-600 italic";
  }
  if (
    token.startsWith("\"") ||
    token.startsWith("'") ||
    token.startsWith("`")
  ) {
    return "text-emerald-300";
  }
  if (/^\d/.test(token)) return "text-amber-300";
  if (CODE_KEYWORDS.has(token)) return "text-violet-300";
  if (CODE_BUILTINS.has(token)) return "text-cyan-300";
  if (/^[A-Z]/.test(token)) return "text-sky-300";
  return "";
}

function renderCodeLine(text: string, language: string): ReactNode[] | string {
  if (!text) return " ";

  const nodes: ReactNode[] = [];
  let lastIndex = 0;

  for (const match of text.matchAll(CODE_TOKEN_RE)) {
    const token = match[0];
    const index = match.index ?? 0;
    if (index > lastIndex) nodes.push(text.slice(lastIndex, index));

    const className = getCodeTokenClass(token, language);
    nodes.push(
      className ? (
        <span key={`${index}-${token}`} className={className}>
          {token}
        </span>
      ) : (
        token
      ),
    );
    lastIndex = index + token.length;
  }

  if (lastIndex < text.length) nodes.push(text.slice(lastIndex));
  return nodes;
}

// ─── Page ─────────────────────────────────────────────────────────────────────

export default function QuickReview() {
  // Mode: "create" shows the form, "view" shows past review results
  const [mode, setMode] = useState<"create" | "view">("create");

  const [repoPath, setRepoPath] = useState("");
  const [branches, setBranches] = useState<string[]>([]);
  const [currentBranch, setCurrentBranch] = useState("");
  const [pullRequests, setPullRequests] = useState<PullRequest[]>([]);
  const [activeTab, setActiveTab] = useState<"branches" | "prs">("branches");
  const [selectedBranch, setSelectedBranch] = useState("");
  const [baseBranch, setBaseBranch] = useState("main");
  const [projectDesc, setProjectDesc] = useState("");
  const [changeDesc, setChangeDesc] = useState("");
  const [isReviewing, setIsReviewing] = useState(false);
  const [isFixing, setIsFixing] = useState<string | null>(null);
  const [fixProgress, setFixProgress] = useState<string[]>([]);
  const [fixResult, setFixResult] = useState<FixFindingsResult | null>(null);
  const fixLogRef = useRef<HTMLDivElement>(null);
  const [selectedFindings, setSelectedFindings] = useState<Set<number>>(new Set());
  const [result, setResult] = useState<CliReviewResult | null>(null);
  const [error, setError] = useState<string | null>(null);

  // Blast radius analysis (graph-aware PR context)
  const [blastReport, setBlastReport] = useState<BlastRadiusReport | null>(null);
  const [blastLoading, setBlastLoading] = useState(false);
  const [blastError, setBlastError] = useState<string | null>(null);

  // Whether the current view-mode review has a known repo path (for enabling fix)
  const [viewHasRepoPath, setViewHasRepoPath] = useState(true);

  // Past reviews
  const [pastReviews, setPastReviews] = useState<LocalReviewRow[]>([]);
  const [pastReviewsLoading, setPastReviewsLoading] = useState(false);
  const [showHistory, setShowHistory] = useState(true);

  // Code viewer state (view mode)
  const [selectedFindingIdx, setSelectedFindingIdx] = useState<number | null>(null);
  const [codeLines, setCodeLines] = useState<FileLineData[]>([]);
  const [codeFilePath, setCodeFilePath] = useState("");
  const [codeLanguage, setCodeLanguage] = useState("");

  // Diff range derived from selection
  const [diffRange, setDiffRange] = useState("");

  // ─── Load saved folder + branches on mount ───────────────────────────────

  const loadFolderData = useCallback(async (dir: string) => {
    setRepoPath(dir);
    const [branchResult, prs] = await Promise.allSettled([
      listGitBranches(dir),
      listPullRequests(dir),
    ]);
    if (branchResult.status === "fulfilled") {
      const { branches: brList, current } = branchResult.value;
      setBranches(brList);
      setCurrentBranch(current ?? "");
      if (brList.includes("main")) setBaseBranch("main");
      else if (brList.includes("master")) setBaseBranch("master");
      else if (brList.length > 0) setBaseBranch(brList[0]);
    } else {
      setBranches([]);
      setCurrentBranch("");
    }
    if (prs.status === "fulfilled") {
      setPullRequests(prs.value);
    } else {
      setPullRequests([]);
    }
    // Load persisted project description
    try {
      const saved = await getPreference(`quick_review_desc_${btoa(dir)}`);
      if (saved != null) setProjectDesc(saved);
      else setProjectDesc("");
    } catch {
      setProjectDesc("");
    }
  }, []);

  useEffect(() => {
    if (!isTauriAvailable()) return;
    void getPreference("quick_review_last_folder")
      .then((dir) => dir ? loadFolderData(dir) : undefined)
      .catch(() => {});
  }, [loadFolderData]);

  // ─── Load past reviews ───────────────────────────────────────────────────

  useEffect(() => {
    if (!isTauriAvailable()) {
      setPastReviewsLoading(false);
      return;
    }
    setPastReviewsLoading(true);
    void listReviews(20, 0)
      .then((reviews) => {
        return setPastReviews(reviews);
      })
      .catch((e) => console.error("[Review] failed to load past reviews:", e))
      .finally(() => setPastReviewsLoading(false));
  }, [result]); // reload after new review completes

  const handleLoadPastReview = useCallback(async (id: string) => {
    try {
      const data = await getReview(id);
      const review = data.review;
      const findings = (data.findings ?? []).map((f) => ({
        severity: f.severity ?? "info",
        title: f.title ?? "",
        summary: f.summary ?? "",
        suggestion: f.suggestion ?? undefined,
        filePath: f.file_path ?? undefined,
        line: f.line ?? undefined,
        confidence: f.confidence ?? undefined,
      }));
      setResult({
        review_id: review.id,
        score: review.score_composite ?? 0,
        findings,
        summary: review.summary_markdown ?? "",
        agent: review.agent_used ?? "claude",
        duration_ms: 0,
        diff_range: review.source_label ?? "",
        findings_count: findings.length,
      });
      setViewHasRepoPath(!!review.repo_path);
      if (review.repo_path) setRepoPath(review.repo_path);
      // Past reviews don't have a stored blast report — clear the panel.
      setBlastReport(null);
      setBlastError(null);
      setMode("view");
    } catch (e) {
      setError(String(e));
    }
  }, []);

  // ─── Folder picker ───────────────────────────────────────────────────────

  const handlePickFolder = useCallback(async () => {
    if (!isTauriAvailable()) {
      setError("Not running in Tauri");
      return;
    }
    try {
      const dir = await pickDirectory("Select a git repository");
      if (!dir) return;

      setResult(null);
      setError(null);
      setSelectedBranch("");
      setDiffRange("");
      setMode("create");

      await loadFolderData(dir);

      // Persist last used folder
      setPreference("quick_review_last_folder", dir).catch(() => {});
    } catch (e) {
      const msg = String(e);
      if (msg.includes("TAURI_NOT_AVAILABLE")) {
        setError("Not running in Tauri");
      } else {
        setError(msg);
      }
    }
  }, [loadFolderData]);

  // ─── Branch/PR selection ─────────────────────────────────────────────────

  const handleSelectBranch = useCallback(
    (branch: string) => {
      setSelectedBranch(branch);
      setDiffRange(`${baseBranch}...${branch}`);
      setResult(null);
      setError(null);
    },
    [baseBranch],
  );

  const handleSelectPR = useCallback((pr: PullRequest) => {
    setSelectedBranch(pr.headRefName);
    setDiffRange(`${pr.baseRefName}...${pr.headRefName}`);
    setResult(null);
    setError(null);
  }, []);

  // ─── Persist project description on blur ─────────────────────────────────

  const handleProjectDescBlur = useCallback(() => {
    if (!repoPath || !isTauriAvailable()) return;
    const prefKey = `quick_review_desc_${btoa(repoPath)}`;
    setPreference(prefKey, projectDesc).catch(() => {});
  }, [repoPath, projectDesc]);

  // ─── Run review ──────────────────────────────────────────────────────────

  const handleReview = useCallback(async () => {
    if (!repoPath || !diffRange) return;

    setIsReviewing(true);
    setError(null);
    setResult(null);
    setBlastReport(null);
    setBlastError(null);
    setBlastLoading(true);

    // Kick off blast-radius analysis in parallel with the LLM review.
    // It's deterministic and fast (git grep), so it usually returns first.
    const blastPromise = analyzeBlastRadius(repoPath, diffRange)
      .then((r) => {
        setBlastReport(r);
        return r;
      })
      .catch((e) => {
        setBlastError(String(e));
        return null;
      })
      .finally(() => setBlastLoading(false));

    try {
      const res = await runCliReview(
        repoPath,
        diffRange,
        projectDesc,
        changeDesc,
        "claude",
      );
      setResult(res);
      setMode("view");
      setViewHasRepoPath(true);
      setSelectedFindings(new Set());
      await blastPromise;
    } catch (e) {
      const msg = String(e);
      if (msg.includes("TAURI_NOT_AVAILABLE")) {
        setError("Not running in Tauri");
      } else {
        setError(msg);
      }
    } finally {
      setIsReviewing(false);
    }
  }, [repoPath, diffRange, projectDesc, changeDesc]);

  // ─── Back to create mode ─────────────────────────────────────────────────

  const handleNewReview = useCallback(() => {
    setMode("create");
    setResult(null);
    setError(null);
    setBlastReport(null);
    setBlastError(null);
    setSelectedFindingIdx(null);
    setCodeLines([]);
    setCodeFilePath("");
    setCodeLanguage("");
    // Re-fetch branches for the current folder
    if (repoPath) {
      loadFolderData(repoPath);
    }
  }, [repoPath, loadFolderData]);

  // ─── Sorted findings ────────────────────────────────────────────────────

  const sortedFindings = useMemo(() => (
    result
      ? [...result.findings].sort(
        (a, b) =>
          (severityOrder[a.severity] ?? 99) -
          (severityOrder[b.severity] ?? 99),
      )
      : []
  ), [result]);

  const patchQueue = useMemo(
    () => sortedFindings.filter((_, idx) => selectedFindings.has(idx)),
    [selectedFindings, sortedFindings],
  );

  const patchQueueSeverityCounts = useMemo(
    () =>
      patchQueue.reduce<Record<string, number>>((acc, finding) => {
        acc[finding.severity] = (acc[finding.severity] ?? 0) + 1;
        return acc;
      }, {}),
    [patchQueue],
  );

  // ─── Fix handlers ───────────────────────────────────────────────────────

  const toggleFinding = useCallback((idx: number) => {
    setSelectedFindings((prev) => {
      const next = new Set(prev);
      if (next.has(idx)) next.delete(idx);
      else next.add(idx);
      return next;
    });
  }, []);

  const toggleSelectAll = useCallback(() => {
    if (!result) return;
    setSelectedFindings((prev) =>
      prev.size === result.findings.length
        ? new Set()
        : new Set(result.findings.map((_, i) => i)),
    );
  }, [result]);

  const handleFixSelected = useCallback(async () => {
    if (!repoPath || !result || selectedFindings.size === 0) return;
    setIsFixing("selected");
    setFixResult(null);
    setFixProgress([]);
    setError(null);

    // Listen for streaming progress events
    let unlisten: (() => void) | undefined;
    try {
      const { listen } = await import("@tauri-apps/api/event");
      unlisten = await listen<string>("fix-progress", (event) => {
        setFixProgress((prev) => {
          const next = [...prev, event.payload];
          // Keep last 50 lines
          return next.length > 50 ? next.slice(-50) : next;
        });
        // Auto-scroll
        if (fixLogRef.current) {
          fixLogRef.current.scrollTop = fixLogRef.current.scrollHeight;
        }
      });
    } catch {
      // Event listening not available, continue without streaming
    }

    try {
      const toFix = sortedFindings.filter((_, i) => selectedFindings.has(i));
      const res = await fixFindings(repoPath, toFix, result.agent);
      setFixResult(res);
    } catch (e) {
      setError(`Fix failed: ${String(e)}`);
    } finally {
      setIsFixing(null);
      unlisten?.();
    }
  }, [repoPath, result, selectedFindings, sortedFindings]);

  const handleRevertFile = useCallback(async (filePath: string) => {
    if (!fixResult?.worktree_path) return;
    try {
      await revertFiles(fixResult.worktree_path, [filePath]);
      // Re-fetch diff to update the view
      const remaining = fixResult.changed_files.filter(f => f.path !== filePath);
      setFixResult({ ...fixResult, changed_files: remaining });
    } catch (e) {
      setError(`Revert failed: ${String(e)}`);
    }
  }, [fixResult]);

  const handleMergeFix = useCallback(async () => {
    if (!repoPath || !fixResult?.worktree_branch) return;
    try {
      await mergeFix(repoPath, fixResult.worktree_branch, fixResult.worktree_path);
      setFixResult(null);
    } catch (e) {
      setError(`Merge failed: ${String(e)}`);
    }
  }, [repoPath, fixResult]);

  const handleDiscardFix = useCallback(async () => {
    if (!repoPath || !fixResult?.worktree_branch) return;
    try {
      await discardFix(repoPath, fixResult.worktree_branch, fixResult.worktree_path);
      setFixResult(null);
    } catch (e) {
      setError(`Discard failed: ${String(e)}`);
    }
  }, [repoPath, fixResult]);

  const handleCommitFixes = useCallback(async () => {
    if (!repoPath || !fixResult) return;
    try {
      const { safeInvoke } = await import("@/lib/tauri-ipc");
      // Stage changed files and commit
      const files = fixResult.changed_files.map(f => f.path);
      for (const file of files) {
        await safeInvoke("run_git_command", { repoPath, args: ["add", file] }).catch(() => {});
      }
      const msg = `fix: resolve ${fixResult.findings_fixed} code review finding${fixResult.findings_fixed !== 1 ? "s" : ""}`;
      await safeInvoke("run_git_command", { repoPath, args: ["commit", "-m", msg] }).catch(() => {});
      setFixResult(null);
      setError(null);
    } catch (e) {
      // Fallback: just tell the user to commit manually
      setError(`Auto-commit not available. Run: cd ${repoPath} && git add -A && git commit -m "fix: resolve review findings"`);
    }
  }, [repoPath, fixResult]);

  const handleOpenInIDE = useCallback(async () => {
    if (!repoPath || !isTauriAvailable()) return;
    try {
      // Try Cursor first, fall back to VS Code
      const { invoke } = await import("@tauri-apps/api/core");
      try {
        await invoke("open_in_app", { appName: "cursor", path: repoPath });
      } catch {
        await invoke("open_in_app", { appName: "vscode", path: repoPath });
      }
    } catch (e) {
      setError(`Could not open IDE: ${String(e)}`);
    }
  }, [repoPath]);

  // Track which diff files are expanded
  const [expandedFiles, setExpandedFiles] = useState<Set<string>>(new Set());
  const toggleFileExpanded = useCallback((path: string) => {
    setExpandedFiles(prev => {
      const next = new Set(prev);
      if (next.has(path)) next.delete(path);
      else next.add(path);
      return next;
    });
  }, []);

  // Parse diff into files when fixResult changes
  const diffFiles = fixResult?.diff ? parseDiffIntoFiles(fixResult.diff) : [];

  const handleReReview = useCallback(() => {
    setFixResult(null);
    setSelectedFindings(new Set());
    setSelectedFindingIdx(null);
    setCodeLines([]);
    setCodeFilePath("");
    setCodeLanguage("");
    handleReview();
  }, [handleReview]);

  // ─── Finding click → load code ──────────────────────────────────────────

  const handleFindingClick = useCallback(
    async (idx: number) => {
      setSelectedFindingIdx(idx);
      const finding = sortedFindings[idx];
      if (!finding?.filePath || finding.line == null) {
        setCodeLines([]);
        setCodeFilePath(finding?.filePath ?? "");
        setCodeLanguage("");
        return;
      }
      try {
        const res = await readFileAroundLine(
          repoPath + "/" + finding.filePath,
          finding.line,
          15,
          15,
        );
        setCodeLines(res.lines);
        setCodeFilePath(res.file_path);
        setCodeLanguage(res.language);
      } catch (e) {
        console.error("[Review] failed to load code:", e);
        setCodeLines([]);
        setCodeFilePath(finding.filePath);
        setCodeLanguage("");
      }
    },
    [sortedFindings, repoPath],
  );

  useEffect(() => {
    if (
      mode !== "view" ||
      fixResult ||
      selectedFindingIdx !== null ||
      sortedFindings.length === 0
    ) {
      return;
    }

    void handleFindingClick(0);
  }, [fixResult, handleFindingClick, mode, selectedFindingIdx, sortedFindings.length]);

  // ─── Jump from blast-radius caller → code viewer ─────────────────────────

  const handleJumpToCaller = useCallback(
    async (file: string, line: number) => {
      setSelectedFindingIdx(null);
      if (!repoPath) return;
      try {
        const res = await readFileAroundLine(
          repoPath + "/" + file,
          line,
          15,
          15,
        );
        setCodeLines(res.lines);
        setCodeFilePath(res.file_path);
        setCodeLanguage(res.language);
      } catch (e) {
        console.error("[Review] failed to load caller code:", e);
        setCodeLines([]);
        setCodeFilePath(file);
        setCodeLanguage("");
      }
    },
    [repoPath],
  );

  // ─── Render ─────────────────────────────────────────────────────────────

  // ─── View mode layout ────────────────────────────────────────────────────

  if (mode === "view" && result) {
    const activeFinding =
      selectedFindingIdx !== null ? sortedFindings[selectedFindingIdx] : null;
    const activeCodePath = codeFilePath || activeFinding?.filePath || "";

    return (
      <div className="flex h-full flex-col px-4 pb-4 pt-20">
        {/* Result header */}
        <div className="cv-frame mb-3 flex h-12 shrink-0 items-center gap-3 overflow-hidden px-3">
          <Button
            variant="ghost"
            size="sm"
            className="h-8 gap-1 text-slate-500 hover:bg-white/[0.04] hover:text-slate-100"
            onClick={handleNewReview}
          >
            <ArrowLeft size={14} />
            Back
          </Button>
          <div className="h-6 w-px bg-[var(--cv-line)]" />
          <div className="min-w-0 flex-1">
            <div className="cv-label truncate text-slate-300">
              review result · {result.agent}
            </div>
            <div className="mt-0.5 truncate font-mono text-[10px] uppercase tracking-[0.16em] text-slate-600">
              {result.diff_range || diffRange || "local diff"}
            </div>
          </div>
          <ScoreBadge score={Math.round(result.score)} size="sm" />
          <div className="cv-label hidden sm:block">
            {result.findings_count ?? sortedFindings.length} findings
          </div>
        </div>

        {/* Error banner */}
        {error && (
          <div className="shrink-0 bg-red-500/10 px-4 py-2 text-xs text-red-400">
            {error}
          </div>
        )}

        {/* Editor + verdict body */}
        <PanelGroup orientation="horizontal" className="min-h-0 flex-1 cv-frame overflow-hidden bg-[#07080a]">
          <Panel defaultSize={72} minSize={45}>
          <div className="cv-scan flex h-full flex-col bg-[#050505]">
            {/* Fix results view */}
            {fixResult ? (
              <div className="flex h-full flex-col">
                {/* File-grouped diff */}
                <div className="flex-1 overflow-y-auto">
                  {diffFiles.length > 0 ? (
                    <div className="divide-y divide-[#1a1a1a]">
                      {diffFiles.map((file) => (
                        <div key={file.path}>
                          {/* File header */}
                          <div
                            className="sticky top-0 z-10 flex cursor-pointer items-center gap-2 border-b border-[var(--cv-line)] bg-[#07080a] px-4 py-2 hover:bg-white/[0.035]"
                            onClick={() => toggleFileExpanded(file.path)}
                          >
                            {expandedFiles.has(file.path) || expandedFiles.size === 0 ? (
                              <ChevronDown size={14} className="text-slate-500" />
                            ) : (
                              <ChevronRight size={14} className="text-slate-500" />
                            )}
                            <FileCode size={14} className="text-slate-500" />
                            <span className="flex-1 font-mono text-[12px] text-slate-300">{file.path}</span>
                            <span className="text-[11px] text-emerald-400">+{file.additions}</span>
                            <span className="text-[11px] text-red-400">-{file.deletions}</span>
                            <Button
                              size="sm"
                              variant="ghost"
                              onClick={(e) => { e.stopPropagation(); handleRevertFile(file.path); }}
                              className="h-6 gap-1 px-2 text-[10px] text-slate-600 hover:text-red-400 hover:bg-red-500/10"
                            >
                              <Undo2 size={10} />
                              Revert
                            </Button>
                          </div>
                          {/* Hunks (expanded by default, collapsible) */}
                          {(expandedFiles.has(file.path) || expandedFiles.size === 0) && (
                            <div>
                              {file.hunks.map((hunk, hi) => (
                                <div key={hi}>
                                  {hunk.split("\n").map((line, li) => (
                                    <div
                                      key={`${hi}-${li}`}
                                      className={cn(
                                        "font-mono text-[12px] leading-[22px] pl-4 pr-4",
                                        line.startsWith("+") && !line.startsWith("+++") && "bg-emerald-500/[0.07] text-emerald-400 border-l-2 border-emerald-500/30",
                                        line.startsWith("-") && !line.startsWith("---") && "bg-red-500/[0.07] text-red-400 border-l-2 border-red-500/30",
                                        line.startsWith("@@") && "bg-[#0a0a0a] text-cyan-500/50 text-[11px] py-1 border-l-2 border-cyan-500/20",
                                        !line.startsWith("+") && !line.startsWith("-") && !line.startsWith("@@") && "text-slate-500 border-l-2 border-transparent",
                                      )}
                                    >
                                      {line}
                                    </div>
                                  ))}
                                </div>
                              ))}
                            </div>
                          )}
                        </div>
                      ))}
                    </div>
                  ) : (
                    <div className="p-4">
                      <div className="mb-2 text-xs font-medium text-yellow-400">No file changes detected — agent output:</div>
                      <pre className="whitespace-pre-wrap font-mono text-[12px] leading-5 text-slate-400">
                        {fixResult.agent_output || "No output captured"}
                      </pre>
                    </div>
                  )}
                </div>
                {/* Bottom action bar */}
                <div className="shrink-0 border-t border-[var(--cv-line)] bg-[#07080a] px-3 py-2">
                  <div className="flex items-center gap-2">
                    <CheckCircle size={14} className="text-emerald-400" />
                    <span className="text-[11px] text-slate-400">
                      {diffFiles.length} file{diffFiles.length !== 1 ? "s" : ""} changed in {formatDuration(fixResult.duration_ms)}
                    </span>
                    <div className="ml-auto flex items-center gap-1">
                      <Button
                        size="sm"
                        variant="ghost"
                        onClick={handleReReview}
                        disabled={isReviewing || !repoPath || !diffRange}
                        className="gap-1 text-[11px] text-[var(--cv-accent)] hover:bg-cyan-500/10 hover:text-cyan-200 disabled:opacity-50"
                      >
                        {isReviewing ? <Loader2 size={12} className="animate-spin" /> : <RefreshCw size={12} />}
                        Re-review
                      </Button>
                      <Button
                        size="sm"
                        variant="ghost"
                        onClick={handleMergeFix}
                        className="gap-1 text-[11px] text-emerald-400 hover:text-emerald-300 hover:bg-emerald-500/10"
                      >
                        <GitMerge size={12} />
                        Merge
                      </Button>
                      <Button
                        size="sm"
                        variant="ghost"
                        onClick={handleDiscardFix}
                        className="gap-1 text-[11px] text-red-400 hover:text-red-300 hover:bg-red-500/10"
                      >
                        <Trash2 size={12} />
                        Discard
                      </Button>
                      <Button
                        size="sm"
                        variant="ghost"
                        onClick={handleOpenInIDE}
                        className="gap-1 text-[11px] text-slate-400 hover:text-slate-200"
                      >
                        <ExternalLink size={12} />
                        Open in IDE
                      </Button>
                    </div>
                  </div>
                </div>
              </div>
            ) : isFixing ? (
              <div className="flex h-full flex-col bg-[#050505]">
                <div className="flex shrink-0 items-center gap-2 border-b border-[var(--cv-line)] px-4 py-2">
                  <Loader2 size={14} className="animate-spin text-[var(--cv-accent)]" />
                  <span className="text-xs font-medium text-[var(--cv-accent)]">Fixing with Claude...</span>
                </div>
                <div ref={fixLogRef} className="flex-1 overflow-y-auto p-4">
                  {fixProgress.length > 0 ? (
                    fixProgress.map((line, i) => (
                      <div key={i} className="font-mono text-[11px] leading-5 text-slate-500">
                        {line}
                      </div>
                    ))
                  ) : (
                    <div className="flex items-center gap-2 text-slate-600 text-sm">
                      <Loader2 size={16} className="animate-spin" />
                      Waiting for output...
                    </div>
                  )}
                </div>
              </div>
            ) : selectedFindingIdx !== null && activeFinding ? (
              <>
                {/* File path header + finding context */}
                <div className="cv-terminal-bar h-11 shrink-0 px-4">
                  <span className="cv-dot" />
                  <span className="cv-dot" />
                  <span className="cv-dot" />
                  <span className="cv-label mx-auto">
                    {activeCodePath || "source unavailable"}
                  </span>
                  {codeLanguage && <span className="cv-label">{codeLanguage}</span>}
                </div>
                <div className="shrink-0 border-b border-[var(--cv-line)] px-6 py-4">
                  <div className="flex items-start justify-between gap-4">
                    <div className="min-w-0">
                      <div className="cv-label mb-2">selected finding</div>
                      <h2 className="truncate text-sm font-semibold text-slate-100">
                        {activeFinding.title}
                      </h2>
                    </div>
                    <Badge
                      variant="outline"
                      className={cn(
                        "shrink-0 rounded-full px-2.5 py-1 font-mono text-[10px] font-semibold uppercase",
                        severityColor(activeFinding.severity),
                      )}
                    >
                      {severityIcon(activeFinding.severity)}
                      <span className="ml-1">{activeFinding.severity}</span>
                    </Badge>
                  </div>
                </div>
                {/* Code lines */}
                <div className="flex-1 overflow-y-auto bg-[#030405] px-6 py-5 font-mono text-[13px] leading-7">
                  {codeLines.length > 0 ? (
                    <div className="grid grid-cols-[42px_1fr] gap-x-4">
                      {codeLines.map((cl) => (
                        <div key={cl.line} className="contents">
                          <span
                            className={cn(
                              "select-none text-right tabular-nums",
                              cl.highlight ? "text-[var(--cv-danger)]/80" : "text-slate-700",
                            )}
                          >
                            {cl.line}
                          </span>
                          <pre
                            className={cn(
                              "min-w-0 whitespace-pre border-l-2 px-3",
                              cl.highlight
                                ? "border-[var(--cv-danger)] bg-red-500/10 text-slate-100"
                                : "border-transparent text-slate-300 hover:bg-white/[0.025]",
                            )}
                          >
                            {renderCodeLine(cl.text, codeLanguage)}
                          </pre>
                        </div>
                      ))}
                    </div>
                  ) : (
                    <div className="grid grid-cols-[42px_1fr] gap-x-4">
                      <span className="text-right text-slate-700">
                        {activeFinding.line ?? 1}
                      </span>
                      <span className="-mx-3 border-l-2 border-[var(--cv-danger)] bg-red-500/10 px-3 text-slate-500">
                        No source snapshot is available for this finding.
                      </span>
                    </div>
                  )}
                </div>
              </>
            ) : (
              <div className="flex h-full flex-col">
                <div className="cv-terminal-bar h-11 px-4">
                  <span className="cv-dot" />
                  <span className="cv-dot" />
                  <span className="cv-dot" />
                  <span className="cv-label mx-auto">review result · select a comment</span>
                  <span className="cv-label">⌘ K</span>
                </div>
                <div className="flex flex-1 flex-col items-center justify-center gap-2 bg-[#030405] text-slate-600">
                  <Zap size={24} className="text-slate-700" />
                  <span className="text-sm">Select a review comment to inspect source</span>
                </div>
              </div>
            )}
          </div>
          </Panel>

          <PanelResizeHandle className="w-1.5 cursor-col-resize bg-[var(--cv-line)] transition-colors hover:bg-cyan-500/30" />

          <Panel defaultSize={28} minSize={22}>
          <aside className="flex h-full flex-col bg-white/[0.015]">
            <div className="shrink-0 border-b border-[var(--cv-line)] p-6">
              <div className="cv-label mb-5">Verdict</div>
              {activeFinding ? (
                <>
                  <Badge
                    variant="outline"
                    className={cn(
                      "rounded-full px-2.5 py-1 font-mono text-[10px] font-semibold uppercase",
                      severityColor(activeFinding.severity),
                    )}
                  >
                    {severityIcon(activeFinding.severity)}
                    <span className="ml-1">{activeFinding.severity}</span>
                  </Badge>
                  <h2 className="mt-5 text-lg font-semibold leading-6 text-white">
                    {activeFinding.title}
                  </h2>
                  <p className="mt-3 text-sm leading-6 text-slate-400">
                    {activeFinding.summary}
                  </p>
                  {activeFinding.filePath && (
                    <div className="mt-4 font-mono text-[11px] uppercase tracking-[0.12em] text-slate-600">
                      {activeFinding.filePath}
                      {activeFinding.line != null && `:${activeFinding.line}`}
                    </div>
                  )}
                  {activeFinding.suggestion && (
                    <div className="mt-6 border-t border-[var(--cv-line)] pt-5">
                      <div className="cv-label mb-3">Suggested action</div>
                      <p className="font-mono text-[12px] leading-6 text-slate-300">
                        {activeFinding.suggestion}
                      </p>
                    </div>
                  )}
                </>
              ) : (
                <div className="flex items-center gap-2 text-sm text-[var(--cv-accent)]">
                  <CheckCircle size={18} />
                  No findings.
                </div>
              )}
            </div>

            {(blastReport || blastLoading || blastError) && (
              <div className="shrink-0 border-b border-[var(--cv-line)]">
                <BlastRadiusPanel
                  report={blastReport}
                  loading={blastLoading}
                  error={blastError}
                  onJump={handleJumpToCaller}
                />
              </div>
            )}

            <div className="min-h-0 flex-1 overflow-y-auto p-4">
              <div className="mb-3 flex items-center justify-between">
                <span className="cv-label">review comments</span>
                <span className="cv-label">{sortedFindings.length} total</span>
              </div>
              <div className="mb-3 rounded-xl border border-[var(--cv-line)] bg-[#050505] p-3">
                <div className="mb-2 flex items-center justify-between gap-2">
                  <div className="flex items-center gap-2">
                    <ListOrdered size={14} className="text-[var(--cv-accent)]" />
                    <span className="cv-label text-slate-300">patch queue</span>
                  </div>
                  <span className="font-mono text-[11px] text-slate-500">
                    {patchQueue.length} selected
                  </span>
                </div>
                <p className="text-[11px] leading-5 text-slate-500">
                  {queueGuidance(patchQueue)}
                </p>
                {patchQueue.length > 0 && (
                  <>
                    <div className="mt-3 flex flex-wrap gap-1.5">
                      {Object.entries(patchQueueSeverityCounts).map(([severity, count]) => (
                        <Badge
                          key={severity}
                          variant="outline"
                          className={cn(
                            "rounded-full px-2 py-0.5 font-mono text-[9px] uppercase",
                            severityColor(severity),
                          )}
                        >
                          {severity} · {count}
                        </Badge>
                      ))}
                    </div>
                    <div className="mt-3 space-y-1.5">
                      {patchQueue.slice(0, 4).map((finding, queueIdx) => (
                        <button
                          key={`${finding.title}-${queueIdx}`}
                          type="button"
                          onClick={() => {
                            const sortedIdx = sortedFindings.indexOf(finding);
                            if (sortedIdx >= 0) handleFindingClick(sortedIdx);
                          }}
                          className="flex w-full items-center gap-2 rounded-lg border border-[var(--cv-line)] bg-[#07080a] px-2 py-2 text-left hover:border-[var(--cv-line-strong)]"
                        >
                          <span className="font-mono text-[10px] text-slate-600">
                            {queueIdx + 1}
                          </span>
                          <span className="min-w-0 flex-1 truncate text-[11px] text-slate-300">
                            {finding.filePath || finding.title}
                          </span>
                          <span className="shrink-0 text-[10px] text-slate-600">
                            {finding.line != null ? `:${finding.line}` : finding.severity}
                          </span>
                        </button>
                      ))}
                      {patchQueue.length > 4 && (
                        <div className="px-2 text-[10px] text-slate-600">
                          +{patchQueue.length - 4} more queued
                        </div>
                      )}
                    </div>
                  </>
                )}
              </div>
              <div className="space-y-2">
                {sortedFindings.map((finding, idx) => (
                  <div
                    key={idx}
                    role="button"
                    tabIndex={0}
                    onClick={() => handleFindingClick(idx)}
                    onKeyDown={(event) => {
                      if (event.key === "Enter" || event.key === " ") {
                        event.preventDefault();
                        handleFindingClick(idx);
                      }
                    }}
                    className={cn(
                      "w-full cursor-pointer border px-3 py-3 text-left transition-colors",
                      selectedFindingIdx === idx
                        ? "border-[rgba(125,211,252,0.42)] bg-cyan-500/10"
                        : "border-[var(--cv-line)] bg-[#07080a] hover:border-[var(--cv-line-strong)] hover:bg-white/[0.035]",
                      selectedFindings.has(idx) && "shadow-[inset_3px_0_0_rgba(125,211,252,0.82)]",
                    )}
                  >
                    <div className="flex items-center gap-2">
                      <button
                        type="button"
                        aria-label={selectedFindings.has(idx) ? "Remove from fix selection" : "Select for fix"}
                        aria-pressed={selectedFindings.has(idx)}
                        onClick={(event) => {
                          event.stopPropagation();
                          toggleFinding(idx);
                        }}
                        className="shrink-0 text-slate-500 transition-colors hover:text-[var(--cv-accent)]"
                      >
                        {selectedFindings.has(idx) ? (
                          <CheckSquare2 size={15} className="text-[var(--cv-accent)]" />
                        ) : (
                          <Square size={15} />
                        )}
                      </button>
                      <Badge
                        variant="outline"
                        className={cn(
                          "shrink-0 rounded-full px-2 py-0.5 font-mono text-[9px] font-semibold uppercase",
                          severityColor(finding.severity),
                        )}
                      >
                        {finding.severity}
                      </Badge>
                      <span className="truncate text-xs font-medium text-slate-100">
                        {finding.title}
                      </span>
                    </div>
                    <p className="mt-2 line-clamp-2 text-[11px] leading-5 text-slate-500">
                      {finding.summary}
                    </p>
                  </div>
                ))}
              </div>
            </div>

            <div className="shrink-0 border-t border-[var(--cv-line)] bg-[#07080a] p-3">
              <div className="flex items-center gap-2">
                <button
                  onClick={toggleSelectAll}
                  className="flex items-center gap-1 text-[11px] text-slate-500 hover:text-slate-300"
                >
                  {selectedFindings.size === sortedFindings.length && sortedFindings.length > 0 ? (
                    <CheckSquare2 size={14} className="text-[var(--cv-accent)]" />
                  ) : (
                    <Square size={14} />
                  )}
                  All
                </button>
                <div className="relative ml-auto group">
                  <Button
                    size="sm"
                    onClick={handleFixSelected}
                    disabled={isFixing !== null || selectedFindings.size === 0 || !viewHasRepoPath}
                    className="gap-1.5 bg-white text-xs text-black hover:bg-slate-200 disabled:opacity-50"
                  >
                    {isFixing === "selected" ? (
                      <Loader2 size={14} className="animate-spin" />
                    ) : (
                      <Zap size={14} />
                    )}
                    {isFixing === "selected"
                      ? "Fixing..."
                      : `Fix${selectedFindings.size > 0 ? ` (${selectedFindings.size})` : ""}`}
                  </Button>
                  {!viewHasRepoPath && (
                    <div className="absolute bottom-full right-0 mb-1.5 hidden whitespace-nowrap border border-[#2a2a2a] bg-[#1a1a1a] px-2 py-1 text-[10px] text-slate-400 shadow-lg group-hover:block">
                      No repo path — can't apply fixes
                    </div>
                  )}
                </div>
              </div>
            </div>
          </aside>
          </Panel>
        </PanelGroup>
      </div>
    );
  }

  // ─── Create mode layout ─────────────────────────────────────────────────

  return (
    <div className="flex h-full gap-4 px-4 pb-4 pt-20">
      {/* Left panel */}
      <div className="cv-frame flex w-[420px] shrink-0 flex-col overflow-hidden">
        {/* Header */}
        <div className="cv-terminal-bar h-11 shrink-0 px-4">
          <div className="flex items-center gap-2">
            <Zap size={16} className="text-[var(--cv-accent)]" />
            <h1 className="cv-label text-slate-200">
              Review
            </h1>
          </div>
        </div>

        {/* Content */}
        <div className="flex-1 overflow-y-auto px-4 py-3 space-y-4">
          {/* Folder picker */}
          <Button
            variant="outline"
            className="w-full justify-start gap-2 border-[var(--cv-line)] bg-[#07080a] text-slate-300 hover:bg-white/[0.04] hover:text-slate-100"
            onClick={handlePickFolder}
          >
            <FolderOpen size={16} />
            {repoPath ? shortenPath(repoPath) : "Select repository..."}
          </Button>

          {!repoPath && error && (
            <div className="border border-red-500/25 bg-red-500/10 px-3 py-2 text-xs text-red-300">
              {error}
            </div>
          )}

          {/* Branch/PR tabs + list */}
          {repoPath && (
            <>
              {/* Tabs */}
              <div className="grid grid-cols-2 gap-1 border border-[var(--cv-line)] bg-[#07080a] p-1">
                <button
                  onClick={() => setActiveTab("branches")}
                  className={cn(
                    "flex items-center justify-center gap-1.5 px-3 py-2 text-xs font-medium transition-colors",
                    activeTab === "branches"
                      ? "bg-cyan-500/10 text-[var(--cv-accent)] shadow-[inset_0_-1px_0_rgba(125,211,252,0.45)]"
                      : "text-slate-500 hover:text-slate-300",
                  )}
                >
                  <GitBranch size={14} />
                  Branches
                </button>
                <button
                  onClick={() => setActiveTab("prs")}
                  className={cn(
                    "flex items-center justify-center gap-1.5 px-3 py-2 text-xs font-medium transition-colors",
                    activeTab === "prs"
                      ? "bg-cyan-500/10 text-[var(--cv-accent)] shadow-[inset_0_-1px_0_rgba(125,211,252,0.45)]"
                      : "text-slate-500 hover:text-slate-300",
                  )}
                >
                  <GitPullRequest size={14} />
                  PRs
                  {pullRequests.length > 0 && (
                    <span className="ml-1 text-[10px] text-slate-500">
                      {pullRequests.length}
                    </span>
                  )}
                </button>
              </div>

              {/* List */}
              <div className="max-h-[240px] overflow-y-auto border border-[var(--cv-line)] bg-[#07080a] p-2">
                {activeTab === "branches" ? (
                  branches.length === 0 ? (
                    <div className="px-3 py-4 text-center text-xs text-slate-500">
                      No branches found
                    </div>
                  ) : (
                    branches.map((branch) => (
                      <button
                        key={branch}
                        onClick={() => handleSelectBranch(branch)}
                        className={cn(
                          "mb-2 flex w-full items-center gap-3 border px-3 py-2.5 text-left text-xs transition-colors last:mb-0",
                          selectedBranch === branch
                            ? "border-[rgba(125,211,252,0.42)] bg-cyan-500/10 text-[var(--cv-accent)]"
                            : "border-[var(--cv-line)] bg-[#050608] text-slate-400 hover:border-[var(--cv-line-strong)] hover:bg-white/[0.04] hover:text-slate-200",
                        )}
                      >
                        <GitBranch size={14} className="shrink-0" />
                        <div className="min-w-0 flex-1">
                          <div className="flex min-w-0 items-center gap-2">
                            <span className="truncate font-medium">{branch}</span>
                            {branch === currentBranch && (
                              <Badge
                                variant="outline"
                                className="shrink-0 rounded-full border-emerald-500/30 px-2 py-0 text-[9px] text-emerald-400"
                              >
                                current
                              </Badge>
                            )}
                          </div>
                          <div className="mt-1 font-mono text-[10px] uppercase tracking-[0.12em] text-slate-600">
                            compare {baseBranch} → {branch}
                          </div>
                        </div>
                      </button>
                    ))
                  )
                ) : pullRequests.length === 0 ? (
                  <div className="px-3 py-4 text-center text-xs text-slate-500">
                    No open PRs
                  </div>
                ) : (
                  pullRequests.map((pr) => (
                    <button
                      key={pr.number}
                      onClick={() => handleSelectPR(pr)}
                      className={cn(
                        "mb-2 flex w-full items-start gap-3 border px-3 py-3 text-left text-xs transition-colors last:mb-0",
                        selectedBranch === pr.headRefName
                          ? "border-[rgba(125,211,252,0.42)] bg-cyan-500/10 text-[var(--cv-accent)]"
                          : "border-[var(--cv-line)] bg-[#050608] text-slate-400 hover:border-[var(--cv-line-strong)] hover:bg-white/[0.04] hover:text-slate-200",
                      )}
                    >
                      <GitPullRequest size={14} className="mt-0.5 shrink-0" />
                      <div className="min-w-0 flex-1">
                        <div className="flex min-w-0 items-center gap-2">
                          <span className="shrink-0 font-mono text-[10px] uppercase tracking-[0.12em] text-slate-500">
                            #{pr.number}
                          </span>
                          <span className="truncate font-medium text-slate-200">{pr.title}</span>
                        </div>
                        <div className="mt-1 flex items-center gap-2 font-mono text-[10px] uppercase tracking-[0.12em] text-slate-600">
                          <span className="truncate">{pr.baseRefName}</span>
                          <GitCommitHorizontal size={11} className="shrink-0" />
                          <span className="truncate">{pr.headRefName}</span>
                        </div>
                        {pr.author?.login && (
                          <div className="mt-1 text-[10px] text-slate-600">
                            opened by {pr.author.login}
                          </div>
                        )}
                      </div>
                    </button>
                  ))
                )}
              </div>

              {/* Diff range indicator */}
              {diffRange && (
                <div className="border border-[var(--cv-line)] bg-[#07080a] px-3 py-2 font-mono text-[11px] text-slate-500">
                  {diffRange}
                </div>
              )}

              <Separator className="bg-[var(--cv-line)]" />

              {/* Project description */}
              <div className="space-y-1.5">
                <label className="text-[11px] font-medium text-slate-400">
                  Project description
                </label>
                <textarea
                  value={projectDesc}
                  onChange={(e) => setProjectDesc(e.target.value)}
                  onBlur={handleProjectDescBlur}
                  placeholder="Describe the project so the reviewer has context..."
                  className="w-full resize-none border border-[var(--cv-line)] bg-[#07080a] px-3 py-2 text-xs text-slate-200 placeholder-slate-600 focus:border-cyan-500/40 focus:outline-none"
                  rows={3}
                />
              </div>

              {/* Change description */}
              <div className="space-y-1.5">
                <label className="text-[11px] font-medium text-slate-400">
                  Change description
                </label>
                <textarea
                  value={changeDesc}
                  onChange={(e) => setChangeDesc(e.target.value)}
                  placeholder="What does this change do?"
                  className="w-full resize-none border border-[var(--cv-line)] bg-[#07080a] px-3 py-2 text-xs text-slate-200 placeholder-slate-600 focus:border-cyan-500/40 focus:outline-none"
                  rows={2}
                />
              </div>

              {/* Review button */}
              <Button
                onClick={handleReview}
                disabled={!diffRange || isReviewing}
                className="w-full gap-2 bg-white text-black hover:bg-slate-200 disabled:opacity-50"
              >
                {isReviewing ? (
                  <Loader2 size={16} className="animate-spin" />
                ) : (
                  <Zap size={16} />
                )}
                {isReviewing ? "Reviewing..." : "Review with Claude"}
              </Button>

              {/* Error */}
              {error && (
                <div className="rounded-md bg-red-500/10 px-3 py-2 text-xs text-red-400">
                  {error}
                </div>
              )}
            </>
          )}

          {/* Past reviews */}
          {pastReviewsLoading ? (
            <>
              <Separator className="bg-[var(--cv-line)]" />
              <div className="flex items-center gap-2 text-[11px] text-slate-500">
                <Loader2 size={12} className="animate-spin" />
                Loading past reviews...
              </div>
            </>
          ) : pastReviews.length > 0 ? (
            <>
              <Separator className="bg-[var(--cv-line)]" />
              <button
                onClick={() => setShowHistory(!showHistory)}
                className="flex w-full items-center justify-between text-[11px] font-medium text-slate-400 hover:text-slate-200"
              >
                <span>Past Reviews ({pastReviews.length})</span>
                <span className="text-slate-600">{showHistory ? "▼" : "▶"}</span>
              </button>
              {showHistory && (
                <div className="space-y-1">
                  {pastReviews.map((r) => (
                    <button
                      key={r.id}
                      onClick={() => handleLoadPastReview(r.id)}
                      className={cn(
                        "flex w-full items-center gap-2 rounded-md px-2 py-1.5 text-left text-xs transition-colors",
                        result?.review_id === r.id
                          ? "bg-cyan-500/10 text-[var(--cv-accent)]"
                          : "text-slate-400 hover:bg-white/[0.04] hover:text-slate-200",
                      )}
                    >
                      <ScoreBadge score={Math.round(r.score_composite ?? 0)} size="sm" />
                      <div className="flex-1 min-w-0">
                        <div className="truncate">
                          {r.repo_path
                            ? shortenPath(r.repo_path).split("/").pop()
                            : r.source_label ?? "Review"}
                        </div>
                        <div className="text-[10px] text-slate-600">
                          {r.findings_count ?? 0} findings · {formatRelativeTime(r.completed_at ?? r.created_at)}
                        </div>
                      </div>
                    </button>
                  ))}
                </div>
              )}
            </>
          ) : null}
        </div>
      </div>

      {/* Right panel */}
      <div className="cv-frame cv-scan flex-1 overflow-hidden">
        {isReviewing ? (
          <div className="flex h-full flex-col items-center justify-center gap-3">
            <Loader2 size={32} className="animate-spin text-[var(--cv-accent)]" />
            <span className="text-sm text-slate-400">
              Reviewing with Claude...
            </span>
          </div>
        ) : (
          <div className="flex h-full flex-col">
            <div className="cv-terminal-bar h-11 px-4">
              <span className="cv-dot" />
              <span className="cv-dot" />
              <span className="cv-dot" />
              <span className="cv-label mx-auto">review preview · select a diff</span>
              <span className="cv-label">⌘ K</span>
            </div>
            <div className="grid min-h-0 flex-1 grid-cols-1 xl:grid-cols-[1fr_280px]">
              <div className="border-r border-[var(--cv-line)] bg-[#050505] p-6 font-mono text-[13px] leading-7 text-slate-400">
                <div className="mb-4 flex items-center justify-between border-b border-[var(--cv-line)] pb-3">
                  <span className="cv-label">apps/api/src/auth/session_manager.ts</span>
                  <span className="cv-label text-[var(--cv-danger)]">+2 / -0</span>
                </div>
                <div className="grid grid-cols-[42px_1fr] gap-x-4">
                  <span className="text-right text-slate-700">36</span>
                  <span><span className="text-purple-400">import</span> {`{`} db {`}`} <span className="text-purple-400">from</span> <span className="text-emerald-400">"@/lib/sql"</span>;</span>
                  <span className="text-right text-slate-700">37</span>
                  <span />
                  <span className="text-right text-slate-700">38</span>
                  <span><span className="text-purple-400">async function</span> <span className="text-cyan-300">validateSession</span>(token: <span className="text-yellow-300">string</span>) {`{`}</span>
                  <span className="text-right text-[var(--cv-danger)]/70">40</span>
                  <span className="-mx-3 border-l-2 border-[var(--cv-danger)] bg-red-500/10 px-3 text-slate-200">const query = `SELECT * FROM sessions WHERE token = '${"{token}"}'`;</span>
                </div>
              </div>
              <aside className="hidden bg-white/[0.015] p-6 xl:block">
                <div className="cv-label mb-5">Verdict</div>
                <Badge variant="outline" className="border-red-500/25 bg-red-500/10 text-red-400">
                  <AlertTriangle size={12} className="mr-1" />
                  Critical
                </Badge>
                <h2 className="mt-5 text-lg font-semibold text-white">SQL injection vector</h2>
                <p className="mt-3 text-sm leading-6 text-slate-400">
                  Select a repository and diff to run the real review against
                  your local code.
                </p>
                <div className="mt-6 border-t border-[var(--cv-line)] pt-5">
                  <div className="cv-label mb-3">Suggested actions</div>
                  <button className="h-10 w-full bg-white text-sm font-medium text-black">Apply Patch</button>
                </div>
              </aside>
            </div>
          </div>
        )}
      </div>
    </div>
  );
}

// ─── FindingItem ──────────────────────────────────────────────────────────────

function FindingItem({
  finding,
  selected,
  onToggle,
}: {
  finding: CliReviewFinding;
  selected: boolean;
  onToggle: () => void;
}) {
  return (
    <div
      className={cn(
        "rounded-lg border bg-[#0a0a0a] p-4 transition-colors",
        selected ? "border-amber-500/30" : "border-[#1a1a1a]",
      )}
    >
      {/* Header: checkbox + severity badge + title */}
      <div className="flex items-start gap-2">
        <button onClick={onToggle} className="mt-0.5 shrink-0 text-slate-500 hover:text-amber-400">
          {selected ? (
            <CheckSquare2 size={16} className="text-amber-400" />
          ) : (
            <Square size={16} />
          )}
        </button>
        <Badge
          variant="outline"
          className={cn(
            "shrink-0 text-[10px] font-semibold uppercase",
            severityColor(finding.severity),
          )}
        >
          {finding.severity}
        </Badge>
        <h3 className="flex-1 text-sm font-medium text-slate-200">{finding.title}</h3>
      </div>

      {/* Summary */}
      <p className="mt-2 text-xs leading-relaxed text-slate-400">
        {finding.summary}
      </p>

      {/* File + line */}
      {finding.filePath && (
        <div className="mt-2 flex items-center gap-1 font-mono text-[11px] text-slate-500">
          <span className="truncate">{finding.filePath}</span>
          {finding.line != null && <span>:{finding.line}</span>}
        </div>
      )}

      {/* Suggestion */}
      {finding.suggestion && (
        <div className="mt-3 rounded-md bg-amber-500/5 border border-amber-500/10 px-3 py-2 text-xs text-amber-300/80">
          <span className="font-semibold text-amber-400">Suggestion: </span>
          {finding.suggestion}
        </div>
      )}

      {/* Confidence */}
      {finding.confidence != null && (
        <div className="mt-2 text-[10px] text-slate-600">
          Confidence: {Math.round(finding.confidence * 100)}%
        </div>
      )}
    </div>
  );
}
