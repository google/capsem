import { Terminal } from "@xterm/xterm";
import { FitAddon } from "@xterm/addon-fit";
import { WebglAddon } from "@xterm/addon-webgl";
import xtermCssText from "@xterm/xterm/css/xterm.css?inline";

const HOST_CSS = `
  :host {
    display: block;
    width: 100%;
    height: 100%;
    overflow: hidden;
  }
  .terminal-wrapper {
    width: 100%;
    height: 100%;
    padding: 4px;
  }
  .xterm {
    height: 100%;
  }
  .xterm-viewport {
    background-color: transparent !important;
  }
`;

export class CapsemTerminal extends HTMLElement {
  private shadow: ShadowRoot;
  private terminal: Terminal;
  private fitAddon: FitAddon;
  private resizeObserver: ResizeObserver | null = null;

  constructor() {
    super();
    this.shadow = this.attachShadow({ mode: "closed" });

    this.terminal = new Terminal({
      cursorBlink: true,
      fontFamily:
        'ui-monospace, SFMono-Regular, Menlo, Monaco, Consolas, "Liberation Mono", "Courier New", monospace',
      fontSize: 14,
      scrollback: 10000,
      theme: {
        background: "#0a0a0a",
        foreground: "#4ade80",
        cursor: "#4ade80",
        selectionBackground: "#4ade8040",
      },
    });

    this.fitAddon = new FitAddon();
    this.terminal.loadAddon(this.fitAddon);
  }

  async connectedCallback() {
    // Inject xterm.css + host styles into shadow DOM
    const style = document.createElement("style");
    style.textContent = xtermCssText + HOST_CSS;
    this.shadow.appendChild(style);

    const wrapper = document.createElement("div");
    wrapper.className = "terminal-wrapper";
    this.shadow.appendChild(wrapper);

    // Wait for fonts to load before opening terminal (prevents grid miscalculation)
    await document.fonts.ready;

    this.terminal.open(wrapper);

    // Try WebGL renderer with canvas fallback
    try {
      const webgl = new WebglAddon();
      webgl.onContextLoss(() => webgl.dispose());
      this.terminal.loadAddon(webgl);
    } catch {
      // Canvas renderer is the default fallback
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

    // ResizeObserver for layout-driven resizes
    this.resizeObserver = new ResizeObserver(() => {
      this.fitAddon.fit();
    });
    this.resizeObserver.observe(this);
  }

  disconnectedCallback() {
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
}

customElements.define("capsem-terminal", CapsemTerminal);
