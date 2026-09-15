const assert = require('node:assert/strict');
const fs = require('node:fs');
const os = require('node:os');
const path = require('node:path');
const test = require('node:test');

const { findBundleDirectories } = require('./tauri-build.cjs');

test('finds native and target-specific Tauri bundle directories', (t) => {
  const root = fs.mkdtempSync(path.join(os.tmpdir(), 'cockpit-tauri-build-test-'));
  t.after(() => fs.rmSync(root, { recursive: true, force: true }));

  const nativeBundle = path.join(root, 'release', 'bundle');
  const targetBundle = path.join(root, 'aarch64-apple-darwin', 'release', 'bundle');
  fs.mkdirSync(nativeBundle, { recursive: true });
  fs.mkdirSync(targetBundle, { recursive: true });
  fs.mkdirSync(path.join(root, 'debug', 'bundle'), { recursive: true });

  assert.deepEqual(findBundleDirectories(root), [nativeBundle, targetBundle].sort());
});
