import type { Diagram } from '@capsem/diagrams/types';
import type { Edge, Snapshot } from './types';

export type DependencySnapshot = Pick<Snapshot, 'date' | 'commit' | 'crates' | 'edges'>;
export function dependencyView(snapshot: DependencySnapshot, focus = '', kind = 'runtime'): Diagram {
  let edges = snapshot.edges.filter(e => kind === 'all' || e.kind === kind);
  let crates = snapshot.crates;
  if (focus) {
    const neighbours = new Set([focus, ...edges.filter(e => e.from === focus || e.to === focus).flatMap(e => [e.from, e.to])]);
    crates = crates.filter(name => neighbours.has(name));
    edges = edges.filter(e => neighbours.has(e.from) && neighbours.has(e.to));
  }
  const pairs = new Map<string, Edge[]>();
  for (const edge of edges) {
    const key = `${edge.from}/${edge.to}`;
    pairs.set(key, [...(pairs.get(key) ?? []), edge]);
  }
  const columns = 4, width = 940, gap = 100;
  const position = (index: number) => ({ x: 30 + index % columns * 230, y: 30 + Math.floor(index / columns) * gap });
  return {
    title: `Crate dependencies · ${snapshot.date}`,
    description: `${focus ? `${focus} and its direct neighbours` : 'All workspace crates'}. Arrows point from a crate to its dependencies.`,
    width, height: Math.max(180, Math.ceil(crates.length / columns) * gap + 50),
    nodes: crates.map((name, i) => ({ id: name, label: name.replace(/^capsem-/, '').replaceAll('-', '\n') + (name === focus ? '\n(selected)' : ''),
      ...position(i), width: 190, height: 74, tone: 'component' })),
    links: [...pairs.values()].map(group => ({ from: group[0].from, to: group[0].to,
      label: [...new Set(group.map(e => e.kind))].join(', '), labelAt: .5 })),
  };
}
