import { readFileSync } from 'node:fs';
import { describe, expect, it } from 'vitest';
import {
  NET_EVENTS_ALL_SQL,
  NET_EVENTS_SEARCH_SQL,
  TRACE_DETAIL_SQL,
} from '../sql';

const source = readFileSync(
  new URL('../components/views/StatsView.svelte', import.meta.url),
  'utf8',
);
const detailSource = readFileSync(
  new URL('../stats-detail.ts', import.meta.url),
  'utf8',
);

describe('StatsView process contract', () => {
  it('distinguishes command executions from process observations', () => {
    expect(source).toContain('Process Exec Events');
    expect(source).toContain('Observed Processes');
    expect(source).toContain('Unique Binaries');
    expect(source).toContain('auditCommand(row)');
    expect(source).toContain("type: 'observed process'");
    expect(source).not.toContain('Process Audit Events');
    expect(source).not.toContain("type: 'process audit'");
  });

  it('does not show process credential-ref counters or tutorial prose', () => {
    const processStart = source.indexOf("{:else if activeTab === 'process'}");
    const credentialsStart = source.indexOf("{:else if activeTab === 'credentials'}");
    expect(processStart).toBeGreaterThan(-1);
    expect(credentialsStart).toBeGreaterThan(processStart);

    const processBlock = source.slice(processStart, credentialsStart);
    expect(processBlock).not.toContain('Credential Refs');
    expect(processBlock).not.toContain('audit-port process records');
    expect(processBlock).not.toContain('command executions are listed separately');
  });
});

describe('StatsView snapshot boundary', () => {
  it('does not expose hypervisor snapshots as a generic stats tab', () => {
    expect(source).not.toContain("id: 'snapshots'");
    expect(source).not.toContain('snapshot_events');
    expect(source).not.toContain('Snapshot Events');
    expect(source).toContain("id: 'tools'");
  });
});

describe('StatsView tool-call contract', () => {
  it('exposes one user-facing tool-call ledger instead of an MCP activity panel', () => {
    expect(source).toContain("type StatsTab = 'model' | 'tools'");
    expect(source).toContain("label: 'Tools'");
    expect(source).toContain('toolRows = detailRows.tool_events');
    expect(source).toContain('Tool Calls');
    expect(source).toContain('Model Origin');
    expect(source).toContain('MCP Origin');
    expect(source).toContain("void showDetail('tool', row)");
    expect(source).not.toContain("label: 'MCP'");
    expect(source).not.toContain("activeTab === 'mcp'");
    expect(source).not.toContain('MCP Events');
  });
});

