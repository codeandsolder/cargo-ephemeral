#!/usr/bin/env bash
set -euo pipefail
upstream=$1
git diff --check "$upstream"...HEAD
unexpected=0
while IFS= read -r file; do
  case "$file" in
    .github/workflows/*|\
    .github/scripts/validate-upstream.sh|\
    .github/renovate.json5|\
    tests/testsuite/https.rs|\
    EPHEMERAL_REGISTRY.md|\
    contrib/codeandsolder/cargo-ephemeral-tmpfiles-clean.service|\
    contrib/codeandsolder/cargo-ephemeral-tmpfiles-clean.timer|\
    contrib/codeandsolder/cargo-ephemeral-tmpfiles.conf|\
    ci/cargo-ephemeral-wrapper|\
    ci/clean-test-output.sh|\
    ci/fetch-smoke-test.sh|\
    ci/test-cargo-ephemeral-wrapper|\
    ci/test-cargo-ephemeral-tmpfiles-lock|\
    src/bin/cargo/main.rs|\
    src/context/mod.rs|\
    src/context/schema.rs|\
    src/ops/resolve.rs|\
    src/sources/git/source.rs|\
    src/sources/git/utils.rs|\
    src/sources/registry/download.rs|\
    src/sources/registry/git_remote.rs|\
    src/sources/registry/http_remote.rs|\
    src/sources/registry/index/cache.rs|\
    src/sources/registry/mod.rs|\
    src/util/cache_lock.rs|\
    src/util/diagnostic_server.rs|\
    src/util/flock.rs|\
    src/util/lockserver.rs|\
    src/workspace/package.rs|\
    tests/testsuite/cache_lock.rs|\
    tests/testsuite/cargo_command.rs|\
    tests/testsuite/build_dir.rs|\
    tests/testsuite/cargo_add/mod.rs|\
    tests/testsuite/cargo_add/target_triple/*|\
    tests/testsuite/cargo_remove/mod.rs|\
    tests/testsuite/cargo_remove/target_triple/*|\
    tests/testsuite/registry.rs)
      ;;
    *)
      echo "unexpected downstream path: $file" >&2
      unexpected=1
      ;;
  esac
done < <(git diff --name-only "$upstream"...HEAD)
test "$unexpected" -eq 0
cargo fmt --check
ci/test-cargo-ephemeral-wrapper
ci/test-cargo-ephemeral-tmpfiles-lock
export CARGO_TEST_DISABLE_NIGHTLY=1
cargo test --test testsuite cache_lock::
cargo test --test testsuite shared_locked_offline_sparse_resolve_uses_shared_lock
cargo test --test testsuite shared_locked_offline_git_
cargo test --test testsuite registry_source_can_be_ephemeral
cargo test --test testsuite target_triple::case
cargo check --workspace --all-targets
cargo build --release --locked --bin cargo
{
  echo "fork_sha=$(git rev-parse HEAD)"
  echo "upstream_sha=$upstream"
} > cargo-ephemeral-build.txt
