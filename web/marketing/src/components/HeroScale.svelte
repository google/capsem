<script lang="ts">
  import { HERO_DEMO } from "$lib/data";
  let { active, terminal = false }: { active: number; terminal?: boolean } = $props();
  const copy = HERO_DEMO.scale;
</script>

<!-- Passive, compact rendering of Capsem's TabBar and Sessions view. -->
<div class="demo-scale" role="group" aria-label={copy.label}>
  <div class="demo-app-bar" aria-hidden="true">
    <span class="demo-traffic-lights"><i></i><i></i><i></i></span>
    <span class="demo-app-tab" class:selected={!terminal}>{copy.tab}</span>
    {#each copy.sessions.slice(0, 3) as session}
      <span class="demo-app-tab" class:selected={terminal && session.name === HERO_DEMO.tui.websiteTitle}>{session.name}</span>
    {/each}
    <span class="demo-app-plus">+</span>
  </div>
  <div class="demo-app-toolbar" aria-hidden="true"><img src="/demo/dots-three-vertical.svg" alt="" />{#if terminal}<div class="demo-app-views">{#each copy.views as view, index}<span class:selected={index === 0}>{view}</span>{/each}</div><small>8.2K · $0.04</small>{/if}</div>
  {#if terminal}
    <div class="demo-gui-terminal"><pre>{copy.terminal}</pre></div>
  {:else}
  <div class="demo-scale-content">
    <header class="demo-scale-heading"><h3>{copy.tab}</h3><span class="demo-app-refresh" aria-hidden="true">{copy.refresh}</span></header>
    <table class="demo-sessions">
      <thead><tr>{#each copy.columns as column}<th scope="col">{column}</th>{/each}</tr></thead>
      <tbody>
        {#each copy.sessions as session, index}
          {@const running = index < active}
          <tr>
            <td><strong>{session.name}</strong></td>
            <td><span class="demo-session-status" class:booting={!running}>{running ? copy.running : copy.booting}</span></td>
            <td>{session.profile}</td>
            <td>{running ? session.uptime : "—"}</td>
            <td>{running ? session.tokens : "—"}</td>
            <td>{running ? session.cost : "—"}</td>
            <td aria-hidden="true"><span class="demo-session-actions">{#each ["pause", "stop", "git-fork", "trash"] as icon}<img src={`/demo/${icon}.svg`} alt="" />{/each}</span></td>
          </tr>
        {/each}
      </tbody>
    </table>
  </div>
  {/if}
</div>
