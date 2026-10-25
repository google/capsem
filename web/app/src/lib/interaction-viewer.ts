import {
  InteractionMessageKind,
  InteractionRequestKind,
  InteractionToolCallKind,
  InteractionToolResultKind,
  JsonContentKind,
  RawContentKind,
  type CapturedPayload,
  type Interaction,
  type InteractionBody,
  type InteractionReport,
  type RawContentReason,
} from '@capsem/sdk';

export type InteractionScope = 'all' | 'model' | 'tools';

export type PayloadDisplay = {
  text: string;
  language: 'json' | 'text';
  status: CapturedPayload['status'];
  reason?: RawContentReason;
};

export function interactionItems(
  report: InteractionReport,
  scope: InteractionScope,
): Interaction[] {
  if (scope !== 'tools') return report.items;
  return report.items.filter(({ content }) => (
    content.kind === InteractionToolCallKind.TOOL_CALL
    || content.kind === InteractionToolResultKind.TOOL_RESULT
  ));
}

function exchangeKey(item: Interaction): string {
  return item.model_event_id ?? (item.model_call_id != null
    ? `${item.trace_id ?? ''}:${item.model_call_id}` : item.event_id);
}

/** Presentation joins use recorded identities; the original report stays intact. */
export function displayInteractions(report: InteractionReport, scope: InteractionScope): Interaction[] {
  const calls = new Map<string, Interaction[]>();
  const results = new Map<string, Interaction[]>();
  const toolKey = (item: Interaction) => item.trace_id && 'call_id' in item.content
    ? `${item.trace_id}:${item.content.call_id}` : undefined;
  for (const item of report.items) {
    const key = toolKey(item);
    if (!key) continue;
    const map = item.content.kind === InteractionToolCallKind.TOOL_CALL ? calls
      : item.content.kind === InteractionToolResultKind.TOOL_RESULT ? results : null;
    if (map) map.set(key, [...(map.get(key) ?? []), item]);
  }
  const consumed = new Set<Interaction>();
  const paired = new Map<Interaction, Interaction>();
  for (const [key, candidates] of calls) {
    const matches = results.get(key) ?? [];
    if (candidates.length !== 1 || matches.length !== 1) continue;
    paired.set(candidates[0], matches[0]);
    consumed.add(matches[0]);
  }
  const exchanges = new Set(report.items.filter(item => item.content.kind !== InteractionRequestKind.REQUEST_PREVIEW)
    .map(exchangeKey));
  const messages = new Map<string, Interaction>();
  const visible: Interaction[] = [];
  for (const original of interactionItems(report, scope)) {
    if (consumed.has(original)) continue;
    if (original.content.kind === InteractionRequestKind.REQUEST_PREVIEW && exchanges.has(exchangeKey(original))) continue;
    const result = paired.get(original);
    let item = original;
    if (original.content.kind === InteractionToolCallKind.TOOL_CALL && result?.content.kind === InteractionToolResultKind.TOOL_RESULT) {
      item = {...original, content: {...original.content, result: {
        ...original.content.result,
        ...result.content,
        response: result.content.response ?? original.content.result?.response,
      }}};
    }
    if (item.content.kind === InteractionMessageKind.MESSAGE) {
      const key = exchangeKey(item);
      const prior = messages.get(key);
      if (prior?.content.kind === InteractionMessageKind.MESSAGE) {
        prior.content.blocks.push(...item.content.blocks);
        continue;
      }
      item = {...item, content: {...item.content, blocks: [...item.content.blocks]}};
      messages.set(key, item);
    }
    visible.push(item);
  }
  return visible;
}

export function interactionBodies(
  report: InteractionReport,
  eventId: string,
): InteractionBody[] {
  return report.bodies.filter(body => body.event_id === eventId);
}

export function payloadDisplay(
  payload: CapturedPayload | null | undefined,
): PayloadDisplay | null {
  if (!payload) return null;
  const { content } = payload;
  if (content.kind === JsonContentKind.JSON) {
    return {
      text: JSON.stringify(content.value, null, 2) ?? 'null',
      language: 'json',
      status: payload.status,
    };
  }
  if (content.kind === RawContentKind.RAW) {
    return {
      text: content.raw,
      language: 'text',
      status: payload.status,
      reason: content.reason,
    };
  }
  return { text: content.text, language: 'text', status: payload.status };
}

export function interactionLabel(item: Interaction): string {
  switch (item.content.kind) {
    case InteractionRequestKind.REQUEST_PREVIEW:
      return 'Request preview';
    case InteractionMessageKind.MESSAGE:
      return 'Assistant message';
    case InteractionToolCallKind.TOOL_CALL:
      return 'Tool call';
    case InteractionToolResultKind.TOOL_RESULT:
      return 'Tool result';
  }
}
