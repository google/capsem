export interface Series { name: string; values: (number | null)[] }
export interface ChartSpec {
  id: string; title: string; type: 'line' | 'bar' | 'pie' | 'radar';
  unit: string; labels: string[]; series: Series[]; stacked?: boolean; note?: string;
}
export interface Variant { label: string; spec: ChartSpec }
export interface Edge { from: string; to: string; kind: string }
export interface Snapshot {
  date: string; month: string; commit: string; committedAt: string; kind: string;
  loc: Record<string, number>; clientLoc: Record<string, number>; tests: Record<string, number>;
  crates: string[]; edges: Edge[];
  dependencies: Record<string, { direct: number; resolvedNames: number | null; resolvedVersions: number | null; duplicateNames: number | null; lockfiles: string[] }>;
}
export interface Statistics {
  schema: string; sourceCommit: string; generatedAt: string; githubObservedAt: string;
  snapshots: Snapshot[];
  releaseOutcomes: { month: string; success: number; failure: number; cancelled: number; failedJobHours: number }[];
  releaseSuccesses: { label: string; lane: string; failures: number; cancelled: number; minutes: number; url: string }[];
  releasePublications: { version: string; publishedAt: string; minutes: number | null; conclusion: string | null; url: string }[];
}
export interface Benchmarks {
  release: string; architecture: string; recordedAt: string; commit: string; sourceUrl: string;
  sections: Record<string, { title: string; description: string; available: boolean; charts: Omit<ChartSpec, 'type'>[] }>;
}
