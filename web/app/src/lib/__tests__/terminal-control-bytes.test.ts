import { describe, expect, it } from 'vitest';
import { Terminal } from '@xterm/xterm';
import { TERMINAL_OPTIONS } from '../terminal/terminal-config';

describe('application terminal control bytes', () => {
  it('preserves the column through LF and backspace in incremental redraws', async () => {
    const terminal = new Terminal({ ...TERMINAL_OPTIONS, cols: 20, rows: 5 });
    try {
      // AGY uses bare LF to move vertically, then BS to update part of a line.
      await new Promise<void>((resolve) => terminal.write('abcdefgh\n\n\b\bX', resolve));
      expect(terminal.buffer.active.getLine(2)?.translateToString(true)).toBe('      X');

      // Ordinary cooked terminal output already supplies CR when needed.
      await new Promise<void>((resolve) => terminal.write('\r\nnext', resolve));
      expect(terminal.buffer.active.getLine(3)?.translateToString(true)).toBe('next');
    } finally {
      terminal.dispose();
    }
  });
});
