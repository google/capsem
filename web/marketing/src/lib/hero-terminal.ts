import type { Terminal } from "@xterm/xterm";

export interface TerminalStep {
  write?: string;
  type?: string;
  wait?: number;
  clear?: boolean;
  title?: string;
  preview?: boolean;
}

/** Feed an ANSI transcript through the same grid/cursor rules as a real PTY. */
export function terminalPlayback(
  terminal: Terminal,
  steps: TerminalStep[],
  onStep: (step: TerminalStep) => void,
  onEnd: () => void,
) {
  let index = 0;
  let elapsed = 0;
  let written = 0;
  let entered = false;

  return {
    reset() {
      index = elapsed = written = 0;
      entered = false;
      terminal.reset();
    },
    tick(delta: number) {
      const step = steps[index];
      if (!step) return;
      if (!entered) {
        if (step.clear) terminal.write("\x1b[2J\x1b[H");
        if (step.write) terminal.write(step.write);
        onStep(step);
        entered = true;
      }
      elapsed += delta;
      const characters = Array.from(step.type ?? "");
      // Type through the terminal buffer so wrapping and the cursor stay native.
      const count = Math.min(characters.length, Math.floor(elapsed / 42));
      if (count > written) {
        terminal.write(characters.slice(written, count).join(""));
        written = count;
      }
      if (elapsed >= characters.length * 42 + (step.wait ?? 350)) {
        index += 1;
        elapsed = written = 0;
        entered = false;
        if (index === steps.length) onEnd();
      }
    },
  };
}
