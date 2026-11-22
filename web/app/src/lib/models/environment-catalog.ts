import { CatalogAvailability, CatalogRefreshError, EnvironmentDraftSchema, type CatalogStatus, type EnvironmentCatalogPin } from '@capsem/sdk';
import { environmentSections, type EnvironmentDraft, type McpCatalogInput } from './environment-editor';

type SkillOrRuleSection = 'skills' | 'access' | 'detection';
type CatalogDraftSection = SkillOrRuleSection | 'mcps';

function readSection(value: string, section: CatalogDraftSection): EnvironmentDraft {
  try {
    const result = EnvironmentDraftSchema.safeParse({ name: 'selection', [section]: JSON.parse(value) });
    if (result.success) return result.data;
  } catch {
    // The editor keeps the original text so the operator can correct it.
  }
  throw new Error(`Check the configuration for ${environmentSections[section]} before staging a catalog entry.`);
}

function selected(draft: EnvironmentDraft, section: CatalogDraftSection) {
  return (section === 'skills' ? draft.skills : draft[section]?.catalog)?.selected ?? [];
}

export function currentCatalogSelection(value: string, section: CatalogDraftSection, id: string) {
  try {
    return selected(readSection(value, section), section).find(item => item.id === id);
  } catch {
    return undefined;
  }
}

export function stageCatalogSelection(value: string, section: SkillOrRuleSection, pin: EnvironmentCatalogPin): string {
  const draft = readSection(value, section);
  const entries = selected(draft, section);
  const previous = entries.find(item => item.id === pin.id);
  if (previous?.version === pin.version && !previous.expected_catalog) return value;
  const selection = catalogSelector(pin);
  const replacement = previous ? entries.map(item => item.id === pin.id ? selection : item) : [...entries, selection];
  if (section === 'skills') return JSON.stringify({ ...draft.skills, selected: replacement }, null, 2);
  const rules = draft[section];
  const parameters = { ...rules?.parameters };
  if (previous && previous.version !== pin.version) delete parameters[pin.id];
  return JSON.stringify({ ...rules, catalog: { ...rules?.catalog, selected: replacement }, parameters }, null, 2);
}

export function catalogSelector(pin: EnvironmentCatalogPin) {
  return { id: pin.id, version: pin.version, expected_catalog: { reference: pin.catalog_reference, digest: pin.resolved_digest } };
}

export function matchesCatalogPin(reference: { reference: string; digest: string }, pin: EnvironmentCatalogPin) {
  return reference.reference === pin.catalog_reference && reference.digest === pin.resolved_digest;
}

export function mcpCatalogDraft(value: string) {
  try {
    return readSection(value, 'mcps').mcps;
  } catch {
    return undefined;
  }
}

export function stageMcpCatalogSelection(
  value: string, pin: EnvironmentCatalogPin, input: McpCatalogInput, saved?: EnvironmentCatalogPin,
): string {
  const { catalog, catalog_configuration, ...other } = readSection(value, 'mcps').mcps ?? {};
  if (Object.hasOwn(other.servers ?? {}, pin.id)) throw new Error('An inline MCP server already uses this ID.');
  if (saved && !matchesCatalogPin(catalogSelector(saved).expected_catalog, pin)) {
    throw new Error('This version differs from the saved digest. Choose a new version to replace it.');
  }
  const next = saved ? { id: pin.id, version: pin.version } : catalogSelector(pin);
  const entries = catalog?.selected ?? [];
  const replacement = entries.some(item => item.id === pin.id)
    ? entries.map(item => item.id === pin.id ? next : item) : [...entries, next];
  const configuration = Object.entries(catalog_configuration ?? {}).filter(([id]) => id !== pin.id);
  if (Object.keys(input.values ?? {}).length || input.binding_id || Object.keys(input.secrets ?? {}).length) {
    configuration.push([pin.id, input]);
  }
  return JSON.stringify({
    ...other, catalog: { ...catalog, selected: replacement },
    ...(configuration.length ? { catalog_configuration: Object.fromEntries(configuration) } : {}),
  }, null, 2);
}

export function removeMcpCatalogSelection(value: string, id: string): string {
  const { catalog, catalog_configuration, permissions, ...other } = readSection(value, 'mcps').mcps ?? {};
  const configuration = Object.entries(catalog_configuration ?? {}).filter(([key]) => key !== id);
  const overrides = Object.entries(permissions ?? {}).filter(([key]) => key !== id);
  return JSON.stringify({
    ...other, catalog: { ...catalog, selected: (catalog?.selected ?? []).filter(entry => entry.id !== id) },
    ...(configuration.length ? { catalog_configuration: Object.fromEntries(configuration) } : {}),
    ...(overrides.length ? { permissions: Object.fromEntries(overrides) } : {}),
  }, null, 2);
}

const availabilityLabels: Record<CatalogAvailability, string> = {
  [CatalogAvailability.AVAILABLE]: 'Verified',
  [CatalogAvailability.STALE]: 'Stale',
  [CatalogAvailability.UNVERIFIED]: 'Unverified',
  [CatalogAvailability.UNAVAILABLE]: 'Unavailable',
};

const refreshErrors: Record<CatalogRefreshError, string> = {
  [CatalogRefreshError.MISSING_SOURCE]: 'No registry source is configured.',
  [CatalogRefreshError.INVALID_SOURCE]: 'The configured registry source is invalid.',
  [CatalogRefreshError.POLICY_UNAVAILABLE]: 'Registry source policy is unavailable.',
  [CatalogRefreshError.REGISTRY_UNAVAILABLE]: 'The registry could not provide this catalog.',
  [CatalogRefreshError.VERIFICATION_FAILED]: 'The most recent catalog failed verification.',
  [CatalogRefreshError.LOCAL_UNAVAILABLE]: 'The local service could not complete the refresh.',
  [CatalogRefreshError.UNSUPPORTED]: 'Catalog verification is unavailable.',
};

export function catalogStatusText(status: CatalogStatus) {
  return { availability: availabilityLabels[status.availability], error: status.error ? refreshErrors[status.error] : null };
}
