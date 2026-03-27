# Preline Plugins: Form Controls

## HSInputNumber

**Init**: `[data-hs-input-number]:not(.--prevent-on-load-init)`

```html
<div data-hs-input-number='{ "min": 0, "max": 100, "step": 1 }'>
  <button data-hs-input-number-decrement class="size-8 flex justify-center items-center border rounded-lg">-</button>
  <input data-hs-input-number-input class="w-16 text-center border-0" type="text" value="0" />
  <button data-hs-input-number-increment class="size-8 flex justify-center items-center border rounded-lg">+</button>
</div>
```

**Options**: `min`: 0, `max`: null (unlimited), `step`: 1, `forceBlankValue`: false

**Internal attrs**: `data-hs-input-number-input`, `data-hs-input-number-increment`, `data-hs-input-number-decrement`

**CSS class toggled**: `disabled` (on root when disabled)

**Event**: `change.hs.inputNumber` with `{ inputValue }`

**Variant**: `hs-input-number-disabled:`

---

## HSPinInput

**Init**: `[data-hs-pin-input]:not(.--prevent-on-load-init)`

```html
<div data-hs-pin-input='{ "availableCharsRE": "^[0-9]+$" }'>
  <input data-hs-pin-input-item class="size-12 text-center border rounded-lg text-sm" type="text" />
  <input data-hs-pin-input-item class="size-12 text-center border rounded-lg text-sm" type="text" />
  <input data-hs-pin-input-item class="size-12 text-center border rounded-lg text-sm" type="text" />
  <input data-hs-pin-input-item class="size-12 text-center border rounded-lg text-sm" type="text" />
</div>
```

**Options**: `availableCharsRE`: `'^[a-zA-Z0-9]+$'` (default regex for allowed chars)

**CSS class toggled**: `active` (on root when all fields filled)

**Event**: `completed.hs.pinInput` with `{ currentValue }`

**Variant**: `hs-pin-input-active:` (all fields filled)

---

## HSTogglePassword

**Init**: `[data-hs-toggle-password]:not(.--prevent-on-load-init)`

```html
<div class="relative">
  <input id="pw" type="password" class="py-3 px-4 pe-11 w-full border rounded-lg text-sm" />
  <button data-hs-toggle-password='{ "target": "#pw" }' class="absolute inset-y-0 end-0 flex items-center pe-3">
    <svg class="hidden hs-password-active:block size-4"><!-- eye icon --></svg>
    <svg class="hs-password-active:hidden size-4"><!-- eye-off icon --></svg>
  </button>
</div>
```

**Options**: `target`: CSS selector string or array of selectors (for multi-field)

**Multi-target**: Use `data-hs-toggle-password-group` on wrapper element

**CSS class toggled**: `active` (on toggle button or group)

**Methods**: `show()`, `hide()`, `destroy()`

**Event**: `toggle.hs.toggle-select`

---

## HSStrongPassword

**Init**: `[data-hs-strong-password]:not(.--prevent-on-load-init)`

```html
<input id="pw-input" type="password" class="py-3 px-4 w-full border rounded-lg text-sm" />

<div data-hs-strong-password='{
  "target": "#pw-input",
  "hints": "#pw-hints",
  "stripClasses": "hs-strong-password:bg-primary hs-strong-password-accepted:bg-teal-500 h-2 flex-auto rounded-full bg-primary-200 dark:bg-neutral-700",
  "minLength": 8,
  "mode": "default",
  "checksExclude": [],
  "specialCharactersSet": "!\"#$%&()*+,-./:;<=>?@[\\\\]^_{|}~"
}' class="flex gap-x-1 mt-2">
</div>

<div id="pw-hints" class="hidden">
  <div>
    <span data-hs-strong-password-hints-rule-text="min-length" class="text-sm hs-strong-password-active:text-teal-500">
      Min 8 characters
    </span>
  </div>
  <div>
    <span data-hs-strong-password-hints-rule-text="lowercase" class="text-sm hs-strong-password-active:text-teal-500">
      Lowercase letter
    </span>
  </div>
  <div>
    <span data-hs-strong-password-hints-rule-text="uppercase" class="text-sm hs-strong-password-active:text-teal-500">
      Uppercase letter
    </span>
  </div>
  <div>
    <span data-hs-strong-password-hints-rule-text="numbers" class="text-sm hs-strong-password-active:text-teal-500">
      Number
    </span>
  </div>
  <div>
    <span data-hs-strong-password-hints-rule-text="special-characters" class="text-sm hs-strong-password-active:text-teal-500">
      Special character
    </span>
  </div>
</div>
```

