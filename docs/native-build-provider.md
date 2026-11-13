# Per-build-unit native prebuilt providers

Cargo can optionally augment individual build-script executions with an
external native-prebuilt provider. This is disabled unless both variables are set:

* CARGO_NATIVE_PREBUILT_PROVIDER=/absolute/path/to/provider
* CARGO_NATIVE_PREBUILT_PACKAGES=btls-sys,boring-sys

The package allowlist is **exact**, not a prefix/regex. Cargo selects it only
when preparing a build-script **run** unit (not the build-script compilation
unit). It supplies the **final resolved unit** package name/version/source,
sorted feature list, compilation target triple, profile, workspace root and
declared SCCACHE_CANONICAL_TARGET_ROOT through these process env variables:

* CARGO_NATIVE_PREBUILT_PACKAGE, CARGO_NATIVE_PREBUILT_VERSION,
  CARGO_NATIVE_PREBUILT_SOURCE, CARGO_NATIVE_PREBUILT_IS_CRATES_IO,
  CARGO_NATIVE_PREBUILT_FEATURES
* CARGO_NATIVE_PREBUILT_TARGET, CARGO_NATIVE_PREBUILT_PROFILE,
  CARGO_NATIVE_PREBUILT_WORKSPACE, CARGO_NATIVE_PREBUILT_CANONICAL_ROOT

The provider writes one JSON object to stdout:
{"env":{"BORING_BSSL_PATH":"/target/boringssl-prebuilt/key","BORING_BSSL_ASSUME_PATCHED":"1"}}

Cargo rejects malformed JSON, untrusted names that overwrite CARGO_/RUST*
invariants, NUL-containing values, and failed providers. Only this build
script receives the returned environment. Provider stderr stays captured for
error reporting; use stdout solely for the response.

Native generation is allowed to launch nested Cargo builds. Cargo marks the
provider process with CARGO_NATIVE_PREBUILT_PROVIDER_ACTIVE=1; nested Cargo
skips the hook and retains its normal compiler/sccache configuration. The
provider and the package's identity/feature/profile/target/root/revision are
hashed into the build-script fingerprint, so opting into a provider, changing
its content or changing CARGO_NATIVE_PREBUILT_PROVIDER_REVISION invalidates
previous build-script output. The revision should include any external helper
scripts/configuration the provider calls, not just the provider executable.

Prebuilts are an optimization, **not a replacement for the original source's
build script**. The script still runs with its exact original Cargo features,
patches and bindgen inputs; it may skip its bundled native C/C++ compile if
a provider returned suitable native libraries. Package selection, source
identity and ABI correctness are the provider's responsibility.

The BoringSSL provider in codeandsolder/sentinelO2-cloud-core is a
reference implementation. It refuses foreign-registry packages, exact-version
ambiguity, mismatched canonical roots, or missing/corrupted native package
contents; it never guesses features from Cargo.lock. Its native static
archives are built using a documented release configuration even when the Rust
consumer uses a dev profile. No site-wide path override is required.

Cargo supplies SourceId's machine-readable as_url() representation, not the
human-readable Display string. IS_CRATES_IO is calculated using Cargo's own
SourceId::is_crates_io predicate, including its handling of sparse indexes and
documented index overrides; consumers must not guess from URL display names.
