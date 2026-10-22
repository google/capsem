<script lang="ts">
  import { HERO_DEMO } from "$lib/data";
  let { overlay, workflow, email, parallel, draft, profile, started }: {
    overlay: "sessions" | "create" | null; workflow: "website" | "email";
    email: boolean; parallel: number; draft: string; profile: "code" | "co-work"; started: boolean;
  } = $props();
  const copy = HERO_DEMO.tui;
  let sessions = $derived(HERO_DEMO.scale.sessions.slice(0, parallel >= 0 ? parallel : email ? 2 : 1));
  let selected = $derived(sessions[workflow === "email" ? 1 : 0] ?? sessions[0]);
  let current = $derived(email && !started ? { ...selected, uptime: "0s", tokens: "0", cost: "$0.00" } : selected);
</script>

<!-- Reproduces capsem-tui/src/ui.rs: bottom status bar and centered overlays. -->
<div class="demo-tui-status" aria-hidden="true">
  <span>1ms <i>●</i></span>
  <div>{#each sessions.slice(0, 4) as session, index}<span class:current={index === (workflow === "email" ? 1 : 0)}><b>{index + 1}</b> {session.name}</span>{/each}{#if sessions.length > 4}<span>›</span>{/if}</div>
  <small>◷ {current.uptime} | # {current.tokens} | $ {current.cost.slice(1)} | {copy.help}</small>
</div>
{#if overlay}
  <div class="demo-tui-overlay" class:demo-tui-create={overlay === "create"} role="group" aria-label={overlay === "create" ? copy.create : copy.sessions}>
    <span class="demo-tui-overlay-title">{overlay === "create" ? copy.create : copy.sessions}</span>
    {#if overlay === "create"}
      <div class="demo-tui-brand">CAPSEM</div>
      <strong>{copy.create}</strong>
      <p class="demo-tui-name"><span>{copy.nameLabel}</span> <b>{draft}<i></i></b></p>
      <p class="demo-tui-hint">{copy.inputHint}</p>
      <p class="demo-tui-hint">{copy.createHint}</p>
      <strong class="demo-tui-profiles">{copy.profiles}</strong>
      <table><thead><tr>{#each copy.profileColumns as column}<th scope="col">{column}</th>{/each}</tr></thead><tbody>
        {#each copy.profileOptions as option}<tr class:selected={profile === option.id}><td>{profile === option.id ? "▶" : ""}</td><td>{option.id}</td><td>{option.name}</td></tr>{/each}
      </tbody></table>
    {:else}
      <strong>{copy.sessions}</strong>
      <table><thead><tr>{#each copy.columns as column}<th scope="col">{column}</th>{/each}</tr></thead><tbody>
        {#each sessions as session, index}
          {@const fresh = index === 1 && !started}
          <tr class:selected={index === (workflow === "email" ? 1 : 0)}><td>{index === (workflow === "email" ? 1 : 0) ? "▶" : ""} {index + 1}</td><td>{session.name}</td><td>{session.profile}</td><td>{HERO_DEMO.scale.running}</td><td>{fresh ? "0s" : session.uptime}</td><td>{fresh ? "0" : session.tokens}</td><td>{fresh ? "$0.00" : session.cost}</td></tr>
        {/each}
      </tbody></table>
    {/if}
  </div>
{/if}
