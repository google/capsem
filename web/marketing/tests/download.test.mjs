import { test } from "node:test";
import assert from "node:assert/strict";
import { downloadIndex } from "../src/lib/download.ts";
test("selects supported desktop packages and respects ARM client hints", () => {
  assert.equal(downloadIndex("Mozilla Macintosh Intel Mac OS X"), 0);
  assert.equal(downloadIndex("Mozilla X11 Linux x86_64"), 1);
  assert.equal(downloadIndex("Mozilla X11 Linux aarch64"), 2);
  assert.equal(downloadIndex("Mozilla X11 Linux x86_64", "arm"), 2);
});
test("mobile and unsupported desktops choose explicitly", () => {
  for (const ua of ["iPhone like Mac OS X", "iPad like Mac OS X", "Linux Android", "Windows NT", ""]) {
    assert.equal(downloadIndex(ua), null);
  }
});
