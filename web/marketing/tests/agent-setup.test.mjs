import assert from 'node:assert/strict';
import test from 'node:test';
import { setupLink, setupPrompt } from '../src/lib/agent-setup.ts';

test('agent handoffs preserve the complete prompt without submitting it', () => {
  for (const [agent, protocol, host, parameter] of [
    ['Claude Code', 'claude:', 'code', 'q'],
    ['Codex', 'codex:', 'new', 'prompt'],
    ['Cursor', 'https:', 'cursor.com', 'text'],
  ]) {
    const link = new URL(setupLink(agent));
    assert.equal(link.protocol, protocol);
    assert.equal(link.host, host);
    assert.equal(link.searchParams.get(parameter), setupPrompt(agent));
    assert.equal([...link.searchParams.keys()].length, 1);
  }
});

test('other agents use the copy fallback and version-aware setup instructions', () => {
  for (const agent of ['Gemini CLI', 'Antigravity', 'Ollama', 'Unknown']) {
    assert.equal(setupLink(agent), null);
    const prompt = setupPrompt(agent);
    assert.ok(prompt.includes(agent));
    assert.ok(prompt.includes('https://docs.capsem.org/getting-started/installation/'));
    assert.ok(prompt.includes('installed version'));
    assert.ok(prompt.includes('password and sign-in prompts'));
  }
});
