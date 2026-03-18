// Log store: live event stream + historical session log loading.
import { onLogEvent, loadSessionLog, listLogSessions } from '../api';
import type { LogEntry, LogLevel, LogSessionInfo } from '../types';

const MAX_ENTRIES = 2000;

const LEVEL_RANK: Record<string, number> = {
  error: 0,
  warn: 1,
  info: 2,
  debug: 3,
};

class LogsStore {
  entries = $state<LogEntry[]>([]);
  filterLevel = $state<LogLevel>('info');
  autoScroll = $state(true);
  selectedSession = $state<string | null>(null);
  sessions = $state<LogSessionInfo[]>([]);

  filteredEntries = $derived.by(() => {
    const threshold = LEVEL_RANK[this.filterLevel] ?? 2;
    return this.entries.filter((e) => {
      const rank = LEVEL_RANK[e.level.toLowerCase()] ?? 2;
      return rank <= threshold;
    });
  });

  errorCount = $derived(
    this.entries.filter((e) => e.level === 'ERROR').length,
  );

  hasErrors = $derived(this.errorCount > 0);

  async init() {
    await this.loadSessions();
    onLogEvent((entry) => {
      if (this.selectedSession !== null) return;
      this.entries.push(entry);
      if (this.entries.length > MAX_ENTRIES) {
        this.entries = this.entries.slice(-MAX_ENTRIES);
      }
    });
  }

  clear() {
    this.entries = [];
  }

  setFilter(level: LogLevel) {
    this.filterLevel = level;
  }

  async loadSessions() {
    try {
      this.sessions = await listLogSessions();
    } catch {
      this.sessions = [];
    }
  }

  async loadSession(id: string) {
    this.selectedSession = id;
    try {
      this.entries = await loadSessionLog(id);
    } catch {
      this.entries = [];
    }
  }

  switchToLive() {
    this.selectedSession = null;
    this.entries = [];
  }
}

export const logsStore = new LogsStore();
