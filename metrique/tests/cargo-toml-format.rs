use std::fs;
use std::path::Path;
use walkdir::WalkDir;

#[test]
/// Test that the Cargo.tomls do not have issues that make `cargo publish` hard
fn test_cargo_toml_format() {
    let workspace_root = Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap();

    let cargo_tomls: Vec<_> = WalkDir::new(workspace_root)
        .into_iter()
        .filter_map(|e| e.ok())
        .filter(|e| {
            e.file_name() == "Cargo.toml" && !e.path().to_str().unwrap().contains("/target/")
        })
        .map(|e| e.path().to_path_buf())
        .collect();

    assert!(!cargo_tomls.is_empty(), "No Cargo.toml files found");

    for toml_path in cargo_tomls {
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
}
