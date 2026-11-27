use super::*;

fn package(name: &str) -> Result<Package> {
    Package::parse(
        PathBuf::from(name),
        &format!("[package]\nname = '{name}'\nversion = '1.2.3-rc.1+build'"),
    )
}

#[test]
fn actual_release_order() -> Result<()> {
    let packages = load_packages(&Path::new(env!("CARGO_MANIFEST_DIR")).join("../.."))?;
    let names: Vec<_> = packages
        .iter()
        .map(|package| package.name.as_str())
        .collect();
    assert_eq!(
        names,
        [
            "cargo-credential",
            "cargo-credential-libsecret",
            "cargo-credential-wincred",
            "cargo-credential-1password",
            "cargo-credential-macos-keychain",
            "rustfix",
            "cargo-platform",
            "cargo-util",
            "crates-io",
            "cargo-util-schemas",
            "cargo-util-terminal",
            "cargo-test-macro",
            "cargo-test-support",
            "build-rs",
            "cargo",
        ]
    );
    Ok(())
}

#[test]
fn package_metadata() -> Result<()> {
    let package = Package::parse(
        PathBuf::from("fixture"),
        "[workspace.package]\nversion = '9.9.9'\n[package]\nversion = '1.2.3-rc.1+build'\nname = 'example'",
    )?;
    assert_eq!(
        package.download_url(),
        "https://static.crates.io/crates/example/1.2.3-rc.1+build/download"
    );
    for manifest in [
        "",
        "[package]\nname = 'bad/path'\nversion = '1.0.0'",
        "[package]\nname = 'example'\nversion = 'not-semver'",
        "[package]\nname = 'example'\nversion.workspace = true",
    ] {
        assert!(
            Package::parse(PathBuf::new(), manifest).is_err(),
            "{manifest}"
        );
    }
    Ok(())
}

#[test]
fn registry_statuses() -> Result<()> {
    for status in [200, 204, 206, 299] {
        assert!(published_status(status)?);
    }
    for status in [400, 403, 404, 429, 499] {
        assert!(!published_status(status)?);
    }
    for status in [0, 302, 399, 500, 503] {
        assert!(published_status(status).is_err());
    }
    Ok(())
}

#[test]
fn publication_order_and_resume() -> Result<()> {
    let packages = [package("first")?, package("second")?, package("third")?];
    let mut published = Vec::new();
    let mut output = Vec::new();
    release(
        &packages,
        false,
        |package| Ok(package.name == "first"),
        |package| {
            published.push(package.name.clone());
            Ok(())
        },
        &mut output,
    )?;
    assert_eq!(published, ["second", "third"]);
    let output = String::from_utf8(output)?;
    assert!(output.contains("first 1.2.3-rc.1+build is already published, skipping"));
    assert!(output.ends_with("Publish complete!\n"));
    Ok(())
}

#[test]
fn dry_run_never_publishes() -> Result<()> {
    let packages = [package("first")?, package("second")?];
    let mut output = Vec::new();
    release(
        &packages,
        true,
        |package| Ok(package.name == "first"),
        |_| bail!("dry run attempted publication"),
        &mut output,
    )?;
    let output = String::from_utf8(output)?;
    assert!(output.contains("first 1.2.3-rc.1+build is already published, skipping"));
    assert!(output.contains("Would publish second 1.2.3-rc.1+build"));
    assert!(output.ends_with("Dry run complete.\n"));
    Ok(())
}

#[test]
fn errors_stop_before_next_crate() -> Result<()> {
    let packages = [package("first")?, package("second")?, package("third")?];
    for registry_failure in [true, false] {
        let mut checked = Vec::new();
        let mut attempted = Vec::new();
        let mut output = Vec::new();
        let result = release(
            &packages,
            false,
            |package| {
                checked.push(package.name.clone());
                ensure!(
                    !(registry_failure && package.name == "second"),
                    "registry failure"
                );
                Ok(false)
            },
            |package| {
                attempted.push(package.name.clone());
                ensure!(package.name != "second", "publish failure");
                Ok(())
            },
            &mut output,
        );
        assert!(result.is_err());
        assert_eq!(checked, ["first", "second"]);
        if registry_failure {
            assert_eq!(attempted, ["first"]);
        } else {
            assert_eq!(attempted, ["first", "second"]);
        }
        assert!(!String::from_utf8(output)?.contains("Publish complete!"));
    }
    Ok(())
}
