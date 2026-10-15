export type Permission = 'minimum' | 'limited' | 'elevated' | 'maximum' | 'external';
export type Tone = Permission | 'accent' | 'secondary';
export interface DiagramNode {
  id: string; label: string; x: number; y: number;
  permission?: Tone; width?: number; height?: number;
  boundary?: 'vm' | 'group' | 'engine';
}
export interface DiagramLink {
  from: string; to: string; label: string; control?: boolean; both?: boolean;
  source?: 'top' | 'bottom' | 'left' | 'right'; target?: 'top' | 'bottom' | 'left' | 'right';
  via?: { x: number; y: number }[];
  labelAt?: number;
  labelOffset?: number;
}
export interface Diagram {
  title: string; description?: string; width: number; height: number;
  nodes: DiagramNode[]; links: DiagramLink[];
}
