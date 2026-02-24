<script lang="ts">
	import { invoke } from '@tauri-apps/api/core';
	import { listen } from '@tauri-apps/api/event';
	import { onMount } from 'svelte';
	import { Terminal } from '@xterm/xterm';
	import { FitAddon } from '@xterm/addon-fit';
	import '@xterm/xterm/css/xterm.css';

	let vmStatus = $state('loading...');
	let terminalContainer: HTMLDivElement | undefined = $state();
	let terminal: Terminal;
	let fitAddon: FitAddon;

	onMount(async () => {
		// Initialize Terminal
		terminal = new Terminal({
			cursorBlink: true,
			fontFamily: 'ui-monospace, SFMono-Regular, Menlo, Monaco, Consolas, "Liberation Mono", "Courier New", monospace',
			fontSize: 14,
			theme: {
				background: '#000000',
				foreground: '#4ade80', // green-400
			}
		});
		
		fitAddon = new FitAddon();
		terminal.loadAddon(fitAddon);

		if (terminalContainer) {
			terminal.open(terminalContainer);
			// Small delay to ensure container is rendered properly
			setTimeout(() => {
				fitAddon.fit();
			}, 10);
		}

		// Handle Terminal input (send to backend)
		const inputDisposable = terminal.onData((data) => {
			invoke('serial_input', { input: data });
		});

		// Handle window resize
		const handleResize = () => {
			if (fitAddon) {
				fitAddon.fit();
			}
		};
		window.addEventListener('resize', handleResize);

		// Poll VM status
		async function pollStatus() {
			try {
				vmStatus = await invoke('vm_status');
			} catch {
				vmStatus = 'error';
			}
		}
		await pollStatus();
		const statusInterval = setInterval(pollStatus, 2000);

		// Listen for serial output events (write to terminal)
		const unlisten = await listen<Uint8Array>('serial-output', (event) => {
			terminal.write(new Uint8Array(event.payload));
		});

		return () => {
			clearInterval(statusInterval);
			unlisten();
			inputDisposable.dispose();
			window.removeEventListener('resize', handleResize);
			terminal.dispose();
		};
	});

	const statusColor = $derived(
		vmStatus === 'running'
			? 'bg-green-500'
			: vmStatus === 'stopped'
				? 'bg-red-500'
				: vmStatus === 'not created'
					? 'bg-gray-500'
					: 'bg-yellow-500'
	);
</script>

<div class="flex flex-col h-screen p-4 gap-4 bg-black">
	<section class="rounded-lg border border-gray-800 bg-gray-900 p-4 shrink-0">
		<div class="flex items-center gap-3">
			<span class="inline-block h-3 w-3 rounded-full {statusColor}"></span>
			<span class="text-sm font-mono text-white">VM: {vmStatus}</span>
		</div>
	</section>

	<section class="rounded-lg border border-gray-800 bg-black flex-1 min-h-0 flex flex-col overflow-hidden">
		<div class="px-4 py-2 border-b border-gray-800 text-xs text-gray-500 font-mono shrink-0">
			Terminal
		</div>
		<div bind:this={terminalContainer} class="flex-1 w-full h-full p-2 overflow-hidden"></div>
	</section>
</div>

<style>
	/* Make the terminal container take full height/width and let fitAddon handle the rest */
	:global(.xterm) {
		height: 100%;
		padding: 0.5rem;
	}
	:global(.xterm-viewport) {
		/* Custom scrollbar for xterm */
		background-color: transparent !important;
	}
</style>