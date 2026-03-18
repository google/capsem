<script lang="ts">
  import type { DetailSelection } from '../../types';

  let { selection, onClose }: {
    selection: DetailSelection;
    onClose: () => void;
  } = $props();

  /** Simple JSON syntax highlighter -- returns HTML with colored spans. */
  function highlightJson(text: string): string {
    let formatted: string;
    try {
      formatted = JSON.stringify(JSON.parse(text), null, 2);
    } catch {
      formatted = text;
    }
    const escaped = formatted
      .replace(/&/g, '&amp;')
      .replace(/</g, '&lt;')
      .replace(/>/g, '&gt;');
    return escaped
      .replace(
        /("(?:[^"\\]|\\.)*")(\s*:)?/g,
        (_, str, colon) => {
          if (colon) return `<span class="json-key">${str}</span>${colon}`;
          return `<span class="json-string">${str}</span>`;
        },
      )
      .replace(/\b(true|false|null)\b/g, '<span class="json-bool">$1</span>')
      .replace(/\b(-?\d+(?:\.\d+)?(?:[eE][+-]?\d+)?)\b/g, '<span class="json-number">$1</span>');
  }

  function tryHighlight(text: string | null | undefined): string {
    if (!text) return '<span class="text-base-content/30">(empty)</span>';
    // Try JSON-highlighting; if it starts with { or [, assume JSON.
    const trimmed = text.trim();
    if (trimmed.startsWith('{') || trimmed.startsWith('[')) {
      return highlightJson(trimmed);
    }
    return trimmed.replace(/&/g, '&amp;').replace(/</g, '&lt;').replace(/>/g, '&gt;');
  }

  const d = $derived(selection.data);
</script>

