/** Official TypeScript OpenAI client probe for the standalone Capsem endpoint. */

import OpenAI, {APIError, APIUserAbortError} from 'openai';
import type {ChatCompletionTool} from 'openai/resources/chat/completions';
import type {Tool} from 'openai/resources/responses/responses';

const baseURL = process.argv[2];
if (!baseURL) throw new Error('usage: standalone-openai-client.mjs BASE_URL');

const client = new OpenAI({
  apiKey: 'sk-capsem-standalone-typescript',
  baseURL,
  maxRetries: 0,
  timeout: 5_000,
});

const tools: ChatCompletionTool[] = [{
  type: 'function',
  function: {
    name: 'fixture_lookup',
    description: 'Read the deterministic fixture.',
    parameters: {
      type: 'object',
      properties: {query: {type: 'string'}},
      required: ['query'],
    },
  },
}];
const chat = await client.chat.completions.create({
  model: 'gpt-fixture',
  messages: [{role: 'user', content: 'Use the fixture.'}],
  tools,
});
const toolCall = chat.choices[0]?.message.tool_calls?.[0];
if (!toolCall || toolCall.type !== 'function') throw new Error('chat function call missing');
if (toolCall.function.name !== 'fixture_lookup') throw new Error('chat tool name changed');
const chatArguments: unknown = JSON.parse(toolCall.function.arguments);
if (
  typeof chatArguments !== 'object'
  || chatArguments === null
  || !('query' in chatArguments)
  || chatArguments.query !== 'Capsem ironbank poem'
) {
  throw new Error('chat tool arguments changed');
}
if (chat.usage?.total_tokens !== 456) throw new Error('chat usage changed');

const chatStream = await client.chat.completions.create({
  model: 'gpt-fixture',
  messages: [{role: 'user', content: 'Stream the fixture.'}],
  stream: true,
});
let streamedChat = '';
for await (const chunk of chatStream) streamedChat += chunk.choices[0]?.delta.content ?? '';
if (streamedChat !== 'Capsem ironbank poem') throw new Error('chat stream changed');

const responseTools: Tool[] = [{
  type: 'function',
  name: 'exec_command',
  description: 'Run the deterministic fixture command.',
  parameters: {
    type: 'object',
    properties: {cmd: {type: 'string'}},
    required: ['cmd'],
  },
  strict: false,
}];
const first = await client.responses.create({
  model: 'gpt-fixture',
  input: 'Call the fixture tool.',
  tools: responseTools,
});
const functionCall = first.output.find(item => item.type === 'function_call');
if (!functionCall || functionCall.name !== 'exec_command') throw new Error('response tool changed');
if (first.usage?.total_tokens !== 48) throw new Error('response usage changed');

const responseStream = await client.responses.create({
  model: 'gpt-fixture',
  input: [{
    type: 'function_call_output',
    call_id: functionCall.call_id,
    output: 'Process exited with code 0',
  }],
  tools: responseTools,
  stream: true,
});
let responseText = '';
let responseUsage;
for await (const event of responseStream) {
  if (event.type === 'response.output_text.delta') responseText += event.delta;
  if (event.type === 'response.completed') responseUsage = event.response.usage;
}
if (responseText !== 'Capsem ironbank poem\nledgers count the sparks\nno secret crosses raw') {
  throw new Error('response stream changed');
}
if (responseUsage?.total_tokens !== 12) throw new Error('response stream usage changed');

const controller = new AbortController();
const pending = client.responses.create(
  {model: 'capsem-sdk-cancel', input: 'wait'},
  {signal: controller.signal},
);
setTimeout(() => controller.abort(), 200);
let cancelled = false;
try {
  await pending;
} catch (error) {
  cancelled = error instanceof APIUserAbortError || (error instanceof Error && error.name === 'AbortError');
}
if (!cancelled) throw new Error('request cancellation was not observed');

const afterCancel = await client.chat.completions.create({
  model: 'gpt-fixture',
  messages: [{role: 'user', content: 'Still alive?'}],
});
if (afterCancel.choices[0]?.message.content !== 'Capsem ironbank poem\nledgers count the sparks\nno secret crosses raw') {
  throw new Error('proxy did not survive client cancellation');
}

let upstreamStatus: number | undefined;
let upstreamCode: string | null | undefined;
try {
  await client.responses.create({model: 'capsem-sdk-upstream-error', input: 'fail'});
} catch (error) {
  if (error instanceof APIError) {
    const status: unknown = error.status;
    const code: unknown = error.code;
    if (typeof status === 'number') upstreamStatus = status;
    if (typeof code === 'string' || code === null || code === undefined) upstreamCode = code;
  }
}
if (upstreamStatus !== 503 || upstreamCode !== 'fixture_unavailable') {
  throw new Error(`upstream failure changed: ${upstreamStatus}/${upstreamCode}`);
}

console.log(JSON.stringify({
  client: 'typescript',
  chat_stream: streamedChat,
  chat_tool: toolCall.function.name,
  chat_total_tokens: chat.usage.total_tokens,
  responses_stream: responseText,
  responses_tool: functionCall.name,
  responses_total_tokens: responseUsage.total_tokens,
  cancelled,
  upstream_status: upstreamStatus,
  upstream_code: upstreamCode,
}));
