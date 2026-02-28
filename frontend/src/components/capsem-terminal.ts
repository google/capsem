import { Terminal } from "@xterm/xterm";
import { FitAddon } from "@xterm/addon-fit";
import { WebglAddon } from "@xterm/addon-webgl";
import xtermCssText from "@xterm/xterm/css/xterm.css?inline";

export interface TerminalTheme {
  background: string;
  foreground: string;
  cursor: string;
  selectionBackground: string;
}

const DARK_THEME: TerminalTheme = {
  background: "#000000",
  foreground: "#c9d1d9",
  cursor: "#58a6ff",
  selectionBackground: "#58a6ff40",
};

const LIGHT_THEME: TerminalTheme = {
  background: "#f5f5f5",
  foreground: "#1b1b2f",
  cursor: "#1a56db",
  selectionBackground: "#1a56db30",
};

const HOST_CSS = `
  :host {
    display: block;
    position: relative;
    width: 100%;
    height: 100%;
    overflow: hidden;
  }
  .terminal-wrapper {
    position: absolute;
    top: 10px;
    right: 10px;
    bottom: 10px;
    left: 10px;
  }
  .xterm {
    height: 100%;
  }
  .xterm-viewport {
    background-color: transparent !important;
  }
  .xterm-viewport::-webkit-scrollbar {
    width: 6px;
  }
  .xterm-viewport::-webkit-scrollbar-track {
    background: transparent;
  }
  .xterm-viewport::-webkit-scrollbar-thumb {
    border-radius: 3px;
  }
`;

export class CapsemTerminal extends HTMLElement {
  private shadow: ShadowRoot;
  private terminal: Terminal;
  private fitAddon: FitAddon;
  private resizeObserver: ResizeObserver | null = null;
  private wrapper: HTMLDivElement | null = null;
  private resizeRafId: number = 0;
  renderer: "webgl" | "canvas" = "canvas";

  constructor() {
    super();
    this.shadow = this.attachShadow({ mode: "closed" });

    this.terminal = new Terminal({
      cursorBlink: true,
      convertEol: true,
      fontFamily:
        'ui-monospace, SFMono-Regular, Menlo, Monaco, Consolas, "Liberation Mono", "Courier New", monospace',
      fontSize: 14,
      scrollback: 10000,
      theme: DARK_THEME,
    });

    this.fitAddon = new FitAddon();
    this.terminal.loadAddon(this.fitAddon);
  }

  async connectedCallback() {
    // Inject xterm.css + host styles into shadow DOM
    const style = document.createElement("style");
    style.textContent = xtermCssText + HOST_CSS;
    this.shadow.appendChild(style);

    // Dynamic scrollbar style (updated by setTheme)
    const scrollbarStyle = document.createElement("style");
    scrollbarStyle.className = "scrollbar-override";
    scrollbarStyle.textContent = `.xterm-viewport::-webkit-scrollbar-thumb { background: rgba(255,255,255,0.15); } .xterm-viewport::-webkit-scrollbar-thumb:hover { background: rgba(255,255,255,0.25); }`;
    this.shadow.appendChild(scrollbarStyle);

    this.wrapper = document.createElement("div");
    this.wrapper.className = "terminal-wrapper";
    this.wrapper.style.backgroundColor = DARK_THEME.background;
    this.style.backgroundColor = DARK_THEME.background;
    this.shadow.appendChild(this.wrapper);

    // Wait for fonts to load before opening terminal (prevents grid miscalculation)
    await document.fonts.ready;

    this.terminal.open(this.wrapper);

    // Try WebGL renderer with canvas fallback
    try {
      const webgl = new WebglAddon();
      webgl.onContextLoss(() => {
        webgl.dispose();
        this.renderer = "canvas";
      });
      this.terminal.loadAddon(webgl);
      this.renderer = "webgl";
    } catch {
      this.renderer = "canvas";
    }

    this.fitAddon.fit();

    // Prevent WKWebView from swallowing Ctrl+key sequences (Ctrl+C, etc.)
    // so xterm can translate them to the correct control bytes.
    this.terminal.attachCustomKeyEventHandler((ev: KeyboardEvent) => {
      if (ev.ctrlKey && !ev.metaKey && !ev.altKey && ev.type === "keydown") {
        ev.preventDefault();
      }
      return true;
    });

    // Forward terminal input as a CustomEvent
    this.terminal.onData((data: string) => {
      this.dispatchEvent(
        new CustomEvent("terminal-input", {
          detail: data,
          bubbles: true,
          composed: true,
        }),
      );
    });

    // Emit resize events when terminal dimensions change.
    this.terminal.onResize(({ cols, rows }) => {
      this.dispatchEvent(
        new CustomEvent("terminal-resize", {
          detail: { cols, rows },
          bubbles: true,
          composed: true,
        }),
      );
    });

    // ResizeObserver for layout-driven resizes (debounced via rAF to prevent
    // IPC flood -- each fit() can trigger a terminal-resize event).
    this.resizeObserver = new ResizeObserver(() => {
      if (this.resizeRafId) cancelAnimationFrame(this.resizeRafId);
      this.resizeRafId = requestAnimationFrame(() => {
        this.resizeRafId = 0;
        this.fitAddon.fit();
      });
    });
    this.resizeObserver.observe(this);
  }

  disconnectedCallback() {
    if (this.resizeRafId) cancelAnimationFrame(this.resizeRafId);
    this.resizeObserver?.disconnect();
    this.terminal.dispose();
  }

  /** Write raw bytes to the terminal. Called by the parent page. */
  write(data: Uint8Array) {
    this.terminal.write(data);
  }

  /** Focus the terminal input. */
  focusTerminal() {
    this.terminal.focus();
  }

  /** Re-fit the terminal to its container and emit a resize event. */
  fit() {
    this.fitAddon.fit();
    this.dispatchEvent(
      new CustomEvent("terminal-resize", {
        detail: { cols: this.terminal.cols, rows: this.terminal.rows },
        bubbles: true,
        composed: true,
      }),
    );
  }

  /** Switch terminal color theme. Host background matches terminal for borderless look. */
  setTheme(mode: "light" | "dark") {
    const theme = mode === "light" ? LIGHT_THEME : DARK_THEME;
    this.terminal.options.theme = theme;
    // Set both wrapper and host background so the padding area matches the
    // terminal background -- gives a borderless appearance in dark mode.
    this.style.backgroundColor = theme.background;
    if (this.wrapper) {
      this.wrapper.style.backgroundColor = theme.background;
    }
    // Update scrollbar thumb color to match theme
    const thumbStyle = this.shadow.querySelector('.scrollbar-override') as HTMLStyleElement | null;
    const thumbColor = mode === 'dark' ? 'rgba(255,255,255,0.15)' : 'rgba(0,0,0,0.15)';
    const thumbHover = mode === 'dark' ? 'rgba(255,255,255,0.25)' : 'rgba(0,0,0,0.25)';
    if (thumbStyle) {
      thumbStyle.textContent = `.xterm-viewport::-webkit-scrollbar-thumb { background: ${thumbColor}; } .xterm-viewport::-webkit-scrollbar-thumb:hover { background: ${thumbHover}; }`;
    }
    // Force full redraw so WebGL renderer picks up the new colors
    this.terminal.refresh(0, this.terminal.rows - 1);
  }
}

customElements.define("capsem-terminal", CapsemTerminal);
