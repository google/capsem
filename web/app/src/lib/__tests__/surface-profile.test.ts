import { readFileSync } from 'node:fs';
import vm from 'node:vm';
import { describe, expect, it } from 'vitest';

const root = new URL('../../../../../crates/capsem-gateway/', import.meta.url);
const client = readFileSync(new URL('vendor/xpra-html5/www/js/Client.js', root), 'utf8');
const policy = readFileSync(new URL('src/surface/static/capsem-surface-profile.js', root), 'utf8');

// Exercise the pinned client's real hello builder and encoding/network caps.
// No constructor or decoder worker is needed to inspect negotiation.
function hello(profile: string, decoded = ['rgb', 'rgb24', 'rgb32', 'jpeg', 'webp', 'png', 'scroll', 'h264', 'avif']) {
  const context = vm.createContext({
    window: {},
    document: { querySelector: () => ({ dataset: { displayProfile: profile } }) },
    METADATA_SUPPORTED: [], lz4: { decode: () => {} }, BrotliDecode: () => {},
    Utilities: { getConnectionInfo: () => ({}) },
  });
  vm.runInContext(client, context);
  vm.runInContext(policy, context);
  context.decoded = decoded;
  return vm.runInContext(`
    const c = Object.create(XpraClient.prototype);
    c.container = {clientWidth: 1200, clientHeight: 800};
    c.supported_encodings = decoded;
    c.RGB_FORMATS = ['RGB', 'RGBX', 'BGRX'];
    c.encoding_options = {'': 'auto', rgb_lz4: true, video_scaling: true};
    c._get_digests = () => [];
    c.capabilities = c._get_network_caps();
    c._update_capabilities = caps => Object.assign(c.capabilities, caps);
    for (const name of ['_get_keyboard_layout', '_get_audio_caps', '_get_clipboard_caps',
      '_get_pointer_caps', '_get_file_caps', '_get_screen_sizes', '_get_DPI']) c[name] = () => null;
    c._make_hello();
    c.capabilities;
  `, context);
}

describe('display profile negotiation', () => {
  it('uses raw RGB without pixel or packet compression for local viewers', () => {
    const caps = hello('local');
    expect(caps.encodings['']).toEqual(['rgb', 'rgb24', 'rgb32', 'scroll']);
    expect(caps.encodings.core).toEqual(caps.encodings['']);
    expect(caps.encoding).toMatchObject({'': 'rgb', rgb_lz4: false, video_scaling: false});
    expect(caps).toMatchObject({compression_level: 0, lz4: false, brotli: false});
    expect(caps.encodings['window-icon']).toEqual(['png']);
    expect(caps.encodings.cursor).toEqual(['png']);
  });

  it('uses supported software image encodings remotely, without video or downscaling', () => {
    const caps = hello('remote');
    expect(caps.encodings['']).toEqual(['rgb', 'rgb24', 'rgb32', 'jpeg', 'webp', 'png', 'scroll']);
    expect(caps.encoding).toMatchObject({'': 'auto', video_scaling: false});
    expect(caps.encoding.full_csc_modes).not.toHaveProperty('h264');
    expect(caps.encoding.full_csc_modes).toHaveProperty('webp');
    expect(caps.lz4).toBe(true);
  });

  it('does not advertise a decoder the browser did not validate', () => {
    expect(hello('remote', ['rgb', 'rgb24', 'jpeg']).encodings['']).toEqual(['rgb', 'rgb24', 'jpeg']);
  });

  it('defaults to remote without trusted bootstrap metadata', () => {
    expect(hello('').encoding['']).toBe('auto');
    expect(hello('locality').encodings['']).toContain('jpeg');
  });
});
