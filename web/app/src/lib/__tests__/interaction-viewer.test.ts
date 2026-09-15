import { describe, expect, it } from 'vitest';
import {
  BodyDirection,
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
    event_id: 'cccccccccccc', direction: BodyDirection.REQUEST,
    body_hash: 'sha256:body', original_bytes: 24, stored_bytes: 24,
    content_type: 'application/json',
    payload: {
      status: CaptureStatus.COMPLETE,
      content: { kind: JsonContentKind.JSON, value: { method: 'tools/call' } },
    },
  }],
};

describe('interaction viewer model', () => {
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
