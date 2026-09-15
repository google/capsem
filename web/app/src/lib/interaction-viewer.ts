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
