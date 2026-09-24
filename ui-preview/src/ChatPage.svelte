<script lang="ts">
  import { onDestroy, onMount } from "svelte";
  import {
    applyRenderDeltas,
    loadNativeWorkspaceSnapshot,
    openWorkspaceStream,
    parseWorkspaceStreamMessage,
    projectionToMaps,
    reportRenderError,
    requestWorkspaceChange,
    resolveWorkspaceTask,
    selectWorkspaceElement,
    storeWorkspaceCheckpoint,
    tagNameForArtifact,
    type AnnotationTarget,
    type NativeArtifact,
    type RenderElement,
    type RenderTopology,
    type WorkspaceFrame,
    type WorkspaceSnapshot,
    type WorkspaceTask,
  } from "./nativeDeck";

  const themes = [
    { label: "Default", value: "default" },
    { label: "Ocean", value: "theme-ocean" },
    { label: "Moon", value: "theme-moon" },
    { label: "Olive", value: "theme-olive" },
    { label: "Bubblegum", value: "theme-bubblegum" },
    { label: "Autumn", value: "theme-autumn" },
    { label: "Cashmere", value: "theme-cashmere" },
    { label: "Harvest", value: "theme-harvest" },
    { label: "Retro", value: "theme-retro" },
  ];

  let elementsById = $state(new Map<string, RenderElement>());
  let tasksById = $state(new Map<string, WorkspaceTask>());
  let topology = $state<RenderTopology>({ roots: [], nodes: {} });
  let selectedId = $state("");
  let lastSeq = $state(0);
  let error = $state("");
  let streamStatus = $state<"connecting" | "live" | "closed">("connecting");
  let selectedTheme = $state("default");
  let darkMode = $state(false);
  let changeText = $state("");
  let changePending = $state(false);
  let requestStatus = $state("");
  let commentMode = $state(false);
  let commentPanelOpen = $state(false);
  let annotationTarget = $state<AnnotationTarget | null>(null);
  let annotationAnchor = $state<AnnotationAnchor | null>(null);
  let previewTargetId = $state("");
  let previewAnnotationAnchor = $state<AnnotationAnchor | null>(null);
  let feedbackThread = $state<SubmittedFeedback[]>([]);
  let renderIssues = $state<RenderIssue[]>([]);
  let activeFeedbackKey = $state("");
  let feedbackInput = $state<HTMLTextAreaElement | undefined>(undefined);
  let socket: WebSocket | undefined;
  let reconnectTimer: ReturnType<typeof window.setTimeout> | undefined;
  const resolveTimers = new Map<string, ReturnType<typeof window.setTimeout>>();
  let streamEpoch = 0;

  let chatIds = $derived(topology.roots);
  let chatElements = $derived(
    chatIds.map((id) => elementsById.get(id)).filter((element): element is RenderElement => !!element),
  );
  let selected = $derived(
    (selectedId ? elementsById.get(selectedId) : undefined) ?? chatElements[0] ?? null,
  );
  let selectedAnnotationLabel = $derived(annotationTarget?.label ?? "Whole card");
  let commentPanelStyle = $derived(panelStyle(annotationAnchor));
  let inspectorBoxStyle = $derived(boxStyle(annotationAnchor));
  let previewBoxStyle = $derived(boxStyle(previewAnnotationAnchor));
  let commentPointerStyle = $derived(pointerStyle(annotationAnchor));
  let projectedFeedbackThread = $derived(tasksToFeedbackThread(tasksById));
  let mergedFeedbackThread = $derived(mergeFeedbackThread(feedbackThread, projectedFeedbackThread));
  let commentMarkers = $derived(markersFor(mergedFeedbackThread));
  let feedbackTasks = $derived([...mergedFeedbackThread].sort((left, right) => right.seq - left.seq));

  type AnnotationAnchor = {
    top: number;
    left: number;
    width: number;
    height: number;
  };

  type SubmittedFeedback = {
    seq: number;
    target: string;
    label: string;
    instruction: string;
    taskId?: string;
    selector?: string;
    annotation: AnnotationTarget | null;
    anchor: AnnotationAnchor | null;
    status: "pending" | "resolved";
    markerVisible: boolean;
  };

  type FeedbackMarker = {
    key: string;
    count: number;
    target: string;
    label: string;
    latestSeq: number;
    latestInstruction: string;
    selector?: string;
    annotation: AnnotationTarget | null;
    anchor: AnnotationAnchor;
    status: "pending" | "resolved";
  };

  type RenderIssue = {
    key: string;
    artifactId: string;
    component: string;
    renderer: string;
    message: string;
    timestamp: string;
  };

  onMount(() => {
    selectedTheme = window.localStorage.getItem("capsem-chat-theme") ?? "default";
    darkMode = window.localStorage.getItem("capsem-chat-dark") === "true";
    applyTheme();
    loadNativeWorkspaceSnapshot()
      .then(applySnapshot)
      .then(connectStream)
      .catch((cause: Error) => {
        error = cause.message;
        connectStream();
      });
  });

  onDestroy(() => {
    if (reconnectTimer) window.clearTimeout(reconnectTimer);
    for (const timer of resolveTimers.values()) window.clearTimeout(timer);
    socket?.close();
  });

  function applySnapshot(snapshot: WorkspaceSnapshot): void {
    const maps = projectionToMaps(snapshot.projection);
    elementsById = maps.elementsById;
    tasksById = maps.tasksById;
    topology = maps.topology;
    selectedId = maps.selectedId;
    lastSeq = snapshot.projection.seq;
    storeWorkspaceCheckpoint(snapshot);
    for (const frame of snapshot.tail) applyFrame(frame);
    error = "";
  }

  function applyFrame(frame: WorkspaceFrame): void {
    const next = applyRenderDeltas(elementsById, topology, selectedId, frame.deltas, tasksById);
    elementsById = next.elementsById;
    tasksById = next.tasksById;
    topology = next.topology;
    selectedId = next.selectedId || selectedId;
    lastSeq = Math.max(lastSeq, frame.record.seq);
  }

  async function selectElement(
    id: string,
    focusFeedback = false,
    annotation: AnnotationTarget | null = null,
    anchor: AnnotationAnchor | null = null,
  ): Promise<void> {
    selectedId = id;
    requestStatus = "";
    commentPanelOpen = true;
    annotationTarget = annotation ?? wholeElementAnnotation();
    annotationAnchor = anchor;
    clearAnnotationPreview();
    if (focusFeedback) queueMicrotask(() => feedbackInput?.focus());
    try {
      const frame = await selectWorkspaceElement(id);
      applyFrame(frame);
      if (focusFeedback) feedbackInput?.focus();
    } catch (cause) {
      error = cause instanceof Error ? cause.message : String(cause);
    }
  }

  function handleElementKeydown(event: KeyboardEvent, id: string): void {
    if (!commentMode) return;
    if (event.key !== "Enter" && event.key !== " ") return;
    event.preventDefault();
    selectElement(id, true, wholeElementAnnotation());
  }

  function closeCommentPanel(): void {
    commentPanelOpen = false;
    changeText = "";
    requestStatus = "";
    annotationTarget = null;
    annotationAnchor = null;
    activeFeedbackKey = "";
    clearAnnotationPreview();
  }

  function reopenFeedbackMarker(marker: FeedbackMarker): void {
    commentMode = true;
    selectedId = marker.target;
    requestStatus = "";
    commentPanelOpen = true;
    annotationTarget = cloneAnnotation(marker.annotation);
    annotationAnchor = { ...marker.anchor };
    activeFeedbackKey = marker.key;
    changeText = marker.latestInstruction;
    clearAnnotationPreview();
    queueMicrotask(() => feedbackInput?.focus());
  }

  function toggleCommentMode(): void {
    commentMode = !commentMode;
    if (!commentMode) closeCommentPanel();
  }

  async function submitChangeRequest(): Promise<void> {
    const instruction = changeText.trim();
    if (!selected || !instruction || changePending) return;
    changePending = true;
    requestStatus = "";
    try {
      const frame = await requestWorkspaceChange(selected.id, instruction, annotationTarget);
      applyFrame(frame);
      feedbackThread = [
        {
          seq: frame.record.seq,
          target: selected.id,
          label: selectedAnnotationLabel,
          instruction,
          taskId: taskIdFromFrame(frame),
          selector: annotationTarget?.selector,
          annotation: cloneAnnotation(annotationTarget),
          anchor: annotationAnchor ? { ...annotationAnchor } : null,
          status: "pending",
          markerVisible: true,
        },
        ...feedbackThread,
      ];
      changeText = "";
      requestStatus = `request recorded · seq ${frame.record.seq}`;
      commentPanelOpen = false;
      annotationTarget = null;
      annotationAnchor = null;
      activeFeedbackKey = "";
      clearAnnotationPreview();
    } catch (cause) {
      error = cause instanceof Error ? cause.message : String(cause);
    } finally {
      changePending = false;
    }
  }

  function resolveFeedbackMarker(): void {
    if (!activeFeedbackKey) return;
    resolveFeedbackKey(activeFeedbackKey);
    closeCommentPanel();
  }

  async function resolveFeedbackKey(key: string): Promise<void> {
    if (!key) return;
    const item = mergedFeedbackThread.find((entry) => feedbackKey(entry) === key);
    if (item?.taskId) {
      try {
        const frame = await resolveWorkspaceTask(item.taskId);
        applyFrame(frame);
      } catch (cause) {
        error = cause instanceof Error ? cause.message : String(cause);
        return;
      }
    }
    feedbackThread = feedbackThread.map((item) =>
      feedbackKey(item) === key ? { ...item, status: "resolved", markerVisible: true } : item,
    );
    const previousTimer = resolveTimers.get(key);
    if (previousTimer) window.clearTimeout(previousTimer);
    resolveTimers.set(
      key,
      window.setTimeout(() => {
        feedbackThread = feedbackThread.map((item) =>
          feedbackKey(item) === key ? { ...item, markerVisible: false } : item,
        );
        resolveTimers.delete(key);
      }, 900),
    );
  }

  function connectStream(): void {
    streamEpoch += 1;
    const epoch = streamEpoch;
    if (reconnectTimer) {
      window.clearTimeout(reconnectTimer);
      reconnectTimer = undefined;
    }
    socket?.close();
    streamStatus = "connecting";
    socket = openWorkspaceStream(lastSeq);
    socket.onopen = () => {
      if (epoch !== streamEpoch) return;
      streamStatus = "live";
    };
    socket.onclose = () => {
      if (epoch !== streamEpoch) return;
      streamStatus = "closed";
      reconnectTimer = window.setTimeout(() => {
        if (epoch === streamEpoch) connectStream();
      }, 750);
    };
    socket.onerror = () => {
      if (epoch !== streamEpoch) return;
      error = "workspace stream failed";
      streamStatus = "closed";
    };
    socket.onmessage = (event) => {
      if (epoch !== streamEpoch) return;
      try {
        const message = parseWorkspaceStreamMessage(JSON.parse(event.data));
        if (message.type === "snapshot") {
          applySnapshot(message.snapshot);
        } else {
          applyFrame(message.frame);
        }
      } catch (cause) {
        error = cause instanceof Error ? cause.message : String(cause);
      }
    };
  }

  function artifactFor(element: RenderElement): NativeArtifact | null {
    return element.artifact ? { ...element.artifact, title: element.title } : null;
  }

  function componentFor(element: RenderElement): string {
    return element.artifact ? tagNameForArtifact(element.artifact) : element.component;
  }

  function annotationForElement(
    kind: string,
    label: string,
    path: string[],
    target: HTMLElement,
  ): AnnotationTarget {
    const selector = getUniqueSelector(target);
    return {
      kind,
      label,
      path,
      selector,
      selectorVerified: document.querySelector(selector) === target,
    };
  }

  function setSpec(node: HTMLElement & { spec?: NativeArtifact }, artifact: NativeArtifact | null) {
    node.spec = artifact ?? undefined;
    return {
      update(next: NativeArtifact | null) {
        node.spec = next ?? undefined;
      },
    };
  }

  function applyElementStylePatches(node: HTMLElement, element: RenderElement) {
    applyLightDomStylePatches(node, element);
    return {
      update(next: RenderElement) {
        applyLightDomStylePatches(node, next);
      },
    };
  }

  function attachRenderTelemetry(node: HTMLElement, element: RenderElement) {
    let current = element;
    const handler = (event: Event) => {
      const detail = (event as CustomEvent).detail;
      const payload = isRecord(detail?.payload) ? detail.payload : {};
      const renderer = typeof payload.renderer === "string" ? payload.renderer : undefined;
      const message =
        typeof payload.message === "string"
          ? payload.message
          : renderer
            ? `${renderer} render failed`
            : "component render failed";
      const artifactId =
        typeof payload.artifactId === "string"
          ? payload.artifactId
          : current.artifact?.id ?? current.id;
      const component = componentFor(current);
      recordRenderIssue({
        artifactId,
        component,
        renderer: renderer ?? "component",
        message,
      });
      void reportRenderError({
        message,
        source: "browser",
        status: "failed",
        artifactId,
        component,
        renderer,
        phase: "componentRender",
      }).catch((cause) => {
        console.warn("failed to report render error", cause);
      });
    };
    node.addEventListener("capsem:error", handler);
    return {
      update(next: RenderElement) {
        current = next;
      },
      destroy() {
        node.removeEventListener("capsem:error", handler);
      },
    };
  }

  function recordRenderIssue(input: {
    artifactId: string;
    component: string;
    renderer: string;
    message: string;
  }): void {
    const key = `${input.artifactId}:${input.component}:${input.renderer}`;
    const issue: RenderIssue = {
      key,
      artifactId: input.artifactId,
      component: input.component,
      renderer: input.renderer,
      message: input.message,
      timestamp: new Date().toISOString(),
    };
    renderIssues = [issue, ...renderIssues.filter((item) => item.key !== key)].slice(0, 6);
  }

  function applyLightDomStylePatches(root: HTMLElement, element: RenderElement): void {
    const patches = Array.isArray(element.artifact?.spec.stylePatches)
      ? element.artifact.spec.stylePatches
      : [];
    for (const patch of patches) {
      if (!isRecord(patch) || typeof patch.selector !== "string" || patch.selector.includes(">>>")) {
        continue;
      }
      const target = lightDomPatchTarget(root, patch.selector);
      if (!target || !isRecord(patch.styles)) continue;
      for (const [name, value] of Object.entries(patch.styles)) {
        const property = styleProperty(name);
        if (!property || typeof value !== "string") continue;
        target.style.setProperty(property, value);
      }
    }
  }

  function lightDomPatchTarget(root: HTMLElement, selector: string): HTMLElement | null {
    if (root.matches(selector)) return root;
    const scopedTarget = root.querySelector<HTMLElement>(selector);
    if (scopedTarget) return scopedTarget;
    const documentTarget = document.querySelector<HTMLElement>(selector);
    return documentTarget && root.contains(documentTarget) ? documentTarget : null;
  }

  function styleProperty(name: string): string | null {
    const allowed: Record<string, string> = {
      backgroundColor: "background-color",
      color: "color",
      fontStyle: "font-style",
      fontWeight: "font-weight",
      opacity: "opacity",
      textDecoration: "text-decoration",
    };
    return allowed[name] ?? null;
  }

  function wholeElementAnnotation(): AnnotationTarget {
    return { kind: "artifact", label: "Whole card", path: ["artifact"] };
  }

  function annotationFromPayload(payload: unknown): AnnotationTarget {
    if (isAnnotationPayload(payload)) {
      return {
        kind: payload.kind,
        label: payload.label,
        path: payload.path,
        topologyId: typeof payload.topologyId === "string" ? payload.topologyId : undefined,
        topologyRole: typeof payload.topologyRole === "string" ? payload.topologyRole : undefined,
        selector: typeof payload.selector === "string" ? payload.selector : undefined,
        hostSelector: typeof payload.hostSelector === "string" ? payload.hostSelector : undefined,
        shadowSelector: typeof payload.shadowSelector === "string" ? payload.shadowSelector : undefined,
        selectorVerified:
          typeof payload.selectorVerified === "boolean" ? payload.selectorVerified : undefined,
        metadata: isRecord(payload.metadata) ? payload.metadata : undefined,
      };
    }
    return wholeElementAnnotation();
  }

  function anchorFromPayload(payload: unknown, fallback: HTMLElement): AnnotationAnchor {
    if (isRectPayload((payload as { rect?: unknown } | null)?.rect)) {
      return (payload as { rect: AnnotationAnchor }).rect;
    }
    return anchorFromElement(fallback);
  }

  function isAnnotationPayload(value: unknown): value is AnnotationTarget {
    return (
      typeof value === "object" &&
      value !== null &&
      typeof (value as AnnotationTarget).kind === "string" &&
      typeof (value as AnnotationTarget).label === "string" &&
      Array.isArray((value as AnnotationTarget).path) &&
      (value as AnnotationTarget).path.every((part) => typeof part === "string")
    );
  }

  function isRectPayload(value: unknown): value is AnnotationAnchor {
    return (
      typeof value === "object" &&
      value !== null &&
      typeof (value as AnnotationAnchor).top === "number" &&
      typeof (value as AnnotationAnchor).left === "number" &&
      typeof (value as AnnotationAnchor).width === "number" &&
      typeof (value as AnnotationAnchor).height === "number"
    );
  }

  function isRecord(value: unknown): value is Record<string, unknown> {
    return typeof value === "object" && value !== null && !Array.isArray(value);
  }

  function attachAnnotate(node: HTMLElement, element: RenderElement) {
    let current = element;
    const handler = (event: Event) => {
      if (!commentMode) return;
      const detail = (event as CustomEvent<{ payload?: unknown }>).detail;
      event.stopPropagation();
      const annotation = annotationFromPayload(detail?.payload);
      const hostSelector = getUniqueSelector(node);
      annotation.hostSelector = annotation.hostSelector ?? hostSelector;
      if (annotation.shadowSelector) {
        annotation.selector = `${hostSelector} >>> ${annotation.shadowSelector}`;
      }
      selectElement(current.id, true, annotation, anchorFromPayload(detail?.payload, node));
    };
    const previewHandler = (event: Event) => {
      if (!commentMode || commentPanelOpen) return;
      const detail = (event as CustomEvent<{ payload?: unknown }>).detail;
      event.stopPropagation();
      if (!detail?.payload) {
        clearAnnotationPreview(current.id);
        return;
      }
      const annotation = annotationFromPayload(detail.payload);
      const hostSelector = getUniqueSelector(node);
      annotation.hostSelector = annotation.hostSelector ?? hostSelector;
      if (annotation.shadowSelector) {
        annotation.selector = `${hostSelector} >>> ${annotation.shadowSelector}`;
      }
      previewAnnotation(current.id, anchorFromPayload(detail.payload, node));
    };
    node.addEventListener("capsem:annotate", handler as EventListener);
    node.addEventListener("capsem:annotate-preview", previewHandler as EventListener);
    return {
      update(next: RenderElement) {
        current = next;
      },
      destroy() {
        node.removeEventListener("capsem:annotate", handler as EventListener);
        node.removeEventListener("capsem:annotate-preview", previewHandler as EventListener);
      },
    };
  }

  function previewAnnotation(id: string, anchor: AnnotationAnchor): void {
    if (!commentMode || commentPanelOpen) return;
    previewTargetId = id;
    previewAnnotationAnchor = anchor;
  }

  function previewLightElement(id: string, target: HTMLElement): void {
    previewAnnotation(id, anchorFromElement(target));
  }

  function clearAnnotationPreview(id?: string): void {
    if (id && previewTargetId && previewTargetId !== id) return;
    previewTargetId = "";
    previewAnnotationAnchor = null;
  }

  function applyTheme(): void {
    const root = document.documentElement;
    if (selectedTheme === "default") {
      root.removeAttribute("data-theme");
    } else {
      root.dataset.theme = selectedTheme;
    }
    root.classList.toggle("dark", darkMode);
  }

  function updateTheme(): void {
    window.localStorage.setItem("capsem-chat-theme", selectedTheme);
    window.localStorage.setItem("capsem-chat-dark", String(darkMode));
    applyTheme();
  }

  function anchorFromElement(element: HTMLElement): AnnotationAnchor {
    const rect = element.getBoundingClientRect();
    return { top: rect.top, left: rect.left, width: rect.width, height: rect.height };
  }

  function panelStyle(anchor: AnnotationAnchor | null): string {
    if (!anchor) return "";
    const viewportWidth = window.innerWidth;
    const viewportHeight = window.innerHeight;
    const panelWidth = Math.min(320, viewportWidth - 24);
    const panelHeight = Math.min(220, viewportHeight - 24);
    const left = Math.max(12, Math.min(anchor.left + anchor.width + 14, viewportWidth - panelWidth - 12));
    const below = anchor.top;
    const above = anchor.top + anchor.height - panelHeight;
    const preferredTop = below + panelHeight <= viewportHeight - 12 ? below : above;
    const top = Math.max(12, Math.min(preferredTop, viewportHeight - panelHeight - 12));
    return `left: ${left}px; top: ${top}px; width: ${panelWidth}px; max-height: ${panelHeight}px;`;
  }

  function boxStyle(anchor: AnnotationAnchor | null): string {
    if (!anchor) return "";
    return `left: ${anchor.left}px; top: ${anchor.top}px; width: ${anchor.width}px; height: ${anchor.height}px;`;
  }

  function pointerStyle(anchor: AnnotationAnchor | null): string {
    if (!anchor) return "";
    const left = Math.max(12, Math.min(anchor.left + anchor.width - 14, window.innerWidth - 44));
    const top = Math.max(12, Math.min(anchor.top - 14, window.innerHeight - 44));
    return `left: ${left}px; top: ${top}px;`;
  }

  function markersFor(items: SubmittedFeedback[]): FeedbackMarker[] {
    const markers = new Map<string, FeedbackMarker>();
    for (const item of items) {
      if (!item.anchor || !item.markerVisible) continue;
      const key = feedbackKey(item);
      const existing = markers.get(key);
      if (existing) {
        existing.count += 1;
        if (item.seq >= existing.latestSeq) {
          existing.latestSeq = item.seq;
          existing.latestInstruction = item.instruction;
          existing.label = item.label;
          existing.annotation = cloneAnnotation(item.annotation);
          existing.anchor = { ...item.anchor };
        }
        if (item.status === "pending") existing.status = "pending";
        continue;
      }
      markers.set(key, {
        key,
        count: 1,
        target: item.target,
        label: item.label,
        latestSeq: item.seq,
        latestInstruction: item.instruction,
        selector: item.selector,
        annotation: cloneAnnotation(item.annotation),
        anchor: { ...item.anchor },
        status: item.status,
      });
    }
    return Array.from(markers.values());
  }

  function tasksToFeedbackThread(tasks: Map<string, WorkspaceTask>): SubmittedFeedback[] {
    return Array.from(tasks.values()).map((task) => ({
      seq: task.createdSeq,
      target: task.target,
      label: task.annotation?.label ?? "Workspace task",
      instruction: task.instruction,
      taskId: task.id,
      selector: task.annotation?.selector,
      annotation: cloneAnnotation(task.annotation ?? null),
      anchor: null,
      status: task.status === "resolved" ? "resolved" : "pending",
      markerVisible: task.status !== "resolved",
    }));
  }

  function mergeFeedbackThread(localItems: SubmittedFeedback[], projectedItems: SubmittedFeedback[]): SubmittedFeedback[] {
    const byKey = new Map<string, SubmittedFeedback>();
    for (const item of projectedItems) byKey.set(feedbackKey(item), item);
    for (const item of localItems) {
      const key = feedbackKey(item);
      const projected = byKey.get(key);
      byKey.set(key, {
        ...(projected ?? item),
        ...item,
        taskId: item.taskId ?? projected?.taskId,
        status: projected?.status ?? item.status,
        markerVisible: item.markerVisible && projected?.status !== "resolved",
      });
    }
    return Array.from(byKey.values());
  }

  function taskIdFromFrame(frame: WorkspaceFrame): string | undefined {
    const delta = frame.deltas.find((item) => item.type === "upsertTask");
    return delta?.type === "upsertTask" ? delta.id : undefined;
  }

  function feedbackKey(item: Pick<SubmittedFeedback, "selector" | "target" | "anchor" | "annotation">): string {
    if (item.annotation?.topologyId) return `${item.target}:${item.annotation.topologyId}`;
    return item.selector ?? `${item.target}:${Math.round(item.anchor?.left ?? 0)}:${Math.round(item.anchor?.top ?? 0)}`;
  }

  function markerStyle(marker: FeedbackMarker): string {
    const left = Math.max(12, Math.min(marker.anchor.left + marker.anchor.width - 12, window.innerWidth - 36));
    const top = Math.max(12, Math.min(marker.anchor.top - 12, window.innerHeight - 36));
    return `left: ${left}px; top: ${top}px;`;
  }

  function cloneAnnotation(annotation: AnnotationTarget | null): AnnotationTarget | null {
    if (!annotation) return null;
    return {
      ...annotation,
      path: [...annotation.path],
      metadata: annotation.metadata ? { ...annotation.metadata } : undefined,
    };
  }

  function getUniqueSelector(el: HTMLElement): string {
    const path: string[] = [];
    let current: HTMLElement | null = el;

    while (current && current.nodeType === Node.ELEMENT_NODE) {
      let selector = current.localName;
      const artifactId = current.getAttribute("data-capsem-artifact-id");
      if (artifactId) {
        selector += `[data-capsem-artifact-id="${cssString(artifactId)}"]`;
        path.unshift(selector);
        break;
      }
      const elementId = current.getAttribute("data-capsem-element-id");
      if (elementId) {
        selector += `[data-capsem-element-id="${cssString(elementId)}"]`;
        path.unshift(selector);
        break;
      }
      if (current.id) {
        selector += `#${cssIdentifier(current.id)}`;
        path.unshift(selector);
        break;
      }

      const classes = Array.from(current.classList).filter(
        (className) =>
          className !== "annotate-hover" &&
          className !== "annotate-selected" &&
          className.trim() !== "",
      );
      if (classes.length > 0) selector += classes.map((className) => `.${cssIdentifier(className)}`).join("");

      const parent = current.parentElement;
      if (parent && parent.children.length > 1) {
        selector += `:nth-child(${Array.from(parent.children).indexOf(current) + 1})`;
      }
      path.unshift(selector);
      current = parent;
    }

    return path.join(" > ");
  }

  function cssIdentifier(value: string): string {
    if (typeof CSS !== "undefined" && CSS.escape) return CSS.escape(value);
    return value.replace(/[^a-zA-Z0-9_-]/g, "\\$&");
  }

  function cssString(value: string): string {
    return value.replace(/\\/g, "\\\\").replace(/"/g, '\\"');
  }
</script>

<main class={commentMode ? "capsem-commenting min-h-screen bg-surface text-foreground" : "min-h-screen bg-surface text-foreground"}>
  <section class="mx-auto flex min-h-screen w-full max-w-4xl flex-col bg-card shadow-2xs">
    <header class="flex flex-wrap items-center justify-between gap-3 border-b border-card-line px-4 py-3">
      <div class="flex min-w-0 items-center gap-3">
        <div class="flex size-9 shrink-0 items-center justify-center rounded-full bg-primary text-sm font-semibold text-primary-foreground">
          C
        </div>
        <div class="min-w-0">
          <h1 class="truncate text-sm font-semibold text-foreground">Capsem Chat</h1>
          <p class="truncate text-xs text-muted-foreground-1">workspace stream projection</p>
        </div>
      </div>
      <div class="flex flex-wrap items-center justify-end gap-2">
        <label class="inline-flex items-center gap-2 text-xs font-medium text-muted-foreground-1">
          <span>Theme</span>
          <select
            class="rounded-lg border border-layer-line bg-layer py-1.5 pe-8 ps-3 text-xs font-medium text-layer-foreground focus:border-primary focus:ring-primary"
            bind:value={selectedTheme}
            onchange={updateTheme}
            aria-label="Theme"
          >
            {#each themes as theme}
              <option value={theme.value}>{theme.label}</option>
            {/each}
          </select>
        </label>
        <label class="inline-flex items-center gap-2 rounded-lg border border-layer-line bg-layer px-3 py-1.5 text-xs font-medium text-layer-foreground">
          <input
            type="checkbox"
            class="rounded border-line-2 bg-surface text-primary focus:ring-primary"
            bind:checked={darkMode}
            onchange={updateTheme}
            aria-label="Dark mode"
          />
          <span>Dark</span>
        </label>
        <span class={streamStatus === "live"
          ? "inline-flex items-center rounded-full bg-success px-2.5 py-1 text-xs font-medium text-success-foreground"
          : "inline-flex items-center rounded-full bg-warning px-2.5 py-1 text-xs font-medium text-warning-foreground"}>
          {streamStatus} · seq {lastSeq}
        </span>
      </div>
    </header>

    {#if error}
      <div class="m-4 rounded-lg border border-destructive bg-destructive/10 p-4 text-sm text-destructive">
        {error}
      </div>
    {/if}

    {#if renderIssues.length > 0}
      <section class="m-4 rounded-xl border border-destructive bg-destructive/10 p-4 shadow-2xs" aria-label="Render errors">
        <div class="mb-3 flex items-center justify-between gap-3">
          <h2 class="text-sm font-semibold text-destructive">Render errors</h2>
          <span class="rounded-full bg-destructive px-2.5 py-1 text-xs font-medium text-destructive-foreground">
            {renderIssues.length}
          </span>
        </div>
        <div class="space-y-2">
          {#each renderIssues as issue (issue.key)}
            <article class="rounded-lg border border-destructive/30 bg-card px-3 py-2">
              <div class="flex flex-wrap items-center gap-2 text-xs font-medium text-muted-foreground-1">
                <span>{issue.artifactId}</span>
                <span>·</span>
                <span>{issue.component}</span>
                <span>·</span>
                <span>{issue.renderer}</span>
              </div>
              <p class="mt-1 text-sm text-foreground">{issue.message}</p>
            </article>
          {/each}
        </div>
      </section>
    {/if}

    <div class="flex-1 space-y-5 overflow-auto bg-surface px-4 py-6">
      <div class="flex justify-end">
        <div class="max-w-[78%] rounded-2xl rounded-tr-sm bg-primary px-4 py-3 text-sm text-primary-foreground shadow-2xs">
          Generate an image and show it in the chat.
        </div>
      </div>

      <div class="flex items-start gap-3">
        <div class="flex size-8 shrink-0 items-center justify-center rounded-full border border-card-line bg-card text-xs font-semibold text-foreground">
          AI
        </div>
        <div class="min-w-0 flex-1">
          <div class="mb-2 flex flex-wrap items-center gap-2 text-xs font-medium text-muted-foreground-1">
            <span>assistant</span>
            {#if selected}
              <span class="rounded-full bg-surface px-2 py-0.5">{selected.title}</span>
            {/if}
          </div>
          <div class="rounded-2xl rounded-tl-sm border border-card-line bg-card p-4 shadow-2xs">
            {#if chatElements.length === 0}
              <div class="rounded-xl border border-dashed border-card-line bg-surface p-5 text-sm text-muted-foreground-1">
                Waiting for workspace records. Calling <code>local.generate.image</code> will append a generated-image card here.
              </div>
            {:else}
              <div class="grid gap-4">
                {#each chatElements as element (element.id)}
                  <div
                    data-capsem-element-id={element.id}
                    use:applyElementStylePatches={element}
                    role="group"
                    aria-label={`Rendered artifact: ${element.title}`}
                    class={commentMode
                      ? "capsem-annotatable-surface overflow-hidden rounded-xl border border-card-line bg-surface shadow-2xs"
                      : element.id === selectedId
                        ? "capsem-annotatable-surface overflow-hidden rounded-xl border border-primary bg-surface shadow-2xs outline outline-2 outline-primary/20"
                        : "capsem-annotatable-surface overflow-hidden rounded-xl border border-card-line bg-surface shadow-2xs"}
                    onpointerleave={() => clearAnnotationPreview(element.id)}
                  >
                    <div class="flex w-full items-center justify-between gap-3 border-b border-card-line px-4 py-3 text-left">
                      <span class="min-w-0">
                        <button
                          type="button"
                          data-capsem-role="card-title"
                          class={commentMode
                            ? "block max-w-full truncate rounded-md text-left text-sm font-semibold text-foreground focus:outline-hidden focus:ring-2 focus:ring-primary/40"
                            : "block max-w-full truncate rounded-md text-left text-sm font-semibold text-foreground"}
                          aria-label={`Annotate title: ${element.title}`}
                          onpointermove={(event) => {
                            if (!commentMode || commentPanelOpen) return;
                            previewLightElement(element.id, event.currentTarget);
                          }}
                          onclick={(event) => {
                            if (!commentMode) return;
                            event.preventDefault();
                            event.stopPropagation();
                            const target = event.currentTarget;
                            selectElement(element.id, true, {
                              ...annotationForElement("cardTitle", "Card title", ["card", "title"], target),
                            }, anchorFromElement(target));
                          }}
                        >
                          {element.title}
                        </button>
                        <span class="mt-0.5 block truncate text-xs text-muted-foreground-1">
                          {element.id} · {element.status}
                        </span>
                      </span>
                      <button
                        type="button"
                        data-capsem-role="card-component"
                        class={commentMode
                          ? "shrink-0 rounded-full bg-card px-2.5 py-1 text-xs font-medium text-muted-foreground-1 focus:outline-hidden focus:ring-2 focus:ring-primary/40"
                          : "shrink-0 rounded-full bg-card px-2.5 py-1 text-xs font-medium text-muted-foreground-1"}
                        aria-label={`Annotate whole card: ${element.title}`}
                        onpointermove={(event) => {
                          if (!commentMode || commentPanelOpen) return;
                          previewLightElement(element.id, event.currentTarget);
                        }}
                        onclick={(event) => {
                          if (!commentMode) return;
                          event.preventDefault();
                          const target = event.currentTarget;
                          selectElement(element.id, true, {
                            ...annotationForElement("artifact", "Whole card", ["artifact"], target),
                          }, anchorFromElement(target));
                        }}
                      >
                        {componentFor(element)}
                      </button>
                    </div>
                    <div
                      class="p-4"
                      role="button"
                      tabindex="0"
                      aria-label={`Select ${element.title} body for feedback`}
                      aria-pressed={element.id === selectedId}
                      onpointermove={(event) => {
                        if (!commentMode || commentPanelOpen) return;
                        if (event.target !== event.currentTarget) return;
                        previewLightElement(element.id, event.currentTarget);
                      }}
                      onclick={(event) => {
                        if (!commentMode) return;
                        event.preventDefault();
                        const target = event.currentTarget;
                        selectElement(element.id, true, {
                          ...annotationForElement("artifact", "Whole card", ["artifact"], target),
                        }, anchorFromElement(target));
                      }}
                      onkeydown={(event) => handleElementKeydown(event, element.id)}
                    >
                      <svelte:element
                        this={componentFor(element)}
                        data-capsem-annotate-mode={commentMode ? "true" : "false"}
                        use:attachAnnotate={element}
                        use:attachRenderTelemetry={element}
                        use:setSpec={artifactFor(element)}
                      ></svelte:element>
                    </div>
                    {#if !commentMode && element.id === selectedId}
                      <div class="border-t border-primary/30 bg-primary/5 px-4 py-2 text-xs font-medium text-primary">
                        Selected · {selectedAnnotationLabel}
                      </div>
                    {/if}
                  </div>
                {/each}
              </div>
            {/if}

            {#if feedbackTasks.length > 0}
              <section class="mt-4 rounded-xl border border-card-line bg-surface p-3" aria-label="Comment tasks">
                <div class="mb-3 flex items-center justify-between gap-3">
                  <h2 class="text-sm font-semibold text-foreground">Comments</h2>
                  <span class="rounded-full bg-layer px-2 py-0.5 text-xs font-medium text-layer-foreground">
                    {feedbackTasks.filter((item) => item.status === "pending").length} open
                  </span>
                </div>
                <div class="space-y-2">
                  {#each feedbackTasks as item (item.seq)}
                    <article class="rounded-lg border border-card-line bg-card px-3 py-2 shadow-2xs">
                      <div class="flex items-start justify-between gap-3">
                        <div class="min-w-0">
                          <p class="text-sm font-medium text-foreground">{item.instruction}</p>
                          <p class="mt-0.5 truncate text-xs text-muted-foreground-1">
                            {item.label} · seq {item.seq}
                          </p>
                        </div>
                        <span class={item.status === "resolved"
                          ? "shrink-0 rounded-full bg-success px-2 py-0.5 text-xs font-medium text-success-foreground"
                          : "shrink-0 rounded-full bg-warning px-2 py-0.5 text-xs font-medium text-warning-foreground"}>
                          {item.status}
                        </span>
                      </div>
                      <div class="mt-2 flex items-center justify-end gap-2">
                        {#if item.status === "pending"}
                          <button
                            type="button"
                            class="inline-flex items-center gap-x-2 rounded-lg border border-success bg-success px-2.5 py-1.5 text-xs font-medium text-success-foreground shadow-2xs hover:opacity-90 focus:outline-hidden"
                            onclick={() => resolveFeedbackKey(feedbackKey(item))}
                          >
                            <svg class="size-3.5 shrink-0" xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.25" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">
                              <path d="M20 6 9 17l-5-5"></path>
                            </svg>
                            Resolve
                          </button>
                        {/if}
                      </div>
                    </article>
                  {/each}
                </div>
              </section>
            {/if}
          </div>
        </div>
      </div>
    </div>

    <footer class="border-t border-card-line bg-card px-4 py-3 text-xs text-muted-foreground-1">
      Use Comment mode to inspect an element, capture its selector, and leave feedback for the AI.
    </footer>
  </section>

  <button
    type="button"
    class={commentMode
      ? "fixed bottom-4 start-4 z-50 inline-flex items-center gap-x-2 rounded-full border border-primary-line bg-primary px-4 py-3 text-sm font-semibold text-primary-foreground shadow-2xl hover:bg-primary-hover focus:bg-primary-focus focus:outline-hidden"
      : "fixed bottom-4 start-4 z-50 inline-flex items-center gap-x-2 rounded-full border border-layer-line bg-layer px-4 py-3 text-sm font-semibold text-layer-foreground shadow-2xl hover:bg-layer-hover focus:bg-layer-focus focus:outline-hidden"}
    aria-pressed={commentMode}
    onclick={toggleCommentMode}
  >
    <svg class="size-4 shrink-0" xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="currentColor" aria-hidden="true">
      <path d="M6.75 4A4.75 4.75 0 0 0 2 8.75v5.5A4.75 4.75 0 0 0 6.75 19H8v2.25a.75.75 0 0 0 1.18.61L13.25 19h4A4.75 4.75 0 0 0 22 14.25v-5.5A4.75 4.75 0 0 0 17.25 4H6.75Z"></path>
    </svg>
    Comment
  </button>

  {#if commentMode && !commentPanelOpen}
    <div class="fixed bottom-20 start-4 z-50 max-w-xs rounded-xl border border-overlay-line bg-overlay px-4 py-3 text-sm font-medium text-foreground shadow-2xl">
      Click any rendered element to capture its selector.
    </div>
  {/if}

  {#if commentMode && !commentPanelOpen && previewAnnotationAnchor}
    <div class="pointer-events-none fixed inset-0 z-40" aria-hidden="true">
      <div
        class="fixed rounded-sm border-2 border-primary bg-primary/10 shadow-[0_0_0_1px_var(--primary)]"
        style={previewBoxStyle}
      ></div>
    </div>
  {/if}

  {#each commentMarkers as marker (marker.key)}
    <button
      type="button"
      class={marker.status === "resolved"
        ? "fixed z-50 inline-flex size-7 items-center justify-center rounded-full border border-success bg-success text-xs font-bold text-success-foreground shadow-lg ring-4 ring-success/15 focus:outline-hidden"
        : "fixed z-50 inline-flex size-7 items-center justify-center rounded-full border border-primary-line bg-layer text-xs font-bold text-primary shadow-lg ring-4 ring-primary/15 hover:bg-layer-hover focus:outline-hidden focus:ring-4 focus:ring-primary/30"}
      style={markerStyle(marker)}
      aria-label={`Open feedback marker: ${marker.label}`}
      title={marker.selector ?? marker.label}
      onclick={() => {
        if (marker.status === "pending") reopenFeedbackMarker(marker);
      }}
    >
      {#if marker.status === "resolved"}
        <svg class="size-4" xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.5" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">
          <path d="M20 6 9 17l-5-5"></path>
        </svg>
      {:else}
        {marker.count}
      {/if}
    </button>
  {/each}

  {#if selected && commentPanelOpen}
    {#if annotationAnchor}
      <div class="pointer-events-none fixed inset-0 z-40" aria-hidden="true">
        <div
          class="fixed rounded-sm border-2 border-primary bg-primary/10"
          style={inspectorBoxStyle}
        ></div>
        <div
          class="fixed flex size-8 items-center justify-center rounded-full bg-primary text-primary-foreground shadow-lg"
          style={commentPointerStyle}
        >
          <svg class="size-4" xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="currentColor" aria-hidden="true">
            <path d="M6.75 4A4.75 4.75 0 0 0 2 8.75v5.5A4.75 4.75 0 0 0 6.75 19H8v2.25a.75.75 0 0 0 1.18.61L13.25 19h4A4.75 4.75 0 0 0 22 14.25v-5.5A4.75 4.75 0 0 0 17.25 4H6.75Z"></path>
          </svg>
        </div>
      </div>
    {/if}

    <aside
      class={annotationAnchor
        ? "fixed z-50"
        : "fixed inset-x-3 bottom-3 z-50 sm:inset-x-auto sm:end-4 sm:w-[26rem]"}
      style={commentPanelStyle}
      aria-label="Comment panel"
    >
      <div class="rounded-xl border border-overlay-line bg-overlay p-3 shadow-xl">
        <div class="space-y-3">
          <div class="rounded-lg border border-input-line bg-input shadow-2xs focus-within:border-primary focus-within:ring-1 focus-within:ring-primary">
            <textarea
              id="comment-feedback-text"
              class="block min-h-24 w-full resize-none rounded-lg border-0 bg-transparent px-3 py-3 text-sm text-input-foreground outline-none placeholder:text-muted-foreground-1 focus:ring-0 disabled:pointer-events-none disabled:opacity-50"
              rows="4"
              bind:this={feedbackInput}
              bind:value={changeText}
              placeholder="What should change?"
              disabled={changePending}
              onkeydown={(event) => {
                if (event.key === "Enter" && (event.metaKey || event.ctrlKey)) {
                  event.preventDefault();
                  submitChangeRequest();
                }
              }}
            ></textarea>
          </div>
          <div class="flex items-center justify-end gap-x-2">
            {#if activeFeedbackKey}
              <button
                type="button"
                class="me-auto inline-flex items-center gap-x-2 rounded-lg border border-success bg-success px-3 py-2 text-sm font-medium text-success-foreground shadow-2xs hover:opacity-90 focus:outline-hidden disabled:pointer-events-none disabled:opacity-50"
                disabled={changePending}
                onclick={resolveFeedbackMarker}
              >
                <svg class="size-4 shrink-0" xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.25" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">
                  <path d="M20 6 9 17l-5-5"></path>
                </svg>
                Resolve
              </button>
            {/if}
            <button
              type="button"
              class="inline-flex items-center gap-x-2 rounded-lg border border-layer-line bg-layer px-3 py-2 text-sm font-medium text-layer-foreground shadow-2xs hover:bg-layer-hover focus:bg-layer-focus focus:outline-hidden disabled:pointer-events-none disabled:opacity-50"
              onclick={closeCommentPanel}
            >
              Cancel
            </button>
            <button
              type="button"
              class="inline-flex items-center gap-x-2 rounded-lg border border-primary-line bg-primary px-3 py-2 text-sm font-medium text-primary-foreground hover:bg-primary-hover focus:bg-primary-focus focus:outline-hidden disabled:pointer-events-none disabled:opacity-50"
              disabled={!changeText.trim() || changePending}
              onclick={submitChangeRequest}
            >
              <svg class="size-4 shrink-0" xmlns="http://www.w3.org/2000/svg" width="24" height="24" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
                <path d="m22 2-7 20-4-9-9-4Z"></path>
                <path d="M22 2 11 13"></path>
              </svg>
              Comment
            </button>
          </div>
        </div>
      </div>
    </aside>
  {/if}
</main>

<style>
  :global(.capsem-commenting .capsem-annotatable-surface),
  :global(.capsem-commenting .capsem-annotatable-surface button),
  :global(.capsem-commenting .capsem-annotatable-surface [role="button"]) {
    cursor: url("data:image/svg+xml,%3Csvg xmlns='http://www.w3.org/2000/svg' width='24' height='24' viewBox='0 0 24 24' fill='%232563eb'%3E%3Cpath d='M6.75 4A4.75 4.75 0 0 0 2 8.75v5.5A4.75 4.75 0 0 0 6.75 19H8v2.25a.75.75 0 0 0 1.18.61L13.25 19h4A4.75 4.75 0 0 0 22 14.25v-5.5A4.75 4.75 0 0 0 17.25 4H6.75Z'/%3E%3C/svg%3E") 4 4, cell;
  }
</style>
