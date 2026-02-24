<script>
	import '../app.css';

	let { children } = $props();
</script>

<div class="min-h-screen bg-gray-950 text-gray-100">
	<header class="border-b border-gray-800 px-6 py-4">
		<h1 class="text-xl font-semibold">Capsem</h1>
	</header>

	<main class="p-6">
		{@render children()}
	</main>
</div>
