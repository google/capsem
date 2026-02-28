// Sidebar state: active view + collapsed toggle.
import type { ViewName } from '../types';

class SidebarStore {
  activeView = $state<ViewName>('terminal');
  collapsed = $state(true);

  setView(view: ViewName) {
    this.activeView = view;
  }

  toggleCollapsed() {
    this.collapsed = !this.collapsed;
  }
}

export const sidebarStore = new SidebarStore();
