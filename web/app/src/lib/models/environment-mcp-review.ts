import type { EnvironmentCatalogPin, McpCatalogDetailResponse } from '@capsem/sdk';
import { catalogSelector, matchesCatalogPin } from './environment-catalog';
import type { EnvironmentDraft, EnvironmentSpecification } from './environment-editor';

export type McpLaunchDefinition =
  | { kind: 'remote'; url: string }
  | { kind: 'sandbox'; executable: string; args?: string[]; workdir?: string | null };

export interface McpLaunchReview {
  pin: EnvironmentCatalogPin;
  definition: McpLaunchDefinition;
}

/** Freeze the exact launch definitions shown at review and guard new selectors on save. */
export async function prepareMcpCatalogReview(
  draft: EnvironmentDraft,
  saved: EnvironmentSpecification | undefined,
  read: (id: string, version: string) => Promise<McpCatalogDetailResponse>,
): Promise<{ draft: EnvironmentDraft; launches: McpLaunchReview[] }> {
  const catalog = draft.mcps?.catalog;
  if (!catalog?.selected?.length) return { draft, launches: [] };
  const launches: McpLaunchReview[] = [];
  const selected: NonNullable<typeof catalog.selected> = [];
  const identities = new Set<string>();
  for (const entry of catalog.selected) {
    if (identities.has(entry.id)) throw new Error('Choose one catalog version for each MCP server.');
    identities.add(entry.id);
    const retained = saved?.mcps?.catalog?.selected?.find(pin => pin.id === entry.id && pin.version === entry.version);
    const definitions = saved?.mcps?.catalog_configuration ?? {};
    const definition = retained && Object.hasOwn(definitions, entry.id) ? definitions[entry.id] : undefined;
    if (retained && definition) {
      requireSnapshot(entry.expected_catalog, retained);
      launches.push({ pin: retained, definition });
      selected.push(entry);
    } else {
      const published = await read(entry.id, entry.version);
      if (retained) requireSnapshot(catalogSelector(retained).expected_catalog, published.selection);
      requireSnapshot(entry.expected_catalog, published.selection);
      launches.push({ pin: published.selection, definition: published.definition });
      selected.push(retained ? entry : catalogSelector(published.selection));
    }
  }
  return { draft: { ...draft, mcps: { ...draft.mcps, catalog: { ...catalog, selected } } }, launches };
}

function requireSnapshot(expected: { reference: string; digest: string } | null | undefined, pin: EnvironmentCatalogPin) {
  if (expected && !matchesCatalogPin(expected, pin)) {
    throw new Error('An MCP catalog selection changed. Browse the catalog again and review the updated draft.');
  }
}