**Options**:

| Option | Type | Default |
|--------|------|---------|
| `target` | string/element | required |
| `hints` | string/element | -- |
| `stripClasses` | string | -- |
| `minLength` | number | `6` |
| `mode` | `'default'`/`'popover'` | `'default'` |
| `popoverSpace` | number | `10` |
| `checksExclude` | string[] | `[]` |
| `specialCharactersSet` | string | common special chars |

**Available checks**: `'lowercase'`, `'uppercase'`, `'numbers'`, `'special-characters'`, `'min-length'`

**Hints attrs**: `data-hs-strong-password-hints-weakness-text='["Weak", "Medium", "Strong", "Very Strong"]'`, `data-hs-strong-password-hints-rule-text="min-length"`

**CSS classes toggled**: `accepted` (on root when all checks pass), `passed` (on strip elements), `active` (on hint rules that pass)

**Event**: `change.hs.strongPassword` with `{ strength, rules }`

**Methods**: `recalculateDirection()`, `destroy()`

**Variants**: `hs-password-active:`, `hs-strong-password:` (strip passed), `hs-strong-password-accepted:` (all passed), `hs-strong-password-active:` (rule active)

---

## HSTextareaAutoHeight

**Init**: `[data-hs-textarea-auto-height]:not(.--prevent-on-load-init)`

```html
<textarea data-hs-textarea-auto-height='{ "defaultHeight": 100 }' class="py-3 px-4 w-full border rounded-lg text-sm" rows="3"></textarea>
```

**Options**: `defaultHeight`: 0 (minimum height in px)

Auto-detects if inside hidden parents (`.hs-overlay.hidden`, `[role="tabpanel"].hidden`, `.hs-collapse.hidden`) and recalculates when parent becomes visible.

---

## HSToggleCount

**Init**: `[data-hs-toggle-count]:not(.--prevent-on-load-init)`

```html
<input type="checkbox" id="toggle" class="hidden" />
<span data-hs-toggle-count='{ "target": "#toggle", "min": 100, "max": 101, "duration": 700 }'>100</span>
<label for="toggle" class="cursor-pointer">Toggle</label>
```

**Options**: `target`: CSS selector for checkbox, `min`: 0, `max`: 0, `duration`: 700 (ms)

**Methods**: `countUp()`, `countDown()`, `destroy()`

---

## HSDatepicker

**Init**: `[data-hs-datepicker]:not(.--prevent-on-load-init)`

**Requires**: `vanilla-calendar-pro` loaded globally as `window.VanillaCalendarPro`

```html
<input data-hs-datepicker='{
  "dateFormat": "MM/DD/YYYY",
  "mode": "default"
}' type="text" class="py-3 px-4 w-full border rounded-lg text-sm" placeholder="Select date" />
```

**Key options**:

| Option | Type | Default |
|--------|------|---------|
| `dateFormat` | string | -- |
| `dateLocale` | string | -- |
| `mode` | `'default'`/`'custom-select'` | `'default'` |
| `inputMode` | boolean | `true` |
| `selectionDatesMode` | `'single'`/`'multiple'`/`'multiple-ranged'` | `'single'` |
| `removeDefaultStyles` | boolean | `false` |
| `applyUtilityClasses` | boolean | `false` |
| `replaceTodayWithText` | boolean | `false` |
| `inputModeOptions.dateSeparator` | string | `'.'` |
| `inputModeOptions.itemsSeparator` | string | `', '` |

