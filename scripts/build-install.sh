#!/bin/sh
set -eu

cd "$(dirname "$0")/.."
cargo install --locked --path . --force
