export type UnpackPhase = 'idle' | 'scanning' | 'generating' | 'asking' | 'ready' | 'error';

import type { LucideIcon } from 'lucide-react';
import {
  Activity,
  BarChart3,
  BookOpenText,
  FileText,
  FolderTree,
  LayoutDashboard,
  Network,
} from 'lucide-react';

export type UnpackWorkspaceSection =
  | 'overview'
  | 'memory'
  | 'rules'
  | 'brief'
  | 'activity'
  | 'inventory'
  | 'intelligence'
  | 'delta';

export type UnpackSectionMeta = {
  id: UnpackWorkspaceSection;
  label: string;
  short: string;
  icon: LucideIcon;
  description: string;
  requiresInventory?: boolean;
  requiresReport?: boolean;
  requiresComparison?: boolean;
};

const UNPACK_SECTIONS: UnpackSectionMeta[] = [
  {
    id: 'overview',
    label: 'Overview',
    short: 'Overview',
    icon: LayoutDashboard,
    description: 'Mission status, metric readout, and next actions.',
  },
  {
    id: 'memory',
    label: 'Handoff',
    short: 'Handoff',
    icon: BookOpenText,
    description: 'Files, rules, and boundaries an agent should read before editing.',
    requiresInventory: true,
  },
  {
    id: 'rules',
    label: 'Rules',
    short: 'Rules',
    icon: BookOpenText,
    description: 'Evidence-traced business rules, exact clauses, source spans, and dependencies.',
    requiresInventory: true,
  },
  {
    id: 'brief',
    label: 'Analysis',
    short: 'AI',
    icon: FileText,
    description: 'Optional AI analysis attached to the selected local snapshot.',
    requiresInventory: true,
  },
  {
    id: 'activity',
    label: 'Activity',
    short: 'Activity',
    icon: BarChart3,
    description: 'Git attribution, churn, authors, and release-health signals.',
    requiresInventory: true,
  },
  {
    id: 'inventory',
    label: 'Inventory',
    short: 'Files',
    icon: FolderTree,
    description: 'Languages, directories, entrypoints, and scan stats.',
    requiresInventory: true,
  },
  {
    id: 'intelligence',
    label: 'Graph',
    short: 'Graph',
    icon: Network,
    description: 'Risk, test posture, dependency graph, deep graph index, and history leads.',
    requiresInventory: true,
  },
  {
    id: 'delta',
    label: 'Delta',
    short: 'Delta',
    icon: Activity,
    description: 'Snapshot diffs, commit range, verification leads, and calibration.',
    requiresInventory: true,
    requiresComparison: true,
  },
];

export function visibleUnpackSections(input: {
  hasInventory: boolean;
  hasReport: boolean;
  hasComparison: boolean;
}): UnpackSectionMeta[] {
  return UNPACK_SECTIONS.filter((section) => {
    if (section.requiresInventory && !input.hasInventory) return false;
    if (section.requiresReport && !input.hasReport) return false;
    if (section.requiresComparison && !input.hasComparison) return false;
    return true;
  });
}

export function isUnpackSection(value: string | null): value is UnpackWorkspaceSection {
  return UNPACK_SECTIONS.some((s) => s.id === value);
}
