//! Class loader for the MagiVM — loads .magc files and resolves dependencies.

use super::classfile::ClassFile;
use std::collections::HashMap;
use std::path::{Path, PathBuf};

pub struct ClassLoader {
    loaded: HashMap<String, ClassFile>,
    search_paths: Vec<PathBuf>,
}

impl ClassLoader {
    pub fn new() -> Self {
        let mut paths = vec![PathBuf::from(".")];
        // Add MAGI_PATH directories
        if let Ok(magi_path) = std::env::var("MAGI_PATH") {
            for p in magi_path.split(':') {
                paths.push(PathBuf::from(p));
            }
        }
        // Add ~/.magi/packages
        if let Ok(home) = std::env::var("HOME") {
            paths.push(PathBuf::from(format!("{}/.magi/packages", home)));
        }
        ClassLoader { loaded: HashMap::new(), search_paths: paths }
    }

    pub fn load(&mut self, name: &str) -> Result<&ClassFile, String> {
        if self.loaded.contains_key(name) {
            return Ok(&self.loaded[name]);
        }

        // Search for .magc file
        let filename = format!("{}.magc", name);
        for dir in &self.search_paths {
            let path = dir.join(&filename);
            if path.exists() {
                let data = std::fs::read(&path)
                    .map_err(|e| format!("cannot read {}: {}", path.display(), e))?;
                let cf = ClassFile::deserialize(&data)?;
                self.loaded.insert(name.to_string(), cf);
                return Ok(&self.loaded[name]);
            }
        }

        Err(format!("class not found: {} (searched: {:?})", name, self.search_paths))
    }

    pub fn load_from_bytes(&mut self, name: &str, data: &[u8]) -> Result<&ClassFile, String> {
        let cf = ClassFile::deserialize(data)?;
        self.loaded.insert(name.to_string(), cf);
        Ok(&self.loaded[name])
    }

    pub fn load_from_file(&mut self, path: &Path) -> Result<&ClassFile, String> {
        let data = std::fs::read(path)
            .map_err(|e| format!("cannot read {}: {}", path.display(), e))?;
        let cf = ClassFile::deserialize(&data)?;
        let name = path.file_stem()
            .map(|s| s.to_string_lossy().to_string())
            .unwrap_or_else(|| "main".to_string());
        self.loaded.insert(name.clone(), cf);
        Ok(&self.loaded[&name])
    }

    pub fn is_loaded(&self, name: &str) -> bool {
        self.loaded.contains_key(name)
    }

    pub fn get(&self, name: &str) -> Option<&ClassFile> {
        self.loaded.get(name)
    }

    pub fn add_search_path(&mut self, path: PathBuf) {
        self.search_paths.push(path);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use super::super::classfile::{Constant, Function};

    #[test]
    fn test_classloader_load_bytes() {
        let mut cl = ClassLoader::new();
        let mut cf = ClassFile::new();
        cf.add_constant(Constant::Int(42));
        cf.add_function(Function {
            name: "main".into(), arity: 0, locals: 0,
            code: vec![0xFF], line_table: vec![],
        });
        let bytes = cf.serialize();
        let loaded = cl.load_from_bytes("test", &bytes).unwrap();
        assert_eq!(loaded.constants.len(), 1);
        assert!(cl.is_loaded("test"));
    }
}
