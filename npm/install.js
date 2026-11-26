'use strict';
// Fetches the native binaries for this platform from the GitHub release that
// matches this package's version. Runs at install time and, if that did not
// work (offline install, scripts disabled), again on first use.
const { execFileSync } = require('node:child_process');
const crypto = require('node:crypto');
const fs = require('node:fs');
const os = require('node:os');
const path = require('node:path');

const { version } = require('./package.json');

const TARGETS = {
  'darwin-arm64': 'aarch64-apple-darwin',
  'darwin-x64': 'x86_64-apple-darwin',
  'linux-x64': 'x86_64-unknown-linux-musl',
  'linux-arm64': 'aarch64-unknown-linux-musl',
  'win32-x64': 'x86_64-pc-windows-msvc',
};

const vendorDir = path.join(__dirname, 'vendor');
const exe = process.platform === 'win32' ? '.exe' : '';

function binaryPath(name) {
  return path.join(vendorDir, name + exe);
}

function download(url, redirects = 0) {
  const client = url.startsWith('http://') ? require('node:http') : require('node:https');
  return new Promise((resolve, reject) => {
    client
      .get(url, { headers: { 'user-agent': `transcribetube-cli-npm/${version}` } }, (res) => {
        if (res.statusCode >= 300 && res.statusCode < 400 && res.headers.location) {
          res.resume();
          if (redirects > 5) return reject(new Error('Too many redirects'));
          return resolve(download(new URL(res.headers.location, url).toString(), redirects + 1));
        }
        if (res.statusCode !== 200) {
          res.resume();
          return reject(new Error(`HTTP ${res.statusCode} for ${url}`));
        }
        const chunks = [];
        res.on('data', (c) => chunks.push(c));
        res.on('end', () => resolve(Buffer.concat(chunks)));
        res.on('error', reject);
      })
      .on('error', reject);
  });
}

async function install() {
  const target = TARGETS[`${process.platform}-${process.arch}`];
  if (!target) {
    throw new Error(
      `No prebuilt binary for ${process.platform}-${process.arch}. ` +
        'Build from source: cargo install --git https://github.com/hicay/transcribetube-cli',
    );
  }
  const archive = `tube-v${version}-${target}.${process.platform === 'win32' ? 'zip' : 'tar.gz'}`;
  const base =
    process.env.TUBE_BINARY_BASE_URL || 'https://github.com/hicay/transcribetube-cli/releases/download';
  const data = await download(`${base}/v${version}/${archive}`);

  // checksums.json is written into the package when it is published.
  let expected;
  try {
    expected = require('./checksums.json')[archive];
  } catch {
    expected = undefined;
  }
  if (expected) {
    const actual = crypto.createHash('sha256').update(data).digest('hex');
    if (actual !== expected) {
      throw new Error(`Checksum mismatch for ${archive}: expected ${expected}, got ${actual}`);
    }
  }

  const tmp = fs.mkdtempSync(path.join(os.tmpdir(), 'tube-'));
  try {
    const file = path.join(tmp, archive);
    fs.writeFileSync(file, data);
    fs.rmSync(vendorDir, { recursive: true, force: true });
    fs.mkdirSync(vendorDir, { recursive: true });
    // tar ships with macOS, Linux and Windows 10+, and reads zip on Windows.
    execFileSync('tar', ['-xf', file, '-C', vendorDir, '--strip-components=1'], { stdio: 'ignore' });
  } finally {
    fs.rmSync(tmp, { recursive: true, force: true });
  }
  for (const name of ['tube', 'transcribetube-mcp']) {
    if (!fs.existsSync(binaryPath(name))) throw new Error(`${archive} did not contain ${name}`);
    if (process.platform !== 'win32') fs.chmodSync(binaryPath(name), 0o755);
  }
}

module.exports = { binaryPath, install };

if (require.main === module) {
  install().catch((err) => {
    // Never fail `npm install` over this: the launcher retries on first use.
    console.error(`transcribetube: could not download the binary now (${err.message}).`);
    console.error('transcribetube: it will be fetched the first time you run `tube`.');
  });
}