**Methods**: `formatDate(date, format?)`, `destroy()`

**Event**: `change.hs.datepicker` with `{ selectedDates, selectedTime }`

**Datepicker variants**: `hs-vc-date-today:`, `hs-vc-date-hover:`, `hs-vc-date-selected:`, `hs-vc-calendar-selected-middle:`, `hs-vc-calendar-selected-first:`, `hs-vc-calendar-selected-last:`, `hs-vc-date-weekend:`, `hs-vc-date-month-prev:`, `hs-vc-date-month-next:`, `hs-vc-months-month-selected:`, `hs-vc-years-year-selected:`

---

## HSRangeSlider

**Init**: `[data-hs-range-slider]:not(.--prevent-on-load-init)`

**Requires**: `nouislider` loaded globally as `window.noUiSlider`

```html
<div data-hs-range-slider='{
  "start": [25, 75],
  "range": { "min": 0, "max": 100 },
  "connect": true,
  "formatter": "integer"
}'>
</div>
<div class="hs-range-slider-current-value"></div>
```

**Options**: Extends noUiSlider options plus:
- `disabled`: boolean
- `wrapper`: element (or `.hs-range-slider-wrapper`)
- `currentValue`: element[] (or `.hs-range-slider-current-value`)
- `formatter`: `'integer'` | `'thousandsSeparatorAndDecimalPoints'` | `{ type, prefix, postfix }`
- `icons.handle`: HTML string for handle icon

**Variant**: `hs-range-slider-disabled:`

---

## HSFileUpload

**Init**: `[data-hs-file-upload]:not(.--prevent-on-load-init)`

**Requires**: `dropzone` + `lodash` loaded globally

```html
<div data-hs-file-upload='{
  "url": "/upload",
  "acceptedFiles": "image/*",
  "maxFiles": 3,
  "singleton": false,
  "autoHideTrigger": false,
  "extensions": {
    "default": { "icon": "<svg>...</svg>", "class": "text-gray-400" },
    "xls": { "icon": "<svg>...</svg>", "class": "text-green-400" }
  }
}'>
  <div data-hs-file-upload-trigger class="cursor-pointer border-2 border-dashed rounded-lg p-12 text-center">
    <span>Drop files here or click to upload</span>
  </div>
  <div data-hs-file-upload-previews class="space-y-3 mt-3">
    <template data-hs-file-upload-preview>
      <div class="flex items-center gap-x-3 p-3 bg-layer border border-layer-line rounded-lg">
        <div data-hs-file-upload-file-icon></div>
        <div>
          <p data-hs-file-upload-file-name class="text-sm font-medium text-foreground"></p>
          <p data-hs-file-upload-file-size class="text-xs text-muted-foreground-1"></p>
        </div>
        <div class="ms-auto">
          <div data-hs-file-upload-progress-bar-pane></div>
          <button data-hs-file-upload-remove>Remove</button>
        </div>
      </div>
    </template>
  </div>
</div>
```

**Internal data attrs**: `data-hs-file-upload-trigger`, `data-hs-file-upload-previews`, `data-hs-file-upload-preview` (template), `data-hs-file-upload-clear`, `data-hs-file-upload-remove`, `data-hs-file-upload-reload`, `data-hs-file-upload-file-name`, `data-hs-file-upload-file-ext`, `data-hs-file-upload-file-size`, `data-hs-file-upload-file-icon`, `data-hs-file-upload-progress-bar`, `data-hs-file-upload-progress-bar-pane`, `data-hs-file-upload-progress-bar-value`

**Options**: Extends Dropzone options + `singleton`: boolean, `autoHideTrigger`: boolean, `extensions`: icon/class map by file type

**Variant**: `hs-file-upload-complete:` (upload finished)
