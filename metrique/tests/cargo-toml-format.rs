use rstest::rstest;
use std::fs;
use std::path::PathBuf;

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
}
