export interface UnpackMetricScoreInput {
  totalMs: number;
  capped: boolean;
  coveragePct: number | null;
  hasWholeRepoMetadata: boolean;
  graphNodes: number;
  graphTruncated: boolean;
  healthFiles: number;
  commits: number;
  workspaceUnits: number;
}

export interface UnpackMetricScores {
  speed: number;
  correctness: number;
  usefulness: number;
}

function speedScore(totalMs: number): number {
  if (totalMs <= 250) return 10;
  if (totalMs <= 1500) return 9;
  if (totalMs <= 2500) return 8;
  if (totalMs <= 5000) return 6;
  return 4;
}

function correctnessScore({
  capped,
  coveragePct,
  hasWholeRepoMetadata,
  hasGraph,
  hasHealth,
}: {
  capped: boolean;
  coveragePct: number | null;
  hasWholeRepoMetadata: boolean;
  hasGraph: boolean;
  hasHealth: boolean;
}): number {
  if (!hasGraph || !hasHealth) return 6;
  if (!capped) return 10;
  if (!hasWholeRepoMetadata) return 6;
  if (coveragePct !== null && coveragePct < 3) return 8;
  return 9;
}

function usefulnessScore({
  coveragePct,
  capped,
  hasWholeRepoMetadata,
  graphNodes,
  graphTruncated,
  healthFiles,
  commits,
  workspaceUnits,
}: {
  coveragePct: number | null;
  capped: boolean;
  hasWholeRepoMetadata: boolean;
  graphNodes: number;
  graphTruncated: boolean;
  healthFiles: number;
  commits: number;
  workspaceUnits: number;
}): number {
  let score = 2;
  if (graphNodes >= 500) score += 2;
  else if (graphNodes >= 80) score += 1;
  if (workspaceUnits >= 3) score += 2;
  else if (workspaceUnits > 0) score += 1;
  if (healthFiles >= 80) score += 2;
  else if (healthFiles >= 20) score += 1;
  if (hasWholeRepoMetadata) score += 1;
  if (commits >= 8) score += 1;
  if (!capped || (coveragePct !== null && coveragePct >= 50)) score += 1;
  else if (coveragePct !== null && coveragePct >= 10) score += 1;
  if (graphTruncated && graphNodes < 500) score -= 1;
  return Math.min(10, score);
}

export function computeUnpackMetricScores(input: UnpackMetricScoreInput): UnpackMetricScores {
  return {
    speed: speedScore(input.totalMs),
    correctness: correctnessScore({
      capped: input.capped,
      coveragePct: input.coveragePct,
      hasWholeRepoMetadata: input.hasWholeRepoMetadata,
      hasGraph: input.graphNodes > 0,
      hasHealth: input.healthFiles > 0,
    }),
    usefulness: usefulnessScore({
      coveragePct: input.coveragePct,
      capped: input.capped,
      hasWholeRepoMetadata: input.hasWholeRepoMetadata,
      graphNodes: input.graphNodes,
      graphTruncated: input.graphTruncated,
      healthFiles: input.healthFiles,
      commits: input.commits,
      workspaceUnits: input.workspaceUnits,
    }),
  };
}
