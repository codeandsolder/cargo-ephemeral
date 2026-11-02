#!/bin/bash
# This script remove test and benchmark output and displays disk usage.

set -euo pipefail

df -h
target_dir="${CARGO_TARGET_DIR:-target}"
rm -rf "$target_dir/tmp"
df -h
