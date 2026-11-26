#!/usr/bin/env node
'use strict';
const { spawnSync } = require('node:child_process');
const fs = require('node:fs');
const path = require('node:path');

const { binaryPath } = require('../install.js');

const binary = binaryPath('tube');
if (!fs.existsSync(binary)) {
  // stdout stays untouched: it may be a pipe, or the MCP channel.
  const fetched = spawnSync(process.execPath, [path.join(__dirname, '..', 'install.js')], {
    stdio: ['ignore', 2, 2],
  });
  if (fetched.status !== 0 || !fs.existsSync(binary)) {
    console.error('transcribetube: the native binary is missing and could not be downloaded.');
    process.exit(1);
  }
}

const result = spawnSync(binary, process.argv.slice(2), { stdio: 'inherit' });
if (result.error) {
  console.error(`transcribetube: could not start ${binary}: ${result.error.message}`);
  process.exit(1);
}
process.exit(result.status === null ? 1 : result.status);
