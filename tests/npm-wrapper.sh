#!/bin/sh
# Installs the npm package from a local archive server and runs the binaries
# through its launchers. Proves the wrapper without touching GitHub or npm.
set -eu
cd "$(dirname "$0")/.."

case "$(uname -s)-$(uname -m)" in
    Darwin-arm64) target="aarch64-apple-darwin" ;;
    Darwin-x86_64) target="x86_64-apple-darwin" ;;
    Linux-x86_64) target="x86_64-unknown-linux-musl" ;;
    Linux-aarch64 | Linux-arm64) target="aarch64-unknown-linux-musl" ;;
    *) echo "unsupported platform for this test" >&2; exit 1 ;;
esac
version="$(grep -m1 '^version' Cargo.toml | cut -d'"' -f2)"

cargo build --release
work="$(mktemp -d)"
trap 'kill "$server" 2>/dev/null || true; rm -rf "$work"' EXIT

name="tube-v$version-$target"
mkdir -p "$work/serve/v$version" "$work/$name"
cp target/release/tube target/release/transcribetube-mcp "$work/$name/"
tar -czf "$work/serve/v$version/$name.tar.gz" -C "$work" "$name"

port=47833
(cd "$work/serve" && exec python3 -m http.server "$port" --bind 127.0.0.1 >/dev/null 2>&1) &
server=$!
sleep 1

(cd npm && npm version "$version" --no-git-tag-version --allow-same-version >/dev/null && npm pack --pack-destination "$work" >/dev/null)
tarball="$(ls "$work"/transcribetube-cli-*.tgz)"

echo "== install with the download at install time"
TUBE_BINARY_BASE_URL="http://127.0.0.1:$port" npm install --prefix "$work/a" "$tarball" >/dev/null 2>&1
got="$("$work/a/node_modules/.bin/tube" --version)"
[ "$got" = "tube $version" ] || { echo "unexpected: $got" >&2; exit 1; }

echo "== npx finds a command: it runs the bin named after the package (cli)"
got="$("$work/a/node_modules/.bin/cli" --version)"
[ "$got" = "tube $version" ] || { echo "unexpected: $got" >&2; exit 1; }

echo "== exit codes and stdout pass through the launcher"
set +e
out="$(XDG_CONFIG_HOME="$work/empty" "$work/a/node_modules/.bin/tube" whoami 2>/dev/null)"
code=$?
set -e
[ "$code" = "2" ] && [ -z "$out" ] || { echo "expected exit 2 and no stdout, got $code / $out" >&2; exit 1; }

echo "== the MCP launcher speaks the protocol on stdout only"
reply="$(printf '%s\n' '{"jsonrpc":"2.0","id":1,"method":"ping"}' | XDG_CONFIG_HOME="$work/empty" "$work/a/node_modules/.bin/transcribetube-mcp" 2>/dev/null)"
[ "$reply" = '{"jsonrpc":"2.0","id":1,"result":{}}' ] || { echo "unexpected: $reply" >&2; exit 1; }

echo "== install with scripts disabled, download on first run"
npm install --ignore-scripts --prefix "$work/b" "$tarball" >/dev/null 2>&1
[ ! -e "$work/b/node_modules/@transcribetube/cli/vendor" ] || { echo "vendor should not exist yet" >&2; exit 1; }
got="$(TUBE_BINARY_BASE_URL="http://127.0.0.1:$port" "$work/b/node_modules/.bin/tube" --version 2>/dev/null)"
[ "$got" = "tube $version" ] || { echo "unexpected: $got" >&2; exit 1; }

echo "== a wrong checksum is refused"
mkdir "$work/c" && tar -xzf "$tarball" -C "$work/c"
printf '{"%s.tar.gz": "%s"}\n' "$name" "0000000000000000000000000000000000000000000000000000000000000000" > "$work/c/package/checksums.json"
set +e
TUBE_BINARY_BASE_URL="http://127.0.0.1:$port" "$work/c/package/bin/tube.js" --version >/dev/null 2>"$work/c/err"
code=$?
set -e
[ "$code" = "1" ] && grep -q "Checksum mismatch" "$work/c/err" || { echo "a bad checksum was accepted" >&2; cat "$work/c/err" >&2; exit 1; }

echo "npm wrapper: all checks passed"
