#!/usr/bin/env node

const { spawnSync } = require('node:child_process');
const fs = require('node:fs');
const os = require('node:os');
const path = require('node:path');

const repoRoot = path.resolve(__dirname, '..');

function findBundleDirectories(rootDir) {
  const bundles = [];
  const pending = [rootDir];

  while (pending.length > 0) {
    const current = pending.pop();
    if (!fs.existsSync(current)) continue;

    for (const entry of fs.readdirSync(current, { withFileTypes: true })) {
      if (!entry.isDirectory()) continue;
      const entryPath = path.join(current, entry.name);
      if (entry.name === 'bundle' && path.basename(current) === 'release') {
        bundles.push(entryPath);
      } else {
        pending.push(entryPath);
      }
    }
  }

  return bundles.sort();
}

function runManagedBuild(args = process.argv.slice(2)) {
  const targetDir = fs.mkdtempSync(path.join(os.tmpdir(), 'cockpit-tools-release-'));
  const outputDir = path.join(repoRoot, 'release-output');
  let buildSucceeded = false;

  console.log(`[tauri-build] Temporary Cargo target: ${targetDir}`);

  try {
    const result = spawnSync(
      process.execPath,
      [path.join(__dirname, 'tauri.cjs'), 'build', ...args],
      {
        cwd: repoRoot,
        stdio: 'inherit',
        env: {
          ...process.env,
          CARGO_TARGET_DIR: targetDir,
        },
      },
    );

    if (result.error) throw result.error;
    if (result.status !== 0) return result.status ?? 1;
    buildSucceeded = true;

    const bundleDirs = findBundleDirectories(targetDir);
    if (bundleDirs.length === 0) {
      if (args.includes('--no-bundle')) {
        console.log('[tauri-build] Build completed without bundles as requested.');
        return 0;
      }
      throw new Error(`Build succeeded but no release bundle was found under ${targetDir}`);
    }
    if (bundleDirs.length > 1) {
      throw new Error(`Expected one release bundle directory, found ${bundleDirs.length}`);
    }

    fs.rmSync(outputDir, { recursive: true, force: true });
    fs.cpSync(bundleDirs[0], outputDir, { recursive: true });
    console.log(`[tauri-build] Release bundle copied to ${outputDir}`);
    return 0;
  } finally {
    fs.rmSync(targetDir, { recursive: true, force: true });
    console.log(
      `[tauri-build] Temporary Cargo target removed${buildSucceeded ? '' : ' after build failure'}.`,
    );
  }
}

if (require.main === module) {
  try {
    process.exitCode = runManagedBuild();
  } catch (error) {
    console.error(`[tauri-build] ${error.message}`);
    process.exitCode = 1;
  }
}

module.exports = { findBundleDirectories, runManagedBuild };
