import { readFileSync } from 'node:fs';
const read = p => readFileSync(new URL('../' + p, import.meta.url), 'utf8');
const version = read('VERSION').trim();
const versions = [JSON.parse(read('package.json')).version, JSON.parse(read('src-tauri/tauri.conf.json')).version, read('src-tauri/Cargo.toml').match(/^version = "([^"]+)"/m)?.[1]];
if (versions.some(v => v !== version)) throw new Error('Application versions differ from VERSION');
console.log('Application versions match VERSION: ' + version);
