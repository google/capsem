<script lang="ts">
	import { invoke } from '@tauri-apps/api/core';
	import { listen } from '@tauri-apps/api/event';
	import { onMount } from 'svelte';

	const MAX_LINES = 5000;

	let vmStatus = $state('loading...');
	let serialLines = $state<string[]>([]);
	let consoleEl: HTMLPreElement | undefined = $state();

	function scrollToBottom() {
		if (consoleEl) {
			consoleEl.scrollTop = consoleEl.scrollHeight;
		}
	}

	function keyToBytes(e: KeyboardEvent): string | null {
		if (e.ctrlKey && e.key.length === 1) {
			const code = e.key.toLowerCase().charCodeAt(0) - 96;
			if (code >= 1 && code <= 26) {
				return String.fromCharCode(code);
			}
			return null;
		}
		switch (e.key) {
			case 'Enter':
				return '\r';
			case 'Backspace':
				return '\x7f';
			case 'Tab':
				return '\t';
			case 'Escape':
				return '\x1b';
			case 'ArrowUp':
				return '\x1b[A';
			case 'ArrowDown':
				return '\x1b[B';
			case 'ArrowRight':
				return '\x1b[C';
			case 'ArrowLeft':
				return '\x1b[D';
			case 'Home':
				return '\x1b[H';
			case 'End':
				return '\x1b[F';
			case 'Delete':
				return '\x1b[3~';
			default:
				if (e.key.length === 1) {
					return e.key;
				}
				return null;
		}
	}

	function handleKeydown(e: KeyboardEvent) {
		const bytes = keyToBytes(e);
		if (bytes !== null) {
			e.preventDefault();
			invoke('serial_input', { input: bytes });
		}
	}

	onMount(async () => {
		// Focus the console element for keyboard capture
		if (consoleEl) {
			consoleEl.focus();
		}

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

		// Listen for serial output events
		const unlisten = await listen<string>('serial-output', (event) => {
			serialLines.push(event.payload);
			if (serialLines.length > MAX_LINES) {
				serialLines.splice(0, serialLines.length - MAX_LINES);
			}
			// Scroll after DOM update
			requestAnimationFrame(scrollToBottom);
		});

		return () => {
			clearInterval(statusInterval);
			unlisten();
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

<div class="flex flex-col h-screen p-4 gap-4">
	<section class="rounded-lg border border-gray-800 bg-gray-900 p-4 shrink-0">
		<div class="flex items-center gap-3">
			<span class="inline-block h-3 w-3 rounded-full {statusColor}"></span>
			<span class="text-sm font-mono">VM: {vmStatus}</span>
		</div>
	</section>

	<section class="rounded-lg border border-gray-800 bg-black flex-1 min-h-0 flex flex-col">
		<div class="px-4 py-2 border-b border-gray-800 text-xs text-gray-500 font-mono shrink-0">
			Serial Console (click to focus)
		</div>
		<pre
			bind:this={consoleEl}
			tabindex="0"
			role="textbox"
			aria-label="Serial console"
			onkeydown={handleKeydown}
			class="flex-1 overflow-y-auto p-4 text-sm font-mono text-green-400 leading-relaxed whitespace-pre-wrap outline-none focus:ring-1 focus:ring-green-800"
		>{#if serialLines.length === 0}<span class="text-gray-600">Waiting for serial output...</span>{:else}{serialLines.join('\n')}{/if}</pre>
	</section>
</div>
