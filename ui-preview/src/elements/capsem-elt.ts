export type CapsemElementCleanup = () => void;

export type CapsemElementEventDetail<TDetail = unknown> = {
  source: string;
  specId?: string;
  payload: TDetail;
};

export type CapsemElementRenderContext<TSpec> = {
  host: CapsemElement<TSpec>;
  root: ShadowRoot;
  container: HTMLElement;
  spec: TSpec;
  emit: <TDetail>(type: string, payload: TDetail) => void;
  cleanup: (callback: CapsemElementCleanup) => void;
};

export type CapsemElementSpec = {
  id?: string;
  title?: string;
  description?: string;
  kind?: string;
};

const baseStyle = `
  :host {
    box-sizing: border-box;
    display: block;
    color: var(--foreground);
    font: inherit;
  }

  *,
  *::before,
  *::after {
    box-sizing: inherit;
  }

  [part="root"] {
    min-width: 0;
  }

  [part="empty"],
  [part="error"] {
    border: 1px solid var(--card-line);
    border-radius: 0.75rem;
    background: var(--card);
    color: var(--muted-foreground-1);
    padding: 1rem;
    font-size: 0.875rem;
    line-height: 1.375;
  }

  [part="error"] {
    border-color: var(--destructive);
    background: color-mix(in oklab, var(--destructive) 10%, transparent);
    color: var(--destructive);
  }
`;

export abstract class CapsemElement<TSpec = unknown> extends HTMLElement {
  readonly root: ShadowRoot;
  readonly container: HTMLElement;

  private currentSpec: TSpec | undefined;
  private cleanupCallbacks: CapsemElementCleanup[] = [];
  private renderPending = false;
  private connected = false;

  protected constructor() {
    super();
    this.root = this.attachShadow({ mode: "open" });
    const style = document.createElement("style");
    style.textContent = this.baseStyle();
    this.container = document.createElement("div");
    this.container.part.add("root");
    this.root.append(style, this.container);
  }

  set spec(value: TSpec | undefined) {
    this.currentSpec = value;
    this.scheduleRender();
  }

  get spec(): TSpec | undefined {
    return this.currentSpec;
  }

  connectedCallback(): void {
    this.connected = true;
    this.scheduleRender();
  }

  disconnectedCallback(): void {
    this.connected = false;
    this.runCleanup();
  }

  protected abstract render(context: CapsemElementRenderContext<TSpec>): void;

  protected baseStyle(): string {
    return baseStyle;
  }

  protected emit<TDetail>(type: string, payload: TDetail): void {
    const detail: CapsemElementEventDetail<TDetail> = {
      source: this.localName,
      specId: specId(this.currentSpec),
      payload,
    };
    this.dispatchEvent(new CustomEvent(`capsem:${type}`, {
      bubbles: true,
      composed: true,
      detail,
    }));
  }

  protected onCleanup(callback: CapsemElementCleanup): void {
    this.cleanupCallbacks.push(callback);
  }

  protected replaceRootChildren(...children: Node[]): void {
    this.container.replaceChildren(...children);
  }

  protected renderEmpty(message = "No component spec provided."): void {
    const empty = document.createElement("div");
    empty.part.add("empty");
    empty.textContent = message;
    this.replaceRootChildren(empty);
  }

  protected renderError(cause: unknown): void {
    const error = document.createElement("div");
    error.part.add("error");
    error.textContent = cause instanceof Error ? cause.message : String(cause);
    this.replaceRootChildren(error);
  }

  private scheduleRender(): void {
    if (!this.connected || this.renderPending) return;
    this.renderPending = true;
    queueMicrotask(() => {
      this.renderPending = false;
      if (!this.connected) return;
      this.renderNow();
    });
  }

  private renderNow(): void {
    this.runCleanup();
    if (this.currentSpec === undefined) {
      this.renderEmpty();
      return;
    }

    try {
      this.render({
        host: this,
        root: this.root,
        container: this.container,
        spec: this.currentSpec,
        emit: (type, payload) => this.emit(type, payload),
        cleanup: (callback) => this.onCleanup(callback),
      });
    } catch (cause) {
      this.renderError(cause);
      this.emit("error", { message: cause instanceof Error ? cause.message : String(cause) });
    }
  }

  private runCleanup(): void {
    const callbacks = this.cleanupCallbacks.splice(0);
    for (const callback of callbacks.reverse()) {
      callback();
    }
  }
}

export class CapsemElt extends CapsemElement<CapsemElementSpec> {
  protected render({ spec, emit, cleanup }: CapsemElementRenderContext<CapsemElementSpec>): void {
    const shell = document.createElement("section");
    shell.part.add("surface");

    const title = document.createElement("h3");
    title.part.add("title");
    title.textContent = spec.title ?? spec.kind ?? "Capsem element";
    shell.append(title);

    if (spec.description) {
      const description = document.createElement("p");
      description.part.add("description");
      description.textContent = spec.description;
      shell.append(description);
    }

    const style = document.createElement("style");
    style.textContent = `
      [part="surface"] {
        border: 1px solid var(--card-line);
        border-radius: 0.75rem;
        background: var(--card);
        color: var(--foreground);
        padding: 1rem;
      }

      [part="title"] {
        margin: 0;
        font-size: 0.875rem;
        font-weight: 600;
        line-height: 1.25rem;
      }

      [part="description"] {
        margin: 0.375rem 0 0;
        color: var(--muted-foreground-1);
        font-size: 0.875rem;
        line-height: 1.375rem;
      }
    `;

    this.replaceRootChildren(style, shell);
    emit("ready", { kind: spec.kind ?? "element" });
    cleanup(() => {
      shell.replaceChildren();
    });
  }
}

export function defineCapsemElement(
  tagName = "capsem-elt",
  elementClass: CustomElementConstructor = CapsemElt,
): void {
  if (!customElements.get(tagName)) {
    customElements.define(tagName, elementClass);
  }
}

function specId(value: unknown): string | undefined {
  return isRecord(value) && typeof value.id === "string" ? value.id : undefined;
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}
