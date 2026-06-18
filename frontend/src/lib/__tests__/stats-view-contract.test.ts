import { readFileSync } from 'node:fs';
import { describe, expect, it } from 'vitest';
import {
  NET_EVENTS_ALL_SQL,
  NET_EVENTS_SEARCH_SQL,
  PRESET_QUERIES,
  TRACE_DETAIL_SQL,
} from '../sql';

const source = readFileSync(
  new URL('../components/views/StatsView.svelte', import.meta.url),
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
    expect(source).toContain("id: 'mcp'");
  });
});

describe('StatsView credential broker contract', () => {
  it('surfaces broker evidence as a first-class tab instead of process activity', () => {
    expect(source).toContain("'credentials'");
    expect(source).toContain("label: 'Credentials'");
    expect(source).toContain('Credential Broker Events');
    expect(source).toContain("type: 'credential broker event'");
    expect(source).toContain('substitution_events');
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

    expect(source).toContain("'substitution_ref'");
    expect(source).toContain("'credential_ref'");
  });

  it('keeps credential reference counts out of protocol tabs', () => {
    const mcpStart = source.indexOf("{:else if activeTab === 'mcp'}");
    const httpStart = source.indexOf("{:else if activeTab === 'http'}");
    expect(mcpStart).toBeGreaterThan(-1);
    expect(httpStart).toBeGreaterThan(mcpStart);

    const mcpBlock = source.slice(mcpStart, httpStart);
    expect(mcpBlock).toContain('Tool Calls');
    expect(mcpBlock).not.toContain('Credential Refs');
    expect(mcpBlock).not.toContain('credential_ref).length');
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
    expect(source).toContain('Inspect session ledger');
    expect(source).toContain('Session {vmId} ledger');
    expect(source).not.toContain('Inspect session database');
    expect(source).not.toContain('Session {vmId} database');
  });

  it('does not render the selected event twice as raw JSON plus repeated fields', () => {
    expect(source).not.toContain("formatAndHighlight(detail.data, 'json')");
    expect(source).toContain('visibleDetailEntries(detail.data)');
    expect(source).toContain('detailPayloadSections(detail.data)');
  });

  it('uses payload-aware syntax highlighting instead of forcing every payload through JSON', () => {
    expect(source).toContain('detailPayloadLang(key, value)');
    expect(source).toContain("ensureShikiLang('http')");
    expect(source).toContain("if (key.endsWith('_headers')) return 'http';");
    expect(source).not.toContain("lang: 'json',");
  });

  it('loads body payloads from event_body_blobs instead of preview columns', () => {
    expect(source).toContain('FROM event_body_blobs');
    expect(source).toContain("'request_body'");
    expect(source).toContain("'response_body'");
    expect(source).toContain("void showDetail('model', row)");
    expect(source).toContain("void showDetail('mcp', row)");
    expect(source).toContain("void showDetail('http', row)");
    expect(source).not.toContain('request_body_preview');
    expect(source).not.toContain('response_body_preview');
    expect(source).not.toContain('request_preview');
    expect(source).not.toContain('response_preview');
    expect(source).not.toContain('text_content');
  });
});

describe('Stats SQL contract', () => {
  it('keeps legacy preview columns out of frontend stats and inspector presets', () => {
    const queries = [
      TRACE_DETAIL_SQL,
      NET_EVENTS_ALL_SQL,
      NET_EVENTS_SEARCH_SQL,
      ...PRESET_QUERIES.map((preset) => preset.sql),
    ].join('\n');

    expect(queries).not.toContain('request_body_preview');
    expect(queries).not.toContain('response_body_preview');
    expect(queries).not.toContain('system_prompt_preview');
  });

  it('uses credential broker vocabulary in presets without exposing refs', () => {
    const credentialPreset = PRESET_QUERIES.find((preset) => preset.label === 'Credential broker events');
    expect(credentialPreset).toBeDefined();
    expect(credentialPreset?.sql).toContain('outcome AS verb');
    expect(credentialPreset?.sql).toContain('event_type AS origin');
    expect(credentialPreset?.sql).not.toContain('substitution_ref');
    expect(credentialPreset?.sql).not.toContain('credential_ref');
    expect(PRESET_QUERIES.some((preset) => preset.label === 'Credential substitutions')).toBe(false);
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
