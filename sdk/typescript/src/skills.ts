import * as api from './operations/index.js';
import type * as models from './models/index.js';
import type {CallOptions, Transport} from './transport.js';

export interface SkillCatalogListOptions extends CallOptions {
  search?: string;
  after?: string;
  limit?: number;
}

/** Published skill versions from the currently configured catalog. */
export class Skills {
  constructor(private readonly transport: Transport) {}

  async list({signal, timeoutMs, ...query}: SkillCatalogListOptions = {}): Promise<models.SkillCatalogListResponse> {
    return api.listCatalogSkills(this.transport, query, {signal, timeoutMs});
  }

  /** Read the complete files for an exact version and listed snapshot. */
  async get(
    skillId: string, version: string, catalog: models.SkillCatalogSnapshot, options: CallOptions = {},
  ): Promise<models.SkillCatalogDetailResponse> {
    const result = await api.getCatalogSkill(this.transport, {skill_id: skillId, version, snapshot: catalog.snapshot}, options);
    if (result.selection.id !== skillId || result.selection.version !== version
      || result.metadata.id !== skillId || result.metadata.version !== version
      || result.selection.catalog_reference !== catalog.reference
      || result.selection.resolved_digest !== catalog.digest) {
      throw new Error('Catalog skill identity changed');
    }
    return result;
  }
}
