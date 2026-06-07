# Preline CSS Components: Basic Forms

These are Tailwind utility patterns for native HTML form elements. For advanced interactive forms (custom select, combobox, etc.), see `plugins-forms.md`.

## Input

```html
<!-- Default -->
<input type="text" class="py-3 px-4 block w-full border-line-2 rounded-lg text-sm focus:border-primary focus:ring-primary disabled:opacity-50 disabled:pointer-events-none bg-layer text-foreground" placeholder="Enter text">

<!-- Small -->
<input type="text" class="py-2 px-3 block w-full border-line-2 rounded-lg text-sm focus:border-primary focus:ring-primary bg-layer text-foreground">

<!-- Large -->
<input type="text" class="py-3 px-4 block w-full border-line-2 rounded-lg text-lg focus:border-primary focus:ring-primary bg-layer text-foreground">

<!-- With icon -->
<div class="relative">
  <input type="text" class="py-3 ps-11 pe-4 block w-full border-line-2 rounded-lg text-sm focus:border-primary focus:ring-primary bg-layer text-foreground">
  <div class="absolute inset-y-0 start-0 flex items-center ps-4 pointer-events-none">
    <svg class="size-4 text-muted-foreground"><!-- icon --></svg>
  </div>
</div>

<!-- Validation states -->
<input type="text" class="py-3 px-4 block w-full border-teal-500 rounded-lg text-sm focus:border-teal-500 focus:ring-teal-500">
<input type="text" class="py-3 px-4 block w-full border-red-500 rounded-lg text-sm focus:border-red-500 focus:ring-red-500">
```

## Input Group

```html
<div class="flex rounded-lg shadow-2xs">
  <span class="px-4 inline-flex items-center min-w-fit rounded-s-lg border border-e-0 border-line-2 bg-muted text-sm text-muted-foreground-2">@</span>
  <input type="text" class="py-3 px-4 block w-full border-line-2 shadow-2xs rounded-e-lg text-sm focus:z-10 focus:border-primary focus:ring-primary bg-layer text-foreground">
</div>
```

## Textarea

```html
<textarea class="py-3 px-4 block w-full border-line-2 rounded-lg text-sm focus:border-primary focus:ring-primary bg-layer text-foreground" rows="3" placeholder="Type here..."></textarea>
```

For auto-expanding, use the HSTextareaAutoHeight plugin: add `data-hs-textarea-auto-height`.

## File Input

```html
<input type="file" class="block w-full border border-line-2 shadow-2xs rounded-lg text-sm focus:z-10 focus:border-primary focus:ring-primary bg-layer text-foreground
  file:bg-muted file:border-0 file:me-4 file:py-3 file:px-4 file:text-muted-foreground-2">
```

## Checkbox

```html
<div class="flex items-center">
  <input type="checkbox" class="shrink-0 mt-0.5 border-line-3 rounded-sm text-primary focus:ring-primary checked:border-primary disabled:opacity-50 disabled:pointer-events-none" id="cb-1">
  <label for="cb-1" class="text-sm text-foreground ms-3">Label</label>
</div>
```

**Indeterminate**: Set via JS `checkbox.indeterminate = true`

## Radio

```html
<div class="flex items-center">
  <input type="radio" name="group" class="shrink-0 mt-0.5 border-line-3 rounded-full text-primary focus:ring-primary checked:border-primary disabled:opacity-50" id="radio-1">
  <label for="radio-1" class="text-sm text-foreground ms-3">Option 1</label>
</div>
```

**Card-style radio**:
```html
<label class="flex p-3 w-full bg-layer border border-layer-line rounded-lg text-sm focus:border-primary focus:ring-primary has-[:checked]:border-primary has-[:checked]:bg-primary-50 cursor-pointer">
  <input type="radio" name="plan" class="shrink-0 mt-0.5 border-line-3 rounded-full text-primary focus:ring-primary">
  <span class="text-sm text-foreground ms-3">Plan name</span>
</label>
```

## Switch

```html
<div class="flex items-center">
  <input type="checkbox" id="switch-1" class="relative w-11 h-6 p-px bg-surface border-transparent text-transparent rounded-full cursor-pointer transition-colors ease-in-out duration-200 focus:ring-primary checked:bg-none checked:text-primary checked:border-primary focus:checked:border-primary" role="switch">
  <label for="switch-1" class="text-sm text-foreground ms-3">Toggle</label>
</div>
```

Token: `bg-switch` for the switch knob color.

## Select (Native)

```html
<select class="py-3 px-4 pe-9 block w-full border-line-2 rounded-lg text-sm focus:border-primary focus:ring-primary bg-layer text-foreground">
  <option selected>Select option</option>
  <option>Option 1</option>
  <option>Option 2</option>
</select>
```

For advanced select with search/tags/API, use the HSSelect plugin.

## Color Picker

```html
<input type="color" class="p-1 h-10 w-14 block bg-layer border border-line-2 cursor-pointer rounded-lg" value="#2563eb">
```

## Time Picker

```html
<input type="time" class="py-3 px-4 block w-full border-line-2 rounded-lg text-sm focus:border-primary focus:ring-primary bg-layer text-foreground">
```

## Range Slider (Native)

```html
<input type="range" class="w-full bg-transparent cursor-pointer appearance-none focus:outline-hidden
  [&::-webkit-slider-thumb]:w-2.5 [&::-webkit-slider-thumb]:h-2.5 [&::-webkit-slider-thumb]:-mt-0.5 [&::-webkit-slider-thumb]:appearance-none [&::-webkit-slider-thumb]:bg-layer [&::-webkit-slider-thumb]:shadow-[0_0_0_4px_rgba(37,99,235,1)] [&::-webkit-slider-thumb]:rounded-full
  [&::-webkit-slider-runnable-track]:w-full [&::-webkit-slider-runnable-track]:h-1.5 [&::-webkit-slider-runnable-track]:bg-surface [&::-webkit-slider-runnable-track]:rounded-full" min="0" max="100">
```

For advanced range slider, use the HSRangeSlider plugin (wraps noUiSlider).
