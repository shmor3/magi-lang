//! .magc bytecode file format — the compiled output of the MAGI compiler.
//!
//! Format:
//!   Magic: "MAGC" (4 bytes)
//!   Version: u16
//!   Constant pool: length u32 + entries
//!   Functions: count u32 + function definitions
//!   Entry point: u32 (index into functions)

use crate::types::DataType;

pub const MAGIC: &[u8; 4] = b"MAGC";
pub const VERSION: u16 = 1;

#[derive(Debug, Clone)]
pub struct ClassFile {
    pub version: u16,
    pub constants: Vec<Constant>,
    pub functions: Vec<Function>,
    pub entry: u32,
    pub source_file: Option<String>,
}

#[derive(Debug, Clone)]
pub enum Constant {
    Int(i64),
    Float(f64),
    String(String),
    Bool(bool),
    Null,
}

#[derive(Debug, Clone)]
pub struct Function {
    pub name: String,
    pub arity: u16,
    pub locals: u16,
    pub code: Vec<u8>,
    pub line_table: Vec<(u32, u32)>, // (bytecode offset, source line)
}

impl ClassFile {
    pub fn new() -> Self {
        ClassFile {
            version: VERSION,
            constants: Vec::new(),
            functions: Vec::new(),
            entry: 0,
            source_file: None,
        }
    }

    pub fn add_constant(&mut self, c: Constant) -> u32 {
        // Dedup constants
        for (i, existing) in self.constants.iter().enumerate() {
            match (existing, &c) {
                (Constant::Int(a), Constant::Int(b)) if a == b => return i as u32,
                (Constant::Float(a), Constant::Float(b)) if a == b => return i as u32,
                (Constant::String(a), Constant::String(b)) if a == b => return i as u32,
                (Constant::Bool(a), Constant::Bool(b)) if a == b => return i as u32,
                (Constant::Null, Constant::Null) => return i as u32,
                _ => {}
            }
        }
        self.constants.push(c);
        (self.constants.len() - 1) as u32
    }

    pub fn add_function(&mut self, f: Function) -> u32 {
        self.functions.push(f);
        (self.functions.len() - 1) as u32
    }

    pub fn serialize(&self) -> Vec<u8> {
        let mut out = Vec::new();
        out.extend_from_slice(MAGIC);
        out.extend_from_slice(&self.version.to_le_bytes());

        // Constant pool
        out.extend_from_slice(&(self.constants.len() as u32).to_le_bytes());
        for c in &self.constants {
            match c {
                Constant::Null => out.push(0),
                Constant::Bool(b) => { out.push(1); out.push(if *b { 1 } else { 0 }); }
                Constant::Int(n) => { out.push(2); out.extend_from_slice(&n.to_le_bytes()); }
                Constant::Float(f) => { out.push(3); out.extend_from_slice(&f.to_le_bytes()); }
                Constant::String(s) => {
                    out.push(4);
                    let bytes = s.as_bytes();
                    out.extend_from_slice(&(bytes.len() as u32).to_le_bytes());
                    out.extend_from_slice(bytes);
                }
            }
        }

        // Functions
        out.extend_from_slice(&(self.functions.len() as u32).to_le_bytes());
        for f in &self.functions {
            let name_bytes = f.name.as_bytes();
            out.extend_from_slice(&(name_bytes.len() as u16).to_le_bytes());
            out.extend_from_slice(name_bytes);
            out.extend_from_slice(&f.arity.to_le_bytes());
            out.extend_from_slice(&f.locals.to_le_bytes());
            out.extend_from_slice(&(f.code.len() as u32).to_le_bytes());
            out.extend_from_slice(&f.code);
            out.extend_from_slice(&(f.line_table.len() as u32).to_le_bytes());
            for (offset, line) in &f.line_table {
                out.extend_from_slice(&offset.to_le_bytes());
                out.extend_from_slice(&line.to_le_bytes());
            }
        }

        // Entry point
        out.extend_from_slice(&self.entry.to_le_bytes());

        // Source file name
        if let Some(ref name) = self.source_file {
            let bytes = name.as_bytes();
            out.extend_from_slice(&(bytes.len() as u32).to_le_bytes());
            out.extend_from_slice(bytes);
        } else {
            out.extend_from_slice(&0u32.to_le_bytes());
        }

        out
    }

