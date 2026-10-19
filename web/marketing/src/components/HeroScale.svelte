<script lang="ts">
  import { HERO_DEMO } from "$lib/data";
  let { active, surface }: { active: number; surface: "gui" | "tui" | "api" | "mcp" } = $props();
  const copy = HERO_DEMO.scale;
</script>

<!-- Passive, compact rendering of Capsem's TabBar and Sessions view. -->
<div class="demo-scale" role="group" aria-label={copy.label}>
  <div class="demo-app-bar" aria-hidden="true">
    <span class="demo-traffic-lights"><i></i><i></i><i></i></span>
    <span class="demo-app-tab selected">{copy.tab}</span>
    {#each copy.sessions.slice(0, 3) as session}
      <span class="demo-app-tab">{session.name}</span>
    {/each}
    <span class="demo-app-plus">+</span>
  </div>
  <div class="demo-app-toolbar" aria-hidden="true"><span>⋮</span><img src="/logo.svg" alt="" /><strong>{HERO_DEMO.desktop.app}</strong><span class="demo-app-connected"></span></div>
  <div class="demo-scale-content" aria-hidden={surface !== "gui"}>
    <header class="demo-scale-heading"><div><h3>{copy.tab}</h3><p>{copy.detail}</p></div><span class="demo-app-new" aria-hidden="true">{copy.new}</span></header>
    <table class="demo-sessions">
      <thead><tr>{#each copy.columns as column}<th scope="col">{column}</th>{/each}</tr></thead>
      <tbody>
        {#each copy.sessions as session, index}
          {@const running = index < active}
          <tr>
            <td><strong>{session.name}</strong><small>{running ? session.task : session.agent}</small></td>
            <td><span class="demo-session-status" class:booting={!running}><i></i>{running ? copy.running : copy.booting}</span></td>
            <td><code>{session.image}</code></td>
            <td>{running ? session.uptime : "—"}</td>
            <td aria-hidden="true"><span class="demo-session-actions">Ⅱ <svg viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="1.3"><path d="M4 2v9a3 3 0 0 0 6 0V8M4 6h6V2"/><circle cx="4" cy="2" r="1.3"/><circle cx="10" cy="2" r="1.3"/></svg> ⋯</span></td>
          </tr>
        {/each}
      </tbody>
    </table>
    <footer class="demo-scale-footer"><span><i></i>{active ? `${active} ${copy.active}` : copy.queued}</span><span>{active} {copy.isolated}</span></footer>
  </div>
  {#if surface !== "gui"}
    <div class="demo-control-window" data-surface={surface}>
      <div class="demo-titlebar"><span class="demo-traffic-lights" aria-hidden="true"><i></i><i></i><i></i></span><span class="demo-window-title">{copy[surface].title}</span></div>
      {#if surface === "tui"}
        <div class="demo-tui">
          <div class="demo-tui-frame"><strong>{copy.tui.heading}</strong>
            <table><thead><tr>{#each copy.tui.columns as column}<th scope="col">{column}</th>{/each}</tr></thead><tbody>
              {#each copy.sessions as session, index}<tr class:selected={index === 0}><td>{index === 0 ? "▸ " : "  "}{session.name}</td><td>{session.image}</td><td>{copy.running}</td><td>{session.uptime}</td></tr>{/each}
            </tbody></table>
          </div>
          <footer>{copy.tui.shortcut}</footer>
        </div>
      {:else if surface === "api"}
        <div class="demo-api"><pre><code>{copy.api.code}</code></pre><p>{copy.api.endpoint}</p><strong>→ {copy.api.result}</strong></div>
      {:else}
        <div class="demo-mcp"><strong>&gt;_ {HERO_DEMO.codex.name}</strong><p>› {copy.mcp.prompt}</p><small>• {copy.mcp.call}</small><p class="demo-mcp-result">{copy.mcp.result}</p><p class="demo-mcp-detail">{copy.mcp.detail}</p></div>
      {/if}
    </div>
  {/if}
  <div class="demo-surfaces" role="group" aria-label={copy.controlsLabel}>
    {#each copy.surfaces as mode}<span class:current={surface === mode.id}>{mode.label}</span>{/each}
  </div>
</div>
