import { dia, shapes } from '@joint/core';
import type { Diagram } from './types';

export function mountDiagram(root: HTMLElement): () => void {
  const diagram: Diagram = JSON.parse(root.dataset.diagram!);
  const canvas = root.querySelector<HTMLElement>('.diagram-canvas')!;
  // JointJS measures SVG anchors during rendering; the canvas must be visible.
  canvas.hidden = false;
  const host = root.querySelector<HTMLElement>('.diagram-paper')!;
  const graph = new dia.Graph({}, { cellNamespace: shapes });
  const paper = new dia.Paper({
    el: host, model: graph, cellViewNamespace: shapes,
    width: diagram.width, height: diagram.height, interactive: false,
    background: { color: 'transparent' },
  });
  for (const node of diagram.nodes) {
    const kind = node.boundary ?? node.permission ?? 'external';
    const cell = new shapes.standard.Rectangle({
      id: node.id, position: { x: node.x, y: node.y },
      size: { width: node.width ?? 220, height: node.height ?? 72 },
      z: node.boundary ? 0 : 2,
      attrs: {
        body: { fill: `var(--diagram-${kind})`, stroke: `var(--diagram-${kind}-line)`, strokeWidth: node.boundary ? 1.5 : 2, rx: 12, ry: 12, strokeDasharray: node.boundary === 'group' ? '6 5' : 'none' },
        label: {
          text: node.label, fill: 'var(--diagram-ink)', fontFamily: 'system-ui, sans-serif',
          fontSize: node.boundary ? 12 : 15, fontWeight: 600, lineHeight: 21,
          ...(node.boundary ? { x: 18, y: 24, textAnchor: 'start', textVerticalAnchor: 'middle' } : {}),
        },
      },
    });
    graph.addCell(cell);
  }
  const arrow = { type: 'path', d: 'M 8 -4 0 0 8 4 z' };
  for (const edge of diagram.links) {
    const link = new shapes.standard.Link({
      source: { id: edge.from, anchor: { name: edge.source ?? 'bottom' } },
      target: { id: edge.to, anchor: { name: edge.target ?? 'top' } },
      vertices: edge.via ?? [], z: 3,
      connector: { name: 'rounded', args: { radius: 8 } },
      attrs: { line: {
        stroke: 'var(--diagram-wire)', strokeWidth: 1.5,
        strokeDasharray: edge.control ? '6 5' : 'none',
        targetMarker: arrow, sourceMarker: edge.both ? arrow : { type: 'path', d: '' },
      } },
      labels: edge.label ? [{
        position: { distance: edge.labelAt ?? .5, offset: edge.labelOffset ?? -12 },
        attrs: {
          text: { text: edge.label, fill: 'var(--diagram-ink)', fontFamily: 'system-ui, sans-serif', fontSize: 12 },
          rect: { fill: 'var(--diagram-external)', rx: 3, ry: 3 },
        },
      }] : [],
    });
    graph.addCell(link);
  }
  const svg = host.querySelector('svg')!;
  svg.setAttribute('role', 'img');
  svg.setAttribute('aria-label', `${diagram.description ?? diagram.title}. Connection details are in the legend.`);
  svg.setAttribute('preserveAspectRatio', 'xMidYMid meet');
  let zoom = 1, x = 0, y = 0;
  const controls = root.querySelector<HTMLElement>('.diagram-toolbar')!;
  const output = root.querySelector('output')!;
  function render() {
    svg.setAttribute('viewBox', `${x} ${y} ${diagram.width / zoom} ${diagram.height / zoom}`);
    output.value = `${Math.round(zoom * 100)}%`;
  }
  function changeZoom(factor: number) {
    const previous = zoom;
    zoom = Math.min(4, Math.max(.5, zoom * factor));
    x += diagram.width / previous / 2 - diagram.width / zoom / 2;
    y += diagram.height / previous / 2 - diagram.height / zoom / 2;
    render();
  }
  function fit() { zoom = 1; x = 0; y = 0; render(); }
  const abort = new AbortController();
  const options = { signal: abort.signal };
  controls.addEventListener('click', (event) => {
    const action = (event.target as HTMLElement).closest('button')?.dataset.action;
    if (action === 'in') changeZoom(1.25);
    if (action === 'out') changeZoom(1 / 1.25);
    if (action === 'fit') fit();
  }, options);
  canvas.addEventListener('keydown', (event) => {
    if (event.target !== canvas) return;
    if (!['+', '=', '-', '0', 'ArrowLeft', 'ArrowRight', 'ArrowUp', 'ArrowDown'].includes(event.key)) return;
    event.preventDefault();
    if (event.key === '+' || event.key === '=') changeZoom(1.25);
    if (event.key === '-') changeZoom(1 / 1.25);
    if (event.key === '0') fit();
    if (event.key === 'ArrowLeft') x -= 50 / zoom;
    if (event.key === 'ArrowRight') x += 50 / zoom;
    if (event.key === 'ArrowUp') y -= 50 / zoom;
    if (event.key === 'ArrowDown') y += 50 / zoom;
    render();
  }, options);
  let drag: { px: number; py: number; x: number; y: number; scale: number } | undefined;
  canvas.addEventListener('pointerdown', (event) => {
    if (event.button !== 0 || controls.contains(event.target as Node)) return;
    const bounds = svg.getBoundingClientRect();
    drag = { px: event.clientX, py: event.clientY, x, y, scale: Math.min(bounds.width / diagram.width, bounds.height / diagram.height) * zoom };
    canvas.setPointerCapture(event.pointerId);
    canvas.focus({ preventScroll: true });
    if (event.pointerType !== 'touch') event.preventDefault();
  }, options);
  canvas.addEventListener('pointermove', (event) => {
    if (!drag) return;
    x = drag.x - (event.clientX - drag.px) / drag.scale;
    y = drag.y - (event.clientY - drag.py) / drag.scale;
    render();
  }, options);
  canvas.addEventListener('lostpointercapture', () => { drag = undefined; }, options);
  controls.hidden = false;
  canvas.hidden = false;
  root.querySelector('details')!.open = false;
  fit();
  return () => { abort.abort(); paper.remove(); graph.clear(); };
}
