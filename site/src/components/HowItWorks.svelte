<section id="how-it-works" class="py-24 md:py-32 section-dark">
  <div class="mx-auto max-w-6xl px-6">
    <div class="text-center mb-16">
      <h2 class="text-4xl md:text-5xl font-extrabold tracking-tight text-heading-dark">
        How it works
      </h2>
      <p class="mt-4 text-lg text-body-dark max-w-2xl mx-auto">
        A native macOS hypervisor creates an air-gapped Linux VM for each session.
        All network traffic is forced through an inspecting proxy on the host.
      </p>
    </div>

    <!-- Architecture diagram -->
    <div class="rounded-2xl border border-border-dark bg-surface-dark-alt p-8 md:p-12">
      <div class="grid md:grid-cols-[1fr_auto_1fr] gap-8 items-stretch">

        <!-- Host side -->
        <div class="space-y-4">
          <div class="text-xs font-semibold uppercase tracking-wider text-muted-dark mb-2">macOS Host</div>

          <div class="rounded-xl border border-border-dark bg-surface-dark p-5">
            <div class="flex items-center gap-3 mb-3">
              <div class="h-8 w-8 rounded-lg bg-accent/15 flex items-center justify-center">
                <svg class="h-4 w-4 text-accent" fill="none" viewBox="0 0 24 24" stroke="currentColor" stroke-width="2"><rect x="2" y="3" width="20" height="14" rx="2" /><path d="M8 21h8M12 17v4" /></svg>
              </div>
              <div>
                <div class="text-sm font-semibold text-heading-dark">Tauri App</div>
                <div class="text-xs text-muted-dark">GUI + CLI interface</div>
              </div>
            </div>
          </div>

          <div class="rounded-xl border border-border-dark bg-surface-dark p-5">
            <div class="flex items-center gap-3 mb-3">
              <div class="h-8 w-8 rounded-lg bg-accent/15 flex items-center justify-center">
                <svg class="h-4 w-4 text-accent" fill="none" viewBox="0 0 24 24" stroke="currentColor" stroke-width="2"><path d="M12 22s8-4 8-10V5l-8-3-8 3v7c0 6 8 10 8 10z" /></svg>
              </div>
              <div>
                <div class="text-sm font-semibold text-heading-dark">MITM Proxy</div>
                <div class="text-xs text-muted-dark">TLS termination + HTTP inspection</div>
              </div>
            </div>
          </div>

          <div class="rounded-xl border border-border-dark bg-surface-dark p-5">
            <div class="flex items-center gap-3 mb-3">
              <div class="h-8 w-8 rounded-lg bg-accent/15 flex items-center justify-center">
                <svg class="h-4 w-4 text-accent" fill="none" viewBox="0 0 24 24" stroke="currentColor" stroke-width="2"><path d="M14 2H6a2 2 0 00-2 2v16a2 2 0 002 2h12a2 2 0 002-2V8z" /><path d="M14 2v6h6M16 13H8M16 17H8" /></svg>
              </div>
              <div>
                <div class="text-sm font-semibold text-heading-dark">Policy Engine</div>
                <div class="text-xs text-muted-dark">Domain + HTTP + MCP rules</div>
              </div>
            </div>
          </div>

          <div class="rounded-xl border border-border-dark bg-surface-dark p-5">
            <div class="flex items-center gap-3">
              <div class="h-8 w-8 rounded-lg bg-accent/15 flex items-center justify-center">
                <svg class="h-4 w-4 text-accent" fill="none" viewBox="0 0 24 24" stroke="currentColor" stroke-width="2"><path d="M18 20V10M12 20V4M6 20v-6" /></svg>
              </div>
              <div>
                <div class="text-sm font-semibold text-heading-dark">Session Telemetry</div>
                <div class="text-xs text-muted-dark">SQLite DB per session</div>
              </div>
            </div>
          </div>
        </div>

        <!-- Connection arrows -->
        <div class="hidden md:flex flex-col items-center justify-center gap-4 py-8">
          <div class="flex flex-col items-center gap-1">
            <span class="text-[10px] text-muted-dark font-mono">vsock</span>
            <div class="w-px h-6 bg-border-dark"></div>
            <span class="text-[10px] text-muted-dark font-mono">:5001</span>
            <svg class="h-4 w-4 text-accent" fill="none" viewBox="0 0 24 24" stroke="currentColor" stroke-width="2"><path stroke-linecap="round" stroke-linejoin="round" d="M8 7l4-4m0 0l4 4m-4-4v18" /><path stroke-linecap="round" stroke-linejoin="round" d="M16 17l-4 4m0 0l-4-4" /></svg>
            <span class="text-[10px] text-muted-dark font-mono">terminal</span>
          </div>
          <div class="w-px h-4 bg-border-dark"></div>
          <div class="flex flex-col items-center gap-1">
            <span class="text-[10px] text-muted-dark font-mono">:5002</span>
            <svg class="h-4 w-4 text-accent-secondary" fill="none" viewBox="0 0 24 24" stroke="currentColor" stroke-width="2"><path stroke-linecap="round" stroke-linejoin="round" d="M8 7l4-4m0 0l4 4m-4-4v18" /><path stroke-linecap="round" stroke-linejoin="round" d="M16 17l-4 4m0 0l-4-4" /></svg>
            <span class="text-[10px] text-muted-dark font-mono">HTTPS</span>
          </div>
          <div class="w-px h-4 bg-border-dark"></div>
          <div class="flex flex-col items-center gap-1">
            <span class="text-[10px] text-muted-dark font-mono">:5003</span>
            <svg class="h-4 w-4 text-accent-secondary" fill="none" viewBox="0 0 24 24" stroke="currentColor" stroke-width="2"><path stroke-linecap="round" stroke-linejoin="round" d="M8 7l4-4m0 0l4 4m-4-4v18" /><path stroke-linecap="round" stroke-linejoin="round" d="M16 17l-4 4m0 0l-4-4" /></svg>
            <span class="text-[10px] text-muted-dark font-mono">MCP</span>
          </div>
        </div>

        <!-- Guest side -->
        <div class="space-y-4">
          <div class="text-xs font-semibold uppercase tracking-wider text-muted-dark mb-2">Linux VM (air-gapped)</div>

          <div class="rounded-xl border border-border-dark bg-surface-dark p-5">
            <div class="flex items-center gap-3 mb-3">
              <div class="h-8 w-8 rounded-lg bg-accent-secondary/15 flex items-center justify-center">
                <svg class="h-4 w-4 text-accent-secondary" fill="none" viewBox="0 0 24 24" stroke="currentColor" stroke-width="2"><path d="M4 17l6-6-6-6M12 19h8" /></svg>
              </div>
              <div>
                <div class="text-sm font-semibold text-heading-dark">AI Agent</div>
                <div class="text-xs text-muted-dark">Claude / Gemini / Codex</div>
              </div>
            </div>
          </div>

          <div class="rounded-xl border border-border-dark bg-surface-dark p-5">
            <div class="flex items-center gap-3 mb-3">
              <div class="h-8 w-8 rounded-lg bg-accent-secondary/15 flex items-center justify-center">
                <svg class="h-4 w-4 text-accent-secondary" fill="none" viewBox="0 0 24 24" stroke="currentColor" stroke-width="2"><rect x="2" y="2" width="20" height="20" rx="2" /><path d="M7 2v20M2 7h5M2 12h5M2 17h5" /></svg>
              </div>
              <div>
                <div class="text-sm font-semibold text-heading-dark">PTY Agent</div>
                <div class="text-xs text-muted-dark">Terminal I/O over vsock</div>
              </div>
            </div>
          </div>

          <div class="rounded-xl border border-border-dark bg-surface-dark p-5">
            <div class="flex items-center gap-3 mb-3">
              <div class="h-8 w-8 rounded-lg bg-accent-secondary/15 flex items-center justify-center">
                <svg class="h-4 w-4 text-accent-secondary" fill="none" viewBox="0 0 24 24" stroke="currentColor" stroke-width="2"><path d="M12 2L2 7l10 5 10-5-10-5zM2 17l10 5 10-5M2 12l10 5 10-5" /></svg>
              </div>
              <div>
                <div class="text-sm font-semibold text-heading-dark">Net Proxy</div>
                <div class="text-xs text-muted-dark">TCP-to-vsock relay (iptables)</div>
              </div>
            </div>
          </div>

          <div class="rounded-xl border border-border-dark bg-surface-dark p-5">
            <div class="flex items-center gap-3">
              <div class="h-8 w-8 rounded-lg bg-accent-secondary/15 flex items-center justify-center">
                <svg class="h-4 w-4 text-accent-secondary" fill="none" viewBox="0 0 24 24" stroke="currentColor" stroke-width="2"><circle cx="12" cy="12" r="3" /><path d="M12 1v6M12 17v6M4.22 4.22l4.24 4.24M15.54 15.54l4.24 4.24M1 12h6M17 12h6M4.22 19.78l4.24-4.24M15.54 8.46l4.24-4.24" /></svg>
              </div>
              <div>
                <div class="text-sm font-semibold text-heading-dark">MCP Server</div>
                <div class="text-xs text-muted-dark">Tool relay over vsock</div>
              </div>
            </div>
          </div>
        </div>
      </div>

      <!-- Bottom: Internet -->
      <div class="mt-8 pt-8 border-t border-border-dark text-center">
        <div class="inline-flex items-center gap-3 rounded-xl border border-border-dark bg-surface-dark px-6 py-3">
          <svg class="h-5 w-5 text-muted-dark" fill="none" viewBox="0 0 24 24" stroke="currentColor" stroke-width="1.5"><circle cx="12" cy="12" r="10" /><path d="M2 12h20M12 2a15.3 15.3 0 014 10 15.3 15.3 0 01-4 10 15.3 15.3 0 01-4-10 15.3 15.3 0 014-10z" /></svg>
          <span class="text-sm text-muted-dark">Internet (via host MITM proxy only)</span>
        </div>
      </div>
    </div>
  </div>
</section>
