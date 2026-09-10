import {expect, it} from 'vitest';
import {CapturedContentSchema, CapturedPayloadSchema, InteractionContentSchema} from '../src/validation/index.js';
import {CaptureStatus, InteractionToolCallKind, JsonContentKind, RawContentKind} from '../src/models/index.js';

it.each([null, true, 4, 1.5, 'text', [], {nested: [null, {x: true}]}])('preserves native JSON %j', value => {
  const wire = {status: 'unknown', content: {kind: 'json', value}};
  const payload = CapturedPayloadSchema.parse(wire);
  expect(payload).toEqual(wire);
  expect(payload.status).toBe(CaptureStatus.UNKNOWN);
  if (payload.content.kind !== JsonContentKind.JSON) throw new Error('Expected JSON');
  expect(payload.content.value).toEqual(value);
});

it.each([
  {kind: 'json'}, {kind: 'invented', value: 1}, {kind: 'json', value: undefined},
  {kind: 'json', value: Symbol('invalid')}, {kind: 'json', value: {nested: () => 1}},
  {kind: 'text', text: 'x', value: 2}, {kind: 'raw', raw: '{', reason: 'invented'},
])('rejects invalid content %j', content => {
  expect(CapturedContentSchema.safeParse(content).success).toBe(false);
});

it('narrows tool/result types by enum discriminator and retains truncated raw evidence', () => {
  const wire = {kind: 'tool_call', call_id: 'call-1', tool_name: 'search', server_name: 'knowledge',
    origin: 'mcp', decision: 'allowed', arguments: {status: 'unknown', content: {kind: 'json', value: {q: 'SDK'}}},
    result: {kind: 'tool_result', call_id: 'call-1', is_error: null, error_message: null,
      payload: {status: 'truncated', content: {kind: 'raw', raw: '{"content":[', reason: 'truncated'}}}};
  const content = InteractionContentSchema.parse(wire);
  if (content.kind !== InteractionToolCallKind.TOOL_CALL) throw new Error('Expected tool call');
  expect(content.tool_name).toBe('search');
  const payload = content.result?.payload;
  if (payload?.content.kind !== RawContentKind.RAW) throw new Error('Expected raw result');
  expect(payload.content.raw).toBe('{"content":[');
  expect(payload.status).toBe(CaptureStatus.TRUNCATED);
  expect(content.result?.is_error).toBe(null);
});
