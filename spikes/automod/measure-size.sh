#!/bin/sh
# Builds the harness once with no engine and once with each engine alone,
# and prints how much each engine adds to the release binary. The text API
# is in every build, so the baseline includes it.
set -eu
cd "$(dirname "$0")"

size() {
    cargo build --release --quiet --no-default-features ${1:+--features "$1"}
    bin=target/release/quill-automod-spike
    [ -f "$bin.exe" ] && bin="$bin.exe"
    wc -c < "$bin" | tr -d ' '
}

base=$(size "")
echo "baseline (text API, no engine): $((base / 1024)) KiB"
for engine in rules rhai luau starlark js; do
    total=$(size "$engine")
    echo "$engine: +$(((total - base) / 1024)) KiB"
done
