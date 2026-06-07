// PostMessage protocol between parent frame (trusted) and VM iframe (sandboxed).
// Initial state (vmId, initial theme) flows via iframe URL params -- this
// contract covers only runtime signals, which are fire-and-forget.

const MAX_TITLE_LENGTH = 128;
const MAX_CLIPBOARD_SIZE = 1_048_576; // 1 MB
const MAX_ERROR_MSG_LENGTH = 256;

// -- Parent -> Iframe --

export interface MsgThemeChange {
  type: 'theme-change';
  mode: 'light' | 'dark';
  terminalTheme: string;
  fontSize: number;
  fontFamily: string;
}

export interface MsgFocus {
  type: 'focus';
}

export interface MsgClipboardPaste {
  type: 'clipboard-paste';
  text: string;
}

export type ParentToIframeMsg =
  | MsgThemeChange
  | MsgFocus
  | MsgClipboardPaste;

// -- Iframe -> Parent --

export interface MsgTitleUpdate {
  type: 'title-update';
  title: string;
}

export interface MsgClipboardCopy {
  type: 'clipboard-copy';
  text: string;
}

export interface MsgClipboardRequest {
  type: 'clipboard-request';
}

export interface MsgConnected {
  type: 'connected';
}

export interface MsgDisconnected {
  type: 'disconnected';
  reason: string;
}

export type ErrorCode = 'ws-failed' | 'ws-closed' | 'token-failed';

export interface MsgError {
  type: 'error';
  code: ErrorCode;
  message: string;
}

export type IframeToParentMsg =
  | MsgTitleUpdate
  | MsgClipboardCopy
  | MsgClipboardRequest
  | MsgConnected
  | MsgDisconnected
  | MsgError;

// -- Validators --

/** Parse and validate a message from an iframe (used in the parent frame). */
export function parseIframeMessage(data: unknown): IframeToParentMsg | null {
  if (typeof data !== 'object' || data === null) return null;
  const msg = data as Record<string, unknown>;

  switch (msg.type) {
    case 'title-update': {
      if (typeof msg.title !== 'string') return null;
      return { type: 'title-update', title: msg.title.slice(0, MAX_TITLE_LENGTH) };
    }
    case 'clipboard-copy': {
      if (typeof msg.text !== 'string') return null;
      if (msg.text.length > MAX_CLIPBOARD_SIZE) return null;
      return { type: 'clipboard-copy', text: msg.text };
    }
    case 'clipboard-request':
      return { type: 'clipboard-request' };
    case 'connected':
      return { type: 'connected' };
    case 'disconnected': {
      const reason = typeof msg.reason === 'string' ? msg.reason.slice(0, MAX_ERROR_MSG_LENGTH) : '';
      return { type: 'disconnected', reason };
    }
    case 'error': {
      const validCodes: ErrorCode[] = ['ws-failed', 'ws-closed', 'token-failed'];
      if (!validCodes.includes(msg.code as ErrorCode)) return null;
      if (typeof msg.message !== 'string') return null;
      return {
        type: 'error',
        code: msg.code as ErrorCode,
        message: msg.message.slice(0, MAX_ERROR_MSG_LENGTH),
      };
    }
    default:
      return null;
  }
}

/** Parse and validate a message from the parent (used in the iframe). */
export function parseParentMessage(data: unknown): ParentToIframeMsg | null {
  if (typeof data !== 'object' || data === null) return null;
  const msg = data as Record<string, unknown>;

  switch (msg.type) {
    case 'theme-change': {
      if (msg.mode !== 'light' && msg.mode !== 'dark') return null;
      if (typeof msg.terminalTheme !== 'string') return null;
      if (msg.terminalTheme.length > 64) return null;
      const fontSize = typeof msg.fontSize === 'number' && msg.fontSize >= 8 && msg.fontSize <= 32
        ? msg.fontSize : 14;
      const fontFamily = typeof msg.fontFamily === 'string' && msg.fontFamily.length <= 256
        ? msg.fontFamily : '';
      return { type: 'theme-change', mode: msg.mode, terminalTheme: msg.terminalTheme, fontSize, fontFamily };
    }
    case 'focus':
      return { type: 'focus' };
    case 'clipboard-paste': {
      if (typeof msg.text !== 'string') return null;
      if (msg.text.length > MAX_CLIPBOARD_SIZE) return null;
      return { type: 'clipboard-paste', text: msg.text };
    }
    default:
      return null;
  }
}
