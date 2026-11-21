import { EnvironmentDraftSchema } from '@capsem/sdk';
import type { EnvironmentChangeRequest, EnvironmentCurrentResponse } from '@capsem/sdk';

type Specification = EnvironmentCurrentResponse['document']['specification'];
export type EnvironmentDraft = EnvironmentChangeRequest['specification'];

export const environmentSections = {
  repositories: 'Repositories', variables: 'Environment variables', secrets: 'Secret bindings',
  access: 'Access rules', detection: 'Detection rules', plugins: 'Plugins',
  mcps: 'MCP servers and permissions', skills: 'Skills', packages: 'Package access',
} as const satisfies Partial<Record<keyof EnvironmentDraft, string>>;
export type EnvironmentSection = keyof typeof environmentSections;
export const environmentSectionKeys = Object.keys(environmentSections) as EnvironmentSection[];

const labels: Record<keyof EnvironmentDraft, string> = {
  name: 'Name', description: 'Description', image: 'Image', startup: 'Startup', ...environmentSections,
};

export interface EnvironmentFields {
  name: string;
  description: string;
  image: string;
  imageUpdates: string;
  hasStartup: boolean;
  script: string;
  timeout: number | undefined;
  sections: Record<EnvironmentSection, string>;
}

function catalogSelections(catalog: Specification['skills']) {
  return { selected: (catalog?.selected ?? []).map(({ id, version }) => ({ id, version })) };
}

/** Translate committed content to editable selectors. Only the service may resolve their pins. */
export function environmentDraft(specification: Specification): EnvironmentDraft {
  const { image, repositories, access, detection, skills, mcps, ...rest } = specification;
  return {
    ...rest,
    ...(image ? { image: { reference: image.reference, ...(image.update_policy ? { update_policy: image.update_policy } : {}) } } : {}),
    ...(repositories ? { repositories: repositories.map(({ url, ref, destination }) => ({ url, ref, destination })) } : {}),
    ...(access ? { access: { ...access, catalog: catalogSelections(access.catalog) } } : {}),
    ...(detection ? { detection: { ...detection, catalog: catalogSelections(detection.catalog) } } : {}),
    ...(skills ? { skills: catalogSelections(skills) } : {}),
    ...(mcps ? { mcps: { ...mcps, catalog: catalogSelections(mcps.catalog) } } : {}),
  };
}

export function editableFields(specification?: Specification): EnvironmentFields {
  const draft = specification ? environmentDraft(specification) : null;
  return {
    name: draft?.name ?? '', description: draft?.description ?? '', image: draft?.image?.reference ?? '',
    imageUpdates: draft?.image?.update_policy ?? 'manual',
    hasStartup: draft?.startup != null, script: draft?.startup?.script ?? '', timeout: draft?.startup?.timeout_seconds ?? 60,
    sections: Object.fromEntries(environmentSectionKeys.map(key => [key, JSON.stringify(draft?.[key] ?? (key === 'repositories' ? [] : {}), null, 2)])) as EnvironmentFields['sections'],
  };
}

export function reviewedDraft(fields: EnvironmentFields): EnvironmentDraft {
  const name = fields.name.trim();
  if (!name) throw new Error('Enter an environment name.');
  const reference = fields.image.trim();
  if (fields.hasStartup && !reference) throw new Error('Choose an image before adding a startup script.');
  if (fields.hasStartup && (!fields.script.trim() || !fields.timeout || !Number.isInteger(fields.timeout) || fields.timeout < 1)) {
    throw new Error('Enter a startup script and a positive timeout in seconds.');
  }
  const sections = Object.fromEntries(environmentSectionKeys.map(key => {
    try {
      return [key, JSON.parse(fields.sections[key]) as unknown];
    } catch {
      throw new Error(`${environmentSections[key]} contains invalid JSON.`);
    }
  }));
  const result = EnvironmentDraftSchema.safeParse({
    ...sections, name, description: fields.description,
    ...(reference ? { image: { reference, update_policy: fields.imageUpdates } } : {}),
    ...(fields.hasStartup ? { startup: { script: fields.script, timeout_seconds: fields.timeout } } : {}),
  });
  if (!result.success) {
    const field = result.error.issues[0]?.path[0] as keyof EnvironmentDraft | undefined;
    throw new Error(field ? `Check the configuration for ${labels[field]}.` : 'Check the environment configuration.');
  }
  return result.data;
}

function ordered(value: unknown): unknown {
  if (Array.isArray(value)) return value.map(ordered);
  if (value == null || typeof value !== 'object') return value;
  return Object.fromEntries(Object.entries(value).sort(([a], [b]) => a.localeCompare(b)).map(([key, nested]) => [key, ordered(nested)]));
}

export function environmentChanges(before: EnvironmentDraft | null, after: EnvironmentDraft) {
  return (Object.keys(labels) as (keyof EnvironmentDraft)[]).flatMap(key => {
    const previous = before?.[key];
    const next = after[key];
    if (before == null && (next === '' || (next != null && typeof next === 'object' && !Object.keys(next).length))) return [];
    if (JSON.stringify(ordered(previous)) === JSON.stringify(ordered(next))) return [];
    function display(value: unknown) {
      if (value == null) return '—';
      if (typeof value === 'string') return value;
      if (key === 'startup' && typeof value === 'object' && 'script' in value && 'timeout_seconds' in value) {
        return `Timeout ${value.timeout_seconds} s\n\n${value.script}`;
      }
      return JSON.stringify(value, null, 2);
    }
    return [{ label: labels[key], before: display(previous), after: display(next) }];
  });
}