    pub fn deserialize(data: &[u8]) -> Result<ClassFile, String> {
        if data.len() < 10 { return Err("file too short".into()); }
        if &data[0..4] != MAGIC { return Err("invalid magic: not a .magc file".into()); }
        let version = u16::from_le_bytes([data[4], data[5]]);
        let mut pos = 6;

        // Constant pool
        let const_count = read_u32(data, &mut pos);
        let mut constants = Vec::with_capacity(const_count as usize);
        for _ in 0..const_count {
            let tag = data[pos]; pos += 1;
            match tag {
                0 => constants.push(Constant::Null),
                1 => { let b = data[pos] != 0; pos += 1; constants.push(Constant::Bool(b)); }
                2 => { let n = i64::from_le_bytes(data[pos..pos+8].try_into().unwrap_or([0;8])); pos += 8; constants.push(Constant::Int(n)); }
                3 => { let f = f64::from_le_bytes(data[pos..pos+8].try_into().unwrap_or([0;8])); pos += 8; constants.push(Constant::Float(f)); }
                4 => {
                    let len = read_u32(data, &mut pos) as usize;
                    let s = String::from_utf8_lossy(&data[pos..pos+len]).to_string();
                    pos += len;
                    constants.push(Constant::String(s));
                }
                _ => return Err(format!("unknown constant tag: {}", tag)),
            }
        }

        // Functions
        let func_count = read_u32(data, &mut pos);
        let mut functions = Vec::with_capacity(func_count as usize);
        for _ in 0..func_count {
            let name_len = u16::from_le_bytes([data[pos], data[pos+1]]) as usize; pos += 2;
            let name = String::from_utf8_lossy(&data[pos..pos+name_len]).to_string(); pos += name_len;
            let arity = u16::from_le_bytes([data[pos], data[pos+1]]); pos += 2;
            let locals = u16::from_le_bytes([data[pos], data[pos+1]]); pos += 2;
            let code_len = read_u32(data, &mut pos) as usize;
            let code = data[pos..pos+code_len].to_vec(); pos += code_len;
            let line_count = read_u32(data, &mut pos) as usize;
            let mut line_table = Vec::with_capacity(line_count);
            for _ in 0..line_count {
                let offset = read_u32(data, &mut pos);
                let line = read_u32(data, &mut pos);
                line_table.push((offset, line));
            }
            functions.push(Function { name, arity, locals, code, line_table });
        }

        let entry = read_u32(data, &mut pos);

        let source_file = if pos + 4 <= data.len() {
            let len = read_u32(data, &mut pos) as usize;
            if len > 0 && pos + len <= data.len() {
                let s = String::from_utf8_lossy(&data[pos..pos+len]).to_string();
                Some(s)
            } else { None }
        } else { None };

        Ok(ClassFile { version, constants, functions, entry, source_file })
    }
}

fn read_u32(data: &[u8], pos: &mut usize) -> u32 {
    let v = u32::from_le_bytes(data[*pos..*pos+4].try_into().unwrap_or([0;4]));
    *pos += 4;
    v
}

impl Constant {
    pub fn to_datatype(&self) -> DataType {
        match self {
            Constant::Null => DataType::Null,
            Constant::Bool(b) => DataType::Bool(*b),
            Constant::Int(n) => DataType::Int64(*n),
            Constant::Float(f) => DataType::Float64(*f),
            Constant::String(s) => DataType::String(s.clone()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_classfile_roundtrip() {
        let mut cf = ClassFile::new();
        cf.add_constant(Constant::Int(42));
        cf.add_constant(Constant::String("hello".into()));
        cf.add_constant(Constant::Float(3.14));
        cf.add_constant(Constant::Bool(true));
        cf.add_constant(Constant::Null);
        cf.add_function(Function {
            name: "main".into(),
            arity: 0,
            locals: 2,
            code: vec![0x01, 0x02, 0x03],
            line_table: vec![(0, 1), (2, 3)],
        });
        cf.source_file = Some("test.magi".into());

        let bytes = cf.serialize();
        let cf2 = ClassFile::deserialize(&bytes).unwrap();

        assert_eq!(cf2.version, VERSION);
        assert_eq!(cf2.constants.len(), 5);
        assert_eq!(cf2.functions.len(), 1);
        assert_eq!(cf2.functions[0].name, "main");
        assert_eq!(cf2.functions[0].code, vec![0x01, 0x02, 0x03]);
        assert_eq!(cf2.source_file, Some("test.magi".into()));
    }
}
