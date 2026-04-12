// PostMessage protocol between parent frame (trusted) and VM iframe (untrusted).
// This is the security contract -- every message validated by type and shape.

// -- Constants --

const MAX_TITLE_LENGTH = 128;
const MAX_CLIPBOARD_SIZE = 1_048_576; // 1 MB
const MAX_ERROR_MSG_LENGTH = 256;
const VM_ID_RE = /^[a-zA-Z0-9][a-zA-Z0-9_-]*$/;

// -- Parent -> Iframe --

export interface MsgVmId {
  type: 'vm-id';
  vmId: string;
}

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

export interface MsgWsTicket {
  type: 'ws-ticket';
  ticket: string;
  gatewayUrl: string;
}

export interface MsgClipboardPaste {
  type: 'clipboard-paste';
  text: string;
}

export type ParentToIframeMsg =
  | MsgVmId
  | MsgThemeChange
  | MsgFocus
  | MsgWsTicket
  | MsgClipboardPaste;

// -- Iframe -> Parent --

export interface MsgReady {
  type: 'ready';
}

export interface MsgTerminalResize {
  type: 'terminal-resize';
  cols: number;
  rows: number;
}

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

export type ErrorCode = 'ws-failed' | 'ws-closed' | 'ticket-expired';

export interface MsgError {
  type: 'error';
  code: ErrorCode;
  message: string;
}

export type IframeToParentMsg =
  | MsgReady
  | MsgTerminalResize
  | MsgTitleUpdate
  | MsgClipboardCopy
  | MsgClipboardRequest
  | MsgError;

// -- Validators --

/** Parse and validate a message from an iframe (used in the parent frame). */
export function parseIframeMessage(data: unknown): IframeToParentMsg | null {
  if (typeof data !== 'object' || data === null) return null;
  const msg = data as Record<string, unknown>;

  switch (msg.type) {
    case 'ready':
      return { type: 'ready' };

    case 'terminal-resize': {
      if (typeof msg.cols !== 'number' || typeof msg.rows !== 'number') return null;
      if (msg.cols < 1 || msg.cols > 500 || msg.rows < 1 || msg.rows > 200) return null;
      return { type: 'terminal-resize', cols: Math.floor(msg.cols), rows: Math.floor(msg.rows) };
    }

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

    case 'error': {
      const validCodes: ErrorCode[] = ['ws-failed', 'ws-closed', 'ticket-expired'];
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
    case 'vm-id': {
      if (typeof msg.vmId !== 'string') return null;
      if (msg.vmId.length > 64 || !VM_ID_RE.test(msg.vmId)) return null;
      return { type: 'vm-id', vmId: msg.vmId };
    }

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

    case 'ws-ticket': {
      if (typeof msg.ticket !== 'string' || msg.ticket.length > 256) return null;
      if (typeof msg.gatewayUrl !== 'string') return null;
      return { type: 'ws-ticket', ticket: msg.ticket, gatewayUrl: msg.gatewayUrl };
    }

    case 'clipboard-paste': {
      if (typeof msg.text !== 'string') return null;
      if (msg.text.length > MAX_CLIPBOARD_SIZE) return null;
      return { type: 'clipboard-paste', text: msg.text };
    }

    default:
      return null;
  }
}
