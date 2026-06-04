export interface CommitIntentFixture {
  id: string;
  author: "agent" | "human";
  sha: string;
  message: string;
  changedFiles: Array<{
    path: string;
    additions: number;
    deletions: number;
    surface: "ui" | "api" | "test" | "docs" | "config";
  }>;
  evidence: Array<{
    kind: "test" | "screenshot" | "manual" | "none";
    label: string;
    status: "pass" | "fail" | "missing";
  }>;
}

export interface CommitIntentReport {
  id: string;
  sha: string;
  author: CommitIntentFixture["author"];
  inferredIntent: string;
  changedSurfaces: string[];
  suspectedRisks: string[];
  verificationGaps: string[];
  evidenceSummary: string;
}
