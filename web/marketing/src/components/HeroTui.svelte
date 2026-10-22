<script lang="ts">
  import { HERO_DEMO } from "$lib/data";
  let { overlay, workflow, count, activity, draft, image }: {
    overlay: "sessions" | "create" | null; workflow: "website" | "email";
    count: number; activity: number; draft: string; image: "plain" | "agy" | "codex";
  } = $props();
  const copy = HERO_DEMO.tui;
  let sessions = $derived(HERO_DEMO.scale.sessions.slice(0, count));
  let selected = $derived(sessions[workflow === "email" ? 1 : 0] ?? sessions[0]);
  let used = $derived((workflow === "email" ? 1 : 0) < activity);
</script>

<!-- OCI TUI: ui.rs home/status and ui/create.rs image catalog fields. -->
<div class="demo-tui-status" aria-hidden="true">
  <span>1ms <i>●</i></span>
  <div>{#each sessions.slice(0, 4) as session, index}<span class:current={index === (workflow === "email" ? 1 : 0)}><b>{index + 1}</b> {session.name}</span>{/each}{#if sessions.length > 4}<span>›</span>{/if}</div>
  <small>{#if selected}◷ {used ? selected.uptime : "0s"} | # {used ? selected.tokens : "0"} | $ {used ? selected.cost.slice(1) : "0.00"}{:else}{copy.noSession}{/if} | {copy.help}</small>
</div>
{#if !count && !overlay}
  <div class="demo-tui-empty"><span>{copy.empty}</span><strong>{copy.emptyHint}</strong></div>
{/if}
{#if overlay}
  <div class="demo-tui-overlay" class:demo-tui-create={overlay === "create"} role="group" aria-label={overlay === "create" ? copy.create : copy.sessions}>
    <span class="demo-tui-overlay-title">{overlay === "create" ? copy.create : copy.sessions}</span>
    {#if overlay === "create"}
      <div class="demo-tui-brand">CAPSEM</div>
      <strong>{copy.create}</strong>
      <p class="demo-tui-name"><span>{copy.nameLabel}</span> <b>{draft}<i></i></b></p>
      <p class="demo-tui-name"><span>{copy.imageLabel}</span> <b>{image === "plain" ? copy.plain : image}</b></p>
      {#if image !== "plain"}<p class="demo-tui-hint">{copy.compatibility}</p>
      <p class="demo-tui-hint">{copy.cache}</p>{/if}
      <p class="demo-tui-hint">{copy.imageHint}</p>
      <p class="demo-tui-hint">{copy.inputHint}</p>
      <p class="demo-tui-hint">{copy.createHint}</p>
    {:else}
      <strong>{copy.sessions}</strong>
      {#if !count}<p>{copy.empty}</p>{:else}
      <table><thead><tr>{#each copy.columns as column}<th scope="col">{column}</th>{/each}</tr></thead><tbody>
        {#each sessions as session, index}
          {@const fresh = index >= activity}
          <tr class:selected={index === (workflow === "email" ? 1 : 0)}><td>{index === (workflow === "email" ? 1 : 0) ? "▶" : ""} {index + 1}</td><td>{session.name}</td><td>{HERO_DEMO.scale.running}</td><td>{fresh ? "0s" : session.uptime}</td><td>{fresh ? "0" : session.tokens}</td><td>{fresh ? "$0.00" : session.cost}</td></tr>
        {/each}
      </tbody></table>
      {/if}
    {/if}
  </div>
{/if}
