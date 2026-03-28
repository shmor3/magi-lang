//! Garbage collector for the MagiVM runtime.
//!
//! Mark-and-sweep collector with generational hints.

use crate::types::DataType;
use std::collections::{HashMap, HashSet};

pub type ObjId = u64;

#[derive(Debug)]
pub struct GarbageCollector {
    objects: HashMap<ObjId, GcObject>,
    next_id: ObjId,
    roots: HashSet<ObjId>,
    bytes_allocated: usize,
    gc_threshold: usize,
    collections: u64,
    total_freed: u64,
}

#[derive(Debug, Clone)]
struct GcObject {
    data: DataType,
    marked: bool,
    size: usize,
}

impl GarbageCollector {
    pub fn new() -> Self {
        GarbageCollector {
            objects: HashMap::new(),
            next_id: 1,
            roots: HashSet::new(),
            bytes_allocated: 0,
            gc_threshold: 1024 * 1024, // 1MB
            collections: 0,
            total_freed: 0,
        }
    }

    pub fn alloc(&mut self, data: DataType) -> ObjId {
        let size = Self::estimate_size(&data);
        let id = self.next_id;
        self.next_id += 1;
        self.objects.insert(id, GcObject { data, marked: false, size });
        self.bytes_allocated += size;

        if self.bytes_allocated > self.gc_threshold {
            self.collect();
        }

        id
    }

    pub fn read(&self, id: ObjId) -> Option<&DataType> {
        self.objects.get(&id).map(|o| &o.data)
    }

    pub fn write(&mut self, id: ObjId, data: DataType) {
        if let Some(obj) = self.objects.get_mut(&id) {
            let old_size = obj.size;
            obj.size = Self::estimate_size(&data);
            obj.data = data;
            self.bytes_allocated = self.bytes_allocated.saturating_sub(old_size) + obj.size;
        }
    }

    pub fn add_root(&mut self, id: ObjId) {
        self.roots.insert(id);
    }

    pub fn remove_root(&mut self, id: ObjId) {
        self.roots.remove(&id);
    }

    pub fn collect(&mut self) {
        // Mark phase
        for obj in self.objects.values_mut() {
            obj.marked = false;
        }

        let roots: Vec<ObjId> = self.roots.iter().copied().collect();
        for root in roots {
            self.mark(root);
        }

        // Sweep phase
        let before = self.objects.len();
        let mut freed_bytes = 0usize;
        self.objects.retain(|_, obj| {
            if obj.marked {
                true
            } else {
                freed_bytes += obj.size;
                false
            }
        });

        self.bytes_allocated = self.bytes_allocated.saturating_sub(freed_bytes);
        self.total_freed += (before - self.objects.len()) as u64;
        self.collections += 1;

        // Grow threshold if we're using > 50% after collection
        if self.bytes_allocated > self.gc_threshold / 2 {
            self.gc_threshold *= 2;
        }
    }

    fn mark(&mut self, id: ObjId) {
        if let Some(obj) = self.objects.get_mut(&id) {
            if obj.marked { return; }
            obj.marked = true;

            // Trace references in the object
            let refs = Self::extract_refs(&obj.data);
            for r in refs {
                self.mark(r);
            }
        }
    }

    fn extract_refs(data: &DataType) -> Vec<ObjId> {
        // In the runtime, references are stored as Int64 object IDs
        match data {
            DataType::Array(arr) => {
                arr.iter().filter_map(|v| {
                    if let DataType::Int64(id) = v { Some(*id as ObjId) } else { None }
                }).collect()
            }
            DataType::Map(m) => {
                m.values().filter_map(|v| {
                    if let DataType::Int64(id) = v { Some(*id as ObjId) } else { None }
                }).collect()
            }
            _ => vec![],
        }
    }

    fn estimate_size(data: &DataType) -> usize {
        match data {
            DataType::Null | DataType::Bool(_) => 8,
            DataType::Int64(_) | DataType::Float64(_) => 8,
            DataType::Int32(_) | DataType::Float32(_) | DataType::Uint32(_) => 4,
            DataType::Uint64(_) => 8,
            DataType::String(s) => 24 + s.len(),
            DataType::Array(a) => 24 + a.len() * 16,
            DataType::Map(m) => 48 + m.len() * 32,
            DataType::Bytes(b) => 24 + b.len(),
            DataType::Set(s) => 24 + s.len() * 16,
            DataType::Tuple(t) => 24 + t.len() * 16,
            DataType::Future(_) => 32,
        }
    }

    pub fn stats(&self) -> GcStats {
        GcStats {
            objects: self.objects.len(),
            bytes_allocated: self.bytes_allocated,
            collections: self.collections,
            total_freed: self.total_freed,
            threshold: self.gc_threshold,
        }
    }
}

#[derive(Debug)]
pub struct GcStats {
    pub objects: usize,
    pub bytes_allocated: usize,
    pub collections: u64,
    pub total_freed: u64,
    pub threshold: usize,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_gc_alloc_read_write() {
        let mut gc = GarbageCollector::new();
        let id = gc.alloc(DataType::Int64(42));
        assert_eq!(gc.read(id), Some(&DataType::Int64(42)));
        gc.write(id, DataType::Int64(100));
        assert_eq!(gc.read(id), Some(&DataType::Int64(100)));
    }

    #[test]
    fn test_gc_collect_unreachable() {
        let mut gc = GarbageCollector::new();
        let id1 = gc.alloc(DataType::Int64(1));
        let id2 = gc.alloc(DataType::Int64(2));
        gc.add_root(id1);
        gc.collect();
        assert!(gc.read(id1).is_some());
        assert!(gc.read(id2).is_none()); // collected
    }

    #[test]
    fn test_gc_stats() {
        let mut gc = GarbageCollector::new();
        for i in 0..100 {
            gc.alloc(DataType::Int64(i));
        }
        gc.collect();
        let stats = gc.stats();
        assert_eq!(stats.objects, 0); // no roots → all collected
        assert_eq!(stats.collections, 1);
    }
}
