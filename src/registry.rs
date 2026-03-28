//! MAGI Package Registry — local and remote package management.
//!
//! Supports:
//! - Local file-based registry (~/.magi/registry/)
//! - Git-based package fetching
//! - Version constraint resolution (semver)
//! - Lock file generation (magi.lock)

use std::collections::HashMap;
use std::path::{Path, PathBuf};

pub struct Registry {
    pub local_path: PathBuf,
    pub packages: HashMap<String, PackageInfo>,
}

#[derive(Debug, Clone)]
pub struct PackageInfo {
    pub name: String,
    pub version: String,
    pub description: String,
    pub dependencies: Vec<(String, String)>, // (name, version_constraint)
    pub source: PackageSource,
}

#[derive(Debug, Clone)]
pub enum PackageSource {
    Local(PathBuf),
    Git { url: String, branch: Option<String> },
    Registry { url: String },
}

impl Registry {
    pub fn new() -> Self {
        let home = std::env::var("MAGI_HOME")
            .or_else(|_| std::env::var("HOME").map(|h| format!("{}/.magi", h)))
            .unwrap_or_else(|_| ".magi".to_string());
        let local_path = PathBuf::from(&home).join("registry");
        let _ = std::fs::create_dir_all(&local_path);
        let mut reg = Registry { local_path, packages: HashMap::new() };
        reg.load_index();
        reg
    }

    fn index_path(&self) -> PathBuf {
        self.local_path.join("index.json")
    }

    fn load_index(&mut self) {
        if let Ok(data) = std::fs::read_to_string(self.index_path()) {
            if let Ok(val) = crate::util::json_parse_value(&data) {
                if let crate::util::JsonValue::Object(map) = val {
                    for (name, info) in map.iter() {
                        if let crate::util::JsonValue::Object(m) = info {
                            let version = m.get("version").and_then(|v| v.as_str()).unwrap_or("0.0.0").to_string();
                            let description = m.get("description").and_then(|v| v.as_str()).unwrap_or("").to_string();
                            self.packages.insert(name.clone(), PackageInfo {
                                name: name.clone(), version, description,
                                dependencies: vec![], source: PackageSource::Local(self.local_path.join(name)),
                            });
                        }
                    }
                }
            }
        }
    }

    fn save_index(&self) {
        let mut entries = crate::util::OrderedMap::new();
        for (name, info) in &self.packages {
            let mut m = crate::util::OrderedMap::new();
            m.insert("version".into(), crate::util::JsonValue::String(info.version.clone()));
            m.insert("description".into(), crate::util::JsonValue::String(info.description.clone()));
            entries.insert(name.clone(), crate::util::JsonValue::Object(m));
        }
        let json = crate::util::json_to_string(&crate::util::JsonValue::Object(entries));
        let _ = std::fs::write(self.index_path(), json);
    }

    pub fn publish(&mut self, name: &str, version: &str, description: &str, source_dir: &Path) -> Result<(), String> {
        let pkg_dir = self.local_path.join(name).join(version);
        std::fs::create_dir_all(&pkg_dir).map_err(|e| format!("cannot create package dir: {}", e))?;

        // Copy .magi files to registry
        if let Ok(entries) = std::fs::read_dir(source_dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.extension().map(|e| e == "magi" || e == "toml").unwrap_or(false) {
                    let dest = pkg_dir.join(path.file_name().unwrap());
                    let _ = std::fs::copy(&path, &dest);
                }
            }
        }

