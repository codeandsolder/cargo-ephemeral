# Ephemeral registry sources

This fork carries one behavioral extension over upstream Cargo.

When EPHEMERAL_CARGO_REGISTRY_SRC is set, registry packages are extracted
under that directory instead of $CARGO_HOME/registry/src.

The verified .crate archive cache remains unchanged. The intended deployment
keeps registry/index on low-latency storage, registry/cache on bulk storage,
and registry source extraction in process-scoped scratch that is deleted when
the Cargo invocation exits.

ci/cargo-ephemeral-wrapper is the reference wrapper. It uses a shared lock so
concurrent Cargo processes can safely reuse the same stable source paths, then
the last process removes the extracted tree. Stable paths matter for compiler
cache keys; a fresh random source path per invocation would defeat sccache
reuse unless it were separately normalized. It also preserves rustup-style
+toolchain invocations by translating the prefix to RUSTUP_TOOLCHAIN before
calling the real forked Cargo binary. The wrapper exports CARGO pointing back
to itself so external subcommands such as cargo-clippy cannot bypass the same
target/source policy when they recursively invoke Cargo.

## Downstream maintenance contract

Upstream wins on every unrelated behavior. The carried semantic invariant is:

1. .crate download, checksum verification, and persistent caching are unchanged.
2. Registry source extraction honors EPHEMERAL_CARGO_REGISTRY_SRC.
3. With the override set, no package source is extracted into
   $CARGO_HOME/registry/src.
4. A second offline build can rematerialize source from the retained .crate.

The scheduled parity workflow rebases this branch onto rust-lang/cargo master,
runs the focused regression test and a workspace check, and only then updates
the branch. Failures are reported through one idempotent GitHub issue.

## Downstream CI policy

The fork also intentionally carries a low-cost CI policy. Routine pull-request
checks keep the primary Linux x86-64-v2 stable lane rather than reproducing
upstream's full OS/toolchain matrix. macOS, Windows, ARM, beta/nightly, MSRV,
build-std, docs, and timing compatibility work is reserved for release tags or
explicit/manual compatibility runs where applicable. Upstream-only audit and
Pages deployment jobs are skipped in this fork.

These CI changes are deliberate downstream policy, not rebase noise. An
automated parity repair must preserve both this section and the ephemeral
registry-source invariants above while otherwise preferring upstream behavior.
