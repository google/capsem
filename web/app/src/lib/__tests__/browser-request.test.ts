import { describe, expect, it } from 'vitest';
import { browserUrl } from '../models/browser';
import { parseIframeMessage } from '../terminal/postmessage';

describe('shared browser navigation policy', () => {
  it('preserves login query spelling while normalizing the host', () => {
    const url = 'HTTPS://EXAMPLE.ORG/login?state=a%2Bb';
    expect(browserUrl(url)).toBe('https://example.org/login?state=a%2Bb');
    expect(parseIframeMessage({ type: 'open-url', url })).toEqual({ type: 'open-url', url: browserUrl(url) });
  });
  it('rejects executable schemes, credentials, control characters and oversized URLs in both callers', () => {
    for (const url of [undefined, '', 'file:///etc/passwd', 'javascript:alert(1)', 'https://user:pass@example.org', '//example.org', 'https://example.org/\nsecret', `https://example.org/${'x'.repeat(8192)}`]) {
      expect(browserUrl(url)).toBeNull();
      expect(parseIframeMessage({ type: 'open-url', url })).toBeNull();
    }
  });
});
