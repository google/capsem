import type { Terminal } from "@xterm/xterm";

export interface TerminalStep {
  write?: string;
  type?: string;
  wait?: number;
  clear?: boolean;
  title?: string;
  preview?: boolean;
  chapter?: number;
  insert?: "build" | "work" | null;
  codex?: boolean;
}

export const stepDuration = (step: TerminalStep) =>
  Array.from(step.type ?? "").length * 42 + (step.wait ?? 350);

/** Feed an ANSI transcript through the same grid/cursor rules as a real PTY. */
export function terminalPlayback(
  terminal: Terminal,
  steps: TerminalStep[],
  onStep: (step: TerminalStep) => void,
  onEnd: () => void,
  onProgress: (milliseconds: number) => void = () => {},
) {
  const starts: number[] = [];
  let duration = 0;
  for (const step of steps) { starts.push(duration); duration += stepDuration(step); }
  let index = 0;
  let elapsed = 0;
  let written = 0;
  let entered = false;

  const player = {
    reset() {
      index = elapsed = written = 0;
      entered = false;
      terminal.reset();
      onProgress(0);
    },
    seek(target: number) {
      player.reset();
      index = Math.max(0, Math.min(target, steps.length - 1));
      for (const step of steps.slice(0, index)) {
        if (step.clear) terminal.write("\x1b[2J\x1b[H");
        if (step.write) terminal.write(step.write);
        if (step.type) terminal.write(step.type);
        onStep(step);
      }
      // Enter the selected chapter immediately, even while paused.
      player.tick(0);
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
      onProgress(Math.min(starts[index] + elapsed, duration));
      const characters = Array.from(step.type ?? "");
      // Type through the terminal buffer so wrapping and the cursor stay native.
      const count = Math.min(characters.length, Math.floor(elapsed / 42));
      if (count > written) {
        terminal.write(characters.slice(written, count).join(""));
        written = count;
      }
      if (elapsed >= stepDuration(step)) {
        index += 1;
        elapsed = written = 0;
        entered = false;
        if (index === steps.length) onEnd();
      }
    },
  };
  return player;
}
