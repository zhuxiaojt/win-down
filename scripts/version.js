'use strict';

const fs = require('node:fs');
const path = require('node:path');

const root = path.resolve(__dirname, '..');
const pkgFile = path.join(root, 'package.json');
const lockFile = path.join(root, 'package-lock.json');
const cargoFile = path.join(root, 'native', 'Cargo.toml');
const cargoLockFile = path.join(root, 'native', 'Cargo.lock');

const SEMVER = /^\d+\.\d+\.\d+$/;

function bump(current, part) {
  const [major, minor, patch] = current.split('.').map(Number);
  if (part === 'major') return `${major + 1}.0.0`;
  if (part === 'minor') return `${major}.${minor + 1}.0`;
  return `${major}.${minor}.${patch + 1}`;
}

const pkg = JSON.parse(fs.readFileSync(pkgFile, 'utf8'));
const current = pkg.version;

let input = process.argv[2];
if (!input) {
  console.log(`current version: ${current}`);
  console.log('usage: npm run bump -- <1.2.3|patch|minor|major>');
  process.exit(1);
}

const next = ['patch', 'minor', 'major'].includes(input) ? bump(current, input) : input;
if (!SEMVER.test(next)) {
  console.error(`invalid version: ${next}`);
  process.exit(1);
}

const changed = [];

let pkgText = fs.readFileSync(pkgFile, 'utf8');
pkgText = pkgText.replace(/("version"\s*:\s*")[^"]*(")/, `$1${next}$2`);
fs.writeFileSync(pkgFile, pkgText);
changed.push(path.relative(root, pkgFile));

if (fs.existsSync(lockFile)) {
  const lock = JSON.parse(fs.readFileSync(lockFile, 'utf8'));
  lock.version = next;
  if (lock.packages && lock.packages['']) lock.packages[''].version = next;
  fs.writeFileSync(lockFile, `${JSON.stringify(lock, null, 2)}\n`);
  changed.push(path.relative(root, lockFile));
}

let cargoName = '';
if (fs.existsSync(cargoFile)) {
  const cargo = fs.readFileSync(cargoFile, 'utf8');
  const nameMatch = cargo.match(/\[package\][\s\S]*?\nname\s*=\s*"([^"]+)"/);
  cargoName = nameMatch ? nameMatch[1] : '';
  const versionRe = /(\[package\][\s\S]*?\nversion\s*=\s*")[^"]*(")/;
  if (!versionRe.test(cargo)) {
    console.warn('warn: no [package] version found in native/Cargo.toml');
  }
  fs.writeFileSync(cargoFile, cargo.replace(versionRe, `$1${next}$2`));
  changed.push(path.relative(root, cargoFile));
}

if (cargoName && fs.existsSync(cargoLockFile)) {
  const lock = fs.readFileSync(cargoLockFile, 'utf8');
  const entryRe = new RegExp(`(\\[\\[package\\]\\]\\nname = "${cargoName}"\\nversion = ")[^"]*(")`);
  if (!entryRe.test(lock)) {
    console.warn(`warn: no "${cargoName}" entry found in native/Cargo.lock`);
  }
  fs.writeFileSync(cargoLockFile, lock.replace(entryRe, `$1${next}$2`));
  changed.push(path.relative(root, cargoLockFile));
}

for (const file of changed) console.log(`updated ${file}`);
console.log(`${current} -> ${next}`);
