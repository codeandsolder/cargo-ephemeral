//! Optional per-build-unit native provider integration tests.
#![cfg(unix)]

use crate::prelude::*;
use cargo_test_support::project;
use std::os::unix::fs::PermissionsExt;

#[cargo_test]
fn native_provider_tracks_unit_features_and_freshness() {
    let p = project()
        .file(
            "Cargo.toml",
            r#"
                [package]
                name = "native-feature-probe"
                version = "0.1.0"
                edition = "2024"
                build = "build.rs"

                [features]
                alpha = []
                beta = []
            "#,
        )
        .file("src/lib.rs", "pub fn probe() {}")
        .file(
            "build.rs",
            r#"
                fn main() {
                    if let Some(value) = std::env::var_os("NATIVE_PROBE") {
                        assert_eq!(value, "provided");
                    }
                    println!("cargo:rerun-if-changed=build.rs");
                }
            "#,
        )
        .file(
            "provider.sh",
            r#"#!/bin/sh
set -e
test "$CARGO_NATIVE_PREBUILT_PROVIDER_ACTIVE" = 1
test "$CARGO_NATIVE_PREBUILT_PACKAGE" = native-feature-probe
test "$CARGO_NATIVE_PREBUILT_VERSION" = 0.1.0
test "$CARGO_NATIVE_PREBUILT_PROFILE" = dev
test -n "$CARGO_NATIVE_PREBUILT_TARGET"
test -n "$CARGO_NATIVE_PREBUILT_SOURCE"
test -d "$CARGO_NATIVE_PREBUILT_CANONICAL_ROOT"
printf '%s\n' "$CARGO_NATIVE_PREBUILT_FEATURES" >> "$CARGO_NATIVE_PREBUILT_WORKSPACE/provider-features.log"
if [ "$NATIVE_PROVIDER_INVALID" = 1 ]; then
    printf '{"env":{"OUT_DIR":"/tmp/wrong"}}\n'
else
    printf '{"env":{"NATIVE_PROBE":"provided"}}\n'
fi
"#,
        )
        .build();

    let script = p.root().join("provider.sh");
    let mut perms = std::fs::metadata(&script).unwrap().permissions();
    perms.set_mode(0o755);
    std::fs::set_permissions(&script, perms).unwrap();
    let target = p.root().join("target");
    let log = p.root().join("provider-features.log");

    p.cargo("build").run();
    assert!(!log.exists());

    macro_rules! configured {
        ($command:expr) => {
            p.cargo($command)
                .env("CARGO_NATIVE_PREBUILT_PROVIDER", &script)
                .env("CARGO_NATIVE_PREBUILT_PACKAGES", "native-feature-probe")
                .env("SCCACHE_CANONICAL_TARGET_ROOT", &target)
                .env("NATIVE_PROVIDER_INVALID", "0")
        };
    }
    configured!("build").run();
    assert_eq!(std::fs::read_to_string(&log).unwrap(), "\n");
    configured!("build").run();
    assert_eq!(std::fs::read_to_string(&log).unwrap(), "\n");

    configured!("build --features alpha").run();
    assert_eq!(std::fs::read_to_string(&log).unwrap(), "\nalpha\n");

    configured!("build --features beta").run();
    assert_eq!(std::fs::read_to_string(&log).unwrap(), "\nalpha\nbeta\n");

    configured!("build")
        .env("CARGO_NATIVE_PREBUILT_PROVIDER_REVISION", "invalid")
        .env("NATIVE_PROVIDER_INVALID", "1")
        .with_status(101)
        .run();
    assert_eq!(std::fs::read_to_string(&log).unwrap(), "\nalpha\nbeta\n\n");
}
