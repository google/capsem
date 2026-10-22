import { DOCS } from "./data.ts";

export function setupPrompt(agent?: string): string {
  const goal = !agent || agent === "Cursor" ? "get started with an isolated AI environment" : `get ${agent} running in an isolated environment`;
  return `Help me install Capsem on this computer and ${goal}. Read ${DOCS.start} and ${DOCS.use} first. Check my operating system, architecture, and existing Capsem installation. Follow the current stable installation instructions, then verify capsem version and capsem status. Let me handle password and sign-in prompts. Use the documented image or profile options for the installed version to create my first environment. Keep existing environments and security settings intact. Show me how to reopen it and start working. If you cannot access this computer, give me the local steps instead.`;
}

// Prefill only; these documented routes never submit or execute the prompt.
// Claude: support.claude.com/en/articles/14729294-open-claude-desktop-with-a-link
// Codex: learn.chatgpt.com/docs/reference/commands#chats
// Cursor: cursor.com/docs/reference/deeplinks
export function setupLink(agent: string): string | null {
  const prompt = encodeURIComponent(setupPrompt(agent));
  switch (agent) {
    case "Claude Code": return `claude://code/new?q=${prompt}`;
    case "Codex": return `codex://new?prompt=${prompt}`;
    case "Cursor": return `https://cursor.com/link/prompt?text=${prompt}`;
    default: return null;
  }
}

