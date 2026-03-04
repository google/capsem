# LayerChart Full Documentation for LLMs

> LayerChart is a powerful, composable charting library for Svelte built on top of D3.

This file contains the complete LLM-optimized documentation for all components and utilities.

## Guides

- [Getting Started](https://layerchart.com/docs/getting-started/llms.txt): Installation and setup guide for LayerChart
- [Layers](https://layerchart.com/docs/guides/layers/llms.txt): Documentation for Layers guide
- [LLMs](https://layerchart.com/docs/guides/LLMs/llms.txt): Documentation for LLMs guide
- [Primitives](https://layerchart.com/docs/guides/primitives/llms.txt): Documentation for Primitives guide
- [Scales](https://layerchart.com/docs/guides/scales/llms.txt): Documentation for Scales guide
- [Styling](https://layerchart.com/docs/guides/styles/llms.txt): Documentation for Styling guide

---

# Components

## AnnotationLine

Annotation component drawing a straight marker across the chart to indicate a specific value, trend, or threshold.

**Category:** annotations

**Supported Layers:** svg, canvas, html

## Usage

See example: horizontal

### API

| Property | Type | Default | Description |
|----------|------|---------|-------------|
| x | `SingleDomainType` | - | x value of the point |
| y | `SingleDomainType` | - | y value of the point |
| label | `string` | - | Label to display for line |
| labelPlacement | `Placement` | - | Placement of the label |
| labelXOffset | `number` | - | X offset of the label |
| labelYOffset | `number` | - | Y offset of the label |
| props | `{ label?: Partial&lt;ComponentProps&lt;typeof Text&gt;&gt;; line?: Partial&lt;ComponentProps&lt;typeof Line&gt;&gt;; }` | - | Classes for inner elements |
| fill | `string` | - | Fill color |
| fillOpacity | `number` | - | Fill opacity (0-1) |
| stroke | `string` | - | Stroke color |
| strokeWidth | `number` | - | Stroke width in pixels |
| strokeOpacity | `number` | - | Stroke opacity (0-1) |
| opacity | `number` | - | Overall opacity (0-1) |

**Extends:** `Without<SVGAttributes<Element>, AnnotationLinePropsWithoutHTML>`

### Examples

- [bar-chart](https://layerchart.com/docs/components/AnnotationLine/bar-chart)
- [horizontal](https://layerchart.com/docs/components/AnnotationLine/horizontal)
- [horizontal-placement](https://layerchart.com/docs/components/AnnotationLine/horizontal-placement)
- [horizontal-with-range](https://layerchart.com/docs/components/AnnotationLine/horizontal-with-range)
- [vertical](https://layerchart.com/docs/components/AnnotationLine/vertical)
- [vertical-placement](https://layerchart.com/docs/components/AnnotationLine/vertical-placement)
- [vertical-with-rotation](https://layerchart.com/docs/components/AnnotationLine/vertical-with-rotation)

### Related

- [AnnotationPoint](https://layerchart.com/docs/components/AnnotationPoint)
- [AnnotationRange](https://layerchart.com/docs/components/AnnotationRange)

## AnnotationPoint

Annotation component marking a specific data value or coordinate on a chart to highlight key events or notable points.

**Category:** annotations

**Supported Layers:** svg, canvas, html

## Usage

See example: line-to-point

### API

| Property | Type | Default | Description |
|----------|------|---------|-------------|
| x | `SingleDomainType` | - | x value of the point |
| y | `SingleDomainType` | - | y value of the point |
| r | `number` | - | Radius of the circle |
| label | `string` | - | Label to display on circle |
| labelPlacement | `Placement` | - | Placement of the label |
| labelXOffset | `number` | - | X offset of the label |
| labelYOffset | `number` | - | Y offset of the label |
| details | `any` | - | Details (description, etc) useful to display in tooltip |
| props | `{ label?: Partial&lt;ComponentProps&lt;typeof Text&gt;&gt;; circle?: Partial&lt;ComponentProps&lt;typeof Circle&gt;&gt;; }` | - | Classes for inner elements |
| fill | `string` | - | Fill color |
| fillOpacity | `number` | - | Fill opacity (0-1) |
| stroke | `string` | - | Stroke color |
| strokeWidth | `number` | - | Stroke width in pixels |
| strokeOpacity | `number` | - | Stroke opacity (0-1) |
| opacity | `number` | - | Overall opacity (0-1) |

**Extends:** `Without<SVGAttributes<Element>, AnnotationPointPropsWithoutHTML>`

### Examples

- [band-scale-on-axis](https://layerchart.com/docs/components/AnnotationPoint/band-scale-on-axis)
- [band-scale-on-value](https://layerchart.com/docs/components/AnnotationPoint/band-scale-on-value)
- [label-placement](https://layerchart.com/docs/components/AnnotationPoint/label-placement)
- [line-to-point](https://layerchart.com/docs/components/AnnotationPoint/line-to-point)
- [on-axis-with-tooltip](https://layerchart.com/docs/components/AnnotationPoint/on-axis-with-tooltip)
- [on-series-with-line-and-tooltip](https://layerchart.com/docs/components/AnnotationPoint/on-series-with-line-and-tooltip)
- [on-series-with-tooltip](https://layerchart.com/docs/components/AnnotationPoint/on-series-with-tooltip)
- [series-annotation](https://layerchart.com/docs/components/AnnotationPoint/series-annotation)

### Related

- [AnnotationLine](https://layerchart.com/docs/components/AnnotationLine)
- [AnnotationRange](https://layerchart.com/docs/components/AnnotationRange)

## AnnotationRange

Annotation component highlighting a continuous span or interval on a chart to emphasize specific data ranges or thresholds.

**Category:** annotations

**Supported Layers:** svg, canvas, html

## Usage

See example: vertical-with-pattern-range

### API

| Property | Type | Default | Description |
|----------|------|---------|-------------|
| x | `[ SingleDomainType, SingleDomainType ] \| SingleDomainType[]` | - | x values of the range |
| y | `[ SingleDomainType, SingleDomainType ] \| SingleDomainType[]` | - | y values of the range |
| label | `string` | - | Label to display for line |
| labelPlacement | `Placement` | - | Placement of the label |
| labelXOffset | `number` | - | X offset of the label |
| labelYOffset | `number` | - | Y offset of the label |
| fill | `string` | - | Add Rect with fill |
| class | `string` | - | Add Rect with class |
| gradient | `ComponentProps&lt;typeof LinearGradient&gt;` | - | Add Rect with gradient |
| pattern | `ComponentProps&lt;typeof Pattern&gt;` | - | Add Rect with pattern |
| props | `{ label?: Partial&lt;ComponentProps&lt;typeof Text&gt;&gt;; rect?: Partial&lt;ComponentProps&lt;typeof Rect&gt;&gt;; }` | - | Classes for inner elements |
| fill | `string` | - | Fill color |
| fillOpacity | `number` | - | Fill opacity (0-1) |
| stroke | `string` | - | Stroke color |
| strokeWidth | `number` | - | Stroke width in pixels |
| strokeOpacity | `number` | - | Stroke opacity (0-1) |
| opacity | `number` | - | Overall opacity (0-1) |

**Extends:** `Without<SVGAttributes<Element>, AnnotationRangePropsWithoutHTML>`

### Examples

- [bar-chart-(multiple)](https://layerchart.com/docs/components/AnnotationRange/bar-chart-(multiple))
- [bar-chart-(single)](https://layerchart.com/docs/components/AnnotationRange/bar-chart-(single))
- [bar-chart-(value)](https://layerchart.com/docs/components/AnnotationRange/bar-chart-(value))
- [hide-tooltip](https://layerchart.com/docs/components/AnnotationRange/hide-tooltip)
- [horizontal-with-fill-multiple](https://layerchart.com/docs/components/AnnotationRange/horizontal-with-fill-multiple)
- [horizontal-with-pattern-lower-bound](https://layerchart.com/docs/components/AnnotationRange/horizontal-with-pattern-lower-bound)
- [horizontal-with-pattern-range](https://layerchart.com/docs/components/AnnotationRange/horizontal-with-pattern-range)
- [horizontal-with-pattern-upper-bound](https://layerchart.com/docs/components/AnnotationRange/horizontal-with-pattern-upper-bound)
- [label-placement](https://layerchart.com/docs/components/AnnotationRange/label-placement)
- [vertical-with-gradient-range](https://layerchart.com/docs/components/AnnotationRange/vertical-with-gradient-range)
- [vertical-with-pattern-lower-bound](https://layerchart.com/docs/components/AnnotationRange/vertical-with-pattern-lower-bound)
- [vertical-with-pattern-range](https://layerchart.com/docs/components/AnnotationRange/vertical-with-pattern-range)
- [vertical-with-pattern-upper-bound](https://layerchart.com/docs/components/AnnotationRange/vertical-with-pattern-upper-bound)

### Related

- [AnnotationLine](https://layerchart.com/docs/components/AnnotationLine)
- [AnnotationPoint](https://layerchart.com/docs/components/AnnotationPoint)

## Arc

Primitive component which draws a curved segment on a chart to represent portions of a whole or highlight specific data ranges.

**Category:** primitives

**Supported Layers:** svg, canvas

> tip: See also: [ArcChart](/docs/components/ArcChart) and [PieChart](/docs/components/PieChart) for simplified examples

## Usage

See example: partial-arc

### Text along path

`Arc` can be used with the `children` snippet, `getArcTextProps`, and `Text` to write text along the `inner`, `outer`, or `middle` of the arc path.

The text will smartly orientate based on the direction (clockwise / counter-clockwise) and location (top, bottom, left, right) of the arc

See example: label-direction

### Playground

See example: playground

### API

| Property | Type | Default | Description |
|----------|------|---------|-------------|
| value | `number` | - | - |
| initialValue | `number` | - | - |
| domain | `[ number, number ]` | `[0, 100];` | Domain [min,max] in degrees |
| range | `[ number, number ]` | `[0, 360]` | Range [min,max] in degrees. See also startAngle/endAngle |
| startAngle | `number` | - | Start angle in radians |
| endAngle | `number` | - | End angle in radians |
| innerRadius | `number` | - | Define innerRadius. Defaults to yRange min   • value &gt;= 1: discrete value   • value &lt; 1: percent of `outerRadius`   • value &lt; 0: offset of `outerRadius` |
| outerRadius | `number` | - | Define outerRadius. Defaults to smallest width (xRange) or height (yRange) dimension (/2)   • value &gt;= 1: discrete value   • value &lt; 1: percent of chart width or height (smallest) / 2   • value &lt; 0: offset of chart width or height (smallest) / 2 |
| cornerRadius | `number` | `0` | Corner radius of the arc |
| padAngle | `number` | `0` | Angle between the arcs |
| trackStartAngle | `number` | - | Start angle in radians |
| trackEndAngle | `number` | - | End angle in radians |
| trackInnerRadius | `number` | - | Define innerRadius. Defaults to yRange min   • value &gt;= 1: discrete value   • value &lt; 1: percent of `outerRadius`   • value &lt; 0: offset of `outerRadius` |
| trackOuterRadius | `number` | - | Define outerRadius. Defaults to smallest width (xRange) or height (yRange) dimension (/2)   • value &gt;= 1: discrete value   • value &lt; 1: percent of chart width or height (smallest) / 2   • value &lt; 0: offset of chart width or height (smallest) / 2 |
| trackCornerRadius | `number` | `0` | Corner radius of the arc |
| trackPadAngle | `number` | `0` | Angle between the arcs |
| offset | `number` | `0` | Offset arc from center |
| tooltipContext | `TooltipContextValue` | - | Tooltip context to setup pointer events to show tooltip for related data.  **Must set `data` prop as well** |
| data | `any` | - | Data to set when showing tooltip |
| track | `boolean \| Partial&lt;ComponentProps&lt;typeof Path&gt;&gt;` | - | Pass true to enable the track with default props, or pass an object of props to enable the track. |
| trackRef | `SVGPathElement` | - | A reference to the track element |
| ref | `SVGPathElement` | - | A reference to the arc element |
| children | `Snippet&lt;[ { centroid: [ number, number ]; boundingBox: DOMRect; value: number; getTrackTextProps: GetArcTextProps; getArcTextProps: GetArcTextProps; } ]&gt;` | - | - |
| motion | `MotionProp` | - | - |
| fill | `string` | - | Fill color |
| fillOpacity | `number` | - | Fill opacity (0-1) |
| stroke | `string` | - | Stroke color |
| strokeWidth | `number` | - | Stroke width in pixels |
| strokeOpacity | `number` | - | Stroke opacity (0-1) |
| opacity | `number` | - | Overall opacity (0-1) |

**Extends:** `Without<SVGAttributes<SVGPathElement>, ArcPropsWithoutHTML & PathPropsWithoutHTML>`

### Examples

- [color-wheel](https://layerchart.com/docs/components/Arc/color-wheel)
- [concentric](https://layerchart.com/docs/components/Arc/concentric)
- [draggable-arc](https://layerchart.com/docs/components/Arc/draggable-arc)
- [label-direction](https://layerchart.com/docs/components/Arc/label-direction)
- [partial-arc](https://layerchart.com/docs/components/Arc/partial-arc)
- [playground](https://layerchart.com/docs/components/Arc/playground)
- [segmented-arc](https://layerchart.com/docs/components/Arc/segmented-arc)
- [segmented-arc-clip-mask](https://layerchart.com/docs/components/Arc/segmented-arc-clip-mask)
- [tween-value-on-mount](https://layerchart.com/docs/components/Arc/tween-value-on-mount)

### Related

- [Pie](https://layerchart.com/docs/components/Pie)
- [ArcChart](https://layerchart.com/docs/components/ArcChart)
- [PieChart](https://layerchart.com/docs/components/PieChart)

## ArcChart

Streamlined visualization which draws curved segment of a chart to represent portions of a circle, such as in pie or radial charts.

**Category:** charts

**Supported Layers:** svg, canvas

## Usage

See example: basic

### API

| Property | Type | Default | Description |
|----------|------|---------|-------------|
| key | `Accessor&lt;TData&gt;` | `'key'` | Key accessor |
| label | `Accessor&lt;TData&gt;` | `'label'` | Label accessor |
| value | `Accessor&lt;TData&gt;` | `'value'` | Value accessor |
| c | `Accessor&lt;TData&gt;` | `key` | Color accessor |
| maxValue | `number` | - | Maximum possible value, useful when `data` is single item |
| props | `ArcChartPropsObjProp` | - | - |
| placement | `'left' \| 'center' \| 'right'` | `'center'` | Placement of the ArcChart |
| center | `boolean` | `placement === 'center'` | Center the chart.  Override and use `props.group` for more control. |
| onArcClick | `(e: MouseEvent, detail: { data: any; series: SeriesData&lt;TData, typeof Arc&gt;; }) =&gt; void` | - | A callback function triggered when the arc is clicked. |
| arc | `SimplifiedChartSnippet&lt;TData, typeof Arc, ArcChartExtraSnippetProps&lt;TData&gt; & { props: ComponentProps&lt;typeof Arc&gt;; seriesIndex: number; }&gt;` | - | - |

**Extends:** `Pick<
    SimplifiedChartProps<TData, typeof Arc, ArcChartExtraSnippetProps<TData>>,
    | 'aboveContext'
    | 'aboveMarks'
    | 'belowContext'
    | 'belowMarks'
    | 'children'
    | 'data'
    | 'debug'
    | 'legend'
    | 'marks'
    | 'onTooltipClick'
    | 'profile'
    | 'layer'
    | 'series'
    | 'tooltip'
    | 'cRange'
    | 'padding'
    | 'context'
    | 'width'
    | 'height'
  >`

### Examples

- [basic](https://layerchart.com/docs/components/ArcChart/basic)
- [color](https://layerchart.com/docs/components/ArcChart/color)
- [gradient-with-text](https://layerchart.com/docs/components/ArcChart/gradient-with-text)
- [radius-fixed](https://layerchart.com/docs/components/ArcChart/radius-fixed)
- [radius-offset](https://layerchart.com/docs/components/ArcChart/radius-offset)
- [radius-percent](https://layerchart.com/docs/components/ArcChart/radius-percent)
- [series](https://layerchart.com/docs/components/ArcChart/series)
- [series-arc](https://layerchart.com/docs/components/ArcChart/series-arc)
- [series-individual](https://layerchart.com/docs/components/ArcChart/series-individual)
- [series-labels](https://layerchart.com/docs/components/ArcChart/series-labels)
- [series-motion-spring](https://layerchart.com/docs/components/ArcChart/series-motion-spring)
- [series-motion-tween](https://layerchart.com/docs/components/ArcChart/series-motion-tween)
- [series-start-180](https://layerchart.com/docs/components/ArcChart/series-start-180)
- [series-start-90](https://layerchart.com/docs/components/ArcChart/series-start-90)
- [series-track-color](https://layerchart.com/docs/components/ArcChart/series-track-color)
- [track-size](https://layerchart.com/docs/components/ArcChart/track-size)

### Related

- [Chart](https://layerchart.com/docs/components/Chart)
- [Pie](https://layerchart.com/docs/components/Pie)

## Area

Marking component which shades the space under a line on a chart to emphasize the magnitude and trend of data over a range.

**Category:** marks

**Supported Layers:** svg, canvas

::info
See also: [AreaChart](/docs/components/AreaChart) for simplified examples

## Usage

See example: basic

### Playground

See example: playground

### API

| Property | Type | Default | Description |
|----------|------|---------|-------------|
| data | `any` | - | Override data instead of using context |
| pathData | `string \| null` | - | Pass `&lt;path d={...} /&gt;` explicitly instead of calculating from data / context |
| x | `Accessor` | - | Override x accessor |
| y0 | `Accessor` | - | Override y0 accessor. Defaults to max($yRange) |
| y1 | `Accessor` | - | Override y1 accessor. Defaults to y accessor |
| motion | `MotionProp` | - | Whether to tween the interpolated path data using d3-interpolate-path |
| clipPath | `string` | - | - |
| curve | `CurveFactory` | - | - |
| defined | `Parameters&lt;D3Area&lt;any&gt;['defined']&gt;[0]` | - | - |
| line | `boolean \| Partial&lt;ComponentProps&lt;typeof Spline&gt;&gt;` | `false` | Enable showing line |
| fill | `string` | - | Fill color |
| fillOpacity | `number` | - | Fill opacity (0-1) |
| stroke | `string` | - | Stroke color |
| strokeWidth | `number` | - | Stroke width in pixels |
| strokeOpacity | `number` | - | Stroke opacity (0-1) |
| opacity | `number` | - | Overall opacity (0-1) |

**Extends:** `Without<SVGAttributes<SVGPathElement>, AreaPropsWithoutHTML>`

### Examples

- [basic](https://layerchart.com/docs/components/Area/basic)
- [clip-tween-on-mount](https://layerchart.com/docs/components/Area/clip-tween-on-mount)
- [clipped-area-on-tooltip](https://layerchart.com/docs/components/Area/clipped-area-on-tooltip)
- [explicit-axis-ticks-min-max](https://layerchart.com/docs/components/Area/explicit-axis-ticks-min-max)
- [gradient](https://layerchart.com/docs/components/Area/gradient)
- [gradient-separate-stroke](https://layerchart.com/docs/components/Area/gradient-separate-stroke)
- [highlight-color-based-on-value-using-color-scale](https://layerchart.com/docs/components/Area/highlight-color-based-on-value-using-color-scale)
- [highlight-color-based-on-value-using-tooltip-slot-prop](https://layerchart.com/docs/components/Area/highlight-color-based-on-value-using-tooltip-slot-prop)
- [multiple-series](https://layerchart.com/docs/components/Area/multiple-series)
- [multiple-series-highlight-on-hover](https://layerchart.com/docs/components/Area/multiple-series-highlight-on-hover)
- [multiple-series-using-overrides](https://layerchart.com/docs/components/Area/multiple-series-using-overrides)
- [multiple-series-with-labels](https://layerchart.com/docs/components/Area/multiple-series-with-labels)
- [playground](https://layerchart.com/docs/components/Area/playground)
- [stack](https://layerchart.com/docs/components/Area/stack)
- [stack-with-gradient](https://layerchart.com/docs/components/Area/stack-with-gradient)
- [threshold-with-lineargradient](https://layerchart.com/docs/components/Area/threshold-with-lineargradient)
- [threshold-with-lineargradient-over-under](https://layerchart.com/docs/components/Area/threshold-with-lineargradient-over-under)
- [threshold-with-rectclippath](https://layerchart.com/docs/components/Area/threshold-with-rectclippath)
- [threshold-with-rectclippath-over-under](https://layerchart.com/docs/components/Area/threshold-with-rectclippath-over-under)
- [with-labels](https://layerchart.com/docs/components/Area/with-labels)
- [with-tooltip-and-highlight](https://layerchart.com/docs/components/Area/with-tooltip-and-highlight)

## AreaChart

Visualization of quantitative data over a continuous interval, with filled areas beneath a line to emphasize magnitude or cumulative values.

**Category:** charts

**Supported Layers:** svg, canvas

## Usage

See example: basic

### API

| Property | Type | Default | Description |
|----------|------|---------|-------------|
| onPointClick | `(e: MouseEvent, details: { data: HighlightPointData; series: SeriesData&lt;TData, typeof Area&gt;; }) =&gt; void` | - | A callback function called when a point in the chart is clicked. |
| props | `AreaChartPropsObjProp` | - | Additional props to be passed to the components rendered internally by the `AreaChart` component. This is useful for customizing the behavior of the individual components, without having to fully override them via a snippet. |

**Extends:** `SimplifiedChartProps<
    TData,
    typeof Area,
    AreaChartExtraSnippetProps<TData>
  >`

### Examples

- [band-scale](https://layerchart.com/docs/components/AreaChart/band-scale)
- [basic](https://layerchart.com/docs/components/AreaChart/basic)
- [brush](https://layerchart.com/docs/components/AreaChart/brush)
- [brush-sync](https://layerchart.com/docs/components/AreaChart/brush-sync)
- [curve](https://layerchart.com/docs/components/AreaChart/curve)
- [custom](https://layerchart.com/docs/components/AreaChart/custom)
- [custom-tooltip](https://layerchart.com/docs/components/AreaChart/custom-tooltip)
- [default-series-label](https://layerchart.com/docs/components/AreaChart/default-series-label)
- [funnel](https://layerchart.com/docs/components/AreaChart/funnel)
- [gradient](https://layerchart.com/docs/components/AreaChart/gradient)
- [labels](https://layerchart.com/docs/components/AreaChart/labels)
- [line-annotation](https://layerchart.com/docs/components/AreaChart/line-annotation)
- [markers](https://layerchart.com/docs/components/AreaChart/markers)
- [null-gaps](https://layerchart.com/docs/components/AreaChart/null-gaps)
- [point-annotations](https://layerchart.com/docs/components/AreaChart/point-annotations)
- [point-scale](https://layerchart.com/docs/components/AreaChart/point-scale)
- [points](https://layerchart.com/docs/components/AreaChart/points)
- [radial](https://layerchart.com/docs/components/AreaChart/radial)
- [range-annotation](https://layerchart.com/docs/components/AreaChart/range-annotation)
- [series](https://layerchart.com/docs/components/AreaChart/series)
- [series-individual-tooltip](https://layerchart.com/docs/components/AreaChart/series-individual-tooltip)
- [series-point-annotations](https://layerchart.com/docs/components/AreaChart/series-point-annotations)
- [series-point-click](https://layerchart.com/docs/components/AreaChart/series-point-click)
- [series-separate-data](https://layerchart.com/docs/components/AreaChart/series-separate-data)
- [series-stack](https://layerchart.com/docs/components/AreaChart/series-stack)
- [series-stack-diverging](https://layerchart.com/docs/components/AreaChart/series-stack-diverging)
- [series-stack-expand](https://layerchart.com/docs/components/AreaChart/series-stack-expand)
- [series-stack-gradient](https://layerchart.com/docs/components/AreaChart/series-stack-gradient)
- [series-stack-legend](https://layerchart.com/docs/components/AreaChart/series-stack-legend)
- [series-stack-legend-labels](https://layerchart.com/docs/components/AreaChart/series-stack-legend-labels)
- [series-stack-legend-placement](https://layerchart.com/docs/components/AreaChart/series-stack-legend-placement)
- [series-stack-separate-data](https://layerchart.com/docs/components/AreaChart/series-stack-separate-data)
- [series-tooltip-click](https://layerchart.com/docs/components/AreaChart/series-tooltip-click)
- [single-axis-x](https://layerchart.com/docs/components/AreaChart/single-axis-x)
- [single-axis-y](https://layerchart.com/docs/components/AreaChart/single-axis-y)
- [sparkline](https://layerchart.com/docs/components/AreaChart/sparkline)
- [threshold](https://layerchart.com/docs/components/AreaChart/threshold)
- [threshold-gradient](https://layerchart.com/docs/components/AreaChart/threshold-gradient)
- [tooltip-external](https://layerchart.com/docs/components/AreaChart/tooltip-external)
- [tooltip-fixed-with-hide-delay](https://layerchart.com/docs/components/AreaChart/tooltip-fixed-with-hide-delay)
- [tooltip-locking](https://layerchart.com/docs/components/AreaChart/tooltip-locking)

### Related

- [Chart](https://layerchart.com/docs/components/Chart)
- [Area](https://layerchart.com/docs/components/Area)

## Async

Use `experimental.async` to load each example and source

**Category:** examples

```svelte
<script lang="ts">
	import AsyncExample from '$lib/components/AsyncExample.svelte';
</script>

<AsyncExample component="AreaChart" name="basic" />
```

## Example

## Issues

- Issue with initial page load (SSR?, but is disabled 🤔)
- Issue with build (`pnpm build && pnpm preview`)
  - [set_context_after_init](https://svelte.dev/docs/svelte/runtime-errors#Client-errors-set_context_after_init)
- Need to determine how to load contents for search index

## Axis

Commonly used component displays the scale and reference lines on a chart, providing context for interpreting data values.

**Category:** common

**Supported Layers:** svg, canvas, html

## Usage

See example: placement-bottom-left

### tickSpacing

If using a continuous scales (ex. linear, time) and ticks become too crowded, you can use `tickSpacing` to control the number of pixels alloted for each tick (higher => fewer ticks).

> note: Default: `80` for horizontal axes (top/bottom/angle) and `50` for vertical axes (left/right/radius).

See example: linechart-tickspacing

> tip: See also: time scale [auto](/docs/components/Axis/time-scale-auto), [multiline](/docs/components/Axis/time-scale-auto-multiline), and [brush](/docs/components/Axis/time-scale-brush-multiline) examples

### band scales

When creating time-series bar charts, it can be useful to use a time scale axis instead of a bar scale axis. This helps show gaps in data (such as on [weekends](/docs/components/BarChart/time-scale-interval)) and provides improved axis ticks.

To enable this, you must define the interval (daily, hourly, etc) of your data using [d3-time interval](https://d3js.org/d3-time#timeMillisecond), such as `xInterval={timeDay}`.

Since band padding is not available when not using a band scale, you can leverage `xInset={...}` to add padding between bars.

See example: barchart-xinterval-xinset

### API

| Property | Type | Default | Description |
|----------|------|---------|-------------|
| **placement** (required) | `'top' \| 'bottom' \| 'left' \| 'right' \| 'angle' \| 'radius'` | - | Location of axis |
| label | `string \| Snippet&lt;[ { props: ComponentProps&lt;typeof Text&gt;; } ]&gt;` | - | The label for the axis.  Can either be a string or a snippet to render custom content. The snippet receives spreadable props to apply to the label. |
| labelPlacement | `'start' \| 'middle' \| 'end'` | `'middle'` | Location of axis label |
| labelProps | `Partial&lt;ComponentProps&lt;typeof Text&gt;&gt;` | - | Props applied to label Text |
| rule | `boolean \| Partial&lt;ComponentProps&lt;typeof Rule&gt;&gt;` | `false` | Draw a rule line. Use Rule component for greater rendering order control |
| grid | `boolean \| Pick&lt;SVGAttributes&lt;SVGElement&gt;, 'class' \| 'style'&gt;` | `false` | Draw grid lines |
| ticks | `TicksConfig` | - | Control the number of ticks |
| tickSpacing | `number` | `80 (top\|bottom\|angle) or 50 (left\|right\|radius)` | Width or height of each tick in pixels (enabling responsive count) |
| tickMultiline | `boolean` | `false` | Whether to render tick labels on multiple lines for additional context |
| tickLength | `number` | `4` | Length of the tick line |
| tickMarks | `boolean` | `true` | Whether to render tick marks. |
| format | `FormatType \| FormatConfig` | - | Format tick labels |
| tickLabelProps | `Partial&lt;ComponentProps&lt;typeof Text&gt;&gt;` | - | Props to apply to each tick label |
| tickLabel | `Snippet&lt;[ { props: ComponentProps&lt;typeof Text&gt;; index: number; } ]&gt;` | - | A snippet to render your own custom tick label. |
| transitionIn | `In` | `defaults to fade if the motion prop is set to tweened` | Transition function for entering elements |
| transitionInParams | `TransitionParams&lt;In&gt;` | `{ easing: cubicIn }` | Parameters for the transitionIn function |
| scale | `any` | - | Override scale for the axis |
| classes | `{ root?: string; label?: string; rule?: string; tick?: string; tickLabel?: string; }` | `{}` | Classes for styling various parts of the axis |
| motion | `MotionProp` | - | - |

**Extends:** `Without<GroupProps, AxisPropsWithoutHTML<In>>`

### Examples

- [arrow-markers](https://layerchart.com/docs/components/Axis/arrow-markers)
- [axis-label-placement-left-right](https://layerchart.com/docs/components/Axis/axis-label-placement-left-right)
- [axis-label-placement-top-bottom](https://layerchart.com/docs/components/Axis/axis-label-placement-top-bottom)
- [barchart-xinterval-xinset](https://layerchart.com/docs/components/Axis/barchart-xinterval-xinset)
- [explicit-ticks](https://layerchart.com/docs/components/Axis/explicit-ticks)
- [extent-ticks-only](https://layerchart.com/docs/components/Axis/extent-ticks-only)
- [grid](https://layerchart.com/docs/components/Axis/grid)
- [grid-dashed](https://layerchart.com/docs/components/Axis/grid-dashed)
- [hide-zero-filter](https://layerchart.com/docs/components/Axis/hide-zero-filter)
- [hide-zero-format](https://layerchart.com/docs/components/Axis/hide-zero-format)
- [inject-ticks](https://layerchart.com/docs/components/Axis/inject-ticks)
- [integer-only-filter](https://layerchart.com/docs/components/Axis/integer-only-filter)
- [integer-only-format](https://layerchart.com/docs/components/Axis/integer-only-format)
- [labels-next-hash](https://layerchart.com/docs/components/Axis/labels-next-hash)
- [linechart-tickspacing](https://layerchart.com/docs/components/Axis/linechart-tickspacing)
- [log-scale](https://layerchart.com/docs/components/Axis/log-scale)
- [multiline-tick-labels](https://layerchart.com/docs/components/Axis/multiline-tick-labels)
- [multiple-axis-grid-and-rules](https://layerchart.com/docs/components/Axis/multiple-axis-grid-and-rules)
- [multiple-axis-grid-and-rules-separate-grid](https://layerchart.com/docs/components/Axis/multiple-axis-grid-and-rules-separate-grid)
- [multiple-axis-grid-with-single-rule](https://layerchart.com/docs/components/Axis/multiple-axis-grid-with-single-rule)
- [multiple-axis-same-placement-bottom](https://layerchart.com/docs/components/Axis/multiple-axis-same-placement-bottom)
- [multiple-axis-same-placement-right](https://layerchart.com/docs/components/Axis/multiple-axis-same-placement-right)
- [override-axis-ticks-scale](https://layerchart.com/docs/components/Axis/override-axis-ticks-scale)
- [placement-bottom-left](https://layerchart.com/docs/components/Axis/placement-bottom-left)
- [placement-bottom-left-rule](https://layerchart.com/docs/components/Axis/placement-bottom-left-rule)
- [placement-top-right](https://layerchart.com/docs/components/Axis/placement-top-right)
- [placement-top-right-rule](https://layerchart.com/docs/components/Axis/placement-top-right-rule)
- [radial-grid](https://layerchart.com/docs/components/Axis/radial-grid)
- [radial-rule](https://layerchart.com/docs/components/Axis/radial-rule)
- [remove-tick-marks](https://layerchart.com/docs/components/Axis/remove-tick-marks)
- [rotate-labels](https://layerchart.com/docs/components/Axis/rotate-labels)
- [tick-count](https://layerchart.com/docs/components/Axis/tick-count)
- [tick-label-styling](https://layerchart.com/docs/components/Axis/tick-label-styling)
- [tick-spacing](https://layerchart.com/docs/components/Axis/tick-spacing)
- [time-scale-auto](https://layerchart.com/docs/components/Axis/time-scale-auto)
- [time-scale-auto-format-filtering](https://layerchart.com/docs/components/Axis/time-scale-auto-format-filtering)
- [time-scale-auto-multiline](https://layerchart.com/docs/components/Axis/time-scale-auto-multiline)
- [time-scale-brush](https://layerchart.com/docs/components/Axis/time-scale-brush)
- [time-scale-brush-multiline](https://layerchart.com/docs/components/Axis/time-scale-brush-multiline)
- [time-scale-explicit](https://layerchart.com/docs/components/Axis/time-scale-explicit)
- [time-scale-explicit-multiline](https://layerchart.com/docs/components/Axis/time-scale-explicit-multiline)

### Related

- [Grid](https://layerchart.com/docs/components/Grid)
- [Rule](https://layerchart.com/docs/components/Rule)

## Bar

Primitive component creating individual rectangular bars to represent and compare discrete data values.

**Category:** primitives

**Supported Layers:** svg, canvas

## Usage

:example{ component="Bars" name="vertical-customize-individual-styles" showCode }

Typically the component is rendering within the `Bars` mark but can be rendered explicitly when you need more control on a per-mark basis

### API

| Property | Type | Default | Description |
|----------|------|---------|-------------|
| **data** (required) | `Object` | - | data to render the bar from |
| x | `Accessor` | `ctx.x` | Override `x` from context. Useful for multiple Bar instances |
| y | `Accessor` | `ctx.y` | Override `y` from context. Useful for multiple Bar instances |
| x1 | `Accessor` | `ctx.x1` | Override `x1` from context. Useful for multiple Bar instances |
| y1 | `Accessor` | `ctx.y1` | Override `y1` from context. Useful for multiple Bar instances |
| radius | `number` | - | - |
| insets | `Insets` | - | - |
| initialX | `number` | - | - |
| initialY | `number` | - | - |
| initialHeight | `number` | - | - |
| initialWidth | `number` | - | - |
| rounded | `'all' \| 'none' \| 'edge' \| 'top' \| 'bottom' \| 'left' \| 'right' \| 'top-left' \| 'top-right' \| 'bottom-left' \| 'bottom-right'` | - | Control which corners are rounded with radius. Uses &lt;path&gt; instead of &lt;rect&gt; when not set to `all` |
| motion | `MotionProp&lt;'x' \| 'y' \| 'width' \| 'height'&gt;` | - | - |
| fill | `string` | - | Fill color |
| fillOpacity | `number` | - | Fill opacity (0-1) |
| stroke | `string` | - | Stroke color |
| strokeWidth | `number` | - | Stroke width in pixels |
| strokeOpacity | `number` | - | Stroke opacity (0-1) |
| opacity | `number` | - | Overall opacity (0-1) |
| onclick | `MouseEventHandler&lt;Element&gt; \| null` | - | Click event handler |
| ondblclick | `MouseEventHandler&lt;Element&gt; \| null` | - | Double click event handler |
| onpointerenter | `PointerEventHandler&lt;Element&gt; \| null` | - | Pointer enter event handler |
| onpointermove | `PointerEventHandler&lt;Element&gt; \| null` | - | Pointer move event handler |
| onpointerleave | `PointerEventHandler&lt;Element&gt; \| null` | - | Pointer leave event handler |
| onpointerover | `PointerEventHandler&lt;Element&gt; \| null` | - | Pointer over event handler |
| onpointerout | `PointerEventHandler&lt;Element&gt; \| null` | - | Pointer out event handler |

**Extends:** `Without<
      Omit<SVGAttributes<SVGElement>, 'width' | 'height' | 'x' | 'y' | 'offset'>,
      BarPropsWithoutHTML
    >`, `CommonEvents`

### Related

- [Bars](https://layerchart.com/docs/components/Bars)

## BarChart

Streamlined visualization displaying categorical data using rectangular bars whose lengths represent the values of each category.

**Category:** charts

**Supported Layers:** svg, canvas

## Usage

See example: vertical-default

> tip: See also: [Axis](/docs/components/Axis) for examples of using time scale axes with bar charts

### API

| Property | Type | Default | Description |
|----------|------|---------|-------------|
| bandPadding | `number` | `0.4` | Padding between primary x or y bands/bars, applied to scaleBand().padding() |
| groupPadding | `number` | `0` | Padding between group/series items when using 'seriesLayout="group"', applied to scaleBand().padding() |
| stackPadding | `number` | `0` | Padding between series items within bars when using 'seriesLayout="stack"' |
| orientation | `'vertical' \| 'horizontal'` | `'vertical'` | The orientation of the bar chart. |
| props | `BarChartPropsObjProp&lt;TData&gt;` | - | - |
| onBarClick | `(event: MouseEvent, detail: { data: any; series: SeriesData&lt;TData, typeof Bars&gt;; }) =&gt; void` | - | A callback function that is called when a bar is clicked. |
| seriesLayout | `SimplifiedChartProps&lt;TData, typeof Bars, BarChartExtraSnippetProps&lt;TData&gt;&gt;['seriesLayout'] \| 'group'` | `'overlap'` | The layout of the series. |

**Extends:** `Omit<
    SimplifiedChartProps<TData, typeof Bars, BarChartExtraSnippetProps<TData>>,
    'seriesLayout'
  >`

### Examples

- [axis-labels-inside-bars](https://layerchart.com/docs/components/BarChart/axis-labels-inside-bars)
- [axis-labels-inside-bars-using-labels](https://layerchart.com/docs/components/BarChart/axis-labels-inside-bars-using-labels)
- [both-axis-grid](https://layerchart.com/docs/components/BarChart/both-axis-grid)
- [both-axis-grid-align-between](https://layerchart.com/docs/components/BarChart/both-axis-grid-align-between)
- [brushing](https://layerchart.com/docs/components/BarChart/brushing)
- [color-bars-class](https://layerchart.com/docs/components/BarChart/color-bars-class)
- [color-per-value](https://layerchart.com/docs/components/BarChart/color-per-value)
- [color-threshold](https://layerchart.com/docs/components/BarChart/color-threshold)
- [color-using-scale](https://layerchart.com/docs/components/BarChart/color-using-scale)
- [custom-chart](https://layerchart.com/docs/components/BarChart/custom-chart)
- [custom-tooltip](https://layerchart.com/docs/components/BarChart/custom-tooltip)
- [duration](https://layerchart.com/docs/components/BarChart/duration)
- [duration-bars](https://layerchart.com/docs/components/BarChart/duration-bars)
- [duration-bars-color](https://layerchart.com/docs/components/BarChart/duration-bars-color)
- [duration-bars-dense](https://layerchart.com/docs/components/BarChart/duration-bars-dense)
- [duration-bars-dense-lanes](https://layerchart.com/docs/components/BarChart/duration-bars-dense-lanes)
- [duration-bars-lanes](https://layerchart.com/docs/components/BarChart/duration-bars-lanes)
- [duration-civilization-timeline](https://layerchart.com/docs/components/BarChart/duration-civilization-timeline)
- [duration-civilization-timeline-dense](https://layerchart.com/docs/components/BarChart/duration-civilization-timeline-dense)
- [duration-points](https://layerchart.com/docs/components/BarChart/duration-points)
- [duration-points-color](https://layerchart.com/docs/components/BarChart/duration-points-color)
- [dynamic-height](https://layerchart.com/docs/components/BarChart/dynamic-height)
- [gradient](https://layerchart.com/docs/components/BarChart/gradient)
- [group-series](https://layerchart.com/docs/components/BarChart/group-series)
- [group-series-bar-click](https://layerchart.com/docs/components/BarChart/group-series-bar-click)
- [group-series-horizontal](https://layerchart.com/docs/components/BarChart/group-series-horizontal)
- [group-series-labels](https://layerchart.com/docs/components/BarChart/group-series-labels)
- [group-series-series-long-data](https://layerchart.com/docs/components/BarChart/group-series-series-long-data)
- [highlight-below-marks](https://layerchart.com/docs/components/BarChart/highlight-below-marks)
- [histogram-date-time-count](https://layerchart.com/docs/components/BarChart/histogram-date-time-count)
- [histogram-date-time-interval](https://layerchart.com/docs/components/BarChart/histogram-date-time-interval)
- [histogram-horizontal](https://layerchart.com/docs/components/BarChart/histogram-horizontal)
- [histogram-random-distribution](https://layerchart.com/docs/components/BarChart/histogram-random-distribution)
- [histogram-vertical](https://layerchart.com/docs/components/BarChart/histogram-vertical)
- [horizontal](https://layerchart.com/docs/components/BarChart/horizontal)
- [labels](https://layerchart.com/docs/components/BarChart/labels)
- [labels-inside-placement](https://layerchart.com/docs/components/BarChart/labels-inside-placement)
- [legend-custom-labels](https://layerchart.com/docs/components/BarChart/legend-custom-labels)
- [legend-group-series](https://layerchart.com/docs/components/BarChart/legend-group-series)
- [legend-placement](https://layerchart.com/docs/components/BarChart/legend-placement)
- [legend-stack-series](https://layerchart.com/docs/components/BarChart/legend-stack-series)
- [line-annotation](https://layerchart.com/docs/components/BarChart/line-annotation)
- [oscilloscope-frequency](https://layerchart.com/docs/components/BarChart/oscilloscope-frequency)
- [override-axis-ticks-with-custom-scale](https://layerchart.com/docs/components/BarChart/override-axis-ticks-with-custom-scale)
- [point-annotation](https://layerchart.com/docs/components/BarChart/point-annotation)
- [radial-horizontal](https://layerchart.com/docs/components/BarChart/radial-horizontal)
- [radial-horizontal-color-per-value](https://layerchart.com/docs/components/BarChart/radial-horizontal-color-per-value)
- [radial-horizontal-duration](https://layerchart.com/docs/components/BarChart/radial-horizontal-duration)
- [radial-horizontal-grid-between](https://layerchart.com/docs/components/BarChart/radial-horizontal-grid-between)
- [radial-vertical](https://layerchart.com/docs/components/BarChart/radial-vertical)
- [radial-vertical-arcpadding](https://layerchart.com/docs/components/BarChart/radial-vertical-arcpadding)
- [radial-vertical-yrange](https://layerchart.com/docs/components/BarChart/radial-vertical-yrange)
- [radial-weather](https://layerchart.com/docs/components/BarChart/radial-weather)
- [range-annotation-multiple](https://layerchart.com/docs/components/BarChart/range-annotation-multiple)
- [range-annotation-single](https://layerchart.com/docs/components/BarChart/range-annotation-single)
- [range-annotation-value](https://layerchart.com/docs/components/BarChart/range-annotation-value)
- [remove-rounding](https://layerchart.com/docs/components/BarChart/remove-rounding)
- [scale-override](https://layerchart.com/docs/components/BarChart/scale-override)
- [series](https://layerchart.com/docs/components/BarChart/series)
- [series-data](https://layerchart.com/docs/components/BarChart/series-data)
- [series-diverging](https://layerchart.com/docs/components/BarChart/series-diverging)
- [series-horizontal](https://layerchart.com/docs/components/BarChart/series-horizontal)
- [series-horizontal-diverging](https://layerchart.com/docs/components/BarChart/series-horizontal-diverging)
- [series-horizontal-diverging-as-percent](https://layerchart.com/docs/components/BarChart/series-horizontal-diverging-as-percent)
- [single-axis-x](https://layerchart.com/docs/components/BarChart/single-axis-x)
- [single-axis-y](https://layerchart.com/docs/components/BarChart/single-axis-y)
- [single-dimension](https://layerchart.com/docs/components/BarChart/single-dimension)
- [single-stack-with-indicator](https://layerchart.com/docs/components/BarChart/single-stack-with-indicator)
- [sparkbar](https://layerchart.com/docs/components/BarChart/sparkbar)
- [sparkbar-fixed-position-tooltip](https://layerchart.com/docs/components/BarChart/sparkbar-fixed-position-tooltip)
- [sparkbar-negative-data](https://layerchart.com/docs/components/BarChart/sparkbar-negative-data)
- [sparkbar-within-a-paragraph](https://layerchart.com/docs/components/BarChart/sparkbar-within-a-paragraph)
- [sparkbar-within-a-paragraph-with-tooltip-and-highlight](https://layerchart.com/docs/components/BarChart/sparkbar-within-a-paragraph-with-tooltip-and-highlight)
- [stack-series](https://layerchart.com/docs/components/BarChart/stack-series)
- [stack-series-diverging](https://layerchart.com/docs/components/BarChart/stack-series-diverging)
- [stack-series-expand](https://layerchart.com/docs/components/BarChart/stack-series-expand)
- [stack-series-horizontal](https://layerchart.com/docs/components/BarChart/stack-series-horizontal)
- [stack-series-padded](https://layerchart.com/docs/components/BarChart/stack-series-padded)
- [time-scale-interval](https://layerchart.com/docs/components/BarChart/time-scale-interval)
- [time-scale-interval-horizontal](https://layerchart.com/docs/components/BarChart/time-scale-interval-horizontal)
- [time-scale-interval-with-inset](https://layerchart.com/docs/components/BarChart/time-scale-interval-with-inset)
- [tooltip-click](https://layerchart.com/docs/components/BarChart/tooltip-click)
- [vertical-default](https://layerchart.com/docs/components/BarChart/vertical-default)

### Related

- [Chart](https://layerchart.com/docs/components/Chart)
- [Bars](https://layerchart.com/docs/components/Bars)

## Bars

Marking component which applies horizontal bars to represent and visually compare discrete data values.

**Category:** marks

**Supported Layers:** svg, canvas

> tip: See also: [BarChart](/docs/components/BarChart) for simplified examples

## Usage

See example: vertical-basic

### API

| Property | Type | Default | Description |
|----------|------|---------|-------------|
| data | `any` | - | Override the data from the context. |
| key | `(d: any, index: number) =&gt; any` | `(d, index) =&gt; index` | Define unique value for {#each} `(key)` expressions to improve transitions. |
| onBarClick | `(e: MouseEvent, detail: { data: any; }) =&gt; void` | - | Event dispatched when an individual bar is clicked. |
| children | `Snippet` | - | - |

**Extends:** `Omit<BarProps, 'data'>`

### Examples

- [horizontal-average-annotation-rule](https://layerchart.com/docs/components/Bars/horizontal-average-annotation-rule)
- [horizontal-basic](https://layerchart.com/docs/components/Bars/horizontal-basic)
- [horizontal-calculated-value-domain-negative](https://layerchart.com/docs/components/Bars/horizontal-calculated-value-domain-negative)
- [horizontal-calculated-value-domain-positive](https://layerchart.com/docs/components/Bars/horizontal-calculated-value-domain-positive)
- [horizontal-candlestick-bars](https://layerchart.com/docs/components/Bars/horizontal-candlestick-bars)
- [horizontal-click-handler](https://layerchart.com/docs/components/Bars/horizontal-click-handler)
- [horizontal-customize-individual-styles](https://layerchart.com/docs/components/Bars/horizontal-customize-individual-styles)
- [horizontal-gradient](https://layerchart.com/docs/components/Bars/horizontal-gradient)
- [horizontal-grouped](https://layerchart.com/docs/components/Bars/horizontal-grouped)
- [horizontal-grouped-and-stacked](https://layerchart.com/docs/components/Bars/horizontal-grouped-and-stacked)
- [horizontal-grouped-stacked-or-both-transition](https://layerchart.com/docs/components/Bars/horizontal-grouped-stacked-or-both-transition)
- [horizontal-highlight-individual-bar](https://layerchart.com/docs/components/Bars/horizontal-highlight-individual-bar)
- [horizontal-highlight-individual-bar-line](https://layerchart.com/docs/components/Bars/horizontal-highlight-individual-bar-line)
- [horizontal-inside-labels](https://layerchart.com/docs/components/Bars/horizontal-inside-labels)
- [horizontal-limit-ticks-count](https://layerchart.com/docs/components/Bars/horizontal-limit-ticks-count)
- [horizontal-limit-ticks-second-scale](https://layerchart.com/docs/components/Bars/horizontal-limit-ticks-second-scale)
- [horizontal-multiple-diverging](https://layerchart.com/docs/components/Bars/horizontal-multiple-diverging)
- [horizontal-multiple-overlapping](https://layerchart.com/docs/components/Bars/horizontal-multiple-overlapping)
- [horizontal-outside-labels-default](https://layerchart.com/docs/components/Bars/horizontal-outside-labels-default)
- [horizontal-rounded-right-only](https://layerchart.com/docs/components/Bars/horizontal-rounded-right-only)
- [horizontal-stacked](https://layerchart.com/docs/components/Bars/horizontal-stacked)
- [horizontal-stacked-percent](https://layerchart.com/docs/components/Bars/horizontal-stacked-percent)
- [horizontal-stagger-tween-on-mount](https://layerchart.com/docs/components/Bars/horizontal-stagger-tween-on-mount)
- [horizontal-time-scale-with-inset](https://layerchart.com/docs/components/Bars/horizontal-time-scale-with-inset)
- [horizontal-time-scale-with-interval](https://layerchart.com/docs/components/Bars/horizontal-time-scale-with-interval)
- [horizontal-time-scale-with-missing-data](https://layerchart.com/docs/components/Bars/horizontal-time-scale-with-missing-data)
- [horizontal-tooltip-and-bar-highlight](https://layerchart.com/docs/components/Bars/horizontal-tooltip-and-bar-highlight)
- [horizontal-tooltip-and-click-handlers-for-individual-stack-grouped-bar](https://layerchart.com/docs/components/Bars/horizontal-tooltip-and-click-handlers-for-individual-stack-grouped-bar)
- [horizontal-tooltip-and-clipped-highlight](https://layerchart.com/docs/components/Bars/horizontal-tooltip-and-clipped-highlight)
- [horizontal-tooltip-and-highlight](https://layerchart.com/docs/components/Bars/horizontal-tooltip-and-highlight)
- [horizontal-tween-on-mount](https://layerchart.com/docs/components/Bars/horizontal-tween-on-mount)
- [horizontal-with-grid-on-top](https://layerchart.com/docs/components/Bars/horizontal-with-grid-on-top)
- [horizontal-with-grid-on-top-mix-blend](https://layerchart.com/docs/components/Bars/horizontal-with-grid-on-top-mix-blend)
- [vertical-average-annotation-rule](https://layerchart.com/docs/components/Bars/vertical-average-annotation-rule)
- [vertical-basic](https://layerchart.com/docs/components/Bars/vertical-basic)
- [vertical-calculated-value-domain-negative](https://layerchart.com/docs/components/Bars/vertical-calculated-value-domain-negative)
- [vertical-calculated-value-domain-positive](https://layerchart.com/docs/components/Bars/vertical-calculated-value-domain-positive)
- [vertical-click-handler](https://layerchart.com/docs/components/Bars/vertical-click-handler)
- [vertical-customize-individual-styles](https://layerchart.com/docs/components/Bars/vertical-customize-individual-styles)
- [vertical-gradient](https://layerchart.com/docs/components/Bars/vertical-gradient)
- [vertical-grouped](https://layerchart.com/docs/components/Bars/vertical-grouped)
- [vertical-grouped-and-stacked](https://layerchart.com/docs/components/Bars/vertical-grouped-and-stacked)
- [vertical-grouped-stacked-or-both-transition](https://layerchart.com/docs/components/Bars/vertical-grouped-stacked-or-both-transition)
- [vertical-highlight-individual-bar](https://layerchart.com/docs/components/Bars/vertical-highlight-individual-bar)
- [vertical-highlight-individual-bar-line](https://layerchart.com/docs/components/Bars/vertical-highlight-individual-bar-line)
- [vertical-inside-labels](https://layerchart.com/docs/components/Bars/vertical-inside-labels)
- [vertical-limit-ticks-count](https://layerchart.com/docs/components/Bars/vertical-limit-ticks-count)
- [vertical-limit-ticks-second-scale](https://layerchart.com/docs/components/Bars/vertical-limit-ticks-second-scale)
- [vertical-multiple-diverging](https://layerchart.com/docs/components/Bars/vertical-multiple-diverging)
- [vertical-multiple-diverging-rounded-specific](https://layerchart.com/docs/components/Bars/vertical-multiple-diverging-rounded-specific)
- [vertical-multiple-overlapping](https://layerchart.com/docs/components/Bars/vertical-multiple-overlapping)
- [vertical-outside-labels-default](https://layerchart.com/docs/components/Bars/vertical-outside-labels-default)
- [vertical-rounded-top-only](https://layerchart.com/docs/components/Bars/vertical-rounded-top-only)
- [vertical-stacked](https://layerchart.com/docs/components/Bars/vertical-stacked)
- [vertical-stacked-percent](https://layerchart.com/docs/components/Bars/vertical-stacked-percent)
- [vertical-stagger-tween-on-mount](https://layerchart.com/docs/components/Bars/vertical-stagger-tween-on-mount)
- [vertical-stagger-tween-on-mount-rounded-edge](https://layerchart.com/docs/components/Bars/vertical-stagger-tween-on-mount-rounded-edge)
- [vertical-time-scale-with-inset](https://layerchart.com/docs/components/Bars/vertical-time-scale-with-inset)
- [vertical-time-scale-with-interval](https://layerchart.com/docs/components/Bars/vertical-time-scale-with-interval)
- [vertical-time-scale-with-interval-months](https://layerchart.com/docs/components/Bars/vertical-time-scale-with-interval-months)
- [vertical-time-scale-with-missing-data](https://layerchart.com/docs/components/Bars/vertical-time-scale-with-missing-data)
- [vertical-tooltip-and-bar-highlight](https://layerchart.com/docs/components/Bars/vertical-tooltip-and-bar-highlight)
- [vertical-tooltip-and-click-handlers-for-individual-stack-grouped-bar](https://layerchart.com/docs/components/Bars/vertical-tooltip-and-click-handlers-for-individual-stack-grouped-bar)
- [vertical-tooltip-and-clipped-highlight](https://layerchart.com/docs/components/Bars/vertical-tooltip-and-clipped-highlight)
- [vertical-tooltip-and-highlight](https://layerchart.com/docs/components/Bars/vertical-tooltip-and-highlight)
- [vertical-tween-on-mount](https://layerchart.com/docs/components/Bars/vertical-tween-on-mount)
- [vertical-tween-on-mount-rounded-edge](https://layerchart.com/docs/components/Bars/vertical-tween-on-mount-rounded-edge)
- [vertical-with-grid-on-top](https://layerchart.com/docs/components/Bars/vertical-with-grid-on-top)
- [vertical-with-grid-on-top-mix-blend](https://layerchart.com/docs/components/Bars/vertical-with-grid-on-top-mix-blend)

### Related

- [Bar](https://layerchart.com/docs/components/Bar)

## Blur

Component applies a Gaussian blur effect to chart elements, softening their appearance for visual emphasis or stylistic purposes.

**Category:** other

**Supported Layers:** svg

### API

| Property | Type | Default | Description |
|----------|------|---------|-------------|
| id | `string` | - | A unique id for the filter. |
| stdDeviation | `number` | `5` | The standard deviation for the blur effect. |
| children | `Snippet` | - | The default children snippet which provides the id for the filter. |

## Bounds

Component provides reactive, animated coordinate scaling for chart layouts, passing live x/y scales to its children within a shared chart context.

**Category:** other

**Supported Layers:** svg, canvas

### API

| Property | Type | Default | Description |
|----------|------|---------|-------------|
| domain | `BoundsExtents \| BoundsExtentsAccessor \| null` | - | - |
| range | `BoundsExtents \| BoundsExtentsAccessor \| null` | - | - |
| children | `Snippet&lt;[ { xScale: AnyScale; yScale: AnyScale; } ]&gt;` | - | - |
| motion | `MotionProp` | - | - |

## BrushContext

Interaction component providing an interactive brush context allowing selection, adjustment, and resetting of x/y domains with draggable handles and event callbacks.

**Category:** interactions

**Supported Layers:** svg, canvas

## Usage

### Basic

See example: basic

### Simple styling

See example: simple-styling

### Striped background

See example: striped-background

### Handle arrows

See example: handle-arrows

### Handle labels

See example: handle-labels

### Constant labels

See example: constant-labels

### Integrated brush (x-axis)

See example: integrated-brush-(x-axis)

### Integrated brush (y-axis)

See example: integrated-brush-(y-axis)

### Integrated brush (both axis / area)

See example: integrated-brush-(both-axis-area)

### Separate chart (clip data)

See example: separate-chart-(clip-data)

### Separate chart (clip data: y-axis)

See example: separate-chart-(clip-data-y-axis)

### Separate chart (filter data)

See example: separate-chart-(filter-data)

### Sync brushes with `bind:xDomain`

See example: sync-brushes-with-bind-xdomain

### Tooltip interop

See example: tooltip-interop

### Selection

See example: selection

### Minimap

See example: minimap

### API

| Property | Type | Default | Description |
|----------|------|---------|-------------|
| axis | `'x' \| 'y' \| 'both'` | `'x'` | The axis to apply brushing |
| handleSize | `number` | `5` | Size of the draggable handles (width/height) |
| resetOnEnd | `boolean` | `false` | Only show range while actively brushing. Useful with `brushEnd` event |
| ignoreResetClick | `boolean` | `false` | Ignore click to reset. Useful to add click handlers to marks.  Requires external resetting (button, another chart, etc) |
| xDomain | `DomainType` | - | - |
| yDomain | `DomainType` | - | - |
| mode | `'integrated' \| 'separated'` | `'integrated'` | Mode of operation  - `integrated`: use with single chart  - `separated`: use with separate (typically smaller) chart and state can be managed externally (sync with other charts, etc).  Show active selection when domain does not equal original |
| disabled | `boolean` | `false` | Disable brush |
| range | `Partial&lt;HTMLAttributes&lt;HTMLElement&gt;&gt;` | - | Attributes passed to the range &lt;div&gt; element |
| handle | `Partial&lt;HTMLAttributes&lt;HTMLElement&gt;&gt;` | - | Attributes passed to the handle &lt;div&gt; elements |
| classes | `{ root?: string; frame?: string; range?: string; handle?: string; labels?: string; }` | `{}` | Classes to apply to the various elements rendered |
| onChange | `(detail: BrushEventPayload) =&gt; void` | - | - |
| onBrushStart | `(detail: BrushEventPayload) =&gt; void` | - | - |
| onBrushEnd | `(detail: BrushEventPayload) =&gt; void` | - | - |
| onReset | `(detail: BrushEventPayload) =&gt; void` | - | - |
| brushContext | `BrushContextValue` | - | A reference to this brush's context for use in parent components. |
| children | `Snippet&lt;[ { brushContext: BrushContextValue; } ]&gt;` | - | - |

### Examples

- [basic](https://layerchart.com/docs/components/BrushContext/basic)
- [constant-labels](https://layerchart.com/docs/components/BrushContext/constant-labels)
- [handle-arrows](https://layerchart.com/docs/components/BrushContext/handle-arrows)
- [handle-labels](https://layerchart.com/docs/components/BrushContext/handle-labels)
- [integrated-brush-(both-axis-area)](https://layerchart.com/docs/components/BrushContext/integrated-brush-(both-axis-area))
- [integrated-brush-(x-axis)](https://layerchart.com/docs/components/BrushContext/integrated-brush-(x-axis))
- [integrated-brush-(y-axis)](https://layerchart.com/docs/components/BrushContext/integrated-brush-(y-axis))
- [minimap](https://layerchart.com/docs/components/BrushContext/minimap)
- [selection](https://layerchart.com/docs/components/BrushContext/selection)
- [separate-chart-(clip-data-y-axis)](https://layerchart.com/docs/components/BrushContext/separate-chart-(clip-data-y-axis))
- [separate-chart-(clip-data)](https://layerchart.com/docs/components/BrushContext/separate-chart-(clip-data))
- [separate-chart-(filter-data)](https://layerchart.com/docs/components/BrushContext/separate-chart-(filter-data))
- [simple-styling](https://layerchart.com/docs/components/BrushContext/simple-styling)
- [striped-background](https://layerchart.com/docs/components/BrushContext/striped-background)
- [sync-brushes-with-bind-xdomain](https://layerchart.com/docs/components/BrushContext/sync-brushes-with-bind-xdomain)
- [tooltip-interop](https://layerchart.com/docs/components/BrushContext/tooltip-interop)

## Calendar

Marking component which highlights specific dates or time periods on a chart to emphasize events, milestones, or temporal patterns.

**Category:** marks

**Supported Layers:** svg, canvas, html

## Usage

See example: basic

### API

| Property | Type | Default | Description |
|----------|------|---------|-------------|
| **start** (required) | `Date` | - | The start date of the calendar. |
| **end** (required) | `Date` | - | The end date of the calendar. |
| cellSize | `number \| [ number, number ]` | - | Size of the cell in the calendar.  - `number`: sets width/height as same value (square). - `array`: sets as [width,height]. - `undefined/omitted`: is derived from Chart width/height |
| monthPath | `boolean \| Partial&lt;ComponentProps&lt;typeof MonthPath&gt;&gt;` | `false` | Enable drawing path around each month.  If object, pass as props to underlying &lt;path&gt; |
| monthLabel | `boolean \| Partial&lt;ComponentProps&lt;typeof Text&gt;&gt;` | - | Props to pass to the `&lt;text&gt;` element for month labels. |
| tooltipContext | `TooltipContextValue` | - | Tooltip context to setup mouse events to show tooltip for related data |
| children | `Snippet&lt;[ { cells: CalendarCell[]; cellSize: [ number, number ]; } ]&gt;` | - | - |

**Extends:** `Without<SVGAttributes<SVGRectElement>, CalendarPropsWithoutHTML>`

### Examples

- [90-days](https://layerchart.com/docs/components/Calendar/90-days)
- [basic](https://layerchart.com/docs/components/Calendar/basic)
- [fixed-cell-size](https://layerchart.com/docs/components/Calendar/fixed-cell-size)
- [html-with-padding](https://layerchart.com/docs/components/Calendar/html-with-padding)
- [last-month](https://layerchart.com/docs/components/Calendar/last-month)
- [multiple-years](https://layerchart.com/docs/components/Calendar/multiple-years)
- [responsive-cell-size-default](https://layerchart.com/docs/components/Calendar/responsive-cell-size-default)
- [rounded-cells](https://layerchart.com/docs/components/Calendar/rounded-cells)

## Canvas

Canvas layer

**Category:** layers

Typically you will use `<Layer type="canvas">`

### API

| Property | Type | Default | Description |
|----------|------|---------|-------------|
| ref | `HTMLCanvasElement` | - | The `&lt;canvas&gt;` tag. Useful for bindings. |
| canvasContext | `CanvasRenderingContext2D` | - | The `&lt;canvas&gt;`'s 2d context. Useful for bindings. |
| willReadFrequently | `boolean` | `false` | Force the use of a software (instead of hardware accelerated) 2D canvas, which can save memory when calling getImageData() frequently. |
| zIndex | `number` | `0` | The `z-index` style to apply to the layer. |
| pointerEvents | `boolean` | `true` |  Whether pointer events should be enabled on the canvas.  - `false`: `pointer-events: none;` will be set on the entire layer. - `true`: pointer events will operate normally. |
| fallback | `string \| Snippet` | - | The content to display if canvas is not supported or cannot be rendered. This can either be a string or a snippet with custom markup. |
| center | `boolean \| 'x' \| 'y'` | `false` | Translate children to center of the canvas (useful for radial layouts). |
| ignoreTransform | `boolean` | `false` | Ignore TransformContext.  Useful to add static elements such as legends. |
| disableHitCanvas | `boolean` | `false` | Disable the hit canvas (useful when animations are playing) |
| debug | `boolean` | `false` | Show the hit canvas for debugging purposes. |
| children | `Snippet&lt;[ { ref: HTMLCanvasElement; canvasContext: CanvasRenderingContext2D \| undefined; } ]&gt;` | - | - |

**Extends:** `Without<HTMLCanvasAttributes, CanvasPropsWithoutHTML>`

### Related

- [Layer](https://layerchart.com/docs/components/Layer)

## Chart

Base component providing chart dimensions and contexts such as TooltipContext, GeoContext, and TransformContext. See also simplified charts such as AreaChart and BarChart for streamlined implementations.

**Category:** charts

**Supported Layers:** svg, canvas, html

## Usage

:example{ component="Area" name="basic" showCode }

> note: Features: Adds support for x and y baselines (always show 0, etc)

### API

| Property | Type | Default | Description |
|----------|------|---------|-------------|
| ssr | `boolean` | `false` | Whether this chart should be rendered server side |
| pointerEvents | `boolean` | `true` | Whether to allow pointer events via CSS. Set this to `false` to set `pointer-events: none;` on all components, disabling all mouse interactions. |
| position | `string` | `'relative'` | Determine the positioning of the wrapper div. Set this to `'absolute'` when you want to stack layers. |
| percentRange | `boolean` | `false` | If `true`, set all scale ranges to `[0, 100]`. Ranges reversed via `xReverse`, `yReverse`, or `rReverse` props will continue to be reversed as usual. |
| ref | `HTMLElement` | - | A bindable reference to the root container element. |
| data | `T[] \| readonly T[] \| HierarchyNode&lt;T&gt; \| SankeyGraph&lt;any, any&gt;` | - | If `data` is not a flat array of objects and you want to use any of the scales, set a flat version of the data via the `flatData` prop. |
| flatData | `T[] \| readonly T[] \| HierarchyNode&lt;T&gt; \| SankeyGraph&lt;any, any&gt;` | - | A flat version of data. |
| x | `Accessor&lt;T&gt;` | - | The x accessor. The key in each row of data that corresponds to the x-field. This can be a string, an accessor function, a number or an array of any combination of those types. This property gets converted to a function when you access it through the context. |
| y | `Accessor&lt;T&gt;` | - | The y accessor. The key in each row of data that corresponds to the y-field. This can be a string, an accessor function, a number or an array of any combination of those types. This property gets converted to a function when you access it through the context. |
| z | `Accessor&lt;T&gt;` | - | The z accessor. The key in each row of data that corresponds to the z-field. This can be a string, an accessor function, a number or an array of any combination of those types. This property gets converted to a function when you access it through the context. |
| r | `Accessor&lt;T&gt;` | - | The r accessor. The key in each row of data that corresponds to the r-field. This can be a string, an accessor function, a number or an array of any combination of those types. This property gets converted to a function when you access it through the context. |
| x1 | `Accessor&lt;T&gt;` | - | The x1 accessor. The key in each row of data that corresponds to the x1-field. This can be a string, an accessor function, a number or an array of any combination of those types. This property gets converted to a function when you access it through the context. |
| y1 | `Accessor&lt;T&gt;` | - | The y1 accessor. The key in each row of data that corresponds to the y1-field. This can be a string, an accessor function, a number or an array of any combination of those types. This property gets converted to a function when you access it through the context. |
| c | `Accessor&lt;T&gt;` | - | The c (color) accessor. The key in each row of data that corresponds to the color. This can be a string or an accessor function. This property gets converted to a function when you access it through the context. |
| xDomain | `DomainType` | - | Set a min or max. For linear scales, if you want to inherit the value from the data's extent, set that value to `null`. This value can also be an array because sometimes your scales are [piecewise](https://github.com/d3/d3-scale#continuous_domain) or are a list of discrete values such as in [ordinal scales](https://github.com/d3/d3-scale#ordinal-scales), useful for color series. Set it to a function that receives the computed domain and lets you return a modified domain, useful for sorting values. |
| yDomain | `DomainType` | - | Set a min or max. For linear scales, if you want to inherit the value from the data's extent, set that value to `null`. This value can also be an array because sometimes your scales are [piecewise](https://github.com/d3/d3-scale#continuous_domain) or are a list of discrete values such as in [ordinal scales](https://github.com/d3/d3-scale#ordinal-scales), useful for color series. Set it to a function that receives the computed domain and lets you return a modified domain, useful for sorting values. |
| zDomain | `DomainType` | - | Set a min or max. For linear scales, if you want to inherit the value from the data's extent, set that value to `null`. This value can also be an array because sometimes your scales are [piecewise](https://github.com/d3/d3-scale#continuous_domain) or are a list of discrete values such as in [ordinal scales](https://github.com/d3/d3-scale#ordinal-scales), useful for color series. Set it to a function that receives the computed domain and lets you return a modified domain, useful for sorting values. |
| rDomain | `DomainType` | - | Set a min or max. For linear scales, if you want to inherit the value from the data's extent, set that value to `null`. This value can also be an array because sometimes your scales are [piecewise](https://github.com/d3/d3-scale#continuous_domain) or are a list of discrete values such as in [ordinal scales](https://github.com/d3/d3-scale#ordinal-scales), useful for color series. Set it to a function that receives the computed domain and lets you return a modified domain, useful for sorting values. |
| x1Domain | `DomainType` | - | Set a min or max. For linear scales, if you want to inherit the value from the data's extent, set that value to `null`. This value can also be an array because sometimes your scales are [piecewise](https://github.com/d3/d3-scale#continuous_domain) or are a list of discrete values such as in [ordinal scales](https://github.com/d3/d3-scale#ordinal-scales), useful for color series. Set it to a function that receives the computed domain and lets you return a modified domain, useful for sorting values. |
| y1Domain | `DomainType` | - | Set a min or max. For linear scales, if you want to inherit the value from the data's extent, set that value to `null`. This value can also be an array because sometimes your scales are [piecewise](https://github.com/d3/d3-scale#continuous_domain) or are a list of discrete values such as in [ordinal scales](https://github.com/d3/d3-scale#ordinal-scales), useful for color series. Set it to a function that receives the computed domain and lets you return a modified domain, useful for sorting values. |
| cDomain | `DomainType` | - | Set the list of color values. |
| xNice | `Nice` | `false` | Applies D3's [scale.nice()](https://github.com/d3/d3-scale#continuous_nice) to the x domain. |
| yNice | `Nice` | `false` | Applies D3's [scale.nice()](https://github.com/d3/d3-scale#continuous_nice) to the y domain. |
| zNice | `Nice` | `false` | Applies D3's [scale.nice()](https://github.com/d3/d3-scale#continuous_nice) to the z domain. |
| rNice | `Nice` | `false` | Applies D3's [scale.nice()](https://github.com/d3/d3-scale#continuous_nice) to the r domain. |
| xPadding | `PaddingArray` | - | Assign a pixel value to add to the min or max of the scale. This will increase the scales domain by the scale unit equivalent of the provided pixels. |
| yPadding | `PaddingArray` | - | Assign a pixel value to add to the min or max of the scale. This will increase the scales domain by the scale unit equivalent of the provided pixels. |
| zPadding | `PaddingArray` | - | Assign a pixel value to add to the min or max of the scale. This will increase the scales domain by the scale unit equivalent of the provided pixels. |
| rPadding | `PaddingArray` | - | Assign a pixel value to add to the min or max of the scale. This will increase the scales domain by the scale unit equivalent of the provided pixels. |
| xScale | `XScale` | `autoScale` | The D3 scale that should be used for the x-dimension. Pass in an instantiated D3 scale if you want to override the default or you want to extra options. |
| yScale | `YScale` | `autoScale` | The D3 scale that should be used for the x-dimension. Pass in an instantiated D3 scale if you want to override the default or you want to extra options. |
| zScale | `AnyScale` | `autoScale` | The D3 scale that should be used for the x-dimension. Pass in an instantiated D3 scale if you want to override the default or you want to extra options. |
| rScale | `AnyScale` | `scaleSqrt` | The D3 scale that should be used for the x-dimension. Pass in an instantiated D3 scale if you want to override the default or you want to extra options. |
| x1Scale | `AnyScale` | `autoScale` | The D3 scale that should be used for the x1-dimension. Pass in an instantiated D3 scale if you want to override the default or you want to extra options. |
| y1Scale | `AnyScale` | `autoScale` | The D3 scale that should be used for the y1-dimension. Pass in an instantiated D3 scale if you want to override the default or you want to extra options. |
| cScale | `AnyScale` | `scaleOrdinal` | The D3 scale that should be used for the  color dimension. Pass in an instantiated D3 scale if you want to override the default or you want to extra options. |
| xRange | `BaseRange` | - | Override the default x range of `[0, width]` by setting an array or function with argument `({ width, height})` that returns an array. Setting this prop overrides `xReverse`. This can also be a list of numbers or strings for scales with discrete ranges like [scaleThreshold](https://github.com/d3/d3-scale#threshold-scales) or [scaleQuantize](https://github.com/d3/d3-scale#quantize-scales). |
| yRange | `BaseRange` | - | Override the default y range of `[0, height]` by setting an array or function with argument `({ width, height})` that returns an array. Setting this prop overrides `yReverse`. This can also be a list of numbers or strings for scales with discrete ranges like [scaleThreshold](https://github.com/d3/d3-scale#threshold-scales) or [scaleQuantize](https://github.com/d3/d3-scale#quantize-scales). |
| zRange | `BaseRange` | - | Override the default z range of `[0, width]` by setting an array or function with argument `({ width, height})` that returns an array. Setting this prop overrides `zReverse`. This can also be a list of numbers or strings for scales with discrete ranges like [scaleThreshold](https://github.com/d3/d3-scale#threshold-scales) or [scaleQuantize](https://github.com/d3/d3-scale#quantize-scales). |
| rRange | `BaseRange` | - | Override the default r range of `[1, 25]` by setting an array or function with argument `({ width, height})` that returns an array. Setting this prop overrides `rReverse`. This can also be a list of numbers or strings for scales with discrete ranges like [scaleThreshold](https://github.com/d3/d3-scale#threshold-scales) or [scaleQuantize](https://github.com/d3/d3-scale#quantize-scales). |
| x1Range | `XRangeWithScale&lt;XScale&gt;` | - | Set the x1 range by setting an array or function with argument `({ xScale, width, height})` that returns an array. This can also be a list of numbers or strings for scales with discrete ranges like [scaleThreshold](https://github.com/d3/d3-scale#threshold-scales) or [scaleQuantize](https://github.com/d3/d3-scale#quantize-scales). |
| y1Range | `YRangeWithScale&lt;YScale&gt;` | - | Set the y1 range by setting an array or function with argument `({ yScale, width, height})` that returns an array. This can also be a list of numbers or strings for scales with discrete ranges like [scaleThreshold](https://github.com/d3/d3-scale#threshold-scales) or [scaleQuantize](https://github.com/d3/d3-scale#quantize-scales). |
| cRange | `string[] \| readonly string[]` | - | Override the default y1 range of `[0, width]` by setting an array or function with argument `({ yScale, width, height})` that returns an array. Setting this prop overrides `x1Reverse`. This can also be a list of numbers or strings for scales with discrete ranges like [scaleThreshold](https://github.com/d3/d3-scale#threshold-scales) or [scaleQuantize](https://github.com/d3/d3-scale#quantize-scales). |
| xReverse | `boolean` | `false` | Reverse the default x range. By default this is `false` and the range is `[0, width]`. Ignored if you set the xRange prop. |
| yReverse | `boolean` | `true` | Reverse the default y range. By default this is `true` and the range is `[height, 0]` unless using an ordinal scale with a `.bandwidth` method for `yScale`. Ignored if you set the `yRange` prop. |
| zReverse | `boolean` | `false` | Reverse the default z range. By default this is `false` and the range is `[0, width]`. Ignored if you set the zRange prop. |
| rReverse | `boolean` | `false` | Reverse the default r range. By default this is `false` and the range is `[1, 25]`. Ignored if you set the rRange prop. |
| xDomainSort | `boolean` | `false` | ***Only used when scale is ordinal.*** Set whether the calculated unique items come back sorted. |
| yDomainSort | `boolean` | `false` | ***Only used when scale is ordinal.*** Set whether the calculated unique items come back sorted. |
| zDomainSort | `boolean` | `false` | ***Only used when scale is ordinal.*** Set whether the calculated unique items come back sorted. |
| rDomainSort | `boolean` | `false` | ***Only used when scale is ordinal.*** Set whether the calculated unique items come back sorted. |
| padding | `{ top?: number; right?: number; bottom?: number; left?: number; } \| number` | - | The amount of padding to put around your chart. It operates like CSS box-sizing: border-box; where values are subtracted from the parent container's width and height, the same as a [D3 margin convention](https://bl.ocks.org/mbostock/3019563).  If a number is passed, it will be applied to all sides. |
| extents | `{ x?: [ min: number, max: number ]; y?: [ min: number, max: number ]; r?: [ min: number, max: number ]; z?: [ min: number, max: number ]; }` | - | Manually set the extents of the x, y or r scale as a two-dimensional array of the min and max you want. Setting values here will skip any dynamic extent calculation of the data for that dimension. |
| meta | `Record&lt;string, any&gt;` | - | Any extra configuration values you want available on the Chart context. This could be useful for color lookups or additional constants. |
| debug | `boolean` | `false` | Enable debug printing to the console. Useful to inspect your scales and dimensions. |
| verbose | `boolean` | `true` | Show warnings in the console. |
| xBaseline | `number \| null` | `null` | x value guaranteed to be visible in xDomain.  Useful with optional negative values since `xDomain={[0, null]}` would ignore negative values |
| yBaseline | `number \| null` | `null` | y value guaranteed to be visible in yDomain.  Useful with optional negative values since `yDomain={[0, null]}` would ignore negative values |
| xInterval | `TimeInterval \| null` | - | Time interval to use for the x-axis when using a time scale. |
| yInterval | `TimeInterval \| null` | - | Time interval to use for the y-axis when using a time scale. |
| radial | `boolean` | `false` | Use radial instead of cartesian coordinates, mapping `x` to `angle` and `y`` to radial. Radial lines are positioned relative to the origin, use transform (ex. `&lt;Group center&gt;`) to change the origin |
| children | `Snippet&lt;[ { context: ChartContextValue&lt;T, XScale, YScale&gt;; } ]&gt;` | - | - |
| context | `ChartContextValue&lt;T, XScale, YScale&gt;` | - | A bindable reference to the chart context. |
| geo | `Partial&lt;ComponentProps&lt;typeof GeoContext&gt;&gt;` | - | Props passed to GeoContext |
| tooltip | `Partial&lt;ComponentProps&lt;typeof TooltipContext&gt;&gt; \| boolean` | - | Props passed to the `TooltipContext` component. |
| transform | `Partial&lt;ComponentProps&lt;typeof TransformContext&gt;&gt;` | - | Props passed to TransformContext |
| brush | `Partial&lt;ComponentProps&lt;typeof BrushContext&gt;&gt; \| boolean` | - | Props passed to BrushContext |
| onResize | `(e: ChartResizeDetail) =&gt; void` | - | A callback function that is called when the chart is resized. |
| clip | `boolean` | `false` | Whether to clip overflow content. When true, sets `overflow: hidden` on the container. |
| ondragstart | `ComponentProps&lt;typeof TransformContext&gt;['ondragstart']` | - | - |
| ondragend | `ComponentProps&lt;typeof TransformContext&gt;['ondragend']` | - | - |
| onTransform | `ComponentProps&lt;typeof TransformContext&gt;['onTransform']` | - | - |
| width | `number` | - | Sets width of the chart container.  Uses parent width if not set (bind:clientWidth) |
| height | `number` | - | Sets height of the chart container.  Uses parent height if not set (bind:clientHeight) |

**Extends:** `Without<HTMLAttributes<HTMLDivElement>, ChartPropsWithoutHTML<T, XScale, YScale>>`

### Examples

- [compound-common-scale-with-extra-marks](https://layerchart.com/docs/components/Chart/compound-common-scale-with-extra-marks)
- [compound-dual-axis-with-single-chart-using-remapped-scale](https://layerchart.com/docs/components/Chart/compound-dual-axis-with-single-chart-using-remapped-scale)
- [compound-dual-axis-with-stacked-charts](https://layerchart.com/docs/components/Chart/compound-dual-axis-with-stacked-charts)
- [compound-separate-scales-with-stacked-charts-and-overridden-marks](https://layerchart.com/docs/components/Chart/compound-separate-scales-with-stacked-charts-and-overridden-marks)
- [compound-separate-scales-with-stacked-charts-with-inverted-range-top-down](https://layerchart.com/docs/components/Chart/compound-separate-scales-with-stacked-charts-with-inverted-range-top-down)

### Related

- [ArcChart](https://layerchart.com/docs/components/ArcChart)
- [AreaChart](https://layerchart.com/docs/components/AreaChart)
- [BarChart](https://layerchart.com/docs/components/BarChart)
- [LineChart](https://layerchart.com/docs/components/LineChart)
- [PieChart](https://layerchart.com/docs/components/PieChart)
- [ScatterChart](https://layerchart.com/docs/components/ScatterChart)
- [TooltipContext](https://layerchart.com/docs/components/TooltipContext)
- [GeoContext](https://layerchart.com/docs/components/GeoContext)

## ChartClipPath

Clipping component which applies a rectangular clip path for specific components (axis labels, etc) within chart while still allowing some to overflow (tooltips, etc).

**Category:** clipping

**Supported Layers:** svg

### API

| Property | Type | Default | Description |
|----------|------|---------|-------------|
| full | `boolean` | `false` | Include padding area (ex. axis) |
| disabled | `boolean` | `false` | Disable clipping (show all) |
| children | `Snippet` | - | - |

**Extends:** `Without<Omit<RectClipPathProps, 'width' | 'height'>, ChartClipPathPropsWithoutHTML>`

### Related

- [RectClipPath](https://layerchart.com/docs/components/RectClipPath)
- [Rect](https://layerchart.com/docs/components/Rect)

## Circle

Primitive component which draws a circular shape to mark specific points or emphasize data visually.

**Category:** primitives

**Supported Layers:** svg, canvas, html

## Usage

See example: styling-using-classes

### API

| Property | Type | Default | Description |
|----------|------|---------|-------------|
| cx | `number` | `0` | The center x position of the circle. |
| initialCx | `number` | `cx` | The initial center x position of the circle. |
| cy | `number` | `0` | The center y position of the circle. |
| initialCy | `number` | `cy` | The initial center y position of the circle. |
| r | `number` | `1` | The radius of the circle. |
| initialR | `number` | `r` | The initial radius of the circle. |
| ref | `SVGCircleElement` | - | A bindable reference to the `&lt;circle&gt;` element |
| motion | `MotionProp` | - | - |
| children | `Snippet` | - | Children content to render.  Note: Only works for Html layers |
| fill | `string` | - | Fill color |
| fillOpacity | `number` | - | Fill opacity (0-1) |
| stroke | `string` | - | Stroke color |
| strokeWidth | `number` | - | Stroke width in pixels |
| strokeOpacity | `number` | - | Stroke opacity (0-1) |
| opacity | `number` | - | Overall opacity (0-1) |

**Extends:** `Without<SVGAttributes<Element>, CirclePropsWithoutHTML>`

### Examples

- [styling-using-attributes](https://layerchart.com/docs/components/Circle/styling-using-attributes)
- [styling-using-classes](https://layerchart.com/docs/components/Circle/styling-using-classes)
- [styling-using-css-variables](https://layerchart.com/docs/components/Circle/styling-using-css-variables)

### Related

- [Points](https://layerchart.com/docs/components/Points)

## CircleClipPath

Clipping component which conditionally applies a circular clip region to its child elements based on the rendering context and provided properties.

**Category:** clipping

**Supported Layers:** svg

### API

| Property | Type | Default | Description |
|----------|------|---------|-------------|
| id | `string` | - | A unique id for the clipPath. |
| cx | `number` | `0` | The center x position of the circle. |
| cy | `number` | `0` | The center y position of the circle. |
| **r** (required) | `number` | - | The radius of the circle. |
| disabled | `boolean` | `false` | Whether to disable clipping (show all). |
| ref | `SVGCircleElement` | - | A bindable reference to the underlying `&lt;circle&gt;` element' |
| children | `ClipPathPropsWithoutHTML['children']` | - | The children snippet to render content inside the clipPath. |
| motion | `MotionProp` | - | - |

## ClipPath

Clipping component which defines a clipping region to constrain the rendering of chart elements within a specified shape or boundary.

**Category:** clipping

**Supported Layers:** svg

### API

| Property | Type | Default | Description |
|----------|------|---------|-------------|
| id | `string` | - | A unique id for the clipPath. |
| useId | `string` | - | Use existing path or shape (by id) for clipPath |
| disabled | `boolean` | `false` | Whether to disable clipping (show all). |
| clip | `Snippet&lt;[ { id: string; } ]&gt;` | - | A snippet to insert content into the clipPath. Provides the id for the clipPath as a snippet prop. |
| children | `Snippet&lt;[ { id: string; url: string; useId?: string; } ]&gt;` | - | Children to render in the `&lt;g&gt;` element that links to the clipPath (if not disabled). Provides the id, url, and useId for the clipPath as snippet props. |

**Extends:** `Without<SVGAttributes<SVGClipPathElement>, ClipPathPropsWithoutHTML>`

### Related

- [components/ChartClipPath](https://layerchart.com/docs/components/components/ChartClipPath)
- [components/CircleClipPath](https://layerchart.com/docs/components/components/CircleClipPath)
- [components/RectClipPath](https://layerchart.com/docs/components/components/RectClipPath)
- [components/Threshold](https://layerchart.com/docs/components/components/Threshold)

## ColorRamp

Component generates a color ramp (gradient) based on specified colors and stops, useful for mapping data values to colors in visualizations.

**Category:** other

**Supported Layers:** svg

## Usage

### Basic

See example: basic

### Pixelated

See example: pixelated

### Schemes

See example: schemes

### API

| Property | Type | Default | Description |
|----------|------|---------|-------------|
| interpolator | `(t: number) =&gt; string` | `(t: number) =&gt; `hsl(${t * 360}, 100%, 50%)`` | The interpolator function to use for the color ramp. |
| steps | `number` | `10` | The number of steps in the color ramp. |
| height | `string \| number` | `'20px'` | The height of the color ramp. |
| width | `string \| number` | `'100%'` | The width of the color ramp. |
| ref | `SVGImageElement` | - | A bindable reference to the underlying `&lt;image&gt;` element. |

**Extends:** `Without<SVGAttributes<SVGImageElement>, ColorRampPropsWithoutHTML>`

### Examples

- [basic](https://layerchart.com/docs/components/ColorRamp/basic)
- [pixelated](https://layerchart.com/docs/components/ColorRamp/pixelated)
- [schemes](https://layerchart.com/docs/components/ColorRamp/schemes)

## Connector

Primitive component which draws a line or curve between two points on a chart to illustrate relationships or connections in the data.

**Category:** primitives

**Supported Layers:** svg, canvas

## Usage

See example: playground

### API

| Property | Type | Default | Description |
|----------|------|---------|-------------|
| **source** (required) | `ConnectorCoords` | `{ x: 0, y: 0 }` | The coordinates of the start point of the connector. |
| **target** (required) | `ConnectorCoords` | `{ x: 100, y: 100 }` | The coordinates of the end point of the connector. |
| sweep | `ConnectorSweep` | `'horizontal-vertical'` | The sweep direction of the connector. |
| type | `ConnectorType` | `'rounded'` | The type of the connector.  Set to `'d3'` to use a D3 curve function via the `curve` prop. |
| radius | `number` | `20` | The radius of the connector.  Only used when type is `'beveled'` or `'rounded'` |
| curve | `CurveFactory` | ``d3.curveLinear`` | The D3 curve function to use for the connector.  Only used when type is `'d3'` |

**Extends:** `Without<PathProps, ConnectorPropsWithoutHTML>`

### Examples

- [playground](https://layerchart.com/docs/components/Connector/playground)

### Related

- [Link](https://layerchart.com/docs/components/Link)

## Dagre

Layout component which arranges directed graphs in layers, positioning nodes to minimize edge crossings and create a clear, hierarchical flow.

**Category:** layout

**Supported Layers:** svg, canvas

## Usage

See example: basic

### API

| Property | Type | Default | Description |
|----------|------|---------|-------------|
| **data** (required) | `DagreGraphData` | - | Data of nodes and edges to build graph |
| nodes | `(d: any) =&gt; any` | `(d: any) =&gt; d.nodes` | Function to extract nodes from data |
| nodeId | `(d: any) =&gt; any` | `(d: any) =&gt; d.id` | Function to extract node ID from node data |
| edges | `(d: any) =&gt; any` | `(d: any) =&gt; d.edges` | Function to extract edges from data |
| directed | `boolean` | `true` | Set graph as directed (true, default) or undirected (false), which does not treat the order of nodes in an edge as significant |
| multigraph | `boolean` | `false` | Allow a graph to have multiple edges between the same pair of nodes |
| compound | `boolean` | `false` | Allow a graph to have compound nodes - nodes which can be the `parent` of other nodes |
| ranker | `'network-simplex' \| 'tight-tree' \| 'longest-path'` | `'network-simplex'` | Type of algorithm to assigns a rank to each node in the input graph |
| direction | `keyof typeof RankDir` | `'top-bottom'` | Direction for rank nodes |
| align | `keyof typeof Align \| undefined` | `undefined` | Alignment for rank nodes |
| rankSeparation | `number` | `50` | Number of pixels between each rank in the layout |
| nodeSeparation | `number` | `50` | Number of pixels that separate nodes horizontally in the layout |
| edgeSeparation | `number` | `10` | Number of pixels that separate edges horizontally in the layout |
| nodeWidth | `number` | `100` | Default node width if not defined on node |
| nodeHeight | `number` | `50` | Default node height if not defined on node |
| edgeLabelWidth | `number` | `100` | Default link label width if not defined on edge |
| edgeLabelHeight | `number` | `20` | Default edge label height if not defined on edge |
| edgeLabelPosition | `keyof typeof EdgeLabelPosition` | `'center'` | Default edge label position |
| edgeLabelOffset | `number` | `10` | Default pixels to move the label away from the edge if not defined on edge Applies only when labelpos is l or r |
| filterNodes | `(nodeId: string, graph: dagre.graphlib.Graph) =&gt; boolean` | `() =&gt; true` | Filter nodes |
| graph | `dagre.graphlib.Graph` | - | Exposed to access to Dagre Graph instance via `bind:graph` |
| children | `Snippet&lt;[ { nodes: Array&lt;dagre.Node&gt;; edges: Array&lt;Edge & EdgeConfig & GraphEdge&gt;; graph: dagre.graphlib.Graph; } ]&gt;` | - | - |

### Examples

- [basic](https://layerchart.com/docs/components/Dagre/basic)
- [playground](https://layerchart.com/docs/components/Dagre/playground)
- [software-user-flow](https://layerchart.com/docs/components/Dagre/software-user-flow)
- [tcp-state-diagram](https://layerchart.com/docs/components/Dagre/tcp-state-diagram)

## Direct

Directly import each component and source

**Category:** examples

```svelte
<script lang="ts">
	import Code from '$lib/components/Code.svelte';

	// repeat for each example
	import Basic from '$examplescomponents/AreaChart/basic.svelte';
	import BasicSource from '$examples/components/AreaChart/basic.svelte?raw';
</script>

<Basic />
<Code source={BasicSource} />
```

## Example

## Benefits

- Most straightforward
- Should be easier to build search index

## Issues

- Very verbose - import each example and source and instantiate each

## Ellipse

Primitive component which draws an oval shape  to highlight areas, emphasize points, or decorate visual elements.

**Category:** primitives

**Supported Layers:** svg, canvas, html

## Usage

See example: styling-using-classes

### API

| Property | Type | Default | Description |
|----------|------|---------|-------------|
| cx | `number` | `0` | The center x position of the ellipse. |
| initialCx | `number` | `cx` | The initial center x position of the ellipse. |
| cy | `number` | `0` | The center y position of the ellipse. |
| initialCy | `number` | `cy` | The initial center y position of the ellipse. |
| rx | `number` | `1` | The radius of the ellipse on the x-axis. |
| initialRx | `number` | `rx` | The initial radius of the ellipse on the x-axis. |
| ry | `number` | `1` | The radius of the ellipse on the y-axis. |
| initialRy | `number` | `ry` | The initial radius of the ellipse on the y-axis. |
| ref | `SVGEllipseElement` | - | A bindable reference to the `&lt;ellipse&gt;` element |
| motion | `MotionProp` | - | - |
| fill | `string` | - | Fill color |
| fillOpacity | `number` | - | Fill opacity (0-1) |
| stroke | `string` | - | Stroke color |
| strokeWidth | `number` | - | Stroke width in pixels |
| strokeOpacity | `number` | - | Stroke opacity (0-1) |
| opacity | `number` | - | Overall opacity (0-1) |

**Extends:** `Without<SVGAttributes<Element>, EllipsePropsWithoutHTML>`

### Examples

- [styling-using-attributes](https://layerchart.com/docs/components/Ellipse/styling-using-attributes)
- [styling-using-classes](https://layerchart.com/docs/components/Ellipse/styling-using-classes)
- [styling-using-css-variables](https://layerchart.com/docs/components/Ellipse/styling-using-css-variables)

## ForceSimulation

Layout components which positions nodes using physics-based forces, simulating attraction, repulsion, and link constraints to create an intuitive, collision-free network visualization.

**Category:** layout

**Supported Layers:** svg, canvas

## Usage

See example: tree

### API

| Property | Type | Default | Description |
|----------|------|---------|-------------|
| **forces** (required) | `Forces&lt;NodeDatum, LinkDatum&gt;` | - | Force simulation parameters |
| **data** (required) | `Data&lt;NodeDatum, LinkDatum&gt;` | - | An object with arrays of nodes and links, to be used for position calculation. |
| alpha | `number` | `DEFAULT_ALPHA` | Current alpha value of the simulation |
| alphaTarget | `number` | `DEFAULT_ALPHA_TARGET` | Target alpha value for the simulation |
| alphaDecay | `number` | `DEFAULT_ALPHA_DECAY` | Alpha decay rate per tick |
| alphaMin | `number` | `DEFAULT_ALPHA_MIN` | Minimum alpha value at which simulation stops |
| velocityDecay | `number` | `DEFAULT_VELOCITY_DECAY` | Velocity decay factor applied to nodes each tick |
| stopped | `boolean` | `false` | Stop simulation |
| static | `boolean` | `false` | If true, will only update nodes after simulation has completed |
| cloneNodes | `boolean` | `false` | Clone nodes since simulation mutates original |
| onStart | `(e: OnStartEvent&lt;NodeDatum, LinkDatum \| undefined&gt;) =&gt; void` | - | Callback function triggered when simulation starts |
| onNodesChange | `(e: OnNodesChangeEvent&lt;NodeDatum, LinkDatum \| undefined&gt;) =&gt; void` | - | Callback function triggered right before nodes get passed to the simulation |
| onTick | `(e: OnTickEvent&lt;NodeDatum, LinkDatum \| undefined&gt;) =&gt; void` | - | Callback function triggered on each simulation tick |
| onEnd | `(e: OnEndEvent&lt;NodeDatum, LinkDatum \| undefined&gt;) =&gt; void` | - | Callback function triggered when simulation ends |
| children | `Snippet&lt;[ { nodes: NodeDatum[]; links: LinkDatum[]; linkPositions: LinkPosition[]; simulation: Simulation&lt;NodeDatum, LinkDatum&gt;; } ]&gt;` | - | - |

### Examples

- [beeswarm](https://layerchart.com/docs/components/ForceSimulation/beeswarm)
- [collision-detection](https://layerchart.com/docs/components/ForceSimulation/collision-detection)
- [disjoint-graph](https://layerchart.com/docs/components/ForceSimulation/disjoint-graph)
- [graph-drag](https://layerchart.com/docs/components/ForceSimulation/graph-drag)
- [graph-playground](https://layerchart.com/docs/components/ForceSimulation/graph-playground)
- [group](https://layerchart.com/docs/components/ForceSimulation/group)
- [lattice](https://layerchart.com/docs/components/ForceSimulation/lattice)
- [text](https://layerchart.com/docs/components/ForceSimulation/text)
- [tree](https://layerchart.com/docs/components/ForceSimulation/tree)

## Frame

Commonly used component representing a rectangular Chart bounding box and respecting padding or full size.

**Category:** common

**Supported Layers:** svg, canvas, html

## Usage

See example: basic

### API

| Property | Type | Default | Description |
|----------|------|---------|-------------|
| full | `boolean` | `false` | Whether to include padding area |

### Examples

- [basic](https://layerchart.com/docs/components/Frame/basic)
- [border](https://layerchart.com/docs/components/Frame/border)
- [full](https://layerchart.com/docs/components/Frame/full)
- [gradient](https://layerchart.com/docs/components/Frame/gradient)

### Related

- [Rect](https://layerchart.com/docs/components/Rect)

## GeoCircle

Geographic component which displays circular areas on a map to represent a specific geographic region or radius around a point.

**Category:** geo

**Supported Layers:** svg, canvas

## Playground

See example: playground

### API

| Property | Type | Default | Description |
|----------|------|---------|-------------|
| radius | `number` | `90` | The radius of the circle in degrees. |
| center | `[ number, number ]` | `[0, 0]` | The center point of the circle in degrees ([longitude, latitude]). |
| precision | `number` | `6` | The precision of the circle in degrees. |

**Extends:** `Without<GeoPathProps, GeoCirclePropsWithoutHTML>`

### Examples

- [earthquake-globe](https://layerchart.com/docs/components/GeoCircle/earthquake-globe)
- [playground](https://layerchart.com/docs/components/GeoCircle/playground)

## GeoContext

Geographic component which provides geographic projection and scaling context to children for accurate rendering of geographic data.

**Category:** geo

**Supported Layers:** svg, canvas

## Playground

See example: projection-playground

### API

| Property | Type | Default | Description |
|----------|------|---------|-------------|
| projection | `() =&gt; GeoProjection` | - | A d3 projection function. Pass this in as an uncalled function, e.g. `projection={geoAlbersUsa}`. |
| fitGeojson | `GeoPermissibleObjects` | - | - |
| fixedAspectRatio | `number` | - | By default, the map fills to fit the $width and $height. If instead you want a fixed-aspect ratio, like for a server-side rendered map, set that here. |
| clipAngle | `number` | - | - |
| clipExtent | `[ [ number, number ], [ number, number ] ]` | - | - |
| rotate | `{ yaw: number; pitch: number; roll: number; }` | - | - |
| scale | `number` | - | - |
| translate | `[ number, number ]` | - | - |
| center | `[ number, number ]` | - | - |
| applyTransform | `('scale' \| 'translate' \| 'rotate')[]` | - | Apply TransformContext to the selected properties.  Typically `translate` or `rotate` are mutually selected |
| reflectX | `boolean` | - | - |
| reflectY | `boolean` | - | - |
| geoContext | `GeoContextValue` | - | Exposed to allow binding in Chart |
| **children** (required) | `Snippet&lt;[ { geoContext: GeoContextValue; } ]&gt;` | - | - |

### Examples

- [geojson-preview](https://layerchart.com/docs/components/GeoContext/geojson-preview)
- [projection-playground](https://layerchart.com/docs/components/GeoContext/projection-playground)
- [shapefile-preview](https://layerchart.com/docs/components/GeoContext/shapefile-preview)
- [topojson-preview](https://layerchart.com/docs/components/GeoContext/topojson-preview)

### Related

- [Chart](https://layerchart.com/docs/components/Chart)

## GeoEdgeFade

Geographic component which visualizes connections or flows with edges that gradually fade, emphasizing direction and intensity across a map.

**Category:** geo

**Supported Layers:** svg, canvas

### API

| Property | Type | Default | Description |
|----------|------|---------|-------------|
| **link** (required) | `{ source: [ number, number ]; target: [ number, number ]; }` | - | - |
| ref | `SVGGElement` | - | A bindable reference to the underlying `&lt;g&gt;` element. |
| children | `Snippet` | - | - |

**Extends:** `Without<GroupProps, GeoEdgeFadePropsWithoutHTML>`

## GeoPath

Geographic component which renders shapes such as countries, states, or regions by drawing their boundaries based on coordinate data.

**Category:** geo

**Supported Layers:** svg, canvas

## Usage

See example: tooltip

### API

| Property | Type | Default | Description |
|----------|------|---------|-------------|
| geojson | `GeoPermissibleObjects \| null` | - | GeoJSON data to render |
| tooltipContext | `TooltipContextValue \| undefined` | - | Tooltip context to setup pointer events to show tooltip for related data |
| onclick | `((e: MouseEvent, geoPath: ReturnType&lt;typeof geoCurvePath&gt; \| undefined) =&gt; void) \| undefined` | - | Click event handler |
| curve | `CurveFactory \| CurveFactoryLineOnly` | `curveLinearClosed` | Curve of path drawn. Imported via d3-shape. |
| geoTransform | `(projection: GeoProjection \| GeoIdentityTransform) =&gt; GeoTransformPrototype` | - | Apply geo transform to projection. Useful to draw straight lines with `geoMercator` projection. |
| ref | `SVGPathElement` | - | A reference to the underlying `&lt;path&gt;` element |
| children | `Snippet&lt;[ { geoPath: ReturnType&lt;typeof geoCurvePath&gt; \| undefined; } ]&gt;` | - | - |
| fill | `string` | - | Fill color |
| fillOpacity | `number` | - | Fill opacity (0-1) |
| stroke | `string` | - | Stroke color |
| strokeWidth | `number` | - | Stroke width in pixels |
| strokeOpacity | `number` | - | Stroke opacity (0-1) |
| opacity | `number` | - | Overall opacity (0-1) |

**Extends:** `Without<SVGAttributes<SVGPathElement>, GeoPathPropsWithoutHTML>`

### Examples

- [animated-globe](https://layerchart.com/docs/components/GeoPath/animated-globe)
- [bubble-map](https://layerchart.com/docs/components/GeoPath/bubble-map)
- [choropleth](https://layerchart.com/docs/components/GeoPath/choropleth)
- [eclipses-globe](https://layerchart.com/docs/components/GeoPath/eclipses-globe)
- [sketchy-globe](https://layerchart.com/docs/components/GeoPath/sketchy-globe)
- [spike-map](https://layerchart.com/docs/components/GeoPath/spike-map)
- [submarine-cables-globe](https://layerchart.com/docs/components/GeoPath/submarine-cables-globe)
- [timezones](https://layerchart.com/docs/components/GeoPath/timezones)
- [tooltip](https://layerchart.com/docs/components/GeoPath/tooltip)
- [transform-canvas](https://layerchart.com/docs/components/GeoPath/transform-canvas)
- [transform-projection](https://layerchart.com/docs/components/GeoPath/transform-projection)
- [translucent-globe](https://layerchart.com/docs/components/GeoPath/translucent-globe)
- [us-country-map](https://layerchart.com/docs/components/GeoPath/us-country-map)
- [us-state](https://layerchart.com/docs/components/GeoPath/us-state)
- [us-state-with-counties](https://layerchart.com/docs/components/GeoPath/us-state-with-counties)
- [us-state-with-surrounding-states](https://layerchart.com/docs/components/GeoPath/us-state-with-surrounding-states)

### Related

- [Graticule](https://layerchart.com/docs/components/Graticule)

## GeoPoint

Geographic component which plots individual geographic locations as points on a map to visualize spatial distributions or events.

**Category:** geo

**Supported Layers:** svg, canvas

## Usage

See example: us-state-capitals

### API

| Property | Type | Default | Description |
|----------|------|---------|-------------|
| **lat** (required) | `number` | - | Latitude of the point. |
| **long** (required) | `number` | - | Longitude of the point. |
| ref | `Element` | - | A bindable reference to the underlying element, which can be a `&lt;circle&gt;` or `&lt;g&gt;` element. |
| children | `Snippet&lt;[ { x: number; y: number; } ]&gt;` | - | - |

### Examples

- [icons](https://layerchart.com/docs/components/GeoPoint/icons)
- [us-airports](https://layerchart.com/docs/components/GeoPoint/us-airports)
- [us-state-capitals](https://layerchart.com/docs/components/GeoPoint/us-state-capitals)
- [world-airports](https://layerchart.com/docs/components/GeoPoint/world-airports)
- [world-capitals](https://layerchart.com/docs/components/GeoPoint/world-capitals)

## GeoSpline

Geographic component which renders smooth, curved lines connecting geographic points to represent paths or flows on a map.

**Category:** geo

**Supported Layers:** svg, canvas

## Usage

See example: world-map

### API

| Property | Type | Default | Description |
|----------|------|---------|-------------|
| **link** (required) | `{ source: [ number, number ]; target: [ number, number ]; }` | - | Link between two points on the globe. |
| loft | `number` | `1.0` | The amount of loft to apply to the middle of the curve. |
| curve | `CurveFactory \| CurveFactoryLineOnly` | `curveNatural` | Curve of spline drawn. Imported via d3-shape. |

**Extends:** `Without<SplineProps, GeoSplinePropsWithoutHTML>`

### Examples

- [draggable-globe](https://layerchart.com/docs/components/GeoSpline/draggable-globe)
- [world-map](https://layerchart.com/docs/components/GeoSpline/world-map)

## GeoTile

Geographic component which renders map tiles in a grid supporting efficient zooming and panning for larger scale maps.

**Category:** geo

**Supported Layers:** svg, canvas

## Usage

See example: basic

### API

| Property | Type | Default | Description |
|----------|------|---------|-------------|
| **url** (required) | `(x: number, y: number, z: number) =&gt; string` | - | - |
| zoomDelta | `number` | `0` | The zoom delta for the tile. |
| tileSize | `number` | `256` | The tile size for the tile. |
| disableCache | `boolean` | `false` | Whether to disable the cache for the tile. |
| group | `Partial&lt;ComponentProps&lt;typeof Group&gt;&gt;` | - | Additional props to apply to the `Group` component. |
| debug | `boolean` | `false` | Whether to enable debug mode for the tile. |
| children | `Snippet&lt;[ { tiles: any[]; } ]&gt;` | - | - |

### Examples

- [basic](https://layerchart.com/docs/components/GeoTile/basic)
- [clipped](https://layerchart.com/docs/components/GeoTile/clipped)
- [zoomable](https://layerchart.com/docs/components/GeoTile/zoomable)
- [zoomable-seamless-layers](https://layerchart.com/docs/components/GeoTile/zoomable-seamless-layers)
- [zoomable-with-padding](https://layerchart.com/docs/components/GeoTile/zoomable-with-padding)

## GeoVisible

Geographic component which determines and renders only the geographic features currently within the visible map viewport to optimize performance and clarity.

**Category:** geo

**Supported Layers:** svg, canvas

### API

| Property | Type | Default | Description |
|----------|------|---------|-------------|
| **lat** (required) | `number` | - | Latitude |
| **long** (required) | `number` | - | Longitude |
| children | `Snippet` | - | - |

## Graticule

Geographic component which overlays latitude and longitude lines on a map to provide geographic reference and context.

**Category:** geo

**Supported Layers:** svg, canvas

## Usage

See example: basic

### API

| Property | Type | Default | Description |
|----------|------|---------|-------------|
| lines | `Partial&lt;ComponentProps&lt;typeof GeoPath&gt;&gt; \| boolean \| undefined` | - | - |
| outline | `Partial&lt;ComponentProps&lt;typeof GeoPath&gt;&gt; \| boolean \| undefined` | - | - |
| stepX | `number` | - | - |
| stepY | `number` | - | - |

**Extends:** `Without<GeoPathProps, Omit<GraticulePropsWithoutHTML, 'ref'>>`

### Examples

- [basic](https://layerchart.com/docs/components/Graticule/basic)

## Grid

Commonly used component which draws horizontal and vertical lines respecting scales across a chart to enhance readability and help align data points with axis values.

**Category:** common

**Supported Layers:** svg, canvas, html

## Usage

See example: basic

### API

| Property | Type | Default | Description |
|----------|------|---------|-------------|
| x | `boolean \| Pick&lt;SVGAttributes&lt;SVGElement&gt;, 'class' \| 'style'&gt;` | `false` | Draw a x-axis lines |
| y | `boolean \| Pick&lt;SVGAttributes&lt;SVGElement&gt;, 'class' \| 'style'&gt;` | `false` | Draw a y-axis lines |
| xTicks | `TicksConfig` | - | Control the number of x-axis ticks |
| yTicks | `TicksConfig` | `!isScaleBand(ctx.yScale) ? 4 : undefined` | Control the number of y-axis ticks |
| bandAlign | `'center' \| 'between'` | `'center'` | Line alignment when band scale is used (x or y axis) |
| radialY | `'circle' \| 'linear'` | `'circle'` | Render `y` lines with circles or linear splines |
| classes | `{ root?: string; line?: string; }` | `{}` | Classes to apply to the rendered elements. |
| transitionIn | `In` | `defaults to fade if motion is tweened` | Transition function for entering elements |
| transitionInParams | `TransitionParams&lt;In&gt;` | `{ easing: cubicIn }` | Parameters for the transitionIn function |
| ref | `SVGGElement` | - | A reference to the underlying outermost `&lt;g&gt;` element. |
| motion | `MotionProp` | - | - |

### Examples

- [band-scale-between](https://layerchart.com/docs/components/Grid/band-scale-between)
- [band-scale-default](https://layerchart.com/docs/components/Grid/band-scale-default)
- [basic](https://layerchart.com/docs/components/Grid/basic)
- [dashed-lines](https://layerchart.com/docs/components/Grid/dashed-lines)
- [explicit-ticks](https://layerchart.com/docs/components/Grid/explicit-ticks)
- [inject-tick-value](https://layerchart.com/docs/components/Grid/inject-tick-value)
- [integer-only](https://layerchart.com/docs/components/Grid/integer-only)
- [padding](https://layerchart.com/docs/components/Grid/padding)
- [radial](https://layerchart.com/docs/components/Grid/radial)
- [radial-linear](https://layerchart.com/docs/components/Grid/radial-linear)
- [tick-count](https://layerchart.com/docs/components/Grid/tick-count)
- [tick-count-null](https://layerchart.com/docs/components/Grid/tick-count-null)
- [tick-count-responsive](https://layerchart.com/docs/components/Grid/tick-count-responsive)
- [x-only](https://layerchart.com/docs/components/Grid/x-only)
- [y-only](https://layerchart.com/docs/components/Grid/y-only)

### Related

- [Axis](https://layerchart.com/docs/components/Axis)
- [Rule](https://layerchart.com/docs/components/Rule)

## Group

Primitive component which clusters multiple chart elements together, allowing them to be managed, styled, or transformed as a single unit.

**Category:** primitives

**Supported Layers:** svg, html

## Usage

See example: basic

### API

| Property | Type | Default | Description |
|----------|------|---------|-------------|
| x | `number` | - | Translate x |
| initialX | `number` | `x` | Initial translate x |
| y | `number` | - | Translate y |
| initialY | `number` | `y` | Initial translate y |
| center | `boolean \| 'x' \| 'y'` | `false` | Center within chart |
| preventTouchMove | `boolean` | `false` | Prevent `touchmove` default, which can interfere with `pointermove` when used with `Tooltip`, for example. |
| opacity | `number` | - | The opacity of the element. (0 to 1) |
| children | `Snippet` | - | - |
| ref | `Element` | - | A reference to the rendered DOM element, which could be either nothing, a `&lt;g&gt;` element (when using `&lt;Svg&gt;`), or a `&lt;div&gt;` element (when using `&lt;Html&gt;`). |
| motion | `MotionProp` | - | - |
| transitionIn | `In` | `defaults to fade if the motion prop is set to tweened` | Transition function for entering elements |
| transitionInParams | `TransitionParams&lt;In&gt;` | `{ easing: cubicIn }` | Parameters for the transitionIn function |

**Extends:** `Without<HTMLAttributes<Element>, GroupPropsWithoutHTML>`

### Examples

- [basic](https://layerchart.com/docs/components/Group/basic)

## Highlight

Interaction component manages and displays tooltips allowing dynamic information to appear in response to user interactions.

**Category:** interactions

**Supported Layers:** svg, canvas

### API

| Property | Type | Default | Description |
|----------|------|---------|-------------|
| data | `any` | - | Highlight specific data (annotate), especte uses tooltip data |
| x | `Accessor` | - | Override `x` from context |
| y | `Accessor` | - | Override `y` from context |
| axis | `'x' \| 'y' \| 'both' \| 'none'` | - | - |
| points | `boolean \| Partial&lt;ComponentProps&lt;typeof Circle&gt;&gt; \| Snippet&lt;[ { points: { x: number; y: number; fill: string; data: HighlightPointData; }[]; } ]&gt;` | `false` | Show points and pass props to Circles |
| lines | `boolean \| Partial&lt;ComponentProps&lt;typeof Line&gt;&gt; \| Snippet&lt;[ { lines: { x1: number; y1: number; x2: number; y2: number; }[]; } ]&gt;` | `false` | Show lines and pass props to Lines |
| area | `boolean \| Partial&lt;ComponentProps&lt;typeof Rect&gt;&gt; \| Snippet&lt;[ { area: { x: number; y: number; width: number; height: number; }; } ]&gt;` | `false` | Show area and pass props to Rect |
| bar | `boolean \| Partial&lt;ComponentProps&lt;typeof Bar&gt;&gt; \| Snippet` | `false` | Show bar and pass props to Rect |
| motion | `MotionProp` | `true` | Set to false to disable spring transitions |
| opacity | `number` | - | The opacity of the element. (0 to 1) |
| onAreaClick | `(e: MouseEvent, detail: { data: any; }) =&gt; void` | - | - |
| onBarClick | `(e: MouseEvent, detail: { data: any; }) =&gt; void` | - | - |
| onPointClick | `(e: MouseEvent, detail: { point: HighlightPoint; data: any; }) =&gt; void` | - | - |
| onPointEnter | `(e: MouseEvent, detail: { point: HighlightPoint; data: any; }) =&gt; void` | - | - |
| onPointLeave | `(e: MouseEvent, detail: { point: HighlightPoint; data: any; }) =&gt; void` | - | - |

### Related

- [Tooltip](https://layerchart.com/docs/components/Tooltip)
- [TooltipContext](https://layerchart.com/docs/components/TooltipContext)

## Html

Html layer

**Category:** layers

Typically you will use `<Layer type="html">`

### API

| Property | Type | Default | Description |
|----------|------|---------|-------------|
| ref | `HTMLElement` | - | A reference to the layer's outermost `&lt;div&gt;` tag. |
| zIndex | `number` | - | The layer's z-index. |
| pointerEvents | `boolean` | - | Set this to `false` to set `pointer-events: none;` on the entire layer. |
| role | `string` | - | A string passed to the `aria-role` on the `&lt;div&gt;` tag. This is `undefined` by default but will be set by default to `'figure'` if `label`, `labelledby` or `describedby` is set. That default will be overridden by whatever is passed in. |
| center | `boolean \| 'x' \| 'y'` | - | Translate children to center (useful for radial layouts) |
| ignoreTransform | `boolean` | - | Ignore TransformContext.  Useful to add static elements such as legends. |
| clip | `boolean` | `false` | Whether to clip overflow content. When true, sets `overflow: hidden` on the layer. |
| children | `Snippet&lt;[ { ref: HTMLElement; } ]&gt;` | - | - |

**Extends:** `Without<HTMLAttributes<HTMLElement>, HTMLPropsWithoutHTML>`

### Related

- [Layer](https://layerchart.com/docs/components/Layer)

## Hull

Marking component which encloses a set of data points within a convex boundary to highlight clusters or groupings on a chart.

**Category:** marks

**Supported Layers:** svg, canvas

## Usage

See example: scatter

### Geo context

Hull can also be used within a geo context (i.e. `<Chart geo={{ projection: ... }}>`)

See example: geo

### API

| Property | Type | Default | Description |
|----------|------|---------|-------------|
| data | `any` | - | Override data instead of using context |
| curve | `ComponentProps&lt;typeof Spline&gt;['curve']` | `curveLinearClosed` | The curve factory to use for the hull |
| classes | `{ root?: string; path?: string; }` | - | Classes to apply to the elements. |
| onpointermove | `(e: PointerEvent, details: { points: [ number, number ][]; polygon: Delaunay.Polygon; }) =&gt; void` | - | - |
| onclick | `(e: MouseEvent, details: { points: [ number, number ][]; polygon: Delaunay.Polygon; }) =&gt; void` | - | - |
| onpointerleave | `(e: PointerEvent) =&gt; void` | - | - |
| ref | `SVGGElement` | - | A bindable reference to the wrapping `&lt;g&gt;` element. |

**Extends:** `Without<GroupProps, HullPropsWithoutHTML>`

### Examples

- [geo](https://layerchart.com/docs/components/Hull/geo)
- [scatter](https://layerchart.com/docs/components/Hull/scatter)

## Labels

Marking component which displays text directly on a chart to identify or annotate specific data points.

**Category:** marks

**Supported Layers:** svg, canvas

## Usage

:example{ component="Bars" name="vertical-outside-labels-default" showCode }

### Bar charts

By default labels will be on the outside of bars, above for positive values and below for negative values

:example{ component="Bars" name="vertical-outside-labels-default" showCode }

You can also use `placement="inside"` to place within the bars (near the value)

:example{ component="Bars" name="vertical-inside-labels" }

### Line charts

:example{ component="Spline" name="with-labels" }

### Scatter charts

:example{ component="Points" name="with-labels" }

### Simplified charts

Labels are also integrated in simplified charts via the `labels` prop

:example{ component="BarChart" name="labels" }

### API

| Property | Type | Default | Description |
|----------|------|---------|-------------|
| data | `T` | - | Override data instead of using context |
| value | `Accessor&lt;T&gt;` | - | Override display value accessor.  By default, uses `y` unless yScale is band scale |
| fill | `string \| Accessor&lt;T&gt;` | - | The fill color of the label, which can either be a string or an accessor function that returns a valid `fill` color value.  The accessor is useful for dynamic fill colors based on the data the label represents. |
| x | `Accessor&lt;T&gt;` | - | Override `x` accessor from Chart context |
| y | `Accessor&lt;T&gt;` | - | Override `y` accessor from Chart context |
| placement | `'inside' \| 'outside' \| 'center'` | `'outside'` | The placement of the label relative to the point |
| offset | `number` | `placement === 'center' ? 0 : 4` | The offset of the label from the point |
| format | `FormatType \| FormatConfig` | - | The format of the label |
| key | `(d: T, index: number) =&gt; any` | `(d, index) =&gt; index` | Define unique value for {#each} `(key)` expressions to improve transitions. `index` position used by default |
| children | `Snippet&lt;[ { data: Point; textProps: ComponentProps&lt;typeof Text&gt;; } ]&gt;` | - | - |

**Extends:** `Without<TextProps, LabelsPropsWithoutHTML<T>>`

## Layer

Convientent wrapper to use Svg, Canvas, or Html layer

**Category:** common

**Supported Layers:** svg, canvas, html

## Usage

### All 3 types

See example: all-3-layers

### API

| Property | Type | Default | Description |
|----------|------|---------|-------------|
| onpointermove | `(e: PointerEvent) =&gt; void` | - | - |

### Examples

- [all-3-layers](https://layerchart.com/docs/components/Layer/all-3-layers)

### Related

- [Canvas](https://layerchart.com/docs/components/Canvas)
- [Html](https://layerchart.com/docs/components/Html)
- [Svg](https://layerchart.com/docs/components/Svg)

## Legend

Commonly used component which explains the symbols, colors, or patterns used in a chart, helping interpret the represented data categories. Filtering and toggling visibility of data series can be enabled for interactivity.

**Category:** common

**Supported Layers:** html

## Usage

See example: sequential

### API

| Property | Type | Default | Description |
|----------|------|---------|-------------|
| scale | `AnyScale` | - | - |
| title | `string` | `''` | The title of the legend. |
| width | `number` | `320` | The width of the legend. |
| height | `number` | `10` | The height of the legend. |
| ticks | `number` | `width / 64` | The number of ticks to show on the scale. |
| tickFormat | `FormatType \| FormatConfig` | - | - |
| tickValues | `any[]` | - | - |
| tickFontSize | `number` | `10` | The font size of the tick labels. |
| tickLength | `number` | `4` | The length of the tick marks. |
| placement | `Placement` | - | The placement of the legend. |
| orientation | `'horizontal' \| 'vertical'` | `'horizontal'` | The orientation of the legend. |
| variant | `'ramp' \| 'swatches'` | `'ramp'` | Determine display ramp (individual color swatches or continuous ramp) |
| selected | `string[]` | - | An array of selected items. If provided, the legend fades unselected items. |
| classes | `{ root?: string; title?: string; label?: string; tick?: string; items?: string; swatch?: string; item?: string \| ((item: LegendItem) =&gt; string); }` | `{}` | Classes to apply to the elements. |
| onclick | `(e: MouseEvent, detail: LegendItem) =&gt; any` | - | - |
| onpointerenter | `(e: MouseEvent, detail: LegendItem) =&gt; any` | - | - |
| onpointerleave | `(e: MouseEvent, detail: LegendItem) =&gt; any` | - | - |
| ref | `HTMLElement` | - | A bindable reference to the wrapping `&lt;div&gt;` element. |
| children | `Snippet&lt;[ { values: any[]; scale: AnyScale \| null; } ]&gt;` | - | - |

**Extends:** `Without<HTMLAttributes<HTMLElement>, LegendPropsWithoutHTML>`

### Examples

- [chart-integration](https://layerchart.com/docs/components/Legend/chart-integration)
- [chart-placement](https://layerchart.com/docs/components/Legend/chart-placement)
- [children-override](https://layerchart.com/docs/components/Legend/children-override)
- [click-handler](https://layerchart.com/docs/components/Legend/click-handler)
- [diverging](https://layerchart.com/docs/components/Legend/diverging)
- [diverging-sqrt](https://layerchart.com/docs/components/Legend/diverging-sqrt)
- [ordinal](https://layerchart.com/docs/components/Legend/ordinal)
- [quantile](https://layerchart.com/docs/components/Legend/quantile)
- [quantize](https://layerchart.com/docs/components/Legend/quantize)
- [responsive-swatches](https://layerchart.com/docs/components/Legend/responsive-swatches)
- [sequential](https://layerchart.com/docs/components/Legend/sequential)
- [sequential-log](https://layerchart.com/docs/components/Legend/sequential-log)
- [sequential-quantile](https://layerchart.com/docs/components/Legend/sequential-quantile)
- [sequential-sqrt](https://layerchart.com/docs/components/Legend/sequential-sqrt)
- [sqrt](https://layerchart.com/docs/components/Legend/sqrt)
- [square-swatch](https://layerchart.com/docs/components/Legend/square-swatch)
- [styling](https://layerchart.com/docs/components/Legend/styling)
- [threshold](https://layerchart.com/docs/components/Legend/threshold)
- [vertical-orientation](https://layerchart.com/docs/components/Legend/vertical-orientation)

## Line

Primitive component which draws a straight line on a chart to represent trends, connections, or boundaries between data points.

**Category:** primitives

**Supported Layers:** svg, canvas, html

## Usage

See example: styling-using-classes

### API

| Property | Type | Default | Description |
|----------|------|---------|-------------|
| **x1** (required) | `number` | - | The x-coordinate of the line's starting point |
| initialX1 | `number` | `x1` | The initial x-coordinate of the line's starting point (defaults to x1 if not provided) |
| **y1** (required) | `number` | - | The y-coordinate of the line's starting point |
| initialY1 | `number` | `y1` | The initial y-coordinate of the line's starting point (defaults to y1 if not provided) |
| **x2** (required) | `number` | - | The x-coordinate of the line's ending point |
| initialX2 | `number` | `x2` | The initial x-coordinate of the line's ending point (defaults to x2 if not provided) |
| **y2** (required) | `number` | `y2` | The y-coordinate of the line's ending point |
| initialY2 | `number` | `y2` | The initial y-coordinate of the line's ending point (defaults to y2 if not provided) |
| marker | `MarkerOptions` | - | Marker to attach to both start and end points of the line |
| markerStart | `MarkerOptions` | - | Marker to attach to the start point of the line |
| markerMid | `MarkerOptions` | - | Marker to attach to the mid point of the line |
| markerEnd | `MarkerOptions` | - | Marker to attach to the end point of the line |
| motion | `MotionProp` | - | - |
| fill | `string` | - | Fill color |
| fillOpacity | `number` | - | Fill opacity (0-1) |
| stroke | `string` | - | Stroke color |
| strokeWidth | `number` | - | Stroke width in pixels |
| strokeOpacity | `number` | - | Stroke opacity (0-1) |
| opacity | `number` | - | Overall opacity (0-1) |

**Extends:** `Without<SVGAttributes<SVGPathElement>, LinePropsWithoutHTML>`

### Examples

- [styling-using-attributes](https://layerchart.com/docs/components/Line/styling-using-attributes)
- [styling-using-classes](https://layerchart.com/docs/components/Line/styling-using-classes)
- [styling-using-css-variables](https://layerchart.com/docs/components/Line/styling-using-css-variables)

### Related

- [Rule](https://layerchart.com/docs/components/Rule)
- [Spline](https://layerchart.com/docs/components/Spline)

## LinearGradient

Fill component providing a linear gradient fill pattern for chart elements.

**Category:** fill

**Supported Layers:** svg, canvas, html

## Usage

### Direction with custom colors

See example: direction-with-custom-colors

### Explicit offsets

See example: explicit-offsets

### Tailwind colors

See example: tailwind-colors

### Units

See example: units

### API

| Property | Type | Default | Description |
|----------|------|---------|-------------|
| id | `string` | - | Unique id for linearGradient |
| stops | `string[] \| [ string \| number, string ][]` | ``['var(--tw-gradient-from)', 'var(--tw-gradient-to)']`` | Array array of strings (colors), will equally distributed from 0-100%. If array of tuples, will use first value as the offset, and second as color |
| vertical | `boolean` | `false` | Apply color stops top-to-bottom (true) or left-to-right (false) |
| x1 | `string` | `'0%'` | - |
| y1 | `string` | `'0%'` | - |
| x2 | `string` | `vertical ? '0%' : '100%'` | - |
| y2 | `string` | `vertical ? '100%' : '0%'` | - |
| rotate | `number` | - | Rotate the gradient by a given angle in degrees |
| units | `'objectBoundingBox' \| 'userSpaceOnUse'` | `'objectBoundingBox'` | Define the coordinate system for attributes (i.e. gradientUnits) |
| ref | `SVGLinearGradientElement` | - | A bindable reference to the underlying `&lt;linearGradient&gt;` element |
| stopsContent | `Snippet` | - | Render as a child of the gradient and will opt out of the default stops being rendered. |
| children | `Snippet&lt;[ { id: string; gradient: string; } ]&gt;` | - | - |

**Extends:** `Without<SVGAttributes<SVGLinearGradientElement>, LinearGradientPropsWithoutHTML>`

### Examples

- [direction-with-custom-colors](https://layerchart.com/docs/components/LinearGradient/direction-with-custom-colors)
- [explicit-offsets](https://layerchart.com/docs/components/LinearGradient/explicit-offsets)
- [tailwind-colors](https://layerchart.com/docs/components/LinearGradient/tailwind-colors)
- [units](https://layerchart.com/docs/components/LinearGradient/units)

### Related

- [RadialGradient](https://layerchart.com/docs/components/RadialGradient)
- [Pattern](https://layerchart.com/docs/components/Pattern)

## LineChart

Streamlined visualization of data points connected by lines to visualize trends or changes over time.

**Category:** charts

**Supported Layers:** svg, canvas

## Usage

See example: basic

### API

| Property | Type | Default | Description |
|----------|------|---------|-------------|
| orientation | `'vertical' \| 'horizontal'` | `'horizontal'` | The orientation of the line chart. |
| props | `LineChartPropsObjProp` | - | - |
| spline | `SimplifiedChartSnippet&lt;TData, typeof Spline, LineChartExtraSnippetProps&lt;TData&gt; & { props: ComponentProps&lt;typeof Spline&gt;; seriesIndex: number; }&gt;` | - | - |
| onPointClick | `(e: MouseEvent, details: { data: HighlightPointData; series: SeriesData&lt;TData, typeof Spline&gt;; }) =&gt; void` | - | The event to be dispatched when the point is clicked. |

**Extends:** `SimplifiedChartProps<
    TData,
    typeof Spline,
    LineChartExtraSnippetProps<TData>
  >`

### Examples

- [axis-labels-inside](https://layerchart.com/docs/components/LineChart/axis-labels-inside)
- [basic](https://layerchart.com/docs/components/LineChart/basic)
- [brush](https://layerchart.com/docs/components/LineChart/brush)
- [curve](https://layerchart.com/docs/components/LineChart/curve)
- [custom](https://layerchart.com/docs/components/LineChart/custom)
- [custom-tooltip](https://layerchart.com/docs/components/LineChart/custom-tooltip)
- [default-series-label](https://layerchart.com/docs/components/LineChart/default-series-label)
- [draw](https://layerchart.com/docs/components/LineChart/draw)
- [dynamic-data](https://layerchart.com/docs/components/LineChart/dynamic-data)
- [gradient-encoding](https://layerchart.com/docs/components/LineChart/gradient-encoding)
- [gradient-threshold](https://layerchart.com/docs/components/LineChart/gradient-threshold)
- [labels](https://layerchart.com/docs/components/LineChart/labels)
- [labels-with-points](https://layerchart.com/docs/components/LineChart/labels-with-points)
- [labels-within-points](https://layerchart.com/docs/components/LineChart/labels-within-points)
- [large-radial-series](https://layerchart.com/docs/components/LineChart/large-radial-series)
- [large-series](https://layerchart.com/docs/components/LineChart/large-series)
- [legend](https://layerchart.com/docs/components/LineChart/legend)
- [line-annotation](https://layerchart.com/docs/components/LineChart/line-annotation)
- [null-dashed-gaps](https://layerchart.com/docs/components/LineChart/null-dashed-gaps)
- [null-gaps](https://layerchart.com/docs/components/LineChart/null-gaps)
- [oscilloscope-time](https://layerchart.com/docs/components/LineChart/oscilloscope-time)
- [override-color](https://layerchart.com/docs/components/LineChart/override-color)
- [perf-dimension-arrays](https://layerchart.com/docs/components/LineChart/perf-dimension-arrays)
- [perf-dimension-arrays-processed](https://layerchart.com/docs/components/LineChart/perf-dimension-arrays-processed)
- [perf-series-arrays](https://layerchart.com/docs/components/LineChart/perf-series-arrays)
- [perf-streaming](https://layerchart.com/docs/components/LineChart/perf-streaming)
- [perf-wide-data](https://layerchart.com/docs/components/LineChart/perf-wide-data)
- [perf-wide-data-processed](https://layerchart.com/docs/components/LineChart/perf-wide-data-processed)
- [point-annotations](https://layerchart.com/docs/components/LineChart/point-annotations)
- [points](https://layerchart.com/docs/components/LineChart/points)
- [radar](https://layerchart.com/docs/components/LineChart/radar)
- [radar-rounded](https://layerchart.com/docs/components/LineChart/radar-rounded)
- [radar-series](https://layerchart.com/docs/components/LineChart/radar-series)
- [range-annotation](https://layerchart.com/docs/components/LineChart/range-annotation)
- [series](https://layerchart.com/docs/components/LineChart/series)
- [series-brush](https://layerchart.com/docs/components/LineChart/series-brush)
- [series-custom-highlight-point](https://layerchart.com/docs/components/LineChart/series-custom-highlight-point)
- [series-individual-tooltip](https://layerchart.com/docs/components/LineChart/series-individual-tooltip)
- [series-labels-hover](https://layerchart.com/docs/components/LineChart/series-labels-hover)
- [series-point-annotations](https://layerchart.com/docs/components/LineChart/series-point-annotations)
- [series-point-click](https://layerchart.com/docs/components/LineChart/series-point-click)
- [series-separate-data](https://layerchart.com/docs/components/LineChart/series-separate-data)
- [series-separate-data-diff-length](https://layerchart.com/docs/components/LineChart/series-separate-data-diff-length)
- [series-vertical](https://layerchart.com/docs/components/LineChart/series-vertical)
- [series-with-nulls](https://layerchart.com/docs/components/LineChart/series-with-nulls)
- [single-axis-x](https://layerchart.com/docs/components/LineChart/single-axis-x)
- [single-axis-y](https://layerchart.com/docs/components/LineChart/single-axis-y)
- [sparkline](https://layerchart.com/docs/components/LineChart/sparkline)
- [sparkline-fixed-position-tooltip](https://layerchart.com/docs/components/LineChart/sparkline-fixed-position-tooltip)
- [sparkline-within-a-paragraph](https://layerchart.com/docs/components/LineChart/sparkline-within-a-paragraph)
- [sparkline-within-a-paragraph-with-tooltip-and-highlight](https://layerchart.com/docs/components/LineChart/sparkline-within-a-paragraph-with-tooltip-and-highlight)
- [sparkline-zero-axis](https://layerchart.com/docs/components/LineChart/sparkline-zero-axis)
- [threshold](https://layerchart.com/docs/components/LineChart/threshold)
- [tooltip-click](https://layerchart.com/docs/components/LineChart/tooltip-click)
- [vertical](https://layerchart.com/docs/components/LineChart/vertical)

### Related

- [Chart](https://layerchart.com/docs/components/Chart)
- [Spline](https://layerchart.com/docs/components/Spline)

## Link

Marking component which highlights or connects specific data points on a chart to emphasize relationships or sequences.

**Category:** marks

**Supported Layers:** svg, canvas

## Usage

:example{ component="Tree" name="basic" }

### API

| Property | Type | Default | Description |
|----------|------|---------|-------------|
| data | `any` | - | - |
| sankey | `boolean` | `false` | Update source and target accessors to be compatible with d3-sankey.  see: https://github.com/d3/d3-sankey#sankeyLinkHorizontal |
| source | `(d: any) =&gt; any` | - | - |
| target | `(d: any) =&gt; any` | - | - |
| orientation | `'vertical' \| 'horizontal'` | - | Convenient property to swap x/y accessor logic |
| x | `(d: any) =&gt; any` | - | - |
| y | `(d: any) =&gt; any` | - | - |
| curve | `CurveFactory` | - | - |
| marker | `MarkerOptions` | - | Marker to attach to both start and end points of the line |
| markerMid | `MarkerOptions` | - | Marker to attach to the middle point of the line |
| markerStart | `MarkerOptions` | - | Marker to attach to the start point of the line |
| markerEnd | `MarkerOptions` | - | Marker to attach to the end point of the line |
| explicitCoords | `{ x1: number; y1: number; x2: number; y2: number; }` | - | Apply explicit coordinates to the line. Useful when dealing with force simulation links. |
| motion | `MotionTweenOption \| MotionNoneOption` | - | - |

**Extends:** `Without<ConnectorProps, LinkPropsWithoutHTML>`

### Related

- [Connector](https://layerchart.com/docs/components/Connector)
- [Points](https://layerchart.com/docs/components/Points)

## Marker

Primitive component which draws visual symbols like circles, squares, or custom shapes at individual data points.

**Category:** primitives

**Supported Layers:** svg

## Usage

See example: spline

### API

| Property | Type | Default | Description |
|----------|------|---------|-------------|
| type | `'arrow' \| 'triangle' \| 'line' \| 'circle' \| 'circle-stroke' \| 'dot'` | - | The type of marker to render (e.g., arrow, triangle, etc.)  Pass `children` to render a custom element/component inside the marker instead. |
| id | `string` | - | Unique identifier for the marker |
| size | `number` | `10` | Size of the marker (used as default for width and height if not overridden) |
| markerWidth | `string \| number` | `size` | Width of the marker (can be a string or number) |
| markerHeight | `string \| number` | `size` | Height of the marker (can be a string or number) |
| markerUnits | `'userSpaceOnUse' \| 'strokeWidth'` | `'userSpaceOnUse'` | Units for marker dimensions ('userSpaceOnUse' or 'strokeWidth') |
| orient | `'auto' \| 'auto-start-reverse' \| number` | `'auto-start-reverse'` | Orientation of the marker ('auto', 'auto-start-reverse', or a specific angle in degrees) |
| refX | `string \| number` | `9 if type is 'arrow' or 'triangle', otherwise 5` | X-coordinate offset of the marker's reference point |
| refY | `string \| number` | `5` | Y-coordinate offset of the marker's reference point |
| viewBox | `string` | `'0 0 10 10'` | Viewbox defining the coordinate system for the marker (e.g., '0 0 10 10') |

**Extends:** `Without<SVGAttributes<SVGMarkerElement>, MarkerPropsWithoutHTML>`

### Examples

- [line](https://layerchart.com/docs/components/Marker/line)
- [orientation](https://layerchart.com/docs/components/Marker/orientation)
- [spline](https://layerchart.com/docs/components/Marker/spline)
- [spline-w-thicker-stroke](https://layerchart.com/docs/components/Marker/spline-w-thicker-stroke)
- [styling](https://layerchart.com/docs/components/Marker/styling)

### Related

- [Spline](https://layerchart.com/docs/components/Spline)
- [Line](https://layerchart.com/docs/components/Line)
- [Rule](https://layerchart.com/docs/components/Rule)
- [GeoSpline](https://layerchart.com/docs/components/GeoSpline)

## MotionPath

Component animates an object along a specified path using configurable motion parameters such as speed, duration, and easing.

**Category:** other

**Supported Layers:** svg

## Usage

### Repeat indefinitely

See example: repeat-indefinitely

### Rotate object with path

See example: rotate-object-with-path

### Sync with draw

See example: sync-with-draw

> Because the draw transition and `animateMotion` using different timers, there is no guarantee they will start at the same time

### API

| Property | Type | Default | Description |
|----------|------|---------|-------------|
| pathId | `string` | - | Id of path to move object along |
| objectId | `string` | - | Id of object to move along path |
| **duration** (required) | `string` | - | Duration of the animation |
| repeatCount | `number \| 'indefinite'` | - | Number of times the animation will occur |
| fill | `'freeze' \| 'remove'` | `'freeze'` | Final state of the animation.  Freeze (last frame) or remove (first frame) https://developer.mozilla.org/en-US/docs/Web/SVG/Attribute/fill#animate |
| rotate | `number \| 'auto' \| 'auto-reverse'` | - | Rotation applied to the element animated along a path, usually to make it pointing in the direction of the animation |
| ref | `SVGAnimateMotionElement` | - | A bindable reference to the underlying `&lt;animateMotion&gt;` element. |
| children | `Snippet&lt;[ { pathId: string; objectId: string; } ]&gt;` | - | - |

**Extends:** `Without<
      Omit<SVGAttributes<SVGAnimateMotionElement>, 'dir' | 'href'>,
      MotionPathPropsWithoutHTML
    >`

### Examples

- [repeat-indefinitely](https://layerchart.com/docs/components/MotionPath/repeat-indefinitely)
- [rotate-object-with-path](https://layerchart.com/docs/components/MotionPath/rotate-object-with-path)
- [sync-with-draw](https://layerchart.com/docs/components/MotionPath/sync-with-draw)

### Related

- [Spline](https://layerchart.com/docs/components/Spline)

## Pack

Layout component which creates a space-efficient, circular layout to represent hierarchical data, where each node is depicted as a circle sized according to its value and nested within its parent node.

**Category:** layout

**Supported Layers:** svg, canvas

## Usage

See example: basic

### API

| Property | Type | Default | Description |
|----------|------|---------|-------------|
| **hierarchy** (required) | `HierarchyNode&lt;T&gt;` | - | d3 hierarchy node |
| size | `[ number, number ]` | - | The size of the pack layout. |
| padding | `number` | - | The padding between nodes in the pack layout. |
| nodes | `HierarchyCircularNode&lt;T&gt;[]` | - | A bindable reference to the computed packed nodes. |
| children | `Snippet&lt;[ { nodes: HierarchyCircularNode&lt;T&gt;[]; } ]&gt;` | - | - |

### Examples

- [basic](https://layerchart.com/docs/components/Pack/basic)

## Partition

Layout component which divides a hierarchical dataset into nested, space-filling rectangles or arcs to represent the structure and relative sizes of each node.

**Category:** layout

**Supported Layers:** svg, canvas

## Usage

See example: horizontal

### API

| Property | Type | Default | Description |
|----------|------|---------|-------------|
| **hierarchy** (required) | `HierarchyNode&lt;T&gt;` | - | d3 hierarchy node |
| orientation | `'vertical' \| 'horizontal'` | `'horizontal'` | The orientation of the partition layout. |
| size | `[ number, number ]` | - | The size of the partition layout. |
| padding | `number` | - | The padding between nodes in the partition layout. see: https://github.com/d3/d3-hierarchy#tree_nodeSize |
| round | `boolean` | - | The round property of the partition layout. see: https://github.com/d3/d3-hierarchy#tree_nodeSize |
| nodes | `HierarchyRectangularNode&lt;T&gt;[]` | - | A bindable reference to the descendants of the partition layout. |
| children | `Snippet&lt;[ { nodes: HierarchyRectangularNode&lt;T&gt;[]; } ]&gt;` | - | - |

### Examples

- [filterable](https://layerchart.com/docs/components/Partition/filterable)
- [horizontal](https://layerchart.com/docs/components/Partition/horizontal)
- [sunburst](https://layerchart.com/docs/components/Partition/sunburst)
- [vertical](https://layerchart.com/docs/components/Partition/vertical)

## Path

Primitive component for rendering path elements with support for animation, markers, and custom content at path endpoints.

**Category:** primitives

**Supported Layers:** svg, canvas

## Usage

See example: basic

### API

| Property | Type | Default | Description |
|----------|------|---------|-------------|
| pathData | `string \| undefined \| null` | - | Pass `&lt;path d={...} /&gt;` explicitly instead of calculating from data / context |
| draw | `boolean \| Parameters&lt;typeof _drawTransition&gt;[1]` | - | Whether to animate the drawing of the path over time. Pass either `true` or an object with transition options to enable the transition.  Works best with `tweened` disabled. |
| marker | `MarkerOptions` | - | Marker to attach to both start and end points of the line |
| markerMid | `MarkerOptions` | - | Marker to attach to the middle point of the line |
| markerStart | `MarkerOptions` | - | Marker to attach to the start point of the line |
| markerEnd | `MarkerOptions` | - | Marker to attach to the end point of the line |
| startContent | `Snippet&lt;[ { point: DOMPoint; value: { x: number; y: number; }; } ]&gt;` | - | Add additional content at the start of the line.  Receives `{ point: DOMPoint; value: { x: number; y: number } }` as a snippet prop. |
| endContent | `Snippet&lt;[ { point: DOMPoint; value: { x: number; y: number; }; } ]&gt;` | - | Add additional content at the end of the line.  Receives `{ point: DOMPoint; value: { x: number; y: number } }` as a snippet prop. |
| pathRef | `SVGPathElement` | - | A reference to the `&lt;path&gt;` element. |
| motion | `MotionProp` | - | - |
| fill | `string` | - | Fill color |
| fillOpacity | `number` | - | Fill opacity (0-1) |
| stroke | `string` | - | Stroke color |
| strokeWidth | `number` | - | Stroke width in pixels |
| strokeOpacity | `number` | - | Stroke opacity (0-1) |
| opacity | `number` | - | Overall opacity (0-1) |

**Extends:** `Without<SVGAttributes<SVGPathElement>, PathPropsWithoutHTML>`

### Examples

- [basic](https://layerchart.com/docs/components/Path/basic)

### Related

- [Arc](https://layerchart.com/docs/components/Arc)
- [Area](https://layerchart.com/docs/components/Area)
- [Bar](https://layerchart.com/docs/components/Bar)
- [Connector](https://layerchart.com/docs/components/Connector)
- [GeoPath](https://layerchart.com/docs/components/GeoPath)
- [MotionPath](https://layerchart.com/docs/components/MotionPath)
- [Spline](https://layerchart.com/docs/components/Spline)

## Pattern

Fill component which provides a line or circle-based fill pattern for chart elements.

**Category:** fill

**Supported Layers:** svg, canvas

## Usage

### Lines

See example: lines

### Circles

See example: circles

### With Fill color

See example: with-fill-color

### With LinearGradient

See example: with-lineargradient

### LinearGradient as Pattern

See example: lineargradient-as-pattern

### Lines (custom pattern - svg only)

See example: lines-custom-pattern-svg-only

### Circles (custom pattern - svg only)

See example: circles-custom-pattern-svg-only

### API

| Property | Type | Default | Description |
|----------|------|---------|-------------|
| id | `string` | - | The id of the pattern |
| size | `number` | - | The size of the pattern (sets `width` and `height` as same value). |
| width | `number` | - | The width of the pattern for custom patterns (set by `lines`, etc) |
| height | `number` | - | The height of the pattern for custom patterns (set by `lines`, etc) |
| lines | `boolean \| PatternLineDef \| PatternLineDef[]` | - | The number of lines to render |
| circles | `boolean \| PatternCircleDef \| PatternCircleDef[]` | - | The number of circles to render |
| background | `string` | - | The background color of the pattern |
| patternContent | `Snippet` | - | Render as a child of the pattern |
| children | `Snippet&lt;[ { id: string; pattern: string; } ]&gt;` | - | - |

**Extends:** `Without<SVGAttributes<SVGPatternElement>, PatternPropsWithoutHTML>`

### Examples

- [circles](https://layerchart.com/docs/components/Pattern/circles)
- [circles-custom-pattern-svg-only](https://layerchart.com/docs/components/Pattern/circles-custom-pattern-svg-only)
- [lineargradient-as-pattern](https://layerchart.com/docs/components/Pattern/lineargradient-as-pattern)
- [lines](https://layerchart.com/docs/components/Pattern/lines)
- [lines-custom-pattern-svg-only](https://layerchart.com/docs/components/Pattern/lines-custom-pattern-svg-only)
- [with-fill-color](https://layerchart.com/docs/components/Pattern/with-fill-color)
- [with-lineargradient](https://layerchart.com/docs/components/Pattern/with-lineargradient)

### Related

- [LinearGradient](https://layerchart.com/docs/components/LinearGradient)
- [RadialGradient](https://layerchart.com/docs/components/RadialGradient)

## Pie

Marking component which represents data as proportional slices of a circle, showing the relative contribution of each category to the whole.

**Category:** marks

**Supported Layers:** svg, canvas

> tip: See also: [PieChart](/docs/components/PieChart) for simplified examples

## Usage

See example: basic

### API

| Property | Type | Default | Description |
|----------|------|---------|-------------|
| data | `any[]` | - | - |
| range | `[ number, number ] \| number[]` | `[0, 360]` | Range [min,max] in degrees.  See also startAngle/endAngle |
| startAngle | `number` | - | Start angle in radians |
| endAngle | `number` | - | End angle in radians |
| innerRadius | `number` | - | Define innerRadius.   value &gt;= 1: discrete value   value &gt;  0: percent of `outerRadius`   value &lt;  0: offset of `outerRadius`   default: yRange min |
| outerRadius | `number` | - | Define outerRadius.  Defaults to yRange max/2 (ie. chart height / 2) |
| cornerRadius | `number` | `0` | Corner radius of the arc |
| padAngle | `number` | `0` | Angle between the arcs |
| offset | `number` | `0` | Offset all arcs from center |
| tooltipContext | `TooltipContextValue` | - | Tooltip context to setup pointer events to show tooltip for related data |
| sort | `((a: any, b: any) =&gt; number) \| null` | - | Sort function to sort the arcs |
| children | `Snippet&lt;[ { arcs: PieArcDatum&lt;any&gt;[]; } ]&gt;` | - | - |
| motion | `MotionProp` | - | - |

### Examples

- [basic](https://layerchart.com/docs/components/Pie/basic)
- [default-slot-render-each-arc](https://layerchart.com/docs/components/Pie/default-slot-render-each-arc)
- [disable-sorting](https://layerchart.com/docs/components/Pie/disable-sorting)
- [inner-radius-negative](https://layerchart.com/docs/components/Pie/inner-radius-negative)
- [inner-radius-positive](https://layerchart.com/docs/components/Pie/inner-radius-positive)
- [inner-radius-zero-one](https://layerchart.com/docs/components/Pie/inner-radius-zero-one)
- [labels](https://layerchart.com/docs/components/Pie/labels)
- [labels-centroid](https://layerchart.com/docs/components/Pie/labels-centroid)
- [labels-centroid-multiple](https://layerchart.com/docs/components/Pie/labels-centroid-multiple)
- [labels-outer](https://layerchart.com/docs/components/Pie/labels-outer)
- [labels-outer-radial](https://layerchart.com/docs/components/Pie/labels-outer-radial)
- [labels-outer-with-padding](https://layerchart.com/docs/components/Pie/labels-outer-with-padding)
- [multiple-data-prop](https://layerchart.com/docs/components/Pie/multiple-data-prop)
- [offset](https://layerchart.com/docs/components/Pie/offset)
- [outer-radius](https://layerchart.com/docs/components/Pie/outer-radius)
- [pad-angle](https://layerchart.com/docs/components/Pie/pad-angle)
- [pad-angle-with-inner-radius](https://layerchart.com/docs/components/Pie/pad-angle-with-inner-radius)
- [partial-range-chart-xrange](https://layerchart.com/docs/components/Pie/partial-range-chart-xrange)
- [partial-range-range-prop](https://layerchart.com/docs/components/Pie/partial-range-range-prop)
- [placement](https://layerchart.com/docs/components/Pie/placement)
- [placement-center](https://layerchart.com/docs/components/Pie/placement-center)
- [placement-left](https://layerchart.com/docs/components/Pie/placement-left)
- [placement-right](https://layerchart.com/docs/components/Pie/placement-right)
- [tooltip](https://layerchart.com/docs/components/Pie/tooltip)
- [tooltip-with-arcs-slot](https://layerchart.com/docs/components/Pie/tooltip-with-arcs-slot)
- [tweened](https://layerchart.com/docs/components/Pie/tweened)

### Related

- [Arc](https://layerchart.com/docs/components/Arc)
- [PieChart](https://layerchart.com/docs/components/PieChart)

## PieChart

Streamlined visualization of data as proportional slices of a circle to represent parts of a whole.

**Category:** charts

**Supported Layers:** svg, canvas

## Usage

See example: basic

### API

| Property | Type | Default | Description |
|----------|------|---------|-------------|
| key | `Accessor&lt;TData&gt;` | `'key'` | Key accessor |
| label | `Accessor&lt;TData&gt;` | `'label'` | Label accessor |
| value | `Accessor&lt;TData&gt;` | `'value'` | Value accessor |
| c | `Accessor&lt;TData&gt;` | `key` | Color accessor |
| maxValue | `number` | - | Maximum possible value, useful when `data` is single item |
| range | `[ number, number ]` | `[0, 360]` | Range [min, max] in degrees.  See also `startAngle`/`endAngle` |
| props | `PieChartPropsObjProp` | - | - |
| innerRadius | `number` | - | Inner radius of the arc.   value &gt;= 1: discrete value   value &gt;  0: percent of `outerRadius`   value &lt;  0: offset of `outerRadius` |
| outerRadius | `number` | - | Outer radius of the arc. |
| cornerRadius | `number` | `0` | Corner radius of the arc |
| padAngle | `number` | `0` | Angle between the arcs |
| placement | `'left' \| 'center' \| 'right'` | `'center'` | Placement of the PieChart |
| center | `boolean` | `placement === 'center'` | Center the chart.  Override and use `props.group` for more control. |
| pie | `SimplifiedChartSnippet&lt;TData, typeof Arc, PieChartExtraSnippetProps&lt;TData&gt; & { props: ComponentProps&lt;typeof Pie&gt;; index: number; }&gt;` | - | Replace the default rendering of the `&lt;Pie&gt;` component internally with your own.  Use the `props` snippet prop to access the default props. |
| arc | `SimplifiedChartSnippet&lt;TData, typeof Arc, PieChartExtraSnippetProps&lt;TData&gt; & { props: ComponentProps&lt;typeof Arc&gt;; index: number; seriesIndex: number; }&gt;` | - | Replace the default rendering of the `&lt;Arc&gt;` component internally with your own.  Use the `props` snippet prop to access the default props. |
| onArcClick | `(e: MouseEvent, detail: { data: any; series: SeriesData&lt;TData, typeof Arc&gt;; }) =&gt; void` | - | A callback function triggered when the arc is clicked. |

**Extends:** `Pick<
    SimplifiedChartProps<TData, typeof Arc, PieChartExtraSnippetProps<TData>>,
    | 'aboveContext'
    | 'aboveMarks'
    | 'belowContext'
    | 'belowMarks'
    | 'children'
    | 'data'
    | 'debug'
    | 'legend'
    | 'marks'
    | 'onTooltipClick'
    | 'profile'
    | 'layer'
    | 'series'
    | 'tooltip'
    | 'tooltipContext'
    | 'cRange'
    | 'padding'
    | 'context'
    | 'width'
    | 'height'
  >`

### Examples

- [arc](https://layerchart.com/docs/components/PieChart/arc)
- [arc-props](https://layerchart.com/docs/components/PieChart/arc-props)
- [basic](https://layerchart.com/docs/components/PieChart/basic)
- [colors-data-prop](https://layerchart.com/docs/components/PieChart/colors-data-prop)
- [colors-interpolator](https://layerchart.com/docs/components/PieChart/colors-interpolator)
- [colors-scheme](https://layerchart.com/docs/components/PieChart/colors-scheme)
- [colors-variables](https://layerchart.com/docs/components/PieChart/colors-variables)
- [donut](https://layerchart.com/docs/components/PieChart/donut)
- [donut-with-text](https://layerchart.com/docs/components/PieChart/donut-with-text)
- [legend](https://layerchart.com/docs/components/PieChart/legend)
- [legend-custom-label](https://layerchart.com/docs/components/PieChart/legend-custom-label)
- [legend-placement](https://layerchart.com/docs/components/PieChart/legend-placement)
- [legend-responsive](https://layerchart.com/docs/components/PieChart/legend-responsive)
- [legend-with-padding](https://layerchart.com/docs/components/PieChart/legend-with-padding)
- [motion-spring](https://layerchart.com/docs/components/PieChart/motion-spring)
- [motion-tween](https://layerchart.com/docs/components/PieChart/motion-tween)
- [offset-slice](https://layerchart.com/docs/components/PieChart/offset-slice)
- [placement-custom](https://layerchart.com/docs/components/PieChart/placement-custom)
- [placement-left](https://layerchart.com/docs/components/PieChart/placement-left)
- [placement-right](https://layerchart.com/docs/components/PieChart/placement-right)
- [radius-fixed](https://layerchart.com/docs/components/PieChart/radius-fixed)
- [radius-offset](https://layerchart.com/docs/components/PieChart/radius-offset)
- [radius-percent](https://layerchart.com/docs/components/PieChart/radius-percent)
- [segments](https://layerchart.com/docs/components/PieChart/segments)
- [series](https://layerchart.com/docs/components/PieChart/series)
- [series-click](https://layerchart.com/docs/components/PieChart/series-click)
- [series-props](https://layerchart.com/docs/components/PieChart/series-props)
- [tooltip-click](https://layerchart.com/docs/components/PieChart/tooltip-click)

### Related

- [Chart](https://layerchart.com/docs/components/Chart)
- [Pie](https://layerchart.com/docs/components/Pie)

## Point

Primitive components which provides a convenient way to translate a data to x/y coordinates.

**Category:** primitives

**Supported Layers:** svg, canvas, html

## Usage

See example: basic

### API

| Property | Type | Default | Description |
|----------|------|---------|-------------|
| **d** (required) | `any` | - | Single data point to translate to x/y |
| children | `Snippet&lt;[ { x: number; y: number; } ]&gt;` | - | Children containing the x and y values |

### Examples

- [basic](https://layerchart.com/docs/components/Point/basic)

## Points

Marking component which plots individual data points on a graph to show distribution, relationships, or clusters without connecting lines.

**Category:** marks

**Supported Layers:** svg, canvas

> tip: See also: [ScatterChart](/docs/components/ScatterChart) for simplified examples

## Usage

See example: basic

### Color

To change the color you can use two main approaches: discrete color settings (`fill`, `class`) or value-based color scales.

#### Discrete

Color can be set for all points using the `fill` and `stroke` props...

See example: color-via-fill

...or using `class` to set fill and stroke via CSS.

See example: color-via-class

#### Value

To color points based on each point's value, you can use the color scale.

For example, to color based on thresholds (e.g. red: <50, yellow: 50-90, green: >90)

See example: color-via-threshold-scale

You can also color points based the comparison of two values in your data.

See example: color-via-ordinal-scale

### API

| Property | Type | Default | Description |
|----------|------|---------|-------------|
| data | `any` | - | Override data instead of using context |
| x | `Accessor` | - | Override `x` accessor from Chart context |
| y | `Accessor` | - | Override `y` accessor from Chart context |
| r | `number` | `5` | Override `r` accessor from Chart context |
| offsetX | `Offset` | - | The offset of the point in the x direction |
| offsetY | `Offset` | - | The offset of the point in the y direction |
| children | `Snippet&lt;[ { points: Point[]; } ]&gt;` | - | - |
| fill | `string` | - | Fill color |
| fillOpacity | `number` | - | Fill opacity (0-1) |
| stroke | `string` | - | Stroke color |
| strokeWidth | `number` | - | Stroke width in pixels |
| strokeOpacity | `number` | - | Stroke opacity (0-1) |
| opacity | `number` | - | Overall opacity (0-1) |

### Examples

- [basic](https://layerchart.com/docs/components/Points/basic)
- [color-via-class](https://layerchart.com/docs/components/Points/color-via-class)
- [color-via-fill](https://layerchart.com/docs/components/Points/color-via-fill)
- [color-via-ordinal-scale](https://layerchart.com/docs/components/Points/color-via-ordinal-scale)
- [color-via-threshold-scale](https://layerchart.com/docs/components/Points/color-via-threshold-scale)
- [with-labels](https://layerchart.com/docs/components/Points/with-labels)
- [with-tooltip-and-highlight](https://layerchart.com/docs/components/Points/with-tooltip-and-highlight)

### Related

- [ScatterChart](https://layerchart.com/docs/components/ScatterChart)

## Polygon

Primitive component which renders a multi-sided shape on a chart to represent complex areas, boundaries, or regions of interest.

**Category:** primitives

**Supported Layers:** svg, canvas

## Usage

See example: playground

### API

| Property | Type | Default | Description |
|----------|------|---------|-------------|
| cx | `number` | `0` | The center x position of the polygon. |
| initialCx | `number` | `cx` | The initial center x position of the polygon. |
| cy | `number` | `0` | The center y position of the polygon. |
| initialCy | `number` | `cy` | The initial center y position of the polygon. |
| r | `number` | `1` | The radius of the polygon. |
| initialR | `number` | `r` | The initial radius of the polygon. |
| points | `number \| { x: number; y: number; }[]` | `4` | The number of points or explicit points to create the polygon. |
| cornerRadius | `number` | `0` | The radius of the curve for rounded corners. |
| rotate | `number` | `0` | The rotation of the polygon. |
| inset | `number` | `0` | The percent to inset the odd points of the star (&lt;1 inset, &gt;1 outset) |
| scaleX | `number` | `1` | The horizontal stretch factor of the polygon. |
| scaleY | `number` | `1` | The vertical stretch factor of the polygon. |
| skewX | `number` | `0` | The skew angle in degrees along the X axis. |
| skewY | `number` | `0` | The skew angle in degrees along the Y axis. |
| tiltX | `number` | `1` | The tilt factor for x-coordinates. |
| tiltY | `number` | `1` | The tilt factor for y-coordinates. |
| ref | `SVGPathElement` | - | A bindable reference to the `&lt;path&gt;` element |
| motion | `MotionProp` | - | - |
| fill | `string` | - | Fill color |
| fillOpacity | `number` | - | Fill opacity (0-1) |
| stroke | `string` | - | Stroke color |
| strokeWidth | `number` | - | Stroke width in pixels |
| strokeOpacity | `number` | - | Stroke opacity (0-1) |
| opacity | `number` | - | Overall opacity (0-1) |

**Extends:** `Without<SVGAttributes<Element>, PolygonPropsWithoutHTML>`

### Examples

- [10-point-star](https://layerchart.com/docs/components/Polygon/10-point-star)
- [12-point-star](https://layerchart.com/docs/components/Polygon/12-point-star)
- [14-point-star](https://layerchart.com/docs/components/Polygon/14-point-star)
- [16-point-star](https://layerchart.com/docs/components/Polygon/16-point-star)
- [18-point-star](https://layerchart.com/docs/components/Polygon/18-point-star)
- [20-point-star](https://layerchart.com/docs/components/Polygon/20-point-star)
- [6-point-star](https://layerchart.com/docs/components/Polygon/6-point-star)
- [8-point-star](https://layerchart.com/docs/components/Polygon/8-point-star)
- [arrow](https://layerchart.com/docs/components/Polygon/arrow)
- [cross](https://layerchart.com/docs/components/Polygon/cross)
- [diamond](https://layerchart.com/docs/components/Polygon/diamond)
- [hexagon](https://layerchart.com/docs/components/Polygon/hexagon)
- [octagon](https://layerchart.com/docs/components/Polygon/octagon)
- [parallelogram](https://layerchart.com/docs/components/Polygon/parallelogram)
- [pentagon](https://layerchart.com/docs/components/Polygon/pentagon)
- [playground](https://layerchart.com/docs/components/Polygon/playground)
- [rectangle](https://layerchart.com/docs/components/Polygon/rectangle)
- [rhombus](https://layerchart.com/docs/components/Polygon/rhombus)
- [square](https://layerchart.com/docs/components/Polygon/square)
- [trapezoid](https://layerchart.com/docs/components/Polygon/trapezoid)
- [triangle](https://layerchart.com/docs/components/Polygon/triangle)

## Promise

Use `{#await}` to load each example and source

**Category:** examples

```svelte
<script lang="ts">
	import PromiseExample from '$lib/components/PromiseExample.svelte';
</script>

<PromiseExample component="AreaChart" name="basic" />
```

## Example

## Benefits

- Concise examples (single line, no imports)

## Issues

- Lazy loads each example
- Need to determine how to load contents for search index

## RadialGradient

Fill component which provides a radial gradient fill pattern for  chart elements.

**Category:** fill

**Supported Layers:** svg

## Usage

### Focal location and radius with custom colors

See example: focal-location-and-radius-with-custom-colors

### Tailwind colors

See example: tailwind-colors

### spreadMethod

See example: spreadmethod

### units

See example: units

### API

| Property | Type | Default | Description |
|----------|------|---------|-------------|
| id | `string` | - | Unique id for radialGradient |
| stops | `string[] \| [ string \| number, string ][]` | `['var(--tw-gradient-from)', 'var(--tw-gradient-to)']` | Array array of strings (colors), will equally distributed from 0-100%. If array of tuples, will use first value as the offset, and second as color |
| cx | `string` | `'50%'` | The x coordinate of the center of the gradient |
| cy | `string` | `'50%'` | The y coordinate of the center of the gradient |
| fx | `string` | `cx` | The x coordinate of the focal point of the gradient |
| fy | `string` | `cy` | The y coordinate of the focal point of the gradient |
| r | `string` | - | The radius of the gradient |
| spreadMethod | `'pad' \| 'reflect' \| 'repeat'` | `'pad'` | Indicates how the gradient behaves if it starts or ends inside the bounds of the shape containing the gradient |
| transform | `string \| null` | - | Transform attribute for the gradient |
| units | `'objectBoundingBox' \| 'userSpaceOnUse'` | `'objectBoundingBox'` | Define the coordinate system for attributes (i.e. gradientUnits) |
| children | `Snippet&lt;[ { id: string; gradient: string; } ]&gt;` | - | - |
| stopsContent | `Snippet` | - | Render as a child of the gradient and will opt out of the default stops being rendered. |

**Extends:** `Without<SVGAttributes<SVGRadialGradientElement>, RadialGradientPropsWithoutHTML>`

### Examples

- [focal-location-and-radius-with-custom-colors](https://layerchart.com/docs/components/RadialGradient/focal-location-and-radius-with-custom-colors)
- [spreadmethod](https://layerchart.com/docs/components/RadialGradient/spreadmethod)
- [tailwind-colors](https://layerchart.com/docs/components/RadialGradient/tailwind-colors)
- [units](https://layerchart.com/docs/components/RadialGradient/units)

### Related

- [LinearGradient](https://layerchart.com/docs/components/LinearGradient)
- [Pattern](https://layerchart.com/docs/components/Pattern)

## Rect

Primitive component which draws a rectangle to highlight areas, ranges, or specific regions of interest.

**Category:** primitives

**Supported Layers:** svg, canvas, html

## Usage

See example: styling-using-classes

### API

| Property | Type | Default | Description |
|----------|------|---------|-------------|
| x | `number` | `0` | - |
| initialX | `number` | `x` | - |
| y | `number` | `0` | - |
| initialY | `number` | `y` | - |
| **width** (required) | `number` | - | - |
| initialWidth | `number` | - | - |
| **height** (required) | `number` | - | - |
| initialHeight | `number` | - | - |
| ref | `SVGRectElement` | - | Underlying `&lt;rect&gt;` tag when using &lt;Svg&gt;. Useful for bindings. |
| motion | `MotionProp&lt;'x' \| 'y' \| 'width' \| 'height'&gt;` | - | - |
| children | `Snippet` | - | Children content to render.  Note: Only works for Html layers |
| fill | `string` | - | Fill color |
| fillOpacity | `number` | - | Fill opacity (0-1) |
| stroke | `string` | - | Stroke color |
| strokeWidth | `number` | - | Stroke width in pixels |
| strokeOpacity | `number` | - | Stroke opacity (0-1) |
| opacity | `number` | - | Overall opacity (0-1) |
| onclick | `MouseEventHandler&lt;Element&gt; \| null` | - | Click event handler |
| ondblclick | `MouseEventHandler&lt;Element&gt; \| null` | - | Double click event handler |
| onpointerenter | `PointerEventHandler&lt;Element&gt; \| null` | - | Pointer enter event handler |
| onpointermove | `PointerEventHandler&lt;Element&gt; \| null` | - | Pointer move event handler |
| onpointerleave | `PointerEventHandler&lt;Element&gt; \| null` | - | Pointer leave event handler |
| onpointerover | `PointerEventHandler&lt;Element&gt; \| null` | - | Pointer over event handler |
| onpointerout | `PointerEventHandler&lt;Element&gt; \| null` | - | Pointer out event handler |

**Extends:** `Without<SVGAttributes<SVGRectElement>, RectPropsWithoutHTML>`, `CommonEvents`

### Examples

- [styling-using-attributes](https://layerchart.com/docs/components/Rect/styling-using-attributes)
- [styling-using-classes](https://layerchart.com/docs/components/Rect/styling-using-classes)
- [styling-using-css-variables](https://layerchart.com/docs/components/Rect/styling-using-css-variables)

### Related

- [Bars](https://layerchart.com/docs/components/Bars)
- [Highlight](https://layerchart.com/docs/components/Highlight)
- [RectClipPath](https://layerchart.com/docs/components/RectClipPath)

## RectClipPath

Clipping component which defines a rectangular clip region  to constrain the rendering of chart elements within a specified shape or boundary.

**Category:** clipping

**Supported Layers:** svg

### API

| Property | Type | Default | Description |
|----------|------|---------|-------------|
| id | `string` | - | A unique id for the clipPath. |
| x | `number` | `0` | The x position of the clipPath. |
| y | `number` | `0` | The y position of the clipPath. |
| **width** (required) | `number` | - | The width of the clipPath. |
| **height** (required) | `number` | - | The height of the clipPath. |
| disabled | `boolean` | `false` | Whether to disable clipping (show all). |
| children | `Snippet&lt;[ { id: string; url: string; } ]&gt;` | - | The default children snippet which provides the id and url for the clipPath. |
| motion | `MotionProp&lt;'x' \| 'y' \| 'width' \| 'height'&gt;` | - | - |
| onclick | `MouseEventHandler&lt;Element&gt; \| null` | - | Click event handler |
| ondblclick | `MouseEventHandler&lt;Element&gt; \| null` | - | Double click event handler |
| onpointerenter | `PointerEventHandler&lt;Element&gt; \| null` | - | Pointer enter event handler |
| onpointermove | `PointerEventHandler&lt;Element&gt; \| null` | - | Pointer move event handler |
| onpointerleave | `PointerEventHandler&lt;Element&gt; \| null` | - | Pointer leave event handler |
| onpointerover | `PointerEventHandler&lt;Element&gt; \| null` | - | Pointer over event handler |
| onpointerout | `PointerEventHandler&lt;Element&gt; \| null` | - | Pointer out event handler |

**Extends:** `Without<SVGAttributes<SVGElement>, RectClipPathPropsWithoutHTML>`, `CommonEvents`

### Related

- [ChartClipPath](https://layerchart.com/docs/components/ChartClipPath)

## Rule

Commonly used component acting as a visual guideline on a chart that helps align and measure data values along an axis.

**Category:** common

**Supported Layers:** svg, canvas, html

## Usage

See example: baseline-x-y

### Use cases

A `Rule` component can be used for various use cases including:

#### Axis baseline

with boolean value: `x={true}` and `y={true}`

See example: baseline-x-y

#### Annotation

with number value: `x={Number}` and `y={Number}`

See example: annotation-y

#### Data mark

using `<Chart data>` and either:

- implicit `x` / `y` accessors (using `<Chart x={..} y={...}>`)

See example: data-x-range

- explicit `x="property"` / `y="property"` accessors

See example: candlestick

### API

| Property | Type | Default | Description |
|----------|------|---------|-------------|
| data | `any` | - | Override the data from the context. |
| x | `number \| Date \| boolean \| '$left' \| '$right' \| Accessor` | `false` | Create a vertical `x` line - If true or 'left', will draw at chart left (xRange[0]) - If 'right', will draw at chart right (xRange[1]) - Use `0` for baseline (yScale(0)) - Use number \| Date value for annotation (yScale(value)) |
| xOffset | `number` | `0` | Pixel offset to apply to `x` coordinate |
| y | `number \| Date \| boolean \| '$top' \| '$bottom' \| Accessor` | `false` | Create a horizontal `y` line - If true or 'bottom', will draw at chart bottom (yRange[0]) - If 'top', will draw at chart top (yRange[1]) - Use `0` for baseline (xScale(0)) - Use number \| Date value for annotation (xScale(value)) |
| yOffset | `number` | `0` | Pixel offset to apply to `y` coordinate |

**Extends:** `Without<SVGAttributes<SVGElement>, RulePropsWithoutHTML>`

### Examples

- [annotation-x](https://layerchart.com/docs/components/Rule/annotation-x)
- [annotation-y](https://layerchart.com/docs/components/Rule/annotation-y)
- [baseline-top-right](https://layerchart.com/docs/components/Rule/baseline-top-right)
- [baseline-x-negative](https://layerchart.com/docs/components/Rule/baseline-x-negative)
- [baseline-x-y](https://layerchart.com/docs/components/Rule/baseline-x-y)
- [baseline-y-negative](https://layerchart.com/docs/components/Rule/baseline-y-negative)
- [candlestick](https://layerchart.com/docs/components/Rule/candlestick)
- [candlestick-open-close-line-color](https://layerchart.com/docs/components/Rule/candlestick-open-close-line-color)
- [candlestick-with-brushing](https://layerchart.com/docs/components/Rule/candlestick-with-brushing)
- [data-x-band](https://layerchart.com/docs/components/Rule/data-x-band)
- [data-x-date](https://layerchart.com/docs/components/Rule/data-x-date)
- [data-x-range](https://layerchart.com/docs/components/Rule/data-x-range)
- [data-y-range](https://layerchart.com/docs/components/Rule/data-y-range)
- [lollipop](https://layerchart.com/docs/components/Rule/lollipop)

### Related

- [Axis](https://layerchart.com/docs/components/Axis)
- [Line](https://layerchart.com/docs/components/Line)
- [AnnotationLine](https://layerchart.com/docs/components/AnnotationLine)

## Sankey

Layout component which arranges nodes and links to visualize flow magnitude between categories, with link widths proportional to the flow and nodes positioned to minimize overlap and crossings.

**Category:** layout

**Supported Layers:** svg, canvas

## Usage

See example: simple

### API

| Property | Type | Default | Description |
|----------|------|---------|-------------|
| nodes | `(d: any) =&gt; any` | `(d: any) =&gt; d.nodes` | The function to get the nodes from the data. |
| nodeId | `(d: any) =&gt; any` | `(d: any) =&gt; d.index` | The function to get the node ID from the node data. |
| nodeAlign | `((node: SankeyNode&lt;any, any&gt;, n: number) =&gt; number) \| 'left' \| 'right' \| 'center' \| 'justify'` | `sankeyJustify` | - |
| nodeWidth | `number` | `4` | The width of the nodes. |
| nodePadding | `number` | `10` | The padding between nodes. |
| nodeSort | `(a: SankeyNode&lt;any, any&gt;, b: SankeyNode&lt;any, any&gt;) =&gt; number \| undefined` | - | The function to sort the nodes. |
| links | `(d: any) =&gt; any` | `(d: any) =&gt; d.links` | The function to get the links from the data. |
| linkSort | `(a: SankeyLink&lt;any, any&gt;, b: SankeyLink&lt;any, any&gt;) =&gt; number \| undefined` | - | The function to sort the links. |
| onUpdate | `(data: SankeyGraph&lt;{}, {}&gt;) =&gt; void` | - | A function to be called when the data is updated. |
| children | `Snippet&lt;[ { nodes: SankeyNode&lt;NodeExtraProperties, any&gt;[]; links: SankeyNode&lt;NodeExtraProperties, any&gt;[]; } ]&gt;` | - | - |

### Examples

- [complex](https://layerchart.com/docs/components/Sankey/complex)
- [hierarchy](https://layerchart.com/docs/components/Sankey/hierarchy)
- [node-select](https://layerchart.com/docs/components/Sankey/node-select)
- [simple](https://layerchart.com/docs/components/Sankey/simple)
- [tooltip](https://layerchart.com/docs/components/Sankey/tooltip)

## ScatterChart

Streamlined visualization of data with individual data points on two axes to reveal relationships, patterns, or correlations between variables.

**Category:** charts

**Supported Layers:** svg, canvas, html

## Usage

See example: basic

### API

| Property | Type | Default | Description |
|----------|------|---------|-------------|
| props | `ScatterChartPropsObjProp` | - | - |
| yDomain | `ComponentProps&lt;typeof BrushContext&gt;['yDomain']` | - | - |
| yScale | `AnyScale` | - | - |

**Extends:** `Omit<
    SimplifiedChartProps<TData, typeof Points, ScatterChartExtraSnippetProps<TData>>,
    'radial'
  >`

### Examples

- [basic](https://layerchart.com/docs/components/ScatterChart/basic)
- [brush](https://layerchart.com/docs/components/ScatterChart/brush)
- [custom](https://layerchart.com/docs/components/ScatterChart/custom)
- [custom-tooltip](https://layerchart.com/docs/components/ScatterChart/custom-tooltip)
- [date-series-color-scale](https://layerchart.com/docs/components/ScatterChart/date-series-color-scale)
- [domain-baseline](https://layerchart.com/docs/components/ScatterChart/domain-baseline)
- [domain-nice](https://layerchart.com/docs/components/ScatterChart/domain-nice)
- [domain-padding](https://layerchart.com/docs/components/ScatterChart/domain-padding)
- [labels](https://layerchart.com/docs/components/ScatterChart/labels)
- [line-annotation](https://layerchart.com/docs/components/ScatterChart/line-annotation)
- [point-annotations](https://layerchart.com/docs/components/ScatterChart/point-annotations)
- [punchcard](https://layerchart.com/docs/components/ScatterChart/punchcard)
- [radius-scale](https://layerchart.com/docs/components/ScatterChart/radius-scale)
- [range-annotation-both](https://layerchart.com/docs/components/ScatterChart/range-annotation-both)
- [range-annotation-horizontal](https://layerchart.com/docs/components/ScatterChart/range-annotation-horizontal)
- [range-annotation-vertical](https://layerchart.com/docs/components/ScatterChart/range-annotation-vertical)
- [series](https://layerchart.com/docs/components/ScatterChart/series)
- [series-custom-labels](https://layerchart.com/docs/components/ScatterChart/series-custom-labels)
- [series-legend](https://layerchart.com/docs/components/ScatterChart/series-legend)
- [series-tween](https://layerchart.com/docs/components/ScatterChart/series-tween)
- [series-with-radius](https://layerchart.com/docs/components/ScatterChart/series-with-radius)
- [single-axis-x](https://layerchart.com/docs/components/ScatterChart/single-axis-x)
- [single-axis-y](https://layerchart.com/docs/components/ScatterChart/single-axis-y)
- [single-dimension](https://layerchart.com/docs/components/ScatterChart/single-dimension)
- [tooltip-click](https://layerchart.com/docs/components/ScatterChart/tooltip-click)

### Related

- [Chart](https://layerchart.com/docs/components/Chart)
- [Points](https://layerchart.com/docs/components/Points)

## SourcePlugin

Use rehype plugin and wrapper component to populate source

**Category:** examples

```svelte
<script lang="ts">
	import SourceExample from '$lib/components/SourceExample.svelte';

	// repeat for each example
	import Basic from '$examples/components/AreaChart/basic.svelte';
</script>

<SourceExample component="AreaChart" name="basic">
	<Basic />
</SourceExample>
```

## Example

## Benefits

- Improvement over direct by not requiring source imports
- Should be easier to build search index

## Issues

- Verbose - still needs to load each example (but not source)
- Disconnect between displayed component and loaded source (no match guarantee)
- Less (direct) control over Code component (add copy button, expand/collapse, etc)

## Spline

Marking component which applies data points connected by smooth, curved lines to show trends or patterns over a continuous range.

**Category:** marks

**Supported Layers:** svg, canvas

> tip: See also: [LineChart](/docs/components/LineChart) for simplified examples

## Usage

See example: basic

### Playground

See example: playground

### API

| Property | Type | Default | Description |
|----------|------|---------|-------------|
| data | `any` | - | Override data instead of using context |
| x | `Accessor` | - | Override `x` accessor from Chart context |
| y | `Accessor` | - | Override `y` accessor from Chart context |
| defined | `Parameters&lt;Line&lt;any&gt;['defined']&gt;[0]` | - | Function to determine if a point is defined |
| curve | `CurveFactory \| CurveFactoryLineOnly` | - | Curve of path drawn. Imported via d3-shape. |

**Extends:** `Without<SVGAttributes<SVGPathElement>, SplinePropsWithoutHTML>`

### Examples

- [basic](https://layerchart.com/docs/components/Spline/basic)
- [basic-start-and-end-snippets](https://layerchart.com/docs/components/Spline/basic-start-and-end-snippets)
- [draw](https://layerchart.com/docs/components/Spline/draw)
- [end-slot-with-draw-with-value](https://layerchart.com/docs/components/Spline/end-slot-with-draw-with-value)
- [end-snippet-with-draw](https://layerchart.com/docs/components/Spline/end-snippet-with-draw)
- [gradient-encoding](https://layerchart.com/docs/components/Spline/gradient-encoding)
- [gradient-threshold](https://layerchart.com/docs/components/Spline/gradient-threshold)
- [label-using-start-end-snippets](https://layerchart.com/docs/components/Spline/label-using-start-end-snippets)
- [markers-arrows](https://layerchart.com/docs/components/Spline/markers-arrows)
- [multiple-series](https://layerchart.com/docs/components/Spline/multiple-series)
- [multiple-series-highlight-on-hover](https://layerchart.com/docs/components/Spline/multiple-series-highlight-on-hover)
- [multiple-series-using-overrides](https://layerchart.com/docs/components/Spline/multiple-series-using-overrides)
- [multiple-series-with-labels](https://layerchart.com/docs/components/Spline/multiple-series-with-labels)
- [playground](https://layerchart.com/docs/components/Spline/playground)
- [radial-line-with-areas](https://layerchart.com/docs/components/Spline/radial-line-with-areas)
- [radial-multi-year-lines](https://layerchart.com/docs/components/Spline/radial-multi-year-lines)
- [radial-radar](https://layerchart.com/docs/components/Spline/radial-radar)
- [tweened](https://layerchart.com/docs/components/Spline/tweened)
- [vertical](https://layerchart.com/docs/components/Spline/vertical)
- [with-labels](https://layerchart.com/docs/components/Spline/with-labels)
- [with-tooltip-and-highlight](https://layerchart.com/docs/components/Spline/with-tooltip-and-highlight)

### Related

- [Path](https://layerchart.com/docs/components/Path)
- [LineChart](https://layerchart.com/docs/components/LineChart)

## Svg

Svg layer

**Category:** layers

Typically you will use `<Layer type="svg">`

### API

| Property | Type | Default | Description |
|----------|------|---------|-------------|
| ref | `SVGSVGElement` | - | A reference to the layer's `&lt;svg&gt;` tag. |
| innerRef | `SVGGElement` | - | A reference to the layer's `&lt;g&gt;` tag. |
| zIndex | `number` | - | The layer's z-index. |
| pointerEvents | `boolean` | - | Set this to `false` to set `pointer-events: none;` on the entire layer. |
| viewBox | `string` | - | A string passed to the `viewBox` property on the `&lt;svg&gt;` tag. |
| title | `string \| Snippet` | - | Shorthand to set the contents of `&lt;title&gt;&lt;/title&gt;` for accessibility. You can also set arbitrary HTML via the title snippet but this is a convenient shorthand. |
| defs | `Snippet` | - | The inner content of the `&lt;defs&gt;` tag. |
| center | `boolean \| 'x' \| 'y'` | - | Translate children to center (useful for radial layouts) |
| ignoreTransform | `boolean` | - | Ignore TransformContext. Useful to add static elements such as legends. |
| clip | `boolean` | `false` | Whether to clip overflow content. When true, sets `overflow: hidden` on the SVG. |
| children | `Snippet&lt;[ { ref: SVGElement; } ]&gt;` | - | - |

**Extends:** `Without<SVGAttributes<SVGElement>, SVGPropsWithoutHTML>`

### Related

- [Layer](https://layerchart.com/docs/components/Layer)

## Text

Primitive component which adds custom text for labeling, annotation, or commentary.

**Category:** primitives

**Supported Layers:** svg, canvas, html

## Usage

See example: playground

### Truncate text of axis labels

Sometimes your axis labels overwhelm the available space. You can use `truncate` to limit the text to a maximum length.

See example: truncate-axis-labels

### Word wrap with text of axis labels

You can use explicit newlines (`\n`) in the text value to force a word wrap. This works regardless of the layer you are using.

> note: Note you can change the rendering layer with the toggle at the top of the page.

See example: word-wrap-axis-labels

### Along path

`Text` can be used with `Arc`'s `children` snippet and `getArcTextProps` to write along the `inner`, `outer`, or `middle` of the arc path.

The text will smartly orientate based on the direction (clockwise / counter-clockwise) and location (top, bottom, left, right) of the arc

:example{ component="Arc" name="label-direction" }

> note: Only supported in `Svg` layers.

### API

| Property | Type | Default | Description |
|----------|------|---------|-------------|
| value | `string \| number` | `0` | text value |
| fill | `string` | - | The fill color of the text |
| width | `number` | - | Maximum width to occupy (approximate as words are not split) |
| x | `string \| number` | `0` | x position of the text |
| initialX | `string \| number` | `x` | Initial x position of the text |
| y | `string \| number` | `0` | y position of the text |
| initialY | `string \| number` | `y` | Initial y position of the text |
| dx | `string \| number` | `0` | dx offset of the text |
| dy | `string \| number` | `0` | dy offset of the text |
| lineHeight | `string` | `"1em"` | Desired "line height" of the text, implemented as y offsets |
| capHeight | `string` | `'0.71em'` | Cap height of the text |
| scaleToFit | `boolean` | `false` | Whether to scale the fontSize to accommodate the specified width |
| textAnchor | `'start' \| 'middle' \| 'end' \| 'inherit'` | `'start'` | Horizontal text anchor |
| verticalAnchor | `'start' \| 'middle' \| 'end' \| 'inherit'` | `'end'` | Vertical text anchor |
| dominantBaseline | `'auto' \| 'text-before-edge' \| 'text-after-edge' \| 'middle' \| 'hanging' \| 'ideographic' \| 'mathematical'` | `'auto'` | The dominant baseline of the text.  Useful for aligning text to the baseline of the axis. |
| rotate | `number` | - | Rotational angle of the text |
| svgRef | `SVGElement` | - | A bindable reference to the wrapping `&lt;svg&gt;` element. |
| svgProps | `Omit&lt;SVGAttributes&lt;SVGElement&gt;, 'children'&gt;` | - | Props to pass to the wrapping `&lt;svg&gt;` element. |
| ref | `SVGTextElement` | - | A bindable reference to the inner `&lt;text&gt;` element |
| motion | `MotionProp` | - | - |
| truncate | `boolean \| TruncateTextOptions` | - | Whether to enable text truncation |
| pathId | `string` | - | A unique identifier for the SVG path element. One is generated by default if not provided. |
| path | `string \| null` | - | The path to render the text along. |
| startOffset | `string \| number` | `'0%'` | Specify the offset for the start of the text along the path. Can be a percentage ('50%') or a length value. |
| fill | `string` | - | Fill color |
| fillOpacity | `number` | - | Fill opacity (0-1) |
| stroke | `string` | - | Stroke color |
| strokeWidth | `number` | - | Stroke width in pixels |
| strokeOpacity | `number` | - | Stroke opacity (0-1) |
| opacity | `number` | - | Overall opacity (0-1) |

**Extends:** `Without<SVGAttributes<SVGTextElement>, TextPropsWithoutHTML>`

### Examples

- [playground](https://layerchart.com/docs/components/Text/playground)
- [truncate-axis-labels](https://layerchart.com/docs/components/Text/truncate-axis-labels)
- [word-wrap-axis-labels](https://layerchart.com/docs/components/Text/word-wrap-axis-labels)
- [word-wrap-with-explicit-newline](https://layerchart.com/docs/components/Text/word-wrap-with-explicit-newline)

## Threshold

Marking component visualizing data relative to predefined limits, highlighting values that exceed or fall below set thresholds.

**Category:** marks

**Supported Layers:** svg

## Usage

See example: basic

### API

| Property | Type | Default | Description |
|----------|------|---------|-------------|
| curve | `CurveFactory` | - | The curve factory to use for the area. |
| defined | `ComponentProps&lt;typeof Area&gt;['defined']` | - | Function to determine if a point is defined. |
| above | `Snippet&lt;[ ThresholdSnippetProps ]&gt;` | - | Content to render above the threshold area. |
| below | `Snippet&lt;[ ThresholdSnippetProps ]&gt;` | - | Content to render below the threshold area. |
| children | `Snippet&lt;[ ThresholdSnippetProps ]&gt;` | - | - |

### Examples

- [basic](https://layerchart.com/docs/components/Threshold/basic)
- [with-labels](https://layerchart.com/docs/components/Threshold/with-labels)
- [with-tooltip-and-highlight](https://layerchart.com/docs/components/Threshold/with-tooltip-and-highlight)

### Related

- [AreaChart](https://layerchart.com/docs/components/AreaChart)

## TileImage

Geographic component which renders map tiles as a background layer, enabling zoomable and pannable visualizations.

**Category:** geo

**Supported Layers:** svg, canvas

### API

| Property | Type | Default | Description |
|----------|------|---------|-------------|
| **x** (required) | `number` | - | x position of the tile |
| **y** (required) | `number` | - | y position of the tile |
| **z** (required) | `number` | - | z position of the tile |
| **tx** (required) | `number` | - | translate x |
| **ty** (required) | `number` | - | translate y |
| **scale** (required) | `number` | - | scale of the tile |
| disableCache | `boolean` | `false` | Whether to disable cache |
| debug | `boolean` | `false` | Whether to enable debug mode |
| **url** (required) | `(x: number, y: number, z: number) =&gt; string` | - | URL function to get the tile image |

### Related

- [GeoTile](https://layerchart.com/docs/components/GeoTile)

## Tooltip

Interaction component manages and displays tooltips allowing dynamic information to appear in response to user interactions.

**Category:** interactions

**Supported Layers:** svg, canvas

Tooltips have 2 parts, the `TooltipContext` (which is integrated into `<Chart tooltip={...})>`
and used for data selection and state management, `Tooltip` components (`Tooltip.Root`, `Tooltip.Header`, `Tooltip.List`, and `Tooltip.Item`) which are used for visual display.

## Features

- HTML first
- Can be interactive (clickable / hover)
- Smart placement (contained in container, window, etc)
- Multiple instances supported
- Different modes (bisect, band, voronoi, path/shape, quadtree, hit canvas)

## Usage

### Modes

There are multiple tooltip modes for different situations, which can be controlled by passing `<Chart tooltip={{ mode: '...' }}>`.

#### `bisect-x` | `bisect-y`

Finds the closest data point along a give axis based on your pointer position.

#### `band`

Uses transparent `<path>` to enable full-bandwidth hit targets (i.e not just the bar itself). This is especially useful for very small values (short bars) and consistent scrubbing across the data.

#### `voronoi`

Path based, easier to reason about than quadtree. Supports max `radius`

#### `quadtree`

In memory and typically faster than `voronoi`. Supports max `radius`

Useful for point-based visualizations such as geographic points and scatter plots

#### `manual`

Useful for shape based triggering such as on geo boundaries and radial charts with arc slices (ex. pie chart).

You can call `tooltip.show(e, DATA)` and `tooltip.hide()` recommended within `onpointerenter`, `onpointermove`, and `onpointerleave`

Canvas layers leverage an integrated "hit canvas" which enables the same shape-based triggering as you are accustomed with Svg.

### Location

Tooltips can be positions based on

- Pointer position
- Data location
- Fixed
  Each of these are set on a per-axis bases, allowing:
- Tooltip following pointer on both axis (i.e. stays next to pointer)
- Tooltip "snaps" to each data point as the pointer moves
- Tooltip stays within the axis/padding but trackings the pointer left/right (a axis) or up/down (y axis)
- Fixed tooltip location (ex. top left) regardless of pointer or data

```html
<Tooltip.Root x={"pointer"|"data"|number} y={"pointer"|"data"|number}>
```

Offsets are available (`xOffset`, `yOffset`) to not overlap data (or provide more space for course pointer devices such as a finger).

You can render any number of `Tooltip` instances, which can be useful to have one in each axis area

Tooltips can be contained within the chart container, window/viewport, or none. When contained, the tooltip will "swap sides" instead of moving outside the container.

```html
<Tooltip.Root contained={"container"|"window"|"none"}>
```

Tooltip locking

Delayed closing

Anchor placement

Externally access tooltip data

```html
<Chart bind:tooltipContext></Chart>
```

### API

| Property | Type | Default | Description |
|----------|------|---------|-------------|
| x | `'pointer' \| 'data' \| number` | `'pointer'` | `x` position of tooltip.  By default uses the pointer/mouse, can also snap to data or an explicit fixed position. |
| y | `'pointer' \| 'data' \| number` | `'pointer'` | `y` position of tooltip.  By default uses the pointer/mouse, can also snap to data or an explicit fixed position. |
| xOffset | `number` | `x === 'pointer' ? 10 : 0` | Offset added to `x` position |
| yOffset | `number` | `y === 'pointer' ? 10 : 0` | Offset added to `y` position |
| anchor | `Placement` | `'top-left'` | Align based on edge of tooltip |
| motion | `MotionProp` | `"spring"` | The default motion state of the tooltip. |
| pointerEvents | `boolean` | `false` | Allow pointer events.  Disabled by default to reduce accidental selection, but useful to enable to allow interactive tooltips (using `locked`) |
| contained | `'container' \| 'window' \| false` | `'container'` | Include padding area (ex. axis) |
| variant | `'default' \| 'invert' \| 'none'` | `'default'` | Tooltip variant |
| classes | `{ root?: string; container?: string; content?: string; header?: string; }` | `{}` | Classes to apply to the various elements of the tooltip. |
| children | `Snippet&lt;[ { data: T; payload: TooltipPayload[]; } ]&gt;` | - | - |
| rootRef | `HTMLElement` | - | A reference to the tooltip's outermost `&lt;div&gt;` tag. |
| props | `{ root?: HTMLAttributes&lt;HTMLElement&gt;; container?: HTMLAttributes&lt;HTMLElement&gt;; content?: HTMLAttributes&lt;HTMLElement&gt;; }` | - | Props to pass to the underlying elements rendered by the Tooltip component |
| context | `ChartContextValue&lt;T, any, any&gt;` | - | Optionally pass the chart's context to the tooltip to get type inference for the data. |

**Extends:** `Without<HTMLAttributes<HTMLElement>, TooltipPropsWithoutHTML<T>>`

### Examples

- [anchor-location](https://layerchart.com/docs/components/Tooltip/anchor-location)
- [area](https://layerchart.com/docs/components/Tooltip/area)
- [basic](https://layerchart.com/docs/components/Tooltip/basic)
- [color-swatch](https://layerchart.com/docs/components/Tooltip/color-swatch)
- [color-swatch-using-theme](https://layerchart.com/docs/components/Tooltip/color-swatch-using-theme)
- [custom-content](https://layerchart.com/docs/components/Tooltip/custom-content)
- [data-snapping](https://layerchart.com/docs/components/Tooltip/data-snapping)
- [default-mouse-position-with-offset](https://layerchart.com/docs/components/Tooltip/default-mouse-position-with-offset)
- [disable-motion](https://layerchart.com/docs/components/Tooltip/disable-motion)
- [duration](https://layerchart.com/docs/components/Tooltip/duration)
- [externally-access-tooltip-data](https://layerchart.com/docs/components/Tooltip/externally-access-tooltip-data)
- [invert-variant](https://layerchart.com/docs/components/Tooltip/invert-variant)
- [multiple-overlapping-bars](https://layerchart.com/docs/components/Tooltip/multiple-overlapping-bars)
- [multiple-overlapping-durations](https://layerchart.com/docs/components/Tooltip/multiple-overlapping-durations)
- [multiple-tooltips-with-fixed-single-axis](https://layerchart.com/docs/components/Tooltip/multiple-tooltips-with-fixed-single-axis)
- [multiple-tooltips-with-fixed-single-axis-scaleband](https://layerchart.com/docs/components/Tooltip/multiple-tooltips-with-fixed-single-axis-scaleband)
- [scatter-plot](https://layerchart.com/docs/components/Tooltip/scatter-plot)
- [simple-bars](https://layerchart.com/docs/components/Tooltip/simple-bars)
- [single-date-time](https://layerchart.com/docs/components/Tooltip/single-date-time)
- [stacked-area](https://layerchart.com/docs/components/Tooltip/stacked-area)

### Related

- [TooltipContext](https://layerchart.com/docs/components/TooltipContext)
- [Highlight](https://layerchart.com/docs/components/Highlight)

## TooltipContext

Interaction component manages and provides the data and behavior needed to display dynamic tooltips within a chart.

**Category:** interactions

**Supported Layers:** svg, canvas

### API

| Property | Type | Default | Description |
|----------|------|---------|-------------|
| mode | `TooltipMode` | `'manual'` | The tooltip interaction mode |
| findTooltipData | `'closest' \| 'left' \| 'right'` | `'closest'` | Method to find tooltip data |
| raiseTarget | `boolean` | `false` | Similar to d3-selection's raise, re-insert the e.target as the last child of its parent, so to be the top-most element |
| locked | `boolean` | `false` | Lock tooltip (keep open, do not update on mouse movement). Allows for clicking on tooltip |
| touchEvents | `'none' \| 'pan-x' \| 'pan-y' \| 'auto'` | `'pan-y'` | Controls the touch event behavior on the tooltip container. By default uses `pan-y` to allow verticle scrolling but horizontal scrubbing. Use `none` to disable all touch events (useful for improved transform/geo charts interactions) |
| radius | `number` | `Infinity` | quadtree search or voronoi clip radius |
| debug | `boolean` | `false` | Enable debug view (show hit targets, etc) |
| onclick | `(e: MouseEvent, { data }: { data: any; }) =&gt; any` | `() =&gt; {}` | Click handler for the tooltip |
| tooltipContext | `TooltipContextValue&lt;T&gt;` | `{ x: 0, y: 0, data: null, show: showTooltip, hide: hideTooltip, mode }` | Exposed to allow binding in Chart |
| hideDelay | `number` | `0` | Delay in ms before hiding tooltip |
| ref | `HTMLElement` | - | A reference to the tooltip container element. |
| children | `Snippet&lt;[ { tooltipContext: TooltipContextValue&lt;T&gt;; } ]&gt;` | - | - |

**Extends:** `Without<HTMLAttributes<HTMLElement>, TooltipContextPropsWithoutHTML<T>>`

### Related

- [Chart](https://layerchart.com/docs/components/Chart)
- [Tooltip](https://layerchart.com/docs/components/Tooltip)
- [Highlight](https://layerchart.com/docs/components/Highlight)

## TransformContext

Interaction component which provides context to support panning, zooming, and dragging interactions for chart elements.

**Category:** interactions

**Supported Layers:** svg, canvas, html

## Playground

See example: playground

### Pan/Zoom SVG image

See example: pan-zoom-svg-image

### Pan/Zoom HTML image

See example: pan-zoom-html-image

### API

| Property | Type | Default | Description |
|----------|------|---------|-------------|
| mode | `TransformMode` | - | - |
| processTranslate | `(x: number, y: number, deltaX: number, deltaY: number) =&gt; { x: number; y: number; }` | - | - |
| disablePointer | `boolean` | `false` | Disable pointer events including move/dragging.  Useful for `mode="canvas" but only want zoomTo() interactions |
| transformContext | `TransformContextValue` | - | A bindable reference to the transform context value. |
| initialScrollMode | `TransformScrollMode` | `'none'` | Initial scroll mode. This is set to `none` by default, but can be set to `scale` or `translate` |
| clickDistance | `number` | `10` | Distance/threshold to consider drag vs click (disable click propagation) |
| initialTranslate | `{ x: number; y: number; }` | - | Initial translate value |
| initialScale | `number` | - | Initial scale value |
| onTransform | `(details: { scale: number; translate: { x: number; y: number; }; }) =&gt; void` | - | A callback function that is called when the transform is applied. |
| ondragstart | `() =&gt; void` | - | - |
| ondragend | `() =&gt; void` | - | - |
| ref | `HTMLElement` | - | - |
| children | `Snippet&lt;[ { transformContext: TransformContextValue; } ]&gt;` | - | - |
| motion | `MotionProp` | - | - |

**Extends:** `Without<HTMLAttributes<HTMLElement>, TransformContextPropsWithoutHTML>`

### Examples

- [pan-zoom-html-image](https://layerchart.com/docs/components/TransformContext/pan-zoom-html-image)
- [pan-zoom-svg-image](https://layerchart.com/docs/components/TransformContext/pan-zoom-svg-image)
- [playground](https://layerchart.com/docs/components/TransformContext/playground)

### Related

- [Chart](https://layerchart.com/docs/components/Chart)
- [Pack/basic](https://layerchart.com/docs/components/Pack/basic)
- [Tree/basic](https://layerchart.com/docs/components/Tree/basic)
- [GeoPath/transform-canvas](https://layerchart.com/docs/components/GeoPath/transform-canvas)
- [GeoPath/transform-projection](https://layerchart.com/docs/components/GeoPath/transform-projection)
- [GeoTile/zoomable](https://layerchart.com/docs/components/GeoTile/zoomable)
- [GeoSpline/draggable-globe](https://layerchart.com/docs/components/GeoSpline/draggable-globe)

## Tree

Layout component which organizes hierarchical data into a branching structure with parent nodes connected to child nodes, visually representing relationships and levels of the hierarchy.

**Category:** layout

**Supported Layers:** svg, canvas

## Usage

See example: basic

### API

| Property | Type | Default | Description |
|----------|------|---------|-------------|
| **hierarchy** (required) | `HierarchyNode&lt;T&gt;` | - | d3 hierarchy node |
| nodeSize | `[ number, number ]` | - | Sets this tree layout’s node size to the specified two-element array of numbers `[width, height]`. If unset, layout size is used instead.  When a node size is specified, the root node is always positioned at `⟨0, 0⟩`.  see: https://github.com/d3/d3-hierarchy#tree_nodeSize |
| separation | `(a: HierarchyPointNode&lt;any&gt;, b: HierarchyPointNode&lt;any&gt;) =&gt; number` | - | see: https://github.com/d3/d3-hierarchy#tree_separation |
| orientation | `'vertical' \| 'horizontal'` | `'horizontal'` | Orientation of the tree layout. |
| children | `Snippet&lt;[ { nodes: HierarchyPointNode&lt;any&gt;[]; links: HierarchyPointLink&lt;any&gt;[]; } ]&gt;` | - | - |

### Examples

- [basic](https://layerchart.com/docs/components/Tree/basic)
- [html-nodes](https://layerchart.com/docs/components/Tree/html-nodes)

## Treemap

Layout component which visualizes hierarchical data as nested rectangles, where each rectangle’s size represents a quantitative value and nesting reflects the hierarchy.

**Category:** layout

**Supported Layers:** svg, canvas

{#if settings.layer === 'canvas'}
> warning: Examples broken due to `Group` not positioning [correctly](https://github.com/techniq/layerchart/issues/662) with `Canvas` layers

{/if}

## Usage

See example: basic

### Playground

See example: playground

### API

| Property | Type | Default | Description |
|----------|------|---------|-------------|
| **hierarchy** (required) | `HierarchyNode&lt;T&gt;` | - | d3 hierarchy node |
| tile | `typeof treemapSquarify \| 'binary' \| 'squarify' \| 'resquarify' \| 'dice' \| 'slice' \| 'sliceDice'` | `treemapSquarify` | The tile function to use for the treemap layout. |
| padding | `number \| ((node: HierarchyRectangularNode&lt;T&gt;) =&gt; number)` | `0` | The padding between nodes. |
| paddingInner | `number \| ((node: HierarchyRectangularNode&lt;T&gt;) =&gt; number)` | `0` | The inner padding between nodes. |
| paddingOuter | `number \| ((node: HierarchyRectangularNode&lt;T&gt;) =&gt; number)` | `0` | The outer padding between nodes. |
| paddingTop | `number \| ((node: HierarchyRectangularNode&lt;T&gt;) =&gt; number)` | `0` | The top padding between nodes. |
| paddingBottom | `number \| ((node: HierarchyRectangularNode&lt;T&gt;) =&gt; number)` | `0` | The bottom padding between nodes. |
| paddingLeft | `number \| ((node: HierarchyRectangularNode&lt;T&gt;) =&gt; number)` | - | The left padding between nodes. |
| paddingRight | `number \| ((node: HierarchyRectangularNode&lt;T&gt;) =&gt; number)` | - | The right padding between nodes. |
| maintainAspectRatio | `boolean` | `false` | Modify tiling function for approapriate aspect ratio when treemap is zoomed in |
| children | `Snippet&lt;[ { nodes: HierarchyRectangularNode&lt;T&gt;[]; } ]&gt;` | - | - |

### Examples

- [basic](https://layerchart.com/docs/components/Treemap/basic)
- [complex](https://layerchart.com/docs/components/Treemap/complex)
- [nested-filter](https://layerchart.com/docs/components/Treemap/nested-filter)
- [nested-zoom](https://layerchart.com/docs/components/Treemap/nested-zoom)
- [playground](https://layerchart.com/docs/components/Treemap/playground)
- [stacked-zoom](https://layerchart.com/docs/components/Treemap/stacked-zoom)

## Voronoi

Interaction component which creates Voronoi diagrams to divide a plane according to the nearest points, aiding spatial analysis and visualization.

**Category:** interactions

**Supported Layers:** svg, canvas

## Usage

See example: radius

### API

| Property | Type | Default | Description |
|----------|------|---------|-------------|
| data | `any` | - | Override data instead of using context |
| r | `number` | - | Radius to clip voronoi cells.  `0` or `undefined` to disables clipping |
| classes | `{ root?: string; path?: string; }` | `= {}` | Classes to apply to the root and path elements |
| onclick | `(e: MouseEvent, details: { data: any; point?: [ number, number ]; feature?: GeoPermissibleObjects; }) =&gt; void` | - | - |
| onpointerenter | `(e: PointerEvent, details: { data: any; point?: [ number, number ]; feature?: GeoPermissibleObjects; }) =&gt; void` | - | - |
| onpointermove | `(e: PointerEvent, details: { data: any; point?: [ number, number ]; feature?: GeoPermissibleObjects; }) =&gt; void` | - | - |
| onpointerdown | `(e: PointerEvent, details: { data: any; point?: [ number, number ]; feature?: GeoPermissibleObjects; }) =&gt; void` | - | - |

**Extends:** `Without<Omit<GroupProps, 'children'>, VoronoiPropsWithoutHTML>`

### Examples

- [radius](https://layerchart.com/docs/components/Voronoi/radius)

### Related

- [TooltipContext](https://layerchart.com/docs/components/TooltipContext)

---

# Utilities

## cls

Utility function wrapper around tailwind-merge and clsx for easy style overrides.

## Usage

### [clsx()](https://github.com/lukeed/clsx)

```svelte live
<script lang="ts">
	import { cls } from '@layerstack/tailwind';
</script>

<div class={cls('text-center p-2 bg-error-500', true && 'bg-success-500')}>
	{`class={cls('bg-error-500', true && 'bg-success-500')}`}<br />becomes<br />class="bg-success-500"
</div>
```

### [twMerge()](https://github.com/dcastil/tailwind-merge)

```svelte live
<script lang="ts">
	import { cls } from '@layerstack/tailwind';
</script>

<div class={cls('text-center p-2 bg-red-500 bg-info-500')}>last class wins with tailwind-merge</div>
```

### Related

- [/docs/components/BrushContext/selection](https://layerchart.com/docs/utils//docs/components/BrushContext/selection)
- [https://github.com/dcastil/tailwind-merge](https://layerchart.com/docs/utils/https://github.com/dcastil/tailwind-merge)
- [https://github.com/lukeed/clsx](https://layerchart.com/docs/utils/https://github.com/lukeed/clsx)
- [https://www.layerstack.dev/docs/tailwind/utils](https://layerchart.com/docs/utils/https://www.layerstack.dev/docs/tailwind/utils)

## format

Utility function to easily manipulate numbers and dates to different formats and locales.

## Usage

### format()

> note: [Full Layerstack API](https://www.layerstack.dev/docs/utils/format)

```svelte live
<script lang="ts">
	import { format } from '@layerstack/utils';
</script>

{format(1234.56, 'integer')}<br />
{format(1234.56, 'decimal')}<br />
{format(1234.56, 'currency')}<br />
{format(1234.56, 'currency', { currency: 'EUR' })}<br />
{format(0.5678, 'percent')}<br />
{format(0.5678, 'percentRound')}<br />
{format(1_234_567, 'metric')}<br />
{format(new Date(), 'day', { variant: 'short' })}<br />
{format(new Date(), 'custom', { custom: 'eee, MMMM do' })}<br />
```

### Related

- [/docs/components/BarChart/sparkbar-fixed-position-tooltip](https://layerchart.com/docs/utils//docs/components/BarChart/sparkbar-fixed-position-tooltip)
- [https://www.layerstack.dev/docs/utils/format#playgrounds](https://layerchart.com/docs/utils/https://www.layerstack.dev/docs/utils/format#playgrounds)

## pivot

Utility function to reshape and aggregate tabular data, converting between long and wide formats.

## Usage

### pivotLonger (columns to rows)

See example: pivot-longer

### pivotWider (rows to columns)

See example: pivot-wider

## string

Utility string functions.

## Usage

> note: See [LayerStack](https://www.layerstack.dev/docs/utils/string) for full API documentation.

### truncate()

```svelte live
<script lang="ts">
	import { truncate } from '@layerstack/utils';

	const str = 'This is a really long string of text.';
</script>

{truncate(str, 21)}
```

> note: Truncation is also built into the [Text](/docs/components/Text) component via the `truncate` prop.

### toTitleCase()

```svelte live
<script lang="ts">
	import { toTitleCase } from '@layerstack/utils';
</script>

{toTitleCase('string of text')}
```

### Related

- [/docs/components/BarChart/duration-civilization-timeline](https://layerchart.com/docs/utils//docs/components/BarChart/duration-civilization-timeline)
- [/docs/components/BarChart/duration-civilization-timeline-dense](https://layerchart.com/docs/utils//docs/components/BarChart/duration-civilization-timeline-dense)
- [/docs/components/Text/playground](https://layerchart.com/docs/utils//docs/components/Text/playground)
- [components/Text](https://layerchart.com/docs/utils/components/Text)
- [https://www.layerstack.dev/docs/utils/string](https://layerchart.com/docs/utils/https://www.layerstack.dev/docs/utils/string)