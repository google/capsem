import * as echarts from 'echarts/core';
import { BarChart, LineChart, PieChart, RadarChart } from 'echarts/charts';
import { AriaComponent, GridComponent, LegendComponent, RadarComponent, TooltipComponent } from 'echarts/components';
import { SVGRenderer } from 'echarts/renderers';
import type { ChartSpec, Variant } from './types';

echarts.use([BarChart, LineChart, PieChart, RadarChart, AriaComponent, GridComponent, LegendComponent, RadarComponent, TooltipComponent, SVGRenderer]);
const format = (value: number) => value.toLocaleString(undefined, { maximumFractionDigits: 3 });
export function options(spec: ChartSpec, root: HTMLElement) {
  const style = getComputedStyle(root);
  const color = Array.from({ length: 14 }, (_, i) => style.getPropertyValue(`--metrics-series-${i + 1}`).trim());
  const legendWidth = root.clientWidth < 480 ? 125 : 245;
  const plotWidth = Math.max(100, root.clientWidth - legendWidth - 35);
  const text = style.color, grid = style.getPropertyValue('--metrics-grid').trim();
  const base = { animation: !matchMedia('(prefers-reduced-motion: reduce)').matches, color,
    textStyle: { color: text, fontFamily: 'inherit' }, aria: { enabled: true },
    tooltip: { trigger: spec.type === 'pie' ? 'item' : 'axis', confine: true, valueFormatter: (v: number) => `${format(v)} ${spec.unit}` },
    legend: { type: 'scroll', orient: 'vertical', right: 10, top: 'middle', height: '88%', width: legendWidth, textStyle: { color: text, width: legendWidth - 35, overflow: 'break', fontSize: root.clientWidth < 480 ? 10 : 12, lineHeight: 15 }, pageTextStyle: { color: text }, pageIconColor: text },
  };
  if (spec.type === 'pie') return { ...base, tooltip: { ...base.tooltip, trigger: 'item' }, series: [{ type: 'pie', radius: [Math.min(plotWidth * .14, 65), Math.min(plotWidth * (root.clientWidth < 480 ? .30 : .39), 135)], center: [plotWidth / 2 + 10, '50%'],
    label: { show: true, position: 'outside', formatter: '{d}%', color: text, fontSize: 11 },
    labelLine: { length: 6, length2: 4 }, avoidLabelOverlap: true,
    emphasis: { label: { show: true } },
    data: spec.labels.map((name, i) => ({ name, value: spec.series[0].values[i] })) }] };
  if (spec.type === 'radar') return { ...base, tooltip: { ...base.tooltip, trigger: 'item' }, legend: { show: false },
    radar: { center: ['50%', '49%'], radius: root.clientWidth < 480 ? '44%' : '62%', indicator: spec.labels.map(name => ({ name, max: spec.radarMax ?? 100 })), axisName: { color: text, fontSize: root.clientWidth < 480 ? 9 : 10, width: root.clientWidth < 480 ? 65 : 85, overflow: 'break' },
      splitArea: { show: false }, splitLine: { lineStyle: { color: grid } }, axisLine: { lineStyle: { color: grid } } },
    series: [{ type: 'radar', data: spec.series.map(s => ({ name: s.name, value: s.values })), areaStyle: { opacity: 0.18 }, symbolSize: 5 }] };
  return { ...base, grid: { left: 15, right: legendWidth + 35, top: 35, bottom: 45, containLabel: true },
    xAxis: { type: 'category', data: spec.labels, axisLabel: { color: text, hideOverlap: true, rotate: spec.labels.some(v => v.length > 16) ? 18 : 0 }, axisLine: { lineStyle: { color: grid } } },
    yAxis: { type: 'value', max: spec.unit === '%' ? 100 : undefined, name: spec.unit, nameTextStyle: { color: text }, axisLabel: { color: text }, splitLine: { lineStyle: { color: grid } } },
    series: spec.series.map(s => ({ name: s.name, type: spec.type, data: s.values,
      ...(spec.stacked ? { stack: 'total', areaStyle: { opacity: 0.65 } } : {}),
      connectNulls: false, symbolSize: 5, smooth: false, emphasis: { focus: 'series' }, barMaxWidth: 45 })) };
}

export function mountChart(root: HTMLElement & { changeVariant?: (index: number) => void }): () => void {
  const variants: Variant[] = JSON.parse(root.dataset.variants ?? '[]');
  let spec: ChartSpec = JSON.parse(root.dataset.spec!);
  const host = root.querySelector<HTMLElement>('.metric-plot')!;
  let chart: echarts.ECharts | undefined;
  const select = root.querySelector<HTMLSelectElement>('[data-variant]');
  const abort = new AbortController();
  function render() {
    if (!chart && host.getBoundingClientRect().width) chart = echarts.init(host, undefined, { renderer: 'svg' });
    chart?.setOption(options(spec, root), true);
  }
  const observer = new IntersectionObserver(entries => { if (entries.some(e => e.isIntersecting)) { render(); observer.disconnect(); } }, { rootMargin: '200px' });
  observer.observe(host);
  const resize = new ResizeObserver(() => { if (chart) { chart.resize(); render(); } });
  resize.observe(host);
  const theme = new MutationObserver(() => { if (chart) render(); });
  theme.observe(document.documentElement, { attributes: true, attributeFilter: ['data-theme'] });
  function updateTable() {
    const table = document.createElement('table');
    const heading = table.createTHead().insertRow();
    for (const text of ['Category / date', ...spec.series.map(s => `${s.name} (${spec.unit})`)]) {
      const cell = document.createElement('th'); cell.textContent = text; heading.append(cell);
    }
    const body = table.createTBody();
    spec.labels.forEach((label, i) => {
      const row = body.insertRow(); row.insertCell().textContent = label;
      spec.series.forEach(s => { row.insertCell().textContent = s.values[i] === null ? 'Not recorded' : format(s.values[i]!); });
    });
    root.querySelector('[data-table]')!.replaceChildren(table);
  }
  root.changeVariant = index => {
    if (!variants[index]) return;
    spec = variants[index].spec;
    if (select) select.value = String(index);
    root.querySelector('[data-chart-title]')!.textContent = spec.title;
    root.querySelector('[data-chart-note]')!.textContent = spec.note ?? `Values in ${spec.unit}.`;
    host.setAttribute('aria-label', spec.title);
    updateTable(); render();
  };
  select?.addEventListener('change', () => {
    const index = Number(select.value); root.changeVariant!(index);
  }, { signal: abort.signal });
  root.addEventListener('click', event => {
    const type = (event.target as HTMLElement).closest<HTMLButtonElement>('[data-export]')?.dataset.export;
    if (!type) return;
    const link = document.createElement('a'); link.download = `${spec.id}.${type}`;
    if (type === 'svg') { render(); link.href = chart!.getDataURL({ type: 'svg' }); }
    else {
      const quote = (v: unknown) => `"${String(v ?? '').replaceAll('"', '""')}"`;
      const rows = [['Category / date', ...spec.series.map(s => `${s.name} (${spec.unit})`)], ...spec.labels.map((label, i) => [label, ...spec.series.map(s => s.values[i])])];
      link.href = URL.createObjectURL(new Blob([rows.map(r => r.map(quote).join(',')).join('\n')], { type: 'text/csv;charset=utf-8' }));
    }
    link.click(); if (type === 'csv') setTimeout(() => URL.revokeObjectURL(link.href), 0);
  }, { signal: abort.signal });
  return () => { abort.abort(); observer.disconnect(); resize.disconnect(); theme.disconnect(); chart?.dispose(); };
}
