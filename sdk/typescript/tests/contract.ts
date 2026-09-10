import {readFileSync} from 'node:fs';

export interface Schema {
  $ref?: string; type?: string | string[]; enum?: string[]; oneOf?: Schema[];
  properties?: Record<string, Schema>; required?: string[]; items?: Schema;
  additionalProperties?: Schema | false; minimum?: number;
}

interface Specification {components: {schemas: Record<string, Schema>}}
const specification = JSON.parse(readFileSync(new URL('../../specification/openapi.json', import.meta.url), 'utf8')) as Specification;
export const schemas = specification.components.schemas;

export function sample(schema: Schema, full = false, variant = 0, depth = 0): unknown {
  if (schema.$ref) {
    const target = schemas[schema.$ref.split('/').at(-1) ?? ''];
    if (!target) throw new Error(`Unknown schema ${schema.$ref}`);
    return sample(target, full, variant, depth + 1);
  }
  if (schema.oneOf) {
    const alternative = schema.oneOf[variant % schema.oneOf.length];
    if (!alternative) throw new Error('Empty union');
    return sample(alternative, full, variant, depth);
  }
  if (schema.enum) return schema.enum[variant % schema.enum.length];
  let kind = schema.type;
  if (Array.isArray(kind)) {
    if (!full || variant % 2) return null;
    kind = kind.find(value => value !== 'null');
  }
  switch (kind) {
    case 'object':
      if (schema.additionalProperties) return {key: sample(schema.additionalProperties, full, variant, depth + 1)};
      return Object.fromEntries(Object.entries(schema.properties ?? {})
        .filter(([key]) => full || schema.required?.includes(key))
        .map(([key, value]) => [key, sample(value, full, variant, depth + 1)]));
    case 'array':
      return full && depth < 8 && schema.items ? [sample(schema.items, full, variant, depth + 1)] : [];
    case 'null': return null;
    case 'string': return 'value';
    case 'boolean': return true;
    case 'integer': return schema.minimum ?? 0;
    case 'number': return schema.minimum ?? 0.5;
    default: throw new Error(`Unsupported fixture type ${String(kind)}`);
  }
}
