export type A2uiMessage =
  | { version: "v0.9"; createSurface: CreateSurface }
  | { version: "v0.9"; updateComponents: UpdateComponents }
  | { version: "v0.9"; updateDataModel: UpdateDataModel }
  | { version: "v0.9"; deleteSurface: DeleteSurface };

export type CreateSurface = {
  surfaceId: string;
  catalogId: string;
  sendDataModel?: boolean;
  theme?: unknown;
};

export type UpdateComponents = {
  surfaceId: string;
  components: A2uiComponent[];
};

export type UpdateDataModel = {
  surfaceId: string;
  path?: string;
  value?: unknown;
};

export type DeleteSurface = {
  surfaceId: string;
};

export type A2uiComponent = {
  id: string;
  component: string;
  [key: string]: unknown;
};

export type PreviewPayload = {
  generatedBy: string;
  catalogId: string;
  examples: PreviewExample[];
};

export type PreviewExample = {
  name: string;
  api: string;
  surface: string;
  messages: A2uiMessage[];
};

export type SurfaceModel = {
  surfaceId: string;
  components: Map<string, A2uiComponent>;
  data: unknown;
};

export type RenderAction = {
  name: string;
  context?: Record<string, unknown>;
  sourceComponentId: string;
};

export function buildSurface(messages: A2uiMessage[]): SurfaceModel {
  let surfaceId = "";
  const components = new Map<string, A2uiComponent>();
  let data: unknown = {};

  for (const message of messages) {
    if ("createSurface" in message) {
      surfaceId = message.createSurface.surfaceId;
    }
    if ("updateComponents" in message) {
      surfaceId = message.updateComponents.surfaceId;
      components.clear();
      for (const component of message.updateComponents.components) {
        components.set(component.id, component);
      }
    }
    if ("updateDataModel" in message) {
      surfaceId = message.updateDataModel.surfaceId;
      data = applyDataUpdate(data, message.updateDataModel.path, message.updateDataModel.value);
    }
  }

  return { surfaceId, components, data };
}

export function resolveDynamic(value: unknown, data: unknown, scope: unknown): string {
  if (typeof value === "string") return value;
  if (typeof value === "number" || typeof value === "boolean") return String(value);
  if (!isRecord(value)) return "";

  if (typeof value.path === "string") {
    return stringify(lookupPath(value.path, data, scope));
  }

  if (value.call === "formatString" && isRecord(value.args)) {
    return formatString(stringify(value.args.value), data, scope);
  }

  if (value.call === "formatDate" && isRecord(value.args)) {
    const raw = isRecord(value.args.value) && typeof value.args.value.path === "string"
      ? lookupPath(value.args.value.path, data, scope)
      : value.args.value;
    return formatDate(stringify(raw), stringify(value.args.format));
  }

  return "";
}

export function resolveChildren(children: unknown, data: unknown, scope: unknown): Array<{
  id: string;
  scope: unknown;
  key: string;
}> {
  if (Array.isArray(children)) {
    return children
      .filter((id): id is string => typeof id === "string")
      .map((id) => ({ id, scope, key: id }));
  }

  if (isRecord(children) && typeof children.path === "string" && typeof children.componentId === "string") {
    const collection = lookupPath(children.path, data, scope);
    if (!Array.isArray(collection)) return [];
    return collection.map((item, index) => ({
      id: children.componentId as string,
      scope: item,
      key: `${children.componentId}:${index}`,
    }));
  }

  return [];
}

export function lookupPath(path: string, data: unknown, scope: unknown): unknown {
  const source = path.startsWith("/") ? data : scope;
  const parts = path.replace(/^\//, "").split("/").filter(Boolean);
  let cursor = source;
  for (const part of parts) {
    if (!isRecord(cursor) && !Array.isArray(cursor)) return undefined;
    cursor = (cursor as Record<string, unknown>)[part];
  }
  return cursor;
}

export function textClass(variant: unknown): string {
  switch (variant) {
    case "h1":
      return "text-4xl font-semibold tracking-normal text-foreground";
    case "h2":
      return "text-2xl font-semibold tracking-normal text-foreground";
    case "h3":
      return "text-lg font-semibold tracking-normal text-foreground";
    case "h4":
      return "text-sm font-semibold tracking-normal text-foreground";
    case "caption":
      return "text-xs text-muted-foreground-1";
    case "body":
    default:
      return "text-sm leading-6 text-muted-foreground-1";
  }
}

export function iconText(name: unknown): string {
  switch (name) {
    case "warning":
      return "!";
    case "info":
      return "i";
    case "check":
      return "ok";
    case "close":
      return "x";
    default:
      return ".";
  }
}

export function actionName(component: A2uiComponent): string {
  const action = component.action;
  if (isRecord(action) && isRecord(action.event) && typeof action.event.name === "string") {
    return action.event.name;
  }
  return "component.action";
}

export function actionContext(component: A2uiComponent): Record<string, unknown> {
  const action = component.action;
  if (isRecord(action) && isRecord(action.event) && isRecord(action.event.context)) {
    return action.event.context;
  }
  return {};
}

function applyDataUpdate(data: unknown, path: string | undefined, value: unknown): unknown {
  if (!path || path === "/") return value ?? {};
  if (!isRecord(data)) return data;
  const parts = path.replace(/^\//, "").split("/").filter(Boolean);
  if (parts.length === 0) return value ?? {};
  const next = { ...data };
  let cursor: Record<string, unknown> = next;
  for (const part of parts.slice(0, -1)) {
    const child = isRecord(cursor[part]) ? { ...cursor[part] } : {};
    cursor[part] = child;
    cursor = child;
  }
  if (value === undefined) {
    delete cursor[parts[parts.length - 1]];
  } else {
    cursor[parts[parts.length - 1]] = value;
  }
  return next;
}

function formatString(template: string, data: unknown, scope: unknown): string {
  return template.replace(/\$\{([^}]+)\}/g, (_match, path: string) => {
    return stringify(lookupPath(path, data, scope));
  });
}

function formatDate(value: string, format: string): string {
  const date = new Date(value);
  if (Number.isNaN(date.valueOf())) return value;
  if (format === "E") {
    return new Intl.DateTimeFormat("en", { weekday: "short", timeZone: "UTC" }).format(date);
  }
  if (format === "h:mm a") {
    return new Intl.DateTimeFormat("en", {
      hour: "numeric",
      minute: "2-digit",
      timeZone: "UTC",
    }).format(date);
  }
  return date.toLocaleString();
}

function stringify(value: unknown): string {
  if (value === undefined || value === null) return "";
  if (typeof value === "string") return value;
  if (typeof value === "number" || typeof value === "boolean") return String(value);
  return JSON.stringify(value);
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}
