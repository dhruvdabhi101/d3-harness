#!/bin/sh
set -eu

repo=dhruvdabhi101/d3-harness
case "$(uname -s)-$(uname -m)" in
    Linux-x86_64) target=x86_64-unknown-linux-gnu ;;
    Linux-aarch64) target=aarch64-unknown-linux-gnu ;;
    Darwin-x86_64) target=x86_64-apple-darwin ;;
    Darwin-arm64) target=aarch64-apple-darwin ;;
    *) echo "Unsupported platform: $(uname -s) $(uname -m)" >&2; exit 1 ;;
esac
command -v curl >/dev/null || { echo 'curl is required' >&2; exit 1; }
command -v sha256sum >/dev/null && hash=sha256sum || hash='shasum -a 256'
tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT HUP INT TERM
base="https://github.com/$repo/releases/latest/download"
asset="d3-$target.tar.gz"
curl -fsSL "$base/$asset" -o "$tmp/$asset"
curl -fsSL "$base/$asset.sha256" -o "$tmp/$asset.sha256"
(cd "$tmp" && $hash -c "$asset.sha256")
tar -xzf "$tmp/$asset" -C "$tmp"
bin_dir=${D3_INSTALL_DIR:-"$HOME/.local/bin"}
mkdir -p "$bin_dir"
install -m 755 "$tmp/d3" "$bin_dir/d3"
printf 'Installed d3 to %s/d3\n' "$bin_dir"
printf 'Ensure %s is on your PATH.\n' "$bin_dir"
