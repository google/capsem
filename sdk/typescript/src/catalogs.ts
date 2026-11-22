import * as api from './operations/index.js';
import type * as models from './models/index.js';
import type {CallOptions, Transport} from './transport.js';

export interface CatalogListOptions extends CallOptions {
  search?: string;
  after?: string;
  limit?: number;
}

/** Published skill versions from the currently configured catalog. */
export class Skills {
  constructor(private readonly transport: Transport) {}

  async list({signal, timeoutMs, ...query}: CatalogListOptions = {}): Promise<models.SkillCatalogListResponse> {
    return api.listCatalogSkills(this.transport, query, {signal, timeoutMs});
  }

  /** Read the complete files for an exact version and listed snapshot. */
  async get(
    skillId: string, version: string, catalog: models.CatalogSnapshot, options: CallOptions = {},
  ): Promise<models.SkillCatalogDetailResponse> {
    const result = await api.getCatalogSkill(this.transport, {skill_id: skillId, version, snapshot: catalog.snapshot}, options);
    verifyEntry(skillId, version, catalog, result.selection, result.metadata);
    return result;
  }
}

/** Published access and detection rules from the configured catalog. */
export class Rules {
  constructor(private readonly transport: Transport) {}

  async list({signal, timeoutMs, ...query}: CatalogListOptions = {}): Promise<models.RuleCatalogListResponse> {
    return api.listCatalogRules(this.transport, query, {signal, timeoutMs});
  }

  /** Read the typed definition and parameters of a listed rule version. */
  async get(
    ruleId: string, version: string, catalog: models.CatalogSnapshot, options: CallOptions = {},
  ): Promise<models.RuleCatalogDetailResponse> {
    const result = await api.getCatalogRule(this.transport, {rule_id: ruleId, version, snapshot: catalog.snapshot}, options);
    verifyEntry(ruleId, version, catalog, result.selection, result.metadata);
    return result;
  }
}

/** Published MCP templates from the configured catalog. */
export class Mcps {
  constructor(private readonly transport: Transport) {}

  async list({signal, timeoutMs, ...query}: CatalogListOptions = {}): Promise<models.McpCatalogListResponse> {
    return api.listCatalogMcps(this.transport, query, {signal, timeoutMs});
  }

  /** Read published configuration requirements and tool metadata for one version. */
  async get(
    mcpId: string, version: string, catalog: models.CatalogSnapshot, options: CallOptions = {},
  ): Promise<models.McpCatalogDetailResponse> {
    const result = await api.getCatalogMcp(this.transport, {mcp_id: mcpId, version, snapshot: catalog.snapshot}, options);
    verifyEntry(mcpId, version, catalog, result.selection, result.metadata);
    return result;
  }
}

function verifyEntry(
  id: string, version: string, catalog: models.CatalogSnapshot,
  selection: models.EnvironmentCatalogPin, metadata: models.CatalogEntryMetadata,
): void {
  if (selection.id !== id || selection.version !== version
    || metadata.id !== id || metadata.version !== version
    || selection.catalog_reference !== catalog.reference
    || selection.resolved_digest !== catalog.digest) {
    throw new Error('Catalog entry identity changed');
  }
}
