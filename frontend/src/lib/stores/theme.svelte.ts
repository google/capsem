// Light/dark theme store using Svelte 5 runes.

const STORAGE_KEY = 'capsem-theme';
type Theme = 'light' | 'dark';

class ThemeStore {
  theme = $state<Theme>('dark');

  init() {
    const stored = localStorage.getItem(STORAGE_KEY) as Theme | null;
    if (stored === 'light' || stored === 'dark') {
      this.theme = stored;
    } else if (
      typeof window !== 'undefined' &&
      window.matchMedia('(prefers-color-scheme: light)').matches
    ) {
      this.theme = 'light';
    }
    this.apply();
  }

  toggle() {
    this.theme = this.theme === 'dark' ? 'light' : 'dark';
    localStorage.setItem(STORAGE_KEY, this.theme);
    this.apply();
  }

  private apply() {
    if (typeof document !== 'undefined') {
      document.documentElement.setAttribute('data-theme', this.theme);
    }
  }
}

export const themeStore = new ThemeStore();
