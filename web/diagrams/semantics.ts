import type { Diagram, DiagramNode } from './types';

// Each meaning owns one Tailwind color family, shared by the SVG and its legend.
export const semantics = {
  minimum: { family: 'emerald', fill: 'emerald-100', stroke: 'emerald-700', label: 'Minimum host permissions', description: 'Granted connections only; no arbitrary host files or new connections.' },
  limited: { family: 'amber', fill: 'amber-100', stroke: 'amber-700', label: 'Limited host permissions', description: 'Assigned files, credentials, or upstream connections.' },
  elevated: { family: 'orange', fill: 'orange-100', stroke: 'orange-700', label: 'Elevated host permissions', description: 'VM control and virtualization resources.' },
  maximum: { family: 'rose', fill: 'rose-100', stroke: 'rose-700', label: 'Maximum host permissions', description: 'Launching-user permissions or trusted coordinator authority.' },
  component: { family: 'neutral', fill: 'white', stroke: 'neutral-400', label: 'Components, inputs and outputs', description: 'No host-permission level assigned by this diagram.' },
  processing: { family: 'blue', fill: 'blue-50', stroke: 'blue-600', label: 'Control and processing', description: 'Components or stages that manage and process operations.' },
  restricted: { family: 'violet', fill: 'violet-100', stroke: 'violet-700', label: 'Refused or awaiting approval', description: 'The operation cannot proceed without authorization.' },
  credential: { family: 'fuchsia', fill: 'fuchsia-50', stroke: 'fuchsia-700', label: 'Host-held credentials', description: 'Credential material stays outside the VM.' },
  group: { family: 'slate', fill: 'slate-100', stroke: 'slate-500', label: 'Per-VM ownership', description: 'Dashed boundary groups components assigned to one VM.' },
  vm: { family: 'cyan', fill: 'cyan-50', stroke: 'cyan-600', label: 'VM hardware boundary', description: 'Separates the Linux VM and its workload from host processes.' },
  engine: { family: 'indigo', fill: 'indigo-50', stroke: 'indigo-600', label: 'Security-engine boundary', description: 'Groups the stages inside the security-engine process.' },
} as const;

export type DiagramMeaning = keyof typeof semantics;

export function nodeMeaning(node: DiagramNode): DiagramMeaning {
  return node.boundary ?? node.tone ?? 'component';
}

export function diagramLegend(diagram: Diagram): DiagramMeaning[] {
  const used = new Set(diagram.nodes.map(nodeMeaning));
  return (Object.keys(semantics) as DiagramMeaning[]).filter(meaning => used.has(meaning));
}

export const colorTokens = Object.entries(semantics).map(([meaning, value]) =>
  `--diagram-${meaning}:var(--color-${value.fill});--diagram-${meaning}-line:var(--color-${value.stroke})`
).join(';');