describe('StatsView credential broker contract', () => {
  it('surfaces broker evidence as a first-class tab instead of process activity', () => {
    expect(source).toContain("'credentials'");
    expect(source).toContain("label: 'Credentials'");
    expect(source).toContain('Credential Broker Events');
    expect(source).toContain("type: 'credential broker event'");
    expect(source).toContain('substitutionRows = detailRows.credential_events');
    expect(source).toContain('Captured');
    expect(source).toContain('Brokered');
    expect(source).toContain('Injected');
    expect(source).not.toContain('Credential Substitutions');

    const processStart = source.indexOf("{:else if activeTab === 'process'}");
    const credentialsStart = source.indexOf("{:else if activeTab === 'credentials'}");
    const securityStart = source.indexOf("{:else if activeTab === 'security'}");
    expect(processStart).toBeGreaterThan(-1);
    expect(credentialsStart).toBeGreaterThan(processStart);
    expect(securityStart).toBeGreaterThan(credentialsStart);

    const processBlock = source.slice(processStart, credentialsStart);
    expect(processBlock).not.toContain('substitutionRows');
    expect(processBlock).not.toContain('Credential Broker Events');
  });

  it('shows credential broker verbs instead of reference hashes or status columns', () => {
    const credentialsStart = source.indexOf("{:else if activeTab === 'credentials'}");
    const securityStart = source.indexOf("{:else if activeTab === 'security'}");
    expect(credentialsStart).toBeGreaterThan(-1);
    expect(securityStart).toBeGreaterThan(credentialsStart);

    const credentialsBlock = source.slice(credentialsStart, securityStart);
    expect(credentialsBlock).toContain('brokerVerb(row)');
    expect(source).toContain('text(row.verb).toLowerCase()');
    expect(credentialsBlock).toContain("columns={['Time', 'Verb', 'Source', 'Provider', 'Origin']}");
    expect(credentialsBlock).toContain('Captured');
    expect(credentialsBlock).toContain('Brokered');
    expect(credentialsBlock).toContain('Injected');
    expect(credentialsBlock).not.toContain('Substituted');
    expect(credentialsBlock).not.toContain('References');
    expect(credentialsBlock).not.toContain('Outcome');
    expect(credentialsBlock).not.toContain('substitution_ref');
    expect(credentialsBlock).not.toContain('confidence');
    expect(credentialsBlock).not.toContain('algorithm');

    expect(detailSource).toContain("'substitution_ref'");
    expect(detailSource).toContain("'credential_ref'");
  });

  it('keeps credential reference counts out of protocol tabs', () => {
    const toolsStart = source.indexOf("{:else if activeTab === 'tools'}");
    const httpStart = source.indexOf("{:else if activeTab === 'http'}");
    expect(toolsStart).toBeGreaterThan(-1);
    expect(httpStart).toBeGreaterThan(toolsStart);

    const toolsBlock = source.slice(toolsStart, httpStart);
    expect(toolsBlock).toContain('Tool Calls');
    expect(toolsBlock).not.toContain('Credential Refs');
    expect(toolsBlock).not.toContain('credential_ref).length');
  });

  it('counts captured, brokered, and injected credential verbs independently', () => {
    expect(source).toContain("brokerVerb(row) === 'captured'");
    expect(source).toContain("brokerVerb(row) === 'brokered'");
    expect(source).toContain("brokerVerb(row) === 'injected'");
    expect(source).toContain("brokerVerb(row) === 'error'");
    expect(source).toContain('brokerErrorCount');
    expect(source).toContain('Errors');
    expect(source).not.toContain('const brokerCapturedCount = $derived(substitutionRows.length)');
  });
});

describe('StatsView detail drawer contract', () => {
  it('uses ledger wording instead of exposing database implementation text', () => {
    expect(source).toContain('Session {vmId} ledger');
    expect(source).not.toContain('Inspect session ledger');
    expect(source).not.toContain('Inspect session database');
    expect(source).not.toContain('Session {vmId} database');
  });

  it('does not render the selected event twice as raw JSON plus repeated fields', () => {
    expect(source).not.toContain("formatAndHighlight(detail.data, 'json')");
    expect(source).toContain('visibleDetailEntries(detail.data)');
    expect(source).toContain('detailPayloadSections(detail.data)');
  });

  it('uses payload-aware syntax highlighting instead of forcing every payload through JSON', () => {
    expect(detailSource).toContain('detailPayloadLang(key, value)');
    expect(source).toContain("ensureShikiLang('http')");
    expect(detailSource).toContain("if (key.endsWith('_headers')) return 'http';");
    expect(source).not.toContain("lang: 'json',");
  });

  it('loads body payloads from event_body_blobs instead of preview columns', () => {
    expect(source).toContain('api.getVmStatsDetail(vmId)');
    expect(source).toContain('bodyBlobs = detailRows.body_blobs');
    expect(detailSource).toContain("'request_body'");
    expect(detailSource).toContain("'response_body'");
    expect(source).toContain('`${direction}_body`');
    expect(source).toContain("void showDetail('model', row)");
    expect(source).toContain("void showDetail('tool', row)");
    expect(source).toContain("void showDetail('http', row)");
    expect(source).not.toContain('request_body_preview');
    expect(source).not.toContain('response_body_preview');
    expect(source).not.toContain('request_preview');
    expect(source).not.toContain('response_preview');
    expect(source).not.toContain('text_content');
  });

  it('keeps body ledger metadata out of the generic field grid', () => {
    expect(detailSource).toContain('DETAIL_BODY_METADATA_KEYS');
    expect(source).toContain('payloadSectionMeta(section, detail.data)');
    expect(source).toContain('Original');
    expect(source).toContain('Stored');
    expect(source).toContain('Truncated');
    expect(source).toContain('Hash');
    expect(detailSource).toContain('&& !DETAIL_BODY_METADATA_KEYS.has(key)');
  });

  it('renders compact structured snapshots instead of null-heavy security projections', () => {
    expect(source).toContain('compactJsonForDisplay(detail.data.rule_json)');
    expect(source).toContain('compactJsonForDisplay(detail.data.event_json)');
    expect(detailSource).toContain('stripEmptyDetailValues');
    expect(source).not.toContain("formatAndHighlight(detail.data.event_json, 'json')");
    expect(source).not.toContain("formatAndHighlight(detail.data.rule_json, 'json')");
  });

  it('gives detail fields enough room to wrap without overlapping values', () => {
    expect(source).toContain('w-[560px]');
    expect(source).toContain('minmax(0,1fr)');
    expect(source).toContain('overflow-wrap:anywhere');
    expect(source).toContain('Event Fields');
  });
});

