// Local UI preference, like appearance; never changes service policy.
const KEY = 'capsem-developer-mode';
function load(): boolean {
  try { return localStorage.getItem(KEY) === 'true'; } catch { return false; }
}
class DeveloperStore {
  enabled = $state(load());
  setEnabled(enabled: boolean) {
    this.enabled = enabled;
    try { localStorage.setItem(KEY, String(enabled)); } catch { /* storage unavailable */ }
  }
}
export const developerStore = new DeveloperStore();
