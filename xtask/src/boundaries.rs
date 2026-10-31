// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! The crate-boundary check: the layering of the crate graph, and one
//! integration test program per crate.

use cargo_metadata::{DependencyKind, MetadataCommand};

/// Layers from the bottom of the graph to the top.
pub const LAYERS: [&str; 7] = [
    "kernel",
    "contracts",
    "foundation",
    "engine",
    "service",
    "edge",
    "app",
];

/// Crates outside the product graph, which need no layer.
const TOOLING: [&str; 1] = ["xtask"];

/// One workspace crate as the check sees it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Crate {
    /// The package name.
    pub name: String,
    /// The declared layer, if any.
    pub layer: Option<String>,
    /// Names of the workspace crates it depends on (normal and build).
    pub depends_on: Vec<String>,
    /// Names of its integration test targets (`tests/`).
    pub test_targets: Vec<String>,
}

/// Loads the workspace and checks it.
pub fn run() -> Result<(), String> {
    let metadata = MetadataCommand::new()
        .no_deps()
        .exec()
        .map_err(|e| format!("cargo metadata failed: {e}"))?;
    let members: Vec<String> = metadata
        .workspace_packages()
        .iter()
        .map(|p| p.name.to_string())
        .collect();
    let crates: Vec<Crate> = metadata
        .workspace_packages()
        .iter()
        .map(|p| Crate {
            name: p.name.to_string(),
            layer: p
                .metadata
                .pointer("/vellum/layer")
                .and_then(|v| v.as_str())
                .map(str::to_owned),
            depends_on: p
                .dependencies
                .iter()
                .filter(|d| d.kind != DependencyKind::Development)
                .filter(|d| members.contains(&d.name))
                .map(|d| d.name.clone())
                .collect(),
            test_targets: p
                .targets
                .iter()
                .filter(|t| t.is_test())
                .map(|t| t.name.clone())
                .collect(),
        })
        .collect();
    let violations = check(&crates);
    if violations.is_empty() {
        Ok(())
    } else {
        Err(format!(
            "crate boundaries violated:\n  {}",
            violations.join("\n  ")
        ))
    }
}

fn rank(layer: &str) -> Option<usize> {
    LAYERS.iter().position(|l| *l == layer)
}

/// Returns one message per violation; empty means the graph is valid.
#[must_use]
pub fn check(crates: &[Crate]) -> Vec<String> {
    let mut violations = Vec::new();
    for krate in crates
        .iter()
        .filter(|c| !TOOLING.contains(&c.name.as_str()))
    {
        // Each test program links the whole dependency graph again; one
        // per test file overflowed the coverage runner (ADR 0076, D324).
        if krate.test_targets.len() > 1 {
            violations.push(format!(
                "{} has {} integration test programs ({}): move them into \
                 modules of tests/it/main.rs",
                krate.name,
                krate.test_targets.len(),
                krate.test_targets.join(", "),
            ));
        }
        let Some(own) = krate.layer.as_deref() else {
            violations.push(format!(
                "{} declares no [package.metadata.vellum] layer",
                krate.name
            ));
            continue;
        };
        let Some(own_rank) = rank(own) else {
            violations
                .push(format!("{} has unknown layer `{own}`", krate.name));
            continue;
        };
        for dep in &krate.depends_on {
            let dep_layer = crates
                .iter()
                .find(|c| &c.name == dep)
                .and_then(|c| c.layer.as_deref());
            match dep_layer.and_then(rank) {
                Some(dep_rank) if dep_rank < own_rank => {}
                _ => violations.push(format!(
                    "{} ({own}) may not depend on {dep} ({}): only lower \
                        layers are allowed",
                    krate.name,
                    dep_layer.unwrap_or("no layer"),
                )),
            }
        }
    }
    violations
}

#[cfg(test)]
mod tests {
    use super::*;

    fn krate(name: &str, layer: Option<&str>, deps: &[&str]) -> Crate {
        Crate {
            name: name.to_owned(),
            layer: layer.map(str::to_owned),
            depends_on: deps.iter().map(|d| (*d).to_owned()).collect(),
            test_targets: vec!["it".to_owned()],
        }
    }

    #[test]
    fn downward_dependencies_pass() {
        let crates = [
            krate("k", Some("kernel"), &[]),
            krate("s", Some("service"), &["k"]),
            krate("a", Some("app"), &["s", "k"]),
            krate("xtask", None, &["k"]),
        ];
        assert!(check(&crates).is_empty());
    }

    #[test]
    fn same_layer_dependency_fails() {
        let crates = [
            krate("pages", Some("service"), &["collab"]),
            krate("collab", Some("service"), &[]),
        ];
        let v = check(&crates);
        assert_eq!(v.len(), 1);
        assert!(
            v[0].contains("pages (service) may not depend on collab (service)")
        );
    }

    #[test]
    fn upward_dependency_fails() {
        let crates = [
            krate("k", Some("kernel"), &["a"]),
            krate("a", Some("app"), &[]),
        ];
        assert_eq!(check(&crates).len(), 1);
    }

    #[test]
    fn missing_or_unknown_layers_fail() {
        let crates = [
            krate("a", None, &[]),
            krate("b", Some("middle"), &[]),
            krate("c", Some("app"), &["a"]),
        ];
        let v = check(&crates);
        assert_eq!(v.len(), 3);
        assert!(v[0].contains("declares no"));
        assert!(v[1].contains("unknown layer `middle`"));
        assert!(v[2].contains("(no layer)"));
    }

    #[test]
    fn a_second_test_program_fails() {
        let mut two = krate("k", Some("kernel"), &[]);
        two.test_targets = vec!["it".to_owned(), "extra".to_owned()];
        let v = check(&[two]);
        assert_eq!(v.len(), 1);
        assert!(v[0].contains("k has 2 integration test programs (it, extra)"));
    }

    #[test]
    fn the_real_workspace_passes() {
        assert_eq!(run(), Ok(()));
    }
}
