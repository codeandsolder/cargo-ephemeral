//! Publish the crates on Cargo's release train in dependency order.

use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

use anyhow::{Context, Result, bail, ensure};
use clap::{Arg, ArgAction};
use curl::easy::Easy;
use semver::Version;
use toml_edit::DocumentMut;

/// Whenever a new crate without the cargo- prefix is added, coordinate with
/// the infra team to authorize it for Cargo's crates.io publishing token.
const TO_PUBLISH: &[&str] = &[
    "credential/cargo-credential",
    "credential/cargo-credential-libsecret",
    "credential/cargo-credential-wincred",
    "credential/cargo-credential-1password",
    "credential/cargo-credential-macos-keychain",
    "crates/rustfix",
    "crates/cargo-platform",
    "crates/cargo-util",
    "crates/crates-io",
    "crates/cargo-util-schemas",
    "crates/cargo-util-terminal",
    "crates/cargo-test-macro",
    "crates/cargo-test-support",
    "crates/build-rs",
    ".",
];

fn cli() -> clap::Command {
    clap::Command::new("xtask-publish")
        .about("Publish Cargo release-train crates in dependency order")
        .arg(
            Arg::new("dry-run")
                .long("dry-run")
                .action(ArgAction::SetTrue)
                .help("Check the registry and print the publication plan without uploading"),
        )
}

#[derive(Debug)]
struct Package {
    path: PathBuf,
    name: String,
    version: Version,
}

impl Package {
    fn read(path: PathBuf) -> Result<Self> {
        let manifest_path = path.join("Cargo.toml");
        let manifest = fs::read_to_string(&manifest_path)
            .with_context(|| format!("reading {}", manifest_path.display()))?;
        Self::parse(path, &manifest)
            .with_context(|| format!("reading package metadata from {}", manifest_path.display()))
    }

    fn parse(path: PathBuf, manifest: &str) -> Result<Self> {
        let document: DocumentMut = manifest.parse()?;
        let package = document.get("package").context("missing [package]")?;
        let name = package
            .get("name")
            .and_then(toml_edit::Item::as_str)
            .context("missing literal package.name")?;
        ensure!(
            !name.is_empty()
                && name
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_')),
            "invalid crate name: {name}"
        );
        let version = package
            .get("version")
            .and_then(toml_edit::Item::as_str)
            .context("missing literal package.version")?
            .parse()?;
        Ok(Self {
            path,
            name: name.to_owned(),
            version,
        })
    }

    fn download_url(&self) -> String {
        format!(
            "https://static.crates.io/crates/{}/{}/download",
            self.name, self.version
        )
    }
}

fn published_status(status: u32) -> Result<bool> {
    match status {
        200..=299 => Ok(true),
        // Preserve the release script's CDN semantics, including 403/404.
        400..=499 => Ok(false),
        _ => bail!("unexpected HTTP response {status}"),
    }
}

fn download_status(url: &str) -> Result<u32> {
    let mut request = Easy::new();
    request.url(url)?;
    request.follow_location(true)?;
    request.max_redirections(10)?;
    request.timeout(Duration::from_secs(60))?;
    // Drain the body without storing crate archives.
    request.write_function(|bytes| Ok(bytes.len()))?;
    request.perform()?;
    Ok(request.response_code()?)
}

fn already_published(package: &Package) -> Result<bool> {
    let url = package.download_url();
    download_status(&url)
        .and_then(published_status)
        .with_context(|| {
            format!(
                "checking if {} {} is already published at {url}",
                package.name, package.version
            )
        })
}

fn publish(package: &Package) -> Result<()> {
    let status = Command::new("cargo")
        .args(["publish", "--no-verify"])
        .current_dir(&package.path)
        .status()
        .context("starting cargo publish")?;
    ensure!(status.success(), "cargo publish exited with {status}");
    Ok(())
}

fn release(
    packages: &[Package],
    dry_run: bool,
    mut check: impl FnMut(&Package) -> Result<bool>,
    mut publish: impl FnMut(&Package) -> Result<()>,
    mut output: impl Write,
) -> Result<()> {
    writeln!(output, "Starting publish...")?;
    for package in packages {
        if check(package)? {
            writeln!(
                output,
                "{} {} is already published, skipping",
                package.name, package.version
            )?;
        } else if dry_run {
            writeln!(
                output,
                "Would publish {} {} ({})",
                package.name,
                package.version,
                package.path.display()
            )?;
        } else {
            output.flush()?;
            publish(package)
                .with_context(|| format!("publishing {} {}", package.name, package.version))?;
        }
    }
    if dry_run {
        writeln!(output, "Dry run complete.")?;
    } else {
        writeln!(output, "Publish complete!")?;
    }
    Ok(())
}

fn load_packages(root: &Path) -> Result<Vec<Package>> {
    TO_PUBLISH
        .iter()
        .map(|path| Package::read(root.join(path)))
        .collect()
}

fn main() -> Result<()> {
    let args = cli().get_matches();
    let root = std::env::current_dir()?;
    let packages = load_packages(&root)?;
    release(
        &packages,
        args.get_flag("dry-run"),
        already_published,
        publish,
        io::stdout().lock(),
    )
}

#[cfg(test)]
mod tests;
