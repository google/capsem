import { describe, expect, it } from 'vitest';
import demo from './fixtures/demo-071-interactions.json';
import claudeDemo from './fixtures/demo-071-claude-interactions.json';
import {
  CaptureStatus,
  InteractionBlockKind,
  InteractionMessageKind,
  InteractionRequestKind,
  InteractionRole,
  InteractionToolCallKind,
  InteractionToolResultKind,
  JsonContentKind,
  RawContentKind,
  RawContentReason,
  TextContentKind,
  ToolDecision,
  ToolOrigin,
  type InteractionReport,
} from '@capsem/sdk';
import {
  interactionBodies,
  interactionLabel,
  interactionItems,
  payloadDisplay,
  displayInteractions,
} from '../interaction-viewer';

const report: InteractionReport = {
  items: [
    {
      event_id: 'aaaaaaaaaaaa', timestamp: '2026-09-15T00:00:00Z',
      content: {
        kind: InteractionRequestKind.REQUEST_PREVIEW,
        payload: {
          status: CaptureStatus.UNKNOWN,
          content: { kind: TextContentKind.TEXT, text: 'user request' },
        },
      },
    },
    {
      event_id: 'bbbbbbbbbbbb', timestamp: '2026-09-15T00:00:01Z',
      content: {
        kind: InteractionMessageKind.MESSAGE,
        role: InteractionRole.ASSISTANT,
        blocks: [{
          kind: InteractionBlockKind.REASONING,
          payload: {
            status: CaptureStatus.COMPLETE,
            content: { kind: TextContentKind.TEXT, text: 'inspect first' },
          },
        }],
      },
    },
    {
      event_id: 'cccccccccccc', timestamp: '2026-09-15T00:00:02Z',
      content: {
        kind: InteractionToolCallKind.TOOL_CALL,
        call_id: 'call-1', tool_name: 'search', server_name: 'catalog',
        origin: ToolOrigin.MCP, decision: ToolDecision.ALLOWED,
        arguments: {
          status: CaptureStatus.COMPLETE,
          content: { kind: JsonContentKind.JSON, value: { query: 'typed SDK' } },
        },
      },
    },
    {
      event_id: 'dddddddddddd', timestamp: '2026-09-15T00:00:03Z',
      content: {
        kind: InteractionToolResultKind.TOOL_RESULT,
        call_id: 'call-1', is_error: false,
        payload: {
          status: CaptureStatus.TRUNCATED,
          content: {
            kind: RawContentKind.RAW,
            raw: '{"partial":',
            reason: RawContentReason.TRUNCATED,
          },
        },
      },
    },
  ],
  bodies: [{
    event_id: 'cccccccccccc', direction: 'request',
    body_hash: 'sha256:body', original_bytes: 24, stored_bytes: 24,
    content_type: 'application/json', truncated: false,
  }],
};

describe('interaction viewer model', () => {
  it('retains unanswered Claude requests and never invents a tool result', () => {
    const items = displayInteractions(claudeDemo as InteractionReport, 'model');
    expect(items.filter(item => item.content.kind === 'request_preview').map(item => item.model_call_id)).toEqual([2, 7]);
    const tool = items.find(item => item.content.kind === 'tool_call');
    expect(tool?.content.kind === 'tool_call' && tool.content.result).toBeUndefined();
    expect(items.filter(item => item.content.kind === 'message')).toHaveLength(7);
  });
  it('groups the sanitized AGY demo continuation and hides duplicate raw request cards', () => {
    const source = structuredClone(demo) as InteractionReport;
    const items = displayInteractions(source, 'model');
    expect(items.filter(item => item.content.kind === 'request_preview')).toHaveLength(0);
    expect(items.filter(item => item.content.kind === 'tool_result')).toHaveLength(0);
    const call = items.find(item => item.content.kind === 'tool_call');
    expect(call?.content.kind === 'tool_call' && call.content.result?.call_id).toBe('fixture-write');
    const message = items.find(item => item.model_call_id === 3 && item.content.kind === 'message');
    expect(message?.content.kind === 'message' && message.content.blocks.map(block => block.kind)).toEqual(['reasoning', 'text']);
    expect(source).toEqual(demo);
  });

  it('never pairs reused tool identifiers across traces or hides an unmatched result', () => {
    const source = structuredClone(report);
    source.items[2].trace_id = 'one'; source.items[3].trace_id = 'two';
    expect(displayInteractions(source, 'tools')).toHaveLength(2);
    source.items[3].trace_id = 'one';
    expect(displayInteractions(source, 'tools')).toHaveLength(1);
  });

  it('keeps ambiguous call identities separate instead of inventing a pairing', () => {
    const source = structuredClone(report);
    source.items[2].trace_id = source.items[3].trace_id = 'one';
    source.items.push({...source.items[2], event_id: 'another-call'});
    expect(displayInteractions(source, 'tools')).toHaveLength(3);
  });
  it('filters the shared ordered interaction objects without rebuilding rows', () => {
    expect(interactionItems(report, 'all')).toEqual(report.items);
    expect(interactionItems(report, 'model').map(item => item.event_id)).toEqual([
      'aaaaaaaaaaaa', 'bbbbbbbbbbbb', 'cccccccccccc', 'dddddddddddd',
    ]);
    expect(interactionItems(report, 'tools').map(item => item.event_id)).toEqual([
      'cccccccccccc', 'dddddddddddd',
    ]);
  });

  it('renders typed JSON, text, and raw captures without guessing their shape', () => {
    expect(payloadDisplay(report.items[0].content.payload)).toEqual({
      text: 'user request', language: 'text', status: CaptureStatus.UNKNOWN,
    });
    const tool = report.items[2].content;
    if (tool.kind !== InteractionToolCallKind.TOOL_CALL) throw new Error('fixture');
    expect(payloadDisplay(tool.arguments)).toEqual({
      text: '{\n  "query": "typed SDK"\n}', language: 'json', status: CaptureStatus.COMPLETE,
    });
    const result = report.items[3].content;
    if (result.kind !== InteractionToolResultKind.TOOL_RESULT) throw new Error('fixture');
    expect(payloadDisplay(result.payload)).toEqual({
      text: '{"partial":', language: 'text', status: CaptureStatus.TRUNCATED,
      reason: RawContentReason.TRUNCATED,
    });
  });

  it('uses discriminated variants for labels and exact body ownership', () => {
    expect(report.items.map(interactionLabel)).toEqual([
      'Request preview', 'Assistant message', 'Tool call', 'Tool result',
    ]);
    expect(interactionBodies(report, 'cccccccccccc')).toEqual(report.bodies);
    expect(interactionBodies(report, 'bbbbbbbbbbbb')).toEqual([]);
  });
});
