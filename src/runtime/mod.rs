//! MAGI Runtime — classfile compilation and virtual machine execution.
//!
//! Pipeline: .magi source → .magc classfile → MagiVM execution

pub mod vm;
pub mod classfile;
pub mod classloader;
pub mod gc;
