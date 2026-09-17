import {HttpError} from '@capsem/sdk';
import {afterEach, expect, it, vi} from 'vitest';
import {DIAGNOSTICS_ENV, MAX_MIRRORED_TEXT_BYTES, failure, success} from '../src/results.js';

afterEach(() => {vi.restoreAllMocks(); vi.unstubAllEnvs();});

// A large result was serialized twice: once as structuredContent and again as
// content[0].text, doubling what an agent pays for one read.
it('mirrors a small result as text but not a large one', () => {
  const small = success({path: '/etc/hostname', content: 'box'});
  expect(small.content[0]).toEqual({type: 'text', text: JSON.stringify({path: '/etc/hostname', content: 'box'})});

  const large = success({content: 'x'.repeat(MAX_MIRRORED_TEXT_BYTES + 1)});
  expect(large.structuredContent).toEqual({content: 'x'.repeat(MAX_MIRRORED_TEXT_BYTES + 1)});
  const text = large.content[0];
  expect(text?.type).toBe('text');
  const mirrored = text?.type === 'text' ? text.text : '';
  expect(mirrored).toMatch(/structuredContent/);
  expect(mirrored.length).toBeLessThan(MAX_MIRRORED_TEXT_BYTES);
});

// stdout is the MCP protocol channel, so diagnostics go to stderr. The tool
// result keeps hiding the gateway's body, which may quote credentials.
it('logs the gateway status and body to stderr only when asked, never into the tool result', () => {
  const stderr = vi.spyOn(process.stderr, 'write').mockReturnValue(true);
  const quiet = failure(new HttpError(403, 'denied for token abc123'));
  expect(JSON.stringify(quiet)).not.toContain('abc123');
  expect(stderr).not.toHaveBeenCalled();

  vi.stubEnv(DIAGNOSTICS_ENV, '1');
  const result = failure(new HttpError(403, 'denied for token abc123'));
  expect(JSON.stringify(result)).not.toContain('abc123');
  expect(result.structuredContent).toEqual({error: {kind: 'http', status: 403}});
  const logged = stderr.mock.calls.map(call => String(call[0])).join('');
  expect(logged).toContain('403');
  expect(logged).toContain('denied for token abc123');
});
