export type TabView = 'new-tab' | 'overview' | 'terminal' | 'stats' | 'files' | 'logs' | 'inspector' | 'settings';

export interface Tab {
  id: string;
  title: string;
  subtitle?: string;
  view: TabView;
  vmId?: string;
}

let nextId = 0;
function genId(): string {
  return `tab-${++nextId}`;
}

function createTab(view: TabView = 'new-tab', title?: string, vmId?: string): Tab {
  return {
    id: genId(),
    title: title ?? 'Dashboard',
    view,
    vmId,
  };
}

class TabStore {
  tabs = $state<Tab[]>([createTab()]);
  activeId = $state<string>(this.tabs[0].id);

  get active(): Tab | undefined {
    return this.tabs.find(t => t.id === this.activeId);
  }

  get activeIndex(): number {
    return this.tabs.findIndex(t => t.id === this.activeId);
  }

  add(view: TabView = 'new-tab', title?: string, vmId?: string) {
    const tab = createTab(view, title, vmId);
    this.tabs.push(tab);
    this.activeId = tab.id;
  }

  close(id: string) {
    const idx = this.tabs.findIndex(t => t.id === id);
    if (idx === -1 || this.tabs.length === 1) return;

    const wasActive = id === this.activeId;
    this.tabs.splice(idx, 1);

    if (wasActive) {
      const newIdx = Math.min(idx, this.tabs.length - 1);
      this.activeId = this.tabs[newIdx].id;
    }
  }

  activate(id: string) {
    if (this.tabs.some(t => t.id === id)) {
      this.activeId = id;
    }
  }

  activateByIndex(index: number) {
    const clamped = Math.max(0, Math.min(index, this.tabs.length - 1));
    this.activeId = this.tabs[clamped].id;
  }

  next() {
    const idx = this.activeIndex;
    this.activateByIndex((idx + 1) % this.tabs.length);
  }

  prev() {
    const idx = this.activeIndex;
    this.activateByIndex((idx - 1 + this.tabs.length) % this.tabs.length);
  }

  reorder(fromIndex: number, toIndex: number) {
    if (fromIndex === toIndex) return;
    const [tab] = this.tabs.splice(fromIndex, 1);
    this.tabs.splice(toIndex, 0, tab);
  }

  updateTitle(id: string, title: string) {
    const tab = this.tabs.find(t => t.id === id);
    if (tab) tab.title = title;
  }

  updateSubtitle(id: string, subtitle: string) {
    const tab = this.tabs.find(t => t.id === id);
    if (tab) tab.subtitle = subtitle;
  }

  updateView(id: string, view: TabView) {
    const tab = this.tabs.find(t => t.id === id);
    if (tab) tab.view = view;
  }

  openSingleton(view: TabView, title: string) {
    const existing = this.tabs.find(t => t.view === view && !t.vmId);
    if (existing) {
      this.activeId = existing.id;
    } else {
      this.add(view, title);
    }
  }

  openVM(vmId: string, vmName: string) {
    const existing = this.tabs.find(t => t.vmId === vmId && t.view === 'terminal');
    if (existing) {
      this.activeId = existing.id;
    } else {
      this.add('terminal', vmName, vmId);
    }
  }
}

export const tabStore = new TabStore();
