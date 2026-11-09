'use strict';

const { spawnSync } = require('node:child_process');
const fs = require('node:fs');
const path = require('node:path');

const root = path.resolve(__dirname, '..');
const manifest = path.join(root, 'native', 'Cargo.toml');

const result = spawnSync('cargo', ['build', '--release', '--manifest-path', manifest], {
  stdio: 'inherit',
  cwd: root,
});
if (result.error) {
  console.error(result.error.message);
  process.exit(1);
}
if (result.status !== 0) {
  process.exit(result.status || 1);
}

const dll = path.join(root, 'native', 'target', 'release', 'win_down.dll');
const out = path.join(root, 'native', 'win_down.node');
fs.copyFileSync(dll, out);
console.log(`built ${out}`);
