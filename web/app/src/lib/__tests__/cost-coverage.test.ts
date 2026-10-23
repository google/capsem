import { expect, it } from 'vitest';
import { formatEstimatedCost, mergeCostCoverage } from '../format';

it('distinguishes free usage from missing or partial model pricing', () => {
  const priced = { priced_calls: 2, unpriced_calls: 0, unknown_calls: 0 };
  expect(formatEstimatedCost(0, priced)).toBe('$0.00');
  expect(formatEstimatedCost(0)).toBe('Unknown');
  expect(formatEstimatedCost(0, { ...priced, unpriced_calls: 4 })).toBe('Unknown');
  expect(formatEstimatedCost(0.00438, { ...priced, unpriced_calls: 4 })).toBe('$0.00438+');
  expect(formatEstimatedCost(0.00438, { ...priced, unknown_calls: 4 })).toBe('$0.00438+');
  expect(formatEstimatedCost(0.00438, priced)).toBe('$0.00438');
  expect(formatEstimatedCost(undefined, priced)).toBe('--');
});

it('combines coverage without treating an old response as measured free usage', () => {
  expect(mergeCostCoverage([])).toEqual({ priced_calls: 0, unpriced_calls: 0, unknown_calls: 0 });
  expect(mergeCostCoverage([undefined])).toBeUndefined();
  expect(mergeCostCoverage([{ priced_calls: 2, unpriced_calls: 4, unknown_calls: 0 }]))
    .toEqual({ priced_calls: 2, unpriced_calls: 4, unknown_calls: 0 });
});
