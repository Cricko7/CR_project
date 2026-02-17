const { spawnSync } = require('node:child_process');
const { existsSync } = require('node:fs');
const { join } = require('node:path');

const executable = process.platform === 'win32'
  ? join('node_modules', '.bin', 'eslint.cmd')
  : join('node_modules', '.bin', 'eslint');
const args = ['src', '--ext', '.ts,.tsx', '--quiet'];

if (!existsSync(executable)) {
  console.warn('[lint] eslint is not installed for this project. Skipping lint.');
  process.exit(0);
}

const result = spawnSync(executable, args, {
  stdio: 'inherit',
  shell: process.platform === 'win32'
});

if (result.error) {
  console.error('[lint] failed to run eslint:', result.error.message);
  process.exit(1);
}

process.exit(typeof result.status === 'number' ? result.status : 1);