describe('Stats SQL contract', () => {
  it('keeps legacy preview columns out of frontend stats', () => {
    const queries = [
      TRACE_DETAIL_SQL,
      NET_EVENTS_ALL_SQL,
      NET_EVENTS_SEARCH_SQL,
    ].join('\n');

    expect(queries).not.toContain('request_body_preview');
    expect(queries).not.toContain('response_body_preview');
    expect(queries).not.toContain('system_prompt_preview');
  });

  it('does not expose raw SQL inspection as a frontend session surface', () => {
    expect(source).not.toContain('InspectorView');
    expect(source).not.toContain('inspectQuery');
    expect(source).not.toContain('/inspect');
    expect(source).not.toContain('PRESET_QUERIES');
  });
});

describe('StatsView file summary contract', () => {
  it('summarizes file actions visible in the event table', () => {
    const filesStart = source.indexOf("{:else if activeTab === 'files'}");
    const processStart = source.indexOf("{:else if activeTab === 'process'}");
    expect(filesStart).toBeGreaterThan(-1);
    expect(processStart).toBeGreaterThan(filesStart);

    const filesBlock = source.slice(filesStart, processStart);
    expect(filesBlock).toContain('Created');
    expect(filesBlock).toContain('Modified');
    expect(filesBlock).toContain('Deleted');
    expect(filesBlock).not.toContain('Imports');
    expect(filesBlock).not.toContain('Exports');
    expect(filesBlock).not.toContain('Brokered Refs');
  });
});

describe('StatsView security summary contract', () => {
  it('shows complete action and detection summaries instead of a partial block/rules-hit headline', () => {
    const securityStart = source.indexOf("{:else if activeTab === 'security'}");
    expect(securityStart).toBeGreaterThan(-1);

    const securityBlock = source.slice(securityStart);
    expect(source).toContain('securityActionRows');
    expect(source).toContain('securityDetectionRows');
    expect(source).toContain("['allow', 'ask', 'block', 'preprocess', 'rewrite', 'postprocess']");
    expect(source).toContain("['none', 'informational', 'low', 'medium', 'high', 'critical']");
    expect(securityBlock).toContain('By Detection Level');
    expect(securityBlock).toContain('securityActionRows');
    expect(securityBlock).toContain('securityDetectionRows');
    expect(securityBlock).not.toContain('Rules Hit');
    expect(securityBlock).not.toContain('Blocks');
  });
});
