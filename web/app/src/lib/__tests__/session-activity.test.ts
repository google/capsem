import { describe, expect, it } from 'vitest';
import { render } from 'svelte/server';
import Activity from '../components/shell/SessionActivity.svelte';

describe('structured session activity', () => {
  it('renders bounded terminal title and observed attention without inferring activity', () => {
    const html = render(Activity, { props: { terminal: {
      revision: 3, title: 'Editing workspace', bell_count: 2,
      last_notification: { id: 'opaque-private-id', title: 'Review requested', body: 'private notification body' },
    } } }).body;
    expect(html).toContain('Editing workspace');
    expect(html).toContain('Review requested');
    expect(html).toContain('Terminal bells: 2');
    for (const value of ['private notification body', 'opaque-private-id', 'Working', 'Waiting', 'unread']) {
      expect(html).not.toContain(value);
    }
  });

  it('does not invent idle or attention for missing metadata and measured zero', () => {
    for (const terminal of [undefined, null, { revision: 0, title: '', bell_count: 0 }]) {
      const html = render(Activity, { props: { terminal } }).body;
      for (const value of ['Terminal bells:', 'Latest notification', 'Idle', 'Working', 'Waiting']) {
        expect(html).not.toContain(value);
      }
    }
  });

  it('escapes guest text, bounds visible and accessible metadata, and never exposes hidden body', () => {
    const html = render(Activity, { props: { terminal: {
      revision: 1, title: '<script>\u001b' + 'x'.repeat(120) + 'HIDDEN_TITLE_TAIL', bell_count: 1,
      last_notification: { id: 'id', title: 'n'.repeat(120) + 'HIDDEN_NOTICE_TAIL', body: 'secret' },
    } } }).body;
    expect(html).toContain('&lt;script>');
    for (const value of ['<script>', '\u001b', 'HIDDEN_TITLE_TAIL', 'HIDDEN_NOTICE_TAIL', 'secret', 'title="']) {
      expect(html).not.toContain(value);
    }
  });
});
