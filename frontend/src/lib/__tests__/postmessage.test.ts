import { describe, it, expect } from 'vitest';
import { parseIframeMessage, parseParentMessage } from '../terminal/postmessage.ts';

// -- parseIframeMessage --

describe('parseIframeMessage', () => {
  it('rejects null', () => {
    expect(parseIframeMessage(null)).toBeNull();
  });

  it('rejects undefined', () => {
    expect(parseIframeMessage(undefined)).toBeNull();
  });

  it('rejects non-object', () => {
    expect(parseIframeMessage('hello')).toBeNull();
    expect(parseIframeMessage(42)).toBeNull();
  });

  it('rejects array', () => {
    expect(parseIframeMessage([1, 2])).toBeNull();
  });

  it('rejects unknown type', () => {
    expect(parseIframeMessage({ type: 'bogus' })).toBeNull();
  });

  it('rejects missing type', () => {
    expect(parseIframeMessage({ title: 'x' })).toBeNull();
  });

  // clipboard-request
  it('parses clipboard-request', () => {
    expect(parseIframeMessage({ type: 'clipboard-request' })).toEqual({ type: 'clipboard-request' });
  });

  // connected
  it('parses connected', () => {
    expect(parseIframeMessage({ type: 'connected' })).toEqual({ type: 'connected' });
  });

  // disconnected
  it('parses disconnected with reason', () => {
    expect(parseIframeMessage({ type: 'disconnected', reason: 'ws closed' }))
      .toEqual({ type: 'disconnected', reason: 'ws closed' });
  });

  it('defaults disconnected reason to empty string when missing', () => {
    expect(parseIframeMessage({ type: 'disconnected' }))
      .toEqual({ type: 'disconnected', reason: '' });
  });

  it('truncates disconnected reason at 256 chars', () => {
    const long = 'r'.repeat(300);
    const result = parseIframeMessage({ type: 'disconnected', reason: long });
    expect(result).not.toBeNull();
    expect(result!.type === 'disconnected' && result!.reason.length).toBe(256);
  });

  // title-update
  it('parses valid title-update', () => {
    expect(parseIframeMessage({ type: 'title-update', title: 'hello' }))
      .toEqual({ type: 'title-update', title: 'hello' });
  });

  it('truncates title at 128 chars', () => {
    const long = 'x'.repeat(200);
    const result = parseIframeMessage({ type: 'title-update', title: long });
    expect(result).not.toBeNull();
    expect(result!.type === 'title-update' && result!.title.length).toBe(128);
  });

  it('rejects non-string title', () => {
    expect(parseIframeMessage({ type: 'title-update', title: 42 })).toBeNull();
  });

  // clipboard-copy
  it('parses valid clipboard-copy', () => {
    expect(parseIframeMessage({ type: 'clipboard-copy', text: 'data' }))
      .toEqual({ type: 'clipboard-copy', text: 'data' });
  });

  it('rejects clipboard-copy over 1MB', () => {
    const big = 'x'.repeat(1_048_577);
    expect(parseIframeMessage({ type: 'clipboard-copy', text: big })).toBeNull();
  });

  it('rejects non-string clipboard-copy text', () => {
    expect(parseIframeMessage({ type: 'clipboard-copy', text: 123 })).toBeNull();
  });

  // error
  it('parses valid error with ws-failed', () => {
    expect(parseIframeMessage({ type: 'error', code: 'ws-failed', message: 'oops' }))
      .toEqual({ type: 'error', code: 'ws-failed', message: 'oops' });
  });

  it('parses valid error with ws-closed', () => {
    expect(parseIframeMessage({ type: 'error', code: 'ws-closed', message: 'bye' }))
      .toEqual({ type: 'error', code: 'ws-closed', message: 'bye' });
  });

  it('parses valid error with token-failed', () => {
    expect(parseIframeMessage({ type: 'error', code: 'token-failed', message: 'no token' }))
      .toEqual({ type: 'error', code: 'token-failed', message: 'no token' });
  });

  it('rejects error with invalid code', () => {
    expect(parseIframeMessage({ type: 'error', code: 'unknown', message: 'x' })).toBeNull();
  });

  it('truncates error message at 256 chars', () => {
    const long = 'm'.repeat(300);
    const result = parseIframeMessage({ type: 'error', code: 'ws-failed', message: long });
    expect(result).not.toBeNull();
    expect(result!.type === 'error' && result!.message.length).toBe(256);
  });

  it('rejects error with non-string message', () => {
    expect(parseIframeMessage({ type: 'error', code: 'ws-failed', message: 42 })).toBeNull();
  });
});

