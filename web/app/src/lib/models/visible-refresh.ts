/** Observe cached page data while the view is visible. Callers render errors. */
export function watchVisibleRefresh(refresh: () => Promise<void>, intervalMs: number): () => void {
  let active = true;
  let pending = false;
  const run = () => {
    if (!active || document.hidden || pending) return;
    pending = true;
    void refresh().catch(error => console.error('[page] refresh failed', error)).finally(() => { pending = false; });
  };
  const timer = window.setInterval(run, intervalMs);
  document.addEventListener('visibilitychange', run);
  window.addEventListener('focus', run);
  window.addEventListener('online', run);
  return () => {
    active = false;
    window.clearInterval(timer);
    document.removeEventListener('visibilitychange', run);
    window.removeEventListener('focus', run);
    window.removeEventListener('online', run);
  };
}