<div class="card w-[400px] flex-shrink-0 border-l border-base-300 flex flex-col overflow-hidden bg-base-100 rounded-none">
  <!-- Header -->
  <div class="flex items-center gap-2 px-3 py-2 border-b border-base-300 bg-base-200/40">
    <span class="text-xs font-semibold flex-1 truncate capitalize">{selection.type.replace('_', ' ')}</span>
    <button class="btn btn-ghost btn-xs" onclick={onClose} aria-label="Close detail panel">
      <svg class="size-3.5" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2"><line x1="18" y1="6" x2="6" y2="18"/><line x1="6" y1="6" x2="18" y2="18"/></svg>
    </button>
  </div>

  <!-- Content -->
  <div class="flex-1 overflow-auto p-3 text-xs space-y-3">
    {#if selection.type === 'thinking'}
      <div>
        <div class="text-[10px] font-semibold text-base-content/40 uppercase tracking-wider mb-1">Thinking</div>
        <pre class="font-mono text-xs whitespace-pre-wrap break-words leading-relaxed bg-base-200/30 rounded p-2 max-h-[80vh] overflow-auto">{d.thinking_content ?? '(empty)'}</pre>
      </div>

    {:else if selection.type === 'text'}
      <div>
        <div class="text-[10px] font-semibold text-base-content/40 uppercase tracking-wider mb-1">Response</div>
        <pre class="font-mono text-xs whitespace-pre-wrap break-words leading-relaxed bg-base-200/30 rounded p-2 max-h-[80vh] overflow-auto">{d.text_content ?? '(empty)'}</pre>
      </div>

    {:else if selection.type === 'model_input'}
      <div class="space-y-1 mb-2">
        <div><span class="text-base-content/40">Messages:</span> <span class="font-mono">{d.messages_count ?? '?'}</span></div>
        <div><span class="text-base-content/40">Tools:</span> <span class="font-mono">{d.tools_count ?? 0}</span></div>
      </div>
      {#if d.system_prompt_preview}
        <div>
          <div class="text-[10px] font-semibold text-base-content/40 uppercase tracking-wider mb-1">System Prompt</div>
          <pre class="font-mono text-xs whitespace-pre-wrap break-words leading-relaxed bg-base-200/30 rounded p-2 overflow-auto max-h-64">{d.system_prompt_preview}</pre>
        </div>
      {/if}
      {#if d.request_body_preview}
        <div>
          <div class="text-[10px] font-semibold text-base-content/40 uppercase tracking-wider mb-1">Request Body</div>
          <pre class="font-mono text-xs whitespace-pre-wrap break-words leading-relaxed bg-base-200/30 rounded p-2 overflow-auto max-h-[60vh]">{@html tryHighlight(d.request_body_preview as string)}</pre>
        </div>
      {/if}

    {:else if selection.type === 'tool'}
      <div>
        <div class="text-[10px] font-semibold text-base-content/40 uppercase tracking-wider mb-1">
          {d.tool_name ?? 'Tool'}
          {#if d.origin && d.origin !== 'native'}
            <span class="badge badge-xs badge-outline ml-1">{d.origin}</span>
          {/if}
        </div>
      </div>
      <div>
        <div class="text-[10px] font-semibold text-base-content/40 uppercase tracking-wider mb-1">Arguments</div>
        <pre class="font-mono text-xs whitespace-pre-wrap break-words leading-relaxed bg-base-200/30 rounded p-2 overflow-auto max-h-64">{@html tryHighlight(d.arguments as string)}</pre>
      </div>
      {#if d.content_preview !== undefined}
        <div>
          <div class="text-[10px] font-semibold text-base-content/40 uppercase tracking-wider mb-1">
            Result
            {#if d.is_error}
              <span class="badge badge-xs bg-denied/15 text-denied ml-1">error</span>
            {/if}
          </div>
          <pre class="font-mono text-xs whitespace-pre-wrap break-words leading-relaxed bg-base-200/30 rounded p-2 overflow-auto max-h-64">{@html tryHighlight(d.content_preview as string)}</pre>
        </div>
      {/if}

    {:else if selection.type === 'net_event'}
      <div>
        <div class="text-[10px] font-semibold text-base-content/40 uppercase tracking-wider mb-1">Request</div>
        <div class="space-y-1">
          <div><span class="text-base-content/40">Method:</span> <span class="font-mono">{d.method ?? 'CONNECT'}</span></div>
          <div><span class="text-base-content/40">Domain:</span> <span class="font-mono">{d.domain}</span></div>
          <div><span class="text-base-content/40">Path:</span> <span class="font-mono">{d.path ?? '/'}</span></div>
          {#if d.query}
            <div><span class="text-base-content/40">Query:</span> <span class="font-mono">{d.query}</span></div>
          {/if}
          <div>
            <span class="text-base-content/40">Decision:</span>
            <span class="badge badge-xs {d.decision === 'allowed' ? 'bg-allowed/15 text-allowed' : 'bg-denied/15 text-denied'}">{d.decision}</span>
          </div>
          {#if d.status_code}
            <div><span class="text-base-content/40">Status:</span> <span class="font-mono">{d.status_code}</span></div>
          {/if}
          {#if d.duration_ms}
            <div><span class="text-base-content/40">Duration:</span> <span class="font-mono">{d.duration_ms}ms</span></div>
          {/if}
          {#if d.matched_rule}
            <div><span class="text-base-content/40">Rule:</span> <span class="font-mono">{d.matched_rule}</span></div>
          {/if}
        </div>
      </div>
      {#if d.request_headers}
        <div>
          <div class="text-[10px] font-semibold text-base-content/40 uppercase tracking-wider mb-1">Request Headers</div>
          <pre class="font-mono text-xs whitespace-pre-wrap break-words leading-relaxed bg-base-200/30 rounded p-2 overflow-auto max-h-40">{@html tryHighlight(d.request_headers as string)}</pre>
        </div>
      {/if}
      {#if d.request_body_preview}
        <div>
          <div class="text-[10px] font-semibold text-base-content/40 uppercase tracking-wider mb-1">Request Body</div>
          <pre class="font-mono text-xs whitespace-pre-wrap break-words leading-relaxed bg-base-200/30 rounded p-2 overflow-auto max-h-40">{@html tryHighlight(d.request_body_preview as string)}</pre>
        </div>
      {/if}
      {#if d.response_headers}
        <div>
          <div class="text-[10px] font-semibold text-base-content/40 uppercase tracking-wider mb-1">Response Headers</div>
          <pre class="font-mono text-xs whitespace-pre-wrap break-words leading-relaxed bg-base-200/30 rounded p-2 overflow-auto max-h-40">{@html tryHighlight(d.response_headers as string)}</pre>
        </div>
      {/if}
      {#if d.response_body_preview}
        <div>
          <div class="text-[10px] font-semibold text-base-content/40 uppercase tracking-wider mb-1">Response Body</div>
          <pre class="font-mono text-xs whitespace-pre-wrap break-words leading-relaxed bg-base-200/30 rounded p-2 overflow-auto max-h-40">{@html tryHighlight(d.response_body_preview as string)}</pre>
        </div>
      {/if}

    {:else if selection.type === 'mcp_call'}
      <div class="space-y-1">
        <div><span class="text-base-content/40">Server:</span> <span class="font-mono">{d.server_name}</span></div>
        <div><span class="text-base-content/40">Method:</span> <span class="font-mono">{d.method}</span></div>
        {#if d.tool_name}
          <div><span class="text-base-content/40">Tool:</span> <span class="font-mono">{d.tool_name}</span></div>
        {/if}
        <div>
          <span class="text-base-content/40">Decision:</span>
          <span class="badge badge-xs {d.decision === 'allowed' ? 'bg-allowed/15 text-allowed' : 'bg-denied/15 text-denied'}">{d.decision}</span>
        </div>
        {#if d.duration_ms}
          <div><span class="text-base-content/40">Duration:</span> <span class="font-mono">{d.duration_ms}ms</span></div>
        {/if}
        {#if d.error_message}
          <div><span class="text-base-content/40">Error:</span> <span class="text-denied">{d.error_message}</span></div>
        {/if}
      </div>
      {#if d.request_preview}
        <div>
          <div class="text-[10px] font-semibold text-base-content/40 uppercase tracking-wider mb-1">Request</div>
          <pre class="font-mono text-xs whitespace-pre-wrap break-words leading-relaxed bg-base-200/30 rounded p-2 overflow-auto max-h-48">{@html tryHighlight(d.request_preview as string)}</pre>
        </div>
      {/if}
      {#if d.response_preview}
        <div>
          <div class="text-[10px] font-semibold text-base-content/40 uppercase tracking-wider mb-1">Response</div>
          <pre class="font-mono text-xs whitespace-pre-wrap break-words leading-relaxed bg-base-200/30 rounded p-2 overflow-auto max-h-48">{@html tryHighlight(d.response_preview as string)}</pre>
        </div>
      {/if}

    {:else if selection.type === 'file_event'}
      <div class="space-y-1">
        <div><span class="text-base-content/40">Action:</span>
          <span class="badge badge-xs {d.action === 'deleted' ? 'bg-file-deleted/15 text-file-deleted' : d.action === 'created' ? 'bg-file-created/15 text-file-created' : 'bg-file-modified/15 text-file-modified'}">{d.action}</span>
        </div>
        <div><span class="text-base-content/40">Path:</span> <span class="font-mono break-all">{d.path}</span></div>
        {#if d.size != null}
          <div><span class="text-base-content/40">Size:</span> <span class="font-mono">{d.size} bytes</span></div>
        {/if}
        {#if d.timestamp}
          <div><span class="text-base-content/40">Time:</span> <span class="font-mono">{d.timestamp}</span></div>
        {/if}
      </div>
    {/if}
  </div>
</div>
