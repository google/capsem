import { CatalogAvailability, CatalogRefreshError, EnvironmentDraftSchema, type CatalogStatus, type EnvironmentCatalogPin } from '@capsem/sdk';
import { environmentSections, type EnvironmentDraft } from './environment-editor';

export type CatalogDraftSection = 'skills' | 'access' | 'detection';

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

export function stageCatalogSelection(value: string, section: CatalogDraftSection, pin: EnvironmentCatalogPin): string {
  const draft = readSection(value, section);
  const entries = selected(draft, section);
  const previous = entries.find(item => item.id === pin.id);
  if (previous?.version === pin.version && !previous.expected_catalog) return value;
  const selection = {
    id: pin.id,
    version: pin.version,
    expected_catalog: { reference: pin.catalog_reference, digest: pin.resolved_digest },
  };
  const replacement = previous ? entries.map(item => item.id === pin.id ? selection : item) : [...entries, selection];
  if (section === 'skills') return JSON.stringify({ ...draft.skills, selected: replacement }, null, 2);
  const rules = draft[section];
  const parameters = { ...rules?.parameters };
  if (previous && previous.version !== pin.version) delete parameters[pin.id];
  return JSON.stringify({ ...rules, catalog: { ...rules?.catalog, selected: replacement }, parameters }, null, 2);
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
