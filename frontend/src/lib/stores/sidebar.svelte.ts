// Sidebar state: active view + sub-section tracking.
import type { ViewName, SettingsSection } from '../types';

class SidebarStore {
  activeView = $state<ViewName>('terminal');
  settingsSection = $state<SettingsSection>('');

  setView(view: ViewName) {
    this.activeView = view;
  }

  setSettingsSection(section: SettingsSection) {
    this.settingsSection = section;
  }
}

export const sidebarStore = new SidebarStore();