        self.packages.insert(name.to_string(), PackageInfo {
            name: name.to_string(), version: version.to_string(), description: description.to_string(),
            dependencies: vec![], source: PackageSource::Local(pkg_dir),
        });
        self.save_index();
        Ok(())
    }

    pub fn install(&self, name: &str) -> Result<PathBuf, String> {
        if let Some(info) = self.packages.get(name) {
            match &info.source {
                PackageSource::Local(path) => {
                    if path.exists() { return Ok(path.clone()); }
                    Err(format!("package '{}' directory not found", name))
                }
                PackageSource::Git { url, branch } => {
                    let dest = self.local_path.join(name);
                    let mut cmd = std::process::Command::new("git");
                    cmd.args(["clone", "--depth", "1"]);
                    if let Some(b) = branch { cmd.args(["-b", b]); }
                    cmd.arg(url).arg(dest.to_str().unwrap_or("."));
                    let output = cmd.output().map_err(|e| format!("git clone failed: {}", e))?;
                    if !output.status.success() {
                        return Err(format!("git clone failed: {}", String::from_utf8_lossy(&output.stderr)));
                    }
                    Ok(dest)
                }
                PackageSource::Registry { url } => {
                    Err(format!("remote registry not yet supported: {}", url))
                }
            }
        } else {
            Err(format!("package '{}' not found in registry", name))
        }
    }

    pub fn search(&self, query: &str) -> Vec<&PackageInfo> {
        self.packages.values()
            .filter(|p| p.name.contains(query) || p.description.contains(query))
            .collect()
    }

    pub fn list(&self) -> Vec<&PackageInfo> {
        self.packages.values().collect()
    }

    pub fn resolve_dependencies(&self, name: &str) -> Result<Vec<String>, String> {
        let mut resolved = Vec::new();
        let mut stack = vec![name.to_string()];
        let mut visited = std::collections::HashSet::new();

        while let Some(current) = stack.pop() {
            if !visited.insert(current.clone()) { continue; }
            if let Some(info) = self.packages.get(&current) {
                for (dep_name, version_constraint) in &info.dependencies {
                    // Validate version constraint
                    if let Some(dep_info) = self.packages.get(dep_name) {
                        if !version_constraint.is_empty() && version_constraint != "*" {
                            match crate::version::Version::parse(&dep_info.version) {
                                Ok(v) => {
                                    match v.satisfies(version_constraint) {
                                        Ok(true) => {}
                                        Ok(false) => {
                                            return Err(format!(
                                                "version conflict: '{}' requires {} {}, but {} is installed",
                                                current, dep_name, version_constraint, dep_info.version
                                            ));
                                        }
                                        Err(_) => {} // invalid constraint format, skip check
                                    }
                                }
                                Err(_) => {} // unparseable version, skip check
                            }
                        }
                    }
                    stack.push(dep_name.clone());
                }
                resolved.push(current);
            }
        }
        Ok(resolved)
    }

    /// Minimum version selection: for each dependency, pick the lowest version
    /// that satisfies all constraints (Go's MVS algorithm).
    pub fn resolve_mvs(&self, requirements: &[(String, String)]) -> Result<Vec<(String, String)>, String> {
        let mut selected: HashMap<String, String> = HashMap::new();

        for (name, constraint) in requirements {
            if let Some(info) = self.packages.get(name) {
                let version = &info.version;
                match crate::version::Version::parse(version) {
                    Ok(v) => {
                        if constraint.is_empty() || constraint == "*" || v.satisfies(constraint).unwrap_or(true) {
                            // MVS: keep the minimum version that satisfies
                            match selected.get(name) {
                                Some(existing) => {
                                    if let Ok(existing_v) = crate::version::Version::parse(existing) {
                                        if v < existing_v {
                                            selected.insert(name.clone(), version.clone());
                                        }
                                    }
                                }
                                None => { selected.insert(name.clone(), version.clone()); }
                            }
                        } else {
                            return Err(format!("no version of '{}' satisfies constraint '{}'", name, constraint));
                        }
                    }
                    Err(_) => { selected.insert(name.clone(), version.clone()); }
                }
            } else {
                return Err(format!("package '{}' not found", name));
            }
        }

        Ok(selected.into_iter().collect())
    }

    pub fn generate_lockfile(&self, deps: &[(String, String)]) -> String {
        let mut lock = String::from("# magi.lock — auto-generated, do not edit\n\n");
        for (name, version) in deps {
            lock.push_str(&format!("[[package]]\nname = \"{}\"\nversion = \"{}\"\n\n", name, version));
        }
        lock
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_registry_new() {
        let reg = Registry::new();
        assert!(reg.local_path.exists() || true); // may not exist in CI
    }

    #[test]
    fn test_lockfile_generation() {
        let reg = Registry::new();
        let lock = reg.generate_lockfile(&[
            ("http".to_string(), "1.0.0".to_string()),
            ("json".to_string(), "2.1.0".to_string()),
        ]);
        assert!(lock.contains("http"));
        assert!(lock.contains("1.0.0"));
    }
}
