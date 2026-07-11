import {
  createContext,
  type ReactNode,
  useCallback,
  useContext,
  useEffect,
  useMemo,
  useRef,
  useState,
} from 'react';

import {
  getPreference,
  isTauriAvailable,
  listRepoProjects,
  pickDirectory,
  preloadDirectoryPicker,
  registerRepoProject,
  removeRepoProject,
  type RepoProject,
  setPreference,
} from '@/lib/tauri-ipc';

const ACTIVE_REPO_PATH_KEY = 'active_repo_path';

const LEGACY_REPO_PATH_KEYS = ['quick_review_last_folder', 'intel_last_repo'] as const;

type ProjectWorkspaceContextValue = {
  projects: RepoProject[];
  loading: boolean;
  ready: boolean;
  addingProject: boolean;
  selectedRepoPath: string | null;
  selectedProject: RepoProject | null;
  selectProject: (path: string) => void;
  removeProject: (path: string) => Promise<void>;
  addProject: () => Promise<string | null>;
  refreshProjects: (opts?: { silent?: boolean }) => Promise<RepoProject[]>;
};

const ProjectWorkspaceContext = createContext<ProjectWorkspaceContextValue | null>(null);

function optimisticProject(path: string, existing?: RepoProject): RepoProject {
  const now = new Date().toISOString();
  if (existing) return { ...existing, last_opened_at: now };
  return {
    id: path,
    repo_path: path,
    display_name: path.split('/').pop() ?? 'repo',
    first_opened_at: now,
    last_opened_at: now,
    last_unpack_at: null,
    last_intel_at: null,
    unpack_snapshot_count: 0,
    intel_snapshot_count: 0,
  };
}

function sortProjects(rows: RepoProject[]): RepoProject[] {
  return [...rows].sort((a, b) => b.last_opened_at.localeCompare(a.last_opened_at));
}

async function resolveStoredRepoPath(): Promise<string | null> {
  if (!isTauriAvailable()) return null;
  try {
    const active = await getPreference(ACTIVE_REPO_PATH_KEY);
    if (active?.trim()) return active.trim();
    for (const key of LEGACY_REPO_PATH_KEYS) {
      const legacy = await getPreference(key);
      if (legacy?.trim()) return legacy.trim();
    }
  } catch {
    /* ignore */
  }
  return null;
}

export function ProjectWorkspaceProvider({ children }: { children: ReactNode }) {
  const [projects, setProjects] = useState<RepoProject[]>([]);
  const [loading, setLoading] = useState(true);
  const [ready, setReady] = useState(false);
  const [addingProject, setAddingProject] = useState(false);
  const [selectedRepoPath, setSelectedRepoPath] = useState<string | null>(null);
  const hydrating = useRef(false);
  const addingProjectRef = useRef(false);

  const mergeProjectRow = useCallback((row: RepoProject) => {
    setProjects((prev) => {
      const without = prev.filter((p) => p.repo_path !== row.repo_path);
      return sortProjects([row, ...without]);
    });
  }, []);

  const refreshProjects = useCallback(async (opts?: { silent?: boolean }) => {
    if (!isTauriAvailable()) {
      setLoading(false);
      return [];
    }
    if (!opts?.silent) setLoading(true);
    try {
      const rows = await listRepoProjects();
      setProjects(sortProjects(rows));
      return rows;
    } catch {
      return [];
    } finally {
      if (!opts?.silent) setLoading(false);
    }
  }, []);

  const persistSelection = useCallback((path: string) => {
    void setPreference(ACTIVE_REPO_PATH_KEY, path).catch(() => {});
    void setPreference('quick_review_last_folder', path).catch(() => {});
    void setPreference('intel_last_repo', path).catch(() => {});
  }, []);

  const selectProject = useCallback(
    (path: string) => {
      const trimmed = path.trim();
      if (!trimmed) return;

      setSelectedRepoPath(trimmed);
      persistSelection(trimmed);

      setProjects((prev) => {
        const existing = prev.find((p) => p.repo_path === trimmed);
        const bumped = optimisticProject(trimmed, existing);
        const without = prev.filter((p) => p.repo_path !== trimmed);
        return sortProjects([bumped, ...without]);
      });

      if (isTauriAvailable()) {
        void registerRepoProject(trimmed)
          .then((row) => mergeProjectRow(row))
          .catch(() => {});
      }
    },
    [mergeProjectRow, persistSelection]
  );

  const addProject = useCallback(async () => {
    if (!isTauriAvailable()) return null;
    if (addingProjectRef.current) return null;
    addingProjectRef.current = true;
    setAddingProject(true);
    try {
      const picked = await pickDirectory('Select a repository');
      if (!picked) return null;
      selectProject(picked);
      return picked;
    } finally {
      addingProjectRef.current = false;
      setAddingProject(false);
    }
  }, [selectProject]);

  const removeProject = useCallback(
    async (path: string) => {
      const trimmed = path.trim();
      if (!trimmed) return;

      const remaining = projects.filter((p) => p.repo_path !== trimmed);
      const nextSelection =
        selectedRepoPath === trimmed ? (remaining[0]?.repo_path ?? null) : selectedRepoPath;
      setProjects(remaining);

      if (selectedRepoPath === trimmed) {
        setSelectedRepoPath(nextSelection);
        const persisted = nextSelection ?? '';
        await Promise.all([
          setPreference(ACTIVE_REPO_PATH_KEY, persisted).catch(() => {}),
          setPreference('quick_review_last_folder', persisted).catch(() => {}),
          setPreference('intel_last_repo', persisted).catch(() => {}),
        ]);
      }

      if (isTauriAvailable()) {
        await removeRepoProject(trimmed).catch(() => {
          void refreshProjects({ silent: true });
        });
      }
    },
    [projects, refreshProjects, selectedRepoPath]
  );

  useEffect(() => {
    if (isTauriAvailable()) preloadDirectoryPicker();
    if (hydrating.current) return;
    hydrating.current = true;

    let cancelled = false;
    void (async () => {
      const rows = await refreshProjects();
      if (cancelled) return;

      const stored = await resolveStoredRepoPath();
      if (stored && rows.some((p) => p.repo_path === stored)) {
        setSelectedRepoPath(stored);
      }

      if (!cancelled) setReady(true);
    })();

    return () => {
      cancelled = true;
      // Allow the next mount to re-hydrate. Without this, StrictMode's dev
      // double-effect leaves `ready` false forever: run 1 is cancelled, run 2
      // bails on the guard.
      hydrating.current = false;
    };
  }, [mergeProjectRow, refreshProjects]);

  const selectedProject = useMemo(
    () => projects.find((p) => p.repo_path === selectedRepoPath) ?? null,
    [projects, selectedRepoPath]
  );

  const value = useMemo(
    () => ({
      projects,
      loading,
      ready,
      addingProject,
      selectedRepoPath,
      selectedProject,
      selectProject,
      removeProject,
      addProject,
      refreshProjects,
    }),
    [
      projects,
      loading,
      ready,
      addingProject,
      selectedRepoPath,
      selectedProject,
      selectProject,
      removeProject,
      addProject,
      refreshProjects,
    ]
  );

  return (
    <ProjectWorkspaceContext.Provider value={value}>{children}</ProjectWorkspaceContext.Provider>
  );
}

export function useProjectWorkspace(): ProjectWorkspaceContextValue {
  const ctx = useContext(ProjectWorkspaceContext);
  if (!ctx) {
    throw new Error('useProjectWorkspace must be used within ProjectWorkspaceProvider');
  }
  return ctx;
}
