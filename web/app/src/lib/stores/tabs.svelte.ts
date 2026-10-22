import { getContainerStatus } from '../api';
import { defaultSessionView } from '../models/surface';

export type TabView = 'new-tab' | 'overview' | 'terminal' | 'ui' | 'stats' | 'files' | 'logs' | 'settings' | 'about';

export interface Tab {
  id: string;
  title: string;
  subtitle?: string;
  view: TabView;
  vmId?: string;
  surface?: boolean;
  presentationPending?: boolean;
  terminalOpened?: boolean;
  surfaceRevision?: number;
  surfaceDebug?: boolean;
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
  /** Tabs in creation order -- used for content rendering to avoid iframe DOM reorder (which reloads iframes). */
  stableTabs = $derived(
    [...this.tabs].sort((a, b) => {
      // Sort by numeric suffix of tab ID (creation order).
      const numA = parseInt(a.id.replace('tab-', ''), 10);
      const numB = parseInt(b.id.replace('tab-', ''), 10);
      return numA - numB;
    })
  );

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
    console.log('[tabs] add id=%s view=%s vmId=%s title=%s', tab.id, view, vmId, title);
  }

  close(id: string) {
    const idx = this.tabs.findIndex(t => t.id === id);
    if (idx === -1 || this.tabs.length === 1) return;

    const wasActive = id === this.activeId;
    this.tabs.splice(idx, 1);
    console.log('[tabs] close id=%s wasActive=%s remaining=%d', id, wasActive, this.tabs.length);

    if (wasActive) {
      const newIdx = Math.min(idx, this.tabs.length - 1);
      this.activeId = this.tabs[newIdx].id;
    }
  }

  activate(id: string) {
    if (this.tabs.some(t => t.id === id)) {
      this.activeId = id;
      console.log('[tabs] activate id=%s', id);
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
    console.log('[tabs] reorder from=%d to=%d id=%s', fromIndex, toIndex, tab.id);
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
    if (tab) {
      if (tab.view === 'terminal' && !tab.presentationPending) tab.terminalOpened = true;
      tab.view = view;
      tab.presentationPending = false;
      if (view === 'terminal') tab.terminalOpened = true;
    }
  }

  reloadSurface(id: string) {
    const tab = this.tabs.find(tab => tab.id === id);
    if (tab) tab.surfaceRevision = (tab.surfaceRevision ?? 0) + 1;
  }

  toggleSurfaceDebug(id: string) {
    const tab = this.tabs.find(tab => tab.id === id);
    if (tab) tab.surfaceDebug = !tab.surfaceDebug;
  }

  openSingleton(view: TabView, title: string) {
    const existing = this.tabs.find(t => t.view === view && !t.vmId);
    if (existing) {
      this.activeId = existing.id;
    } else {
      this.add(view, title);
    }
  }

  openVM(vmId: string, vmName: string, preferred?: 'ui' | 'terminal') {
    const existing = this.tabs.find(t => t.vmId === vmId);
    if (existing) {
      console.log('[tabs] openVM reuse existing tab=%s vmId=%s', existing.id, vmId);
      this.activeId = existing.id;
    } else {
      console.log('[tabs] openVM new tab vmId=%s name=%s', vmId, vmName);
      this.add(preferred ?? 'terminal', vmName, vmId);
      const tab = this.active!;
      tab.surface = preferred === 'ui';
      tab.terminalOpened = preferred === 'terminal';
      tab.presentationPending = preferred === undefined;
      if (preferred === undefined) {
        void getContainerStatus(vmId).then(status => {
          if (!this.tabs.some(current => current.id === tab.id)) return;
          const view = defaultSessionView(status);
          tab.surface = view === 'ui';
          if (tab.presentationPending) {
            tab.view = view;
            tab.terminalOpened = view === 'terminal';
          }
        }).catch(error => {
          console.error('[tabs] session presentation unavailable:', error);
          if (tab.presentationPending) tab.terminalOpened = true;
        }).finally(() => { tab.presentationPending = false; });
      }
    }
  }
}

export const tabStore = new TabStore();
