/** Browsers cannot read installed host credentials; callers connect explicitly. */
export function localConnection(): [string, string] {
  throw new TypeError('Local Capsem discovery requires Node.js; provide both gateway URL and token in a browser');
}
