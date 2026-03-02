// Sidebar state: active view + sub-section tracking.
import type { ViewName, AnalyticsSection, SettingsSection } from '../types';

class SidebarStore {
  activeView = $state<ViewName>('terminal');
  analyticsSection = $state<AnalyticsSection>('dashboard');
  settingsSection = $state<SettingsSection>('');

  setView(view: ViewName) {
    this.activeView = view;
  }

  setAnalyticsSection(section: AnalyticsSection) {
    this.analyticsSection = section;
  }

  setSettingsSection(section: SettingsSection) {
    this.settingsSection = section;
  }
}

export const sidebarStore = new SidebarStore();
