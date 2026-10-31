# Ephemeral registry sources

This fork carries one behavioral extension over upstream Cargo.

When EPHEMERAL_CARGO_REGISTRY_SRC is set, registry packages are extracted
under that directory instead of $CARGO_HOME/registry/src.

The verified .crate archive cache remains unchanged by the fork. Deployments may
place registry/index and registry/cache on different storage tiers. Registry
source extraction is intended to live in bounded scratch with stable paths; the
reference wrapper keeps that scratch warm across nearby invocations instead of
deleting it after every idle transition.

ci/cargo-ephemeral-wrapper is the reference wrapper. It uses a shared lock so
concurrent Cargo processes can safely reuse the same stable source paths. The
last process leaves the extracted tree warm; an exclusive idle owner reclaims it
when it exceeds the configured size cap or idle TTL. Cleanup scans are themselves
rate-limited. Defaults are 1 GiB, 6 hours idle and a 5 minute prune interval,
overridable with EPHEMERAL_CARGO_SOURCE_MAX_BYTES,
EPHEMERAL_CARGO_SOURCE_MAX_IDLE_SECONDS and
EPHEMERAL_CARGO_SOURCE_PRUNE_INTERVAL_SECONDS. Stable paths matter for compiler
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


## Package-cache lock tracing

This fork also carries an opt-in diagnostic for global package-cache contention.
Set `CARGO_PACKAGE_CACHE_LOCK_TRACE=1` to emit one stderr line for each package
cache lock guard. Blocking acquisitions report their wait time when the guard is
released, along with hold time, lock mode, PID, recursive-acquisition status,
and the source callsite that requested the lock. Non-blocking lock attempts that
fail immediately emit `result=would-block`.

The diagnostic is disabled by default and does not change lock semantics.
It is intended for profiling coarse `.package-cache` contention before making
locking behavior changes.

## Ephemeral intermediate build artifacts

The fork also adds `build.ephemeral-intermediates`. When enabled and no explicit
`build.build-dir` is configured, Cargo places intermediate build state in a
process-scoped temporary directory. Final artifacts remain in the normal target
directory through Cargo's native build-dir artifact uplift.

Cleanup is layered rather than relying on a single best-effort destructor:

- ordinary success and error paths drop the Rust `tempfile::TempDir` and remove
  the intermediate tree immediately;
- on Linux every temporary root must acquire a kernel-backed exclusive lease;
  the lease FD is inherited across exec, so compiler wrappers/build scripts keep
  the root owned even if the Cargo parent dies and host tmpfiles cleanup stays
  away from live descendants. On Linux/glibc, the next ephemeral Cargo invocation
  also reaps same-user abandoned roots after the process tree releases the lease,
  but only when the temporary filesystem is positively identified as non-NFS.
  NFS roots, filesystem-type probe failures, and unsupported platforms do not opt
  into Cargo's self-reaping path;
- roots that never reach the published/leased state are deliberately not guessed
  about by Cargo and remain subject to the host operating system's temporary-file
  cleanup policy.

The lease is published only after it is already locked, so concurrent Cargo
processes cannot mistake a newly-created or live build root for an abandoned one.
This also covers `SIGKILL`/abort-style termination on the next Cargo invocation,
which signal-handler-only tempfile schemes cannot guarantee.

An explicit `build.build-dir` always wins and therefore acts as the persistence
opt-in for hot development loops. The setting can also be overridden per
invocation with `CARGO_BUILD_EPHEMERAL_INTERMEDIATES=false`.
