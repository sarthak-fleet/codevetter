import type { ScenarioCompilerCandidate } from '../tauri-ipc';

export function mergeRefreshedCandidate(
  candidates: readonly ScenarioCompilerCandidate[],
  refreshed: ScenarioCompilerCandidate | null,
  limit: number
): ScenarioCompilerCandidate[] {
  if (!refreshed) return candidates.slice(0, limit);
  const existing = candidates.findIndex(
    ({ candidate_id }) => candidate_id === refreshed.candidate_id
  );
  const merged = [...candidates];
  if (existing === -1) merged.unshift(refreshed);
  else merged[existing] = refreshed;
  return merged.slice(0, limit);
}
