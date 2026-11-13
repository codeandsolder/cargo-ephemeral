//! Optional, per-build-unit native prebuilt provider.
//!
//! The provider is an external executable. Cargo passes the *resolved* package
//! identity, feature set, compilation kind and canonical target root, then
//! applies the provider's validated JSON environment map only to that build
//! script. This avoids guessing features using Cargo.lock or workspace-wide
//! environment overrides.
use super::Unit;
use crate::util::errors::CargoResult;
use anyhow::{Context as _, bail};
use serde::Deserialize;
use std::collections::BTreeMap;
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::process::Command;

const PROVIDER: &str = "CARGO_NATIVE_PREBUILT_PROVIDER";
const PACKAGES: &str = "CARGO_NATIVE_PREBUILT_PACKAGES";
const ACTIVE: &str = "CARGO_NATIVE_PREBUILT_PROVIDER_ACTIVE";
const REVISION: &str = "CARGO_NATIVE_PREBUILT_PROVIDER_REVISION";
const CANONICAL: &str = "SCCACHE_CANONICAL_TARGET_ROOT";

#[derive(Debug)]
pub(super) struct NativePrebuiltProvider {
    executable: OsString,
    package: String,
    version: String,
    source: String,
    features: Vec<String>,
    target: String,
    profile: String,
    workspace: PathBuf,
    canonical_root: PathBuf,
    revision: Option<OsString>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ProviderResponse {
    env: BTreeMap<String, String>,
}

impl NativePrebuiltProvider {
    pub(super) fn select(unit: &Unit, workspace: &Path, target: &str) -> CargoResult<Option<Self>> {
        if std::env::var_os(ACTIVE).is_some() {
            // Building a native package may itself invoke Cargo.
            return Ok(None);
        }
        let Some(executable) = std::env::var_os(PROVIDER) else {
            return Ok(None);
        };
        let packages = std::env::var(PACKAGES)
            .with_context(|| format!("{PACKAGES} is required when {PROVIDER} is set"))?;
        if packages.split(',').any(|p| p.trim().is_empty()) {
            bail!("{PACKAGES} contains an empty package name");
        }
        if !packages
            .split(',')
            .any(|p| p.trim() == unit.pkg.name().as_str())
        {
            return Ok(None);
        }
        let canonical_root = std::env::var_os(CANONICAL)
            .with_context(|| format!("{CANONICAL} must be declared for native prebuilts"))?;
        let canonical_root = PathBuf::from(canonical_root);
        if !canonical_root.is_absolute() {
            bail!("{CANONICAL} must be an absolute path");
        }
        let id = unit.pkg.package_id();
        Ok(Some(Self {
            executable,
            package: unit.pkg.name().to_string(),
            version: unit.pkg.version().to_string(),
            source: id.source_id().to_string(),
            features: unit.features.iter().map(ToString::to_string).collect(),
            target: target.to_string(),
            profile: unit.profile.name.to_string(),
            workspace: workspace.to_path_buf(),
            canonical_root,
            revision: std::env::var_os(REVISION),
        }))
    }

    /// Include provider identity, source bytes, revision, feature set and
    /// target root in Cargo's build-script fingerprint. A switch between
    /// unprovided and provided native libraries must invalidate prior output.
    pub(super) fn fingerprint(&self) -> CargoResult<u64> {
        let bytes = std::fs::read(&self.executable).with_context(|| {
            format!(
                "failed to read native prebuilt provider {:?}",
                self.executable
            )
        })?;
        Ok(crate::util::hash_u64(&(
            &self.executable,
            bytes,
            &self.package,
            &self.version,
            &self.source,
            &self.features,
            &self.target,
            &self.profile,
            &self.workspace,
            &self.canonical_root,
            &self.revision,
        )))
    }

    /// Execute at build-script *run* time after Cargo's dependency graph has
    /// scheduled all inputs. Do not run while constructing a job/fingerprint.
    pub(super) fn configure(&self, cmd: &mut cargo_util::ProcessBuilder) -> CargoResult<()> {
        let mut provider = Command::new(&self.executable);
        provider
            .env(ACTIVE, "1")
            .env("CARGO_NATIVE_PREBUILT_PACKAGE", &self.package)
            .env("CARGO_NATIVE_PREBUILT_VERSION", &self.version)
            .env("CARGO_NATIVE_PREBUILT_SOURCE", &self.source)
            .env("CARGO_NATIVE_PREBUILT_FEATURES", self.features.join(","))
            .env("CARGO_NATIVE_PREBUILT_TARGET", &self.target)
            .env("CARGO_NATIVE_PREBUILT_PROFILE", &self.profile)
            .env("CARGO_NATIVE_PREBUILT_WORKSPACE", &self.workspace)
            .env("CARGO_NATIVE_PREBUILT_CANONICAL_ROOT", &self.canonical_root);
        let output = provider.output().with_context(|| {
            format!(
                "failed to invoke native prebuilt provider {:?}",
                self.executable
            )
        })?;
        if !output.status.success() {
            bail!(
                "native prebuilt provider failed for {} {} ({}): {}",
                self.package,
                self.version,
                output.status,
                String::from_utf8_lossy(&output.stderr)
            );
        }
        let response: ProviderResponse = serde_json::from_slice(&output.stdout)
            .context("native prebuilt provider must write a JSON object with an env map")?;
        for (name, value) in response.env {
            if name.is_empty()
                || !name
                    .bytes()
                    .all(|b| b.is_ascii_uppercase() || b.is_ascii_digit() || b == b'_')
                || name.starts_with("CARGO_")
                || name.starts_with("RUST")
                || matches!(
                    name.as_str(),
                    "HOST" | "TARGET" | "OUT_DIR" | "PROFILE" | "DEBUG" | "OPT_LEVEL" | "NUM_JOBS"
                )
                || value.contains('\0')
            {
                bail!(
                    "native prebuilt provider returned invalid build-script environment name/value for {name:?}"
                );
            }
            cmd.env(&name, value);
        }
        Ok(())
    }
}
