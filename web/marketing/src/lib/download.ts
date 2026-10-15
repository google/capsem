// Browser OS detection is a convenience; all packages remain in Other downloads.
export function downloadIndex(userAgent: string, architecture = ""): number | null {
  if (/iPhone|iPad|Android/.test(userAgent)) return null;
  if (/Macintosh|Mac OS X/.test(userAgent)) return 0;
  if (/Linux/.test(userAgent)) return /arm|aarch64/i.test(architecture || userAgent) ? 2 : 1;
  return null;
}