// -- parseParentMessage --

describe('parseParentMessage', () => {
  it('rejects null', () => {
    expect(parseParentMessage(null)).toBeNull();
  });

  it('rejects undefined', () => {
    expect(parseParentMessage(undefined)).toBeNull();
  });

  it('rejects non-object', () => {
    expect(parseParentMessage('hi')).toBeNull();
  });

  it('rejects unknown type', () => {
    expect(parseParentMessage({ type: 'nope' })).toBeNull();
  });

  // theme-change
  it('parses valid theme-change', () => {
    expect(parseParentMessage({
      type: 'theme-change', mode: 'dark', terminalTheme: 'dracula', fontSize: 16, fontFamily: 'Menlo',
    })).toEqual({
      type: 'theme-change', mode: 'dark', terminalTheme: 'dracula', fontSize: 16, fontFamily: 'Menlo',
    });
  });

  it('rejects invalid mode', () => {
    expect(parseParentMessage({
      type: 'theme-change', mode: 'auto', terminalTheme: 'x', fontSize: 14, fontFamily: '',
    })).toBeNull();
  });

  it('defaults fontSize to 14 when out of range', () => {
    const result = parseParentMessage({
      type: 'theme-change', mode: 'light', terminalTheme: 'x', fontSize: 50, fontFamily: '',
    });
    expect(result).not.toBeNull();
    expect(result!.type === 'theme-change' && result!.fontSize).toBe(14);
  });

  it('defaults fontSize to 14 when not a number', () => {
    const result = parseParentMessage({
      type: 'theme-change', mode: 'light', terminalTheme: 'x', fontFamily: '',
    });
    expect(result).not.toBeNull();
    expect(result!.type === 'theme-change' && result!.fontSize).toBe(14);
  });

  it('defaults fontFamily to empty string when too long', () => {
    const result = parseParentMessage({
      type: 'theme-change', mode: 'dark', terminalTheme: 'x', fontSize: 14, fontFamily: 'x'.repeat(257),
    });
    expect(result).not.toBeNull();
    expect(result!.type === 'theme-change' && result!.fontFamily).toBe('');
  });

  it('rejects non-string terminalTheme', () => {
    expect(parseParentMessage({
      type: 'theme-change', mode: 'dark', terminalTheme: 42, fontSize: 14, fontFamily: '',
    })).toBeNull();
  });

  it('rejects terminalTheme over 64 chars', () => {
    expect(parseParentMessage({
      type: 'theme-change', mode: 'dark', terminalTheme: 'x'.repeat(65), fontSize: 14, fontFamily: '',
    })).toBeNull();
  });

  // focus
  it('parses focus', () => {
    expect(parseParentMessage({ type: 'focus' })).toEqual({ type: 'focus' });
  });

  // clipboard-paste
  it('parses valid clipboard-paste', () => {
    expect(parseParentMessage({ type: 'clipboard-paste', text: 'hello' }))
      .toEqual({ type: 'clipboard-paste', text: 'hello' });
  });

  it('rejects clipboard-paste over 1MB', () => {
    expect(parseParentMessage({ type: 'clipboard-paste', text: 'x'.repeat(1_048_577) })).toBeNull();
  });

  it('rejects clipboard-paste with non-string text', () => {
    expect(parseParentMessage({ type: 'clipboard-paste', text: 123 })).toBeNull();
  });
});
