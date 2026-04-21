<script lang="ts">
  import InstallCommand from "./InstallCommand.svelte";
</script>

<section class="relative pt-32 pb-20 md:pt-44 md:pb-32 overflow-hidden">
  <div class="absolute inset-0 bg-[linear-gradient(to_right,#e5e7eb_1px,transparent_1px),linear-gradient(to_bottom,#e5e7eb_1px,transparent_1px)] bg-[size:4rem_4rem] opacity-40"></div>
  <div class="absolute inset-0 bg-[radial-gradient(ellipse_60%_50%_at_50%_-20%,var(--color-accent-glow),transparent)]"></div>

  <div class="relative mx-auto max-w-6xl px-6">
    <div class="max-w-3xl">
      <h1 class="text-5xl md:text-7xl font-extrabold tracking-tight text-heading leading-[1.08]">
        <span class="gradient-text">CAPSEM</span><br />
        The fastest way to<br />
        ship with AI securely.
      </h1>

      <p class="mt-6 text-lg md:text-xl text-body max-w-xl leading-relaxed">
        A Rust-powered hypervisor that sandboxes every AI coding agent
        in its own air-gapped Linux VM. See everything, control everything.
      </p>

      <div class="mt-10 flex flex-col gap-4 max-w-lg">
        <InstallCommand />
        <div class="flex flex-wrap gap-4">
          <a href="#features" class="btn-outline">Learn More</a>
        </div>
      </div>
    </div>
  </div>
</section>
