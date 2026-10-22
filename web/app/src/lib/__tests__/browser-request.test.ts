import { describe, expect, it } from 'vitest';
import { browserUrl, BrowserRequestCursor } from '../models/browser';
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

describe('fresh active-session browser requests', () => {
  const login = (id: number) => ({id, url: `https://example.org/login?state=${id}`});

  it('opens a newly requested login once, without reopening the retained login on attachment', () => {
    const requests = new BrowserRequestCursor();
    expect(requests.next('vm-a', login(18))).toBeNull();
    expect(requests.next('vm-a', login(19))).toBe(login(19).url);
    expect(requests.next('vm-a', login(19))).toBeNull();
    expect(requests.next('vm-a', login(18))).toBeNull();
    expect(requests.next('vm-a', login(19))).toBeNull();
  });

  it('does not open a retained request on session switches, and accepts fresh requests after restart', () => {
    const requests = new BrowserRequestCursor();
    expect(requests.next('vm-a', login(18))).toBeNull();
    expect(requests.next('vm-b', login(7))).toBeNull();
    expect(requests.next('vm-b', login(8))).toBe(login(8).url);
    expect(requests.next('vm-a', login(19))).toBeNull();
    expect(requests.next('vm-a', null)).toBeNull();
    expect(requests.next('vm-a', login(1))).toBe(login(1).url);
  });

  it('opens the first request from a session without a retained request and refuses unsafe navigation', () => {
    const requests = new BrowserRequestCursor();
    expect(requests.next('vm-a', undefined)).toBeNull();
    expect(requests.next('vm-a', login(1))).toBe(login(1).url);
    for (const url of ['javascript:alert(1)', 'file:///etc/passwd', 'https://user:pass@example.org']) {
      expect(requests.next('vm-a', {id: 2, url})).toBeNull();
    }
    // Rewriting an existing request must not turn it into a fresh host action.
    expect(requests.next('vm-a', login(2))).toBeNull();
    expect(requests.next('vm-a', login(3))).toBe(login(3).url);
  });
});
