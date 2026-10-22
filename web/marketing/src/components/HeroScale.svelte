<script lang="ts">
  import { HERO_DEMO, HERO_AGY_BANNER, HERO_TERMINAL_SCRIPT } from "$lib/data";
  import { stepDuration } from "$lib/hero-terminal";
  import HeroAgentTerminal from "./HeroAgentTerminal.svelte";
  let { active, view, session, stage, input, draft, position } : {
    active: number; view: "sessions" | "create" | "boot" | "terminal"; session: number;
    stage: "shell" | "agent"; input: string | null; draft: string; position: number;
  } = $props();
  const copy = HERO_DEMO.scale;
  let clock = 0;
  const launches: number[] = [];
  for (const step of HERO_TERMINAL_SCRIPT) {
    if (step.launch !== undefined) launches[step.launch] = clock;
    clock += stepDuration(step);
  }
  let ages = $derived(copy.sessions.map((_, index) => position - (launches[index] ?? Infinity)));
  let metrics = $derived(ages.map((age, index) => {
    const tokens = [8200, 3100, 0, 0, 0, 0][index] + Math.max(0, Math.floor(age / 1000 * 180));
    return { tokens: tokens ? `${(tokens / 1000).toFixed(1)}K` : "0", cost: `$${(tokens * .000005).toFixed(2)}` };
  }));
  let current = $derived(copy.sessions[session]);
  let content = $derived.by(() => {
    if (stage === "shell") return "❯ ";
    const header = session === 0 ? `${HERO_AGY_BANNER}\r\n\r\n` : "";
    if (ages[session] < 0) return session === 0 ? `${header}${copy.websiteReady}` : session === 1 ? copy.emailReady : HERO_DEMO.codex.welcome;
    const lines = current.output.filter((_, index) => ages[session] >= [500, 1800, 4200, 9500, 18000][index]);
    const spinner = ["⠋", "⠙", "⠹", "⠸"][Math.floor(Math.max(0, ages[session]) / 140) % 4];
    return `${header}› ${current.prompt}\r\n\r\n${lines.map(line => `\x1b[38;5;111m●\x1b[0m ${line}`).join("\r\n")}\r\n\r\n${spinner} ${copy.working}`;
  });
</script>

<!-- Passive excerpts of TabBar, NewTabPage and CreateSandboxDialog. -->
<div class="demo-scale" role="group" aria-label={copy.label}>
  <div class="demo-app-bar" aria-hidden="true">
    <span class="demo-traffic-lights"><i></i><i></i><i></i></span>
    <span class="demo-app-tab" class:selected={view === "sessions" || view === "create"} data-gui-sessions>{copy.tab}</span>
    {#each copy.sessions.slice(0, active) as item, index}
      <span class="demo-app-tab" class:selected={(view === "terminal" || view === "boot") && session === index} data-session-tab={index}>{item.name}</span>
    {/each}
    <span class="demo-app-plus">+</span>
  </div>
  <div class="demo-app-toolbar" aria-hidden="true"><img src="/demo/dots-three-vertical.svg" alt="" />{#if view === "terminal"}<div class="demo-app-views">{#each copy.views as label, index}<span class:selected={index === 0}>{label}</span>{/each}</div><small>{metrics[session].tokens} · {metrics[session].cost}</small>{/if}</div>
  {#if view === "terminal"}
    {#key session}<HeroAgentTerminal {content} {input} agent={stage === "shell" ? "shell" : session === 0 ? "agy" : "codex"} directory={`~/${session === 1 ? "inbox" : current.name.toLowerCase()}`} />{/key}
  {:else if view === "boot"}
    <div class="demo-gui-boot"><strong>{current.name}</strong><p>{copy.boot}</p><span class="demo-boot-spinner" aria-hidden="true"></span></div>
  {:else}
    <div class="demo-scale-content">
      <header class="demo-scale-heading"><h3>{copy.tab}</h3><span class="demo-app-refresh" aria-hidden="true">{copy.refresh}</span></header>
      <h4 class="demo-profile-title">{copy.start}</h4>
      <div class="demo-profile-card"><div><strong>{copy.profileName}</strong><p>{copy.profileDescription}</p></div><span class="demo-app-create-button">{copy.new}</span><span class="demo-app-refresh" data-gui-customize>{copy.customize}</span></div>
      <table class="demo-sessions">
        <thead><tr>{#each copy.columns as column}<th scope="col">{column}</th>{/each}</tr></thead>
        <tbody>
          {#each copy.sessions.slice(0, active) as item, index}
            <tr><td><strong>{item.name}</strong></td><td><span class="demo-session-status">{copy.running}</span></td><td>{item.profile}</td><td>{Math.max(Number.parseInt(item.uptime), Math.floor(Math.max(0, ages[index]) / 1000))}s</td><td data-session-tokens={index}>{metrics[index].tokens}</td><td>{metrics[index].cost}</td><td aria-hidden="true"><span class="demo-session-actions">{#each ["pause", "stop", "git-fork", "trash"] as icon}<img src={`/demo/${icon}.svg`} alt="" />{/each}</span></td></tr>
          {/each}
        </tbody>
      </table>
    </div>
    {#if view === "create"}
      <div class="demo-gui-dialog-backdrop">
        <div class="demo-gui-dialog" role="group" aria-label={copy.dialog}>
          <h4>{copy.dialog}</h4>
          <span>{copy.profileLabel}</span><div class="demo-gui-field">{copy.profileName}<span>⌄</span></div>
          <span>{copy.nameLabel}</span><div class="demo-gui-field" data-gui-name>{draft}<i></i></div>
          <div class="demo-gui-resources"><div><span>{copy.ramLabel}</span><div class="demo-gui-field">2048 MB</div></div><div><span>{copy.cpuLabel}</span><div class="demo-gui-field">2</div></div></div>
          <footer><span class="demo-app-refresh">{copy.cancel}</span><span class="demo-app-create-button" data-gui-create>{copy.create}</span></footer>
        </div>
      </div>
    {/if}
  {/if}
</div>
