#!/usr/bin/env bash
# PostToolUse: keep the two Rust crates formatted after an edit.
# Never blocks — formatting is not a reason to stop work.
set -uo pipefail

path=$(jq -r '.tool_input.file_path // empty')
[[ "$path" == *.rs ]] || exit 0

# Walk up to the owning crate. The Rust packages are plain cargo crates (decision 31), so the
# nearest Cargo.toml is the right manifest.
dir=$(dirname "$path")
while [ "$dir" != "/" ] && [ -n "$dir" ]; do
  if [ -f "$dir/Cargo.toml" ]; then
    cargo fmt --manifest-path "$dir/Cargo.toml" >/dev/null 2>&1
    break
  fi
  dir=$(dirname "$dir")
done

exit 0
