use rstest::rstest;
use std::fs;
use std::path::PathBuf;

const MSRV: &'static str = "1.89.0";

// return just major and minor versions of msrv
fn msrv_major_minor() -> String {
    MSRV.split('.').take(2).collect::<Vec<_>>().join(".")
}

#[rstest]
/// Test that the Cargo.tomls do not have issues that make `cargo publish` hard
fn test_cargo_toml_format(
    // .. since workspace root is parent of package root
    #[files("../**/Cargo.toml")]
    #[exclude("/target/")]
    toml_path: PathBuf,
) {
    let content = fs::read_to_string(&toml_path)
        .unwrap_or_else(|e| panic!("Failed to read {}: {}", toml_path.display(), e));

    let toml = toml::from_str::<toml::Value>(&content)
        .unwrap_or_else(|e| panic!("Failed to parse {}: {}", toml_path.display(), e));

    if let Some(deps) = toml.get("dependencies").and_then(|d| d.as_table()) {
        for (name, value) in deps {
            if name.starts_with("metrique") {
                let dep_table = value.as_table().unwrap_or_else(|| {
                    panic!(
                        "metrique dependency '{}' in {} must be a table",
                        name,
                        toml_path.display()
                    )
                });
                assert!(
                    dep_table.contains_key("path"),
                    "metrique dependency '{}' in {} must have 'path' property to use crate from property",
                    name,
                    toml_path.display()
                );
                assert!(
                    dep_table.contains_key("version"),
                    "metrique dependency '{}' in {} must have a 'version' property to allow publishing",
                    name,
                    toml_path.display()
                );
            }
        }
    }

    if let Some(deps) = toml.get("dev-dependencies").and_then(|d| d.as_table()) {
        for (name, value) in deps {
            if name.starts_with("metrique") {
                let dep_table = value.as_table().unwrap_or_else(|| {
                    panic!(
                        "metrique dependency '{}' in {} must be a table",
                        name,
                        toml_path.display()
                    )
                });
                assert!(
                    dep_table.contains_key("path"),
                    "metrique dependency '{}' in {} must have 'path' property to use crate from property",
                    name,
                    toml_path.display()
                );
                assert!(
                    !dep_table.contains_key("version"),
                    "metrique dependency '{}' in {} must not use the 'version' property to prevent chicken-and-egg problems",
                    name,
                    toml_path.display()
                );
            }
        }
    }

    // Check that there is a consistent package.rust-version amongst all packages since proper
    // MSRV support requires it.
    let package = toml.get("package").and_then(|p| p.as_table());
    let workspace = toml.get("workspace").and_then(|p| p.as_table());

    if package.is_none() && workspace.is_none() {
        panic!(
            "{} is neither a package nor a workspace?",
            toml_path.display()
        );
    }

    if let Some(package) = package {
        let rust_version = package
            .get("rust-version")
            .and_then(|v| v.as_str())
            .unwrap_or_else(|| panic!("Missing package.rust-version in {}", toml_path.display()));

        assert_eq!(
            rust_version,
            msrv_major_minor(),
            "package.rust-version in {} must equal MSRV ({})",
            toml_path.display(),
            msrv_major_minor()
        );
    }
}

#[rstest]
/// Check that the UI tests run on the MSRV
fn test_msrv_ui(
    // .. since workspace root is parent of package root
    #[files("../metrique/tests/ui.rs")] rs_path: PathBuf,
) {
    let msrv_string = format!("stable({MSRV})");
    let file = std::fs::read_to_string(rs_path).unwrap();
    assert!(
        file.contains("rustversion"),
        "ui.rs does not contain rustversion, this test needs to be updated to the new mechanism"
    );
    for line in file.lines() {
        if line.contains("rustversion") {
            assert!(
                line.contains(&msrv_string),
                "version {} does not contain msrv {}",
                line,
                msrv_string
            );
        }
    }
}

#[rstest]
/// Check that build yml tests on the MSRV
fn test_build_yml(
    // .. since workspace root is parent of package root
    #[files("../Cargo.toml")] base_path: PathBuf,
) {
    let rs_path = base_path
        .parent()
        .unwrap()
        .join(".github/workflows/build.yml");
    let msrv_string = format!("- \"{MSRV}\" # Current MSRV");
    let file = std::fs::read_to_string(rs_path).unwrap();
    assert!(
        file.contains(&msrv_string),
        "build.yml must run at the msrv"
    );
}
