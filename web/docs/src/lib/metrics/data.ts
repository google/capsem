import { readFileSync } from 'node:fs';
import type { Benchmarks, Statistics } from './types';

function read(name: string) {
  try {
    return JSON.parse(readFileSync(new URL(`../../../public/data/metrics/${name}.json`, import.meta.url), 'utf8'));
  } catch (error) {
    throw new Error(`Generate documentation metrics before building: pnpm run generate:metrics (${name})`, { cause: error });
  }
}
export const statistics = read('statistics') as Statistics;
export const benchmarks = read('benchmarks') as Benchmarks;
