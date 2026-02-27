//! Core types for the MAGI language
//!
//! This module provides the fundamental types used throughout the MAGI system,
//! including the strict DataType system for type-safe plugin communication.

pub mod operations;

pub use operations::*;

use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap};

// =============================================================================
// DataType
// =============================================================================

/// Strict data type enum for type-safe plugin communication.
///
/// Note: The API version uses `#[serde(tag, content)]` for JSON representation.
/// The PDK version uses default serde enum encoding. Both share the same
/// variant set and method surface.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", content = "value")]
pub enum DataType {
    String(String),
    Int32(i32),
    Int64(i64),
    Uint32(u32),
    Uint64(u64),
    Float32(f32),
    Float64(f64),
    Bool(bool),
    Bytes(Vec<u8>),
    Array(Vec<DataType>),
    Map(BTreeMap<String, DataType>),
    Future(Box<FutureState>),
    #[default]
    Null,
}

/// State of an asynchronous computation.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum FutureState {
    Pending,
    Resolved(Box<DataType>),
    Rejected(String),
}

impl DataType {
    // =========================================================================
    // Strict accessors
    // =========================================================================

    pub fn as_str(&self) -> Option<&str> {
        match self {
            DataType::String(s) => Some(s),
            _ => None,
        }
    }

    pub fn as_bool(&self) -> Option<bool> {
        match self {
            DataType::Bool(b) => Some(*b),
            _ => None,
        }
    }

    pub fn as_bytes(&self) -> Option<&[u8]> {
        match self {
            DataType::Bytes(bytes) => Some(bytes),
            _ => None,
        }
    }

    pub fn as_array(&self) -> Option<&Vec<DataType>> {
        match self {
            DataType::Array(arr) => Some(arr),
            _ => None,
        }
    }

    pub fn as_map(&self) -> Option<&BTreeMap<String, DataType>> {
        match self {
            DataType::Map(map) => Some(map),
            _ => None,
        }
    }

    // =========================================================================
    // Type discrimination
    // =========================================================================

    pub fn is_null(&self) -> bool {
        matches!(self, DataType::Null)
    }

    pub fn is_bytes(&self) -> bool {
        matches!(self, DataType::Bytes(_))
    }

    pub fn type_name(&self) -> &'static str {
        match self {
            DataType::String(_) => "string",
            DataType::Int32(_) => "int32",
            DataType::Int64(_) => "int64",
            DataType::Uint32(_) => "uint32",
            DataType::Uint64(_) => "uint64",
            DataType::Float32(_) => "float32",
            DataType::Float64(_) => "float64",
            DataType::Bool(_) => "bool",
            DataType::Bytes(_) => "bytes",
            DataType::Array(_) => "array",
            DataType::Map(_) => "map",
            DataType::Future(_) => "future",
            DataType::Null => "null",
        }
    }

    // =========================================================================
    // Map/Array/Collection operations
    // =========================================================================

    pub fn get(&self, key: &str) -> Option<&DataType> {
        match self {
            DataType::Map(map) => map.get(key),
            _ => None,
        }
    }

    pub fn len(&self) -> usize {
        match self {
            DataType::Array(arr) => arr.len(),
            DataType::Map(map) => map.len(),
            DataType::Bytes(b) => b.len(),
            DataType::String(s) => s.chars().count(),
            _ => 0,
        }
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    // =========================================================================
    // Coercion methods
    // =========================================================================

    pub fn to_bool(&self) -> bool {
        match self {
            DataType::Bool(b) => *b,
            DataType::Int32(i) => *i != 0,
            DataType::Int64(i) => *i != 0,
            DataType::Uint32(u) => *u != 0,
            DataType::Uint64(u) => *u != 0,
            DataType::Float32(f) => *f != 0.0 && !f.is_nan(),
            DataType::Float64(f) => *f != 0.0 && !f.is_nan(),
            DataType::String(s) => !matches!(
                s.trim().to_lowercase().as_str(),
                "" | "false" | "0" | "no" | "off"
            ),
            DataType::Null => false,
            DataType::Bytes(b) => !b.is_empty(),
            DataType::Array(a) => !a.is_empty(),
            DataType::Map(m) => !m.is_empty(),
            DataType::Future(_) => true,
        }
    }

    pub fn to_i64(&self) -> Option<i64> {
        match self {
            DataType::Int32(i) => Some(*i as i64),
            DataType::Int64(i) => Some(*i),
            DataType::Uint32(u) => Some(*u as i64),
            DataType::Uint64(u) => i64::try_from(*u).ok(),
            DataType::Float32(f) => {
                let v = *f as f64;
                if v.is_finite() && v >= i64::MIN as f64 && v < i64::MAX as f64 {
                    Some(v as i64)
                } else {
                    None
                }
            }
            DataType::Float64(f) => {
                if f.is_finite() && *f >= i64::MIN as f64 && *f < i64::MAX as f64 {
                    Some(*f as i64)
                } else {
                    None
                }
            }
            DataType::Bool(b) => Some(if *b { 1 } else { 0 }),
            DataType::String(s) => s.trim().parse::<i64>().ok(),
            _ => None,
        }
    }

    pub fn to_f64(&self) -> Option<f64> {
        match self {
            DataType::Float64(f) => Some(*f),
            DataType::Float32(f) => Some(*f as f64),
            DataType::Int32(i) => Some(*i as f64),
            DataType::Int64(i) => Some(*i as f64),
            DataType::Uint32(u) => Some(*u as f64),
            DataType::Uint64(u) => Some(*u as f64),
            DataType::Bool(b) => Some(if *b { 1.0 } else { 0.0 }),
            DataType::String(s) => s.trim().parse::<f64>().ok(),
            _ => None,
        }
    }

    pub fn to_string_lossy(&self) -> String {
        self.to_string()
    }

    pub fn to_bytes(&self) -> Option<Vec<u8>> {
        match self {
            DataType::Bytes(b) => Some(b.clone()),
            DataType::String(s) => Some(s.as_bytes().to_vec()),
            _ => None,
        }
    }

    // =========================================================================
    // JSON interop
    // =========================================================================

    pub fn from_json(value: serde_json::Value) -> DataType {
        match value {
            serde_json::Value::Null => DataType::Null,
            serde_json::Value::Bool(b) => DataType::Bool(b),
            serde_json::Value::Number(n) => {
                if let Some(i) = n.as_i64() {
                    DataType::Int64(i)
                } else if let Some(u) = n.as_u64() {
                    DataType::Uint64(u)
                } else {
                    DataType::Float64(n.as_f64().unwrap_or(0.0))
                }
            }
            serde_json::Value::String(s) => DataType::String(s),
            serde_json::Value::Array(arr) => {
                DataType::Array(arr.into_iter().map(DataType::from_json).collect())
            }
            serde_json::Value::Object(obj) => DataType::Map(
                obj.into_iter()
                    .map(|(k, v)| (k, DataType::from_json(v)))
                    .collect(),
            ),
        }
    }

    pub fn to_json(&self) -> serde_json::Value {
        match self {
            DataType::Null => serde_json::Value::Null,
            DataType::Bool(b) => serde_json::Value::Bool(*b),
            DataType::Int32(i) => serde_json::json!(*i),
            DataType::Int64(i) => serde_json::json!(*i),
            DataType::Uint32(u) => serde_json::json!(*u),
            DataType::Uint64(u) => serde_json::json!(*u),
            DataType::Float32(f) => {
                if f.is_finite() {
                    serde_json::json!(*f)
                } else {
                    serde_json::Value::Null
                }
            }
            DataType::Float64(f) => {
                if f.is_finite() {
                    serde_json::json!(*f)
                } else {
                    serde_json::Value::Null
                }
            }
            DataType::String(s) => serde_json::Value::String(s.clone()),
            DataType::Bytes(b) => serde_json::json!(b),
            DataType::Array(arr) => {
                serde_json::Value::Array(arr.iter().map(|v| v.to_json()).collect())
            }
            DataType::Map(map) => {
                let obj: serde_json::Map<String, serde_json::Value> =
                    map.iter().map(|(k, v)| (k.clone(), v.to_json())).collect();
                serde_json::Value::Object(obj)
            }
            DataType::Future(state) => match state.as_ref() {
                FutureState::Pending => serde_json::json!({"state": "pending"}),
                FutureState::Resolved(val) => {
                    serde_json::json!({"state": "resolved", "value": val.to_json()})
                }
                FutureState::Rejected(err) => {
                    serde_json::json!({"state": "rejected", "error": err})
                }
            },
        }
    }
}

// =============================================================================
// Display
// =============================================================================

impl std::fmt::Display for DataType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            DataType::String(s) => write!(f, "{}", s),
            DataType::Int32(i) => write!(f, "{}", i),
            DataType::Int64(i) => write!(f, "{}", i),
            DataType::Uint32(u) => write!(f, "{}", u),
            DataType::Uint64(u) => write!(f, "{}", u),
            DataType::Float32(v) => write!(f, "{}", v),
            DataType::Float64(v) => write!(f, "{}", v),
            DataType::Bool(b) => write!(f, "{}", b),
            DataType::Bytes(b) => write!(f, "<{} bytes>", b.len()),
            DataType::Array(arr) => write!(f, "[{} items]", arr.len()),
            DataType::Map(map) => write!(f, "{{{} entries}}", map.len()),
            DataType::Future(state) => match state.as_ref() {
                FutureState::Pending => write!(f, "<future:pending>"),
                FutureState::Resolved(val) => write!(f, "<future:resolved({})>", val),
                FutureState::Rejected(err) => write!(f, "<future:rejected({})>", err),
            },
            DataType::Null => write!(f, "null"),
        }
    }
}

// =============================================================================
// From impls
// =============================================================================

impl From<String> for DataType {
    fn from(s: String) -> Self {
        DataType::String(s)
    }
}

impl From<&str> for DataType {
    fn from(s: &str) -> Self {
        DataType::String(s.to_string())
    }
}

impl From<i8> for DataType {
    fn from(i: i8) -> Self {
        DataType::Int32(i as i32)
    }
}

impl From<i16> for DataType {
    fn from(i: i16) -> Self {
        DataType::Int32(i as i32)
    }
}

impl From<i32> for DataType {
    fn from(i: i32) -> Self {
        DataType::Int32(i)
    }
}

impl From<i64> for DataType {
    fn from(i: i64) -> Self {
        DataType::Int64(i)
    }
}

impl From<u8> for DataType {
    fn from(u: u8) -> Self {
        DataType::Uint32(u as u32)
    }
}

impl From<u16> for DataType {
    fn from(u: u16) -> Self {
        DataType::Uint32(u as u32)
    }
}

impl From<u32> for DataType {
    fn from(u: u32) -> Self {
        DataType::Uint32(u)
    }
}

impl From<u64> for DataType {
    fn from(u: u64) -> Self {
        DataType::Uint64(u)
    }
}

impl From<f32> for DataType {
    fn from(f: f32) -> Self {
        DataType::Float32(f)
    }
}

impl From<f64> for DataType {
    fn from(f: f64) -> Self {
        DataType::Float64(f)
    }
}

impl From<bool> for DataType {
    fn from(b: bool) -> Self {
        DataType::Bool(b)
    }
}

impl From<Vec<u8>> for DataType {
    fn from(b: Vec<u8>) -> Self {
        DataType::Bytes(b)
    }
}

impl From<&[u8]> for DataType {
    fn from(b: &[u8]) -> Self {
        DataType::Bytes(b.to_vec())
    }
}

impl From<Vec<DataType>> for DataType {
    fn from(arr: Vec<DataType>) -> Self {
        DataType::Array(arr)
    }
}

impl From<BTreeMap<String, DataType>> for DataType {
    fn from(map: BTreeMap<String, DataType>) -> Self {
        DataType::Map(map)
    }
}

impl From<HashMap<String, DataType>> for DataType {
    fn from(map: HashMap<String, DataType>) -> Self {
        DataType::Map(map.into_iter().collect())
    }
}

impl From<Option<DataType>> for DataType {
    fn from(opt: Option<DataType>) -> Self {
        opt.unwrap_or(DataType::Null)
    }
}

// =============================================================================
// ChannelType
// =============================================================================

/// The kind of data a channel carries.
///
/// The kind of data a graph channel carries. Aligned with the UI's `DataTypeId`
/// from `channel-types.ts` for backend/frontend consistency.
///
/// Aligned with `DataType` primitives — every variant corresponds to a
/// concrete data representation, not a semantic domain concept.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ChannelType {
    /// UTF-8 text data.
    String,
    /// 32-bit signed integer.
    Int32,
    /// 64-bit signed integer.
    Int64,
    /// 32-bit unsigned integer.
    Uint32,
    /// 64-bit unsigned integer.
    Uint64,
    /// 32-bit floating point.
    Float32,
    /// 64-bit floating point.
    Float64,
    /// Boolean value.
    Bool,
    /// Raw binary data.
    Bytes,
    /// Ordered collection of values.
    Array,
    /// Key-value map.
    Map,
    /// Null / universal acceptor — accepts any type.
    Null,
}

impl ChannelType {
    pub fn as_str(&self) -> &'static str {
        match self {
            ChannelType::String => "string",
            ChannelType::Int32 => "int32",
            ChannelType::Int64 => "int64",
            ChannelType::Uint32 => "uint32",
            ChannelType::Uint64 => "uint64",
            ChannelType::Float32 => "float32",
            ChannelType::Float64 => "float64",
            ChannelType::Bool => "bool",
            ChannelType::Bytes => "bytes",
            ChannelType::Array => "array",
            ChannelType::Map => "map",
            ChannelType::Null => "null",
        }
    }

    /// Parse a channel type from a string.
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "string" => Some(ChannelType::String),
            "int32" => Some(ChannelType::Int32),
            "int64" => Some(ChannelType::Int64),
            "uint32" => Some(ChannelType::Uint32),
            "uint64" => Some(ChannelType::Uint64),
            "float32" => Some(ChannelType::Float32),
            "float64" => Some(ChannelType::Float64),
            "bool" => Some(ChannelType::Bool),
            "bytes" => Some(ChannelType::Bytes),
            "array" => Some(ChannelType::Array),
            "map" => Some(ChannelType::Map),
            "null" => Some(ChannelType::Null),
            _ => None,
        }
    }

    /// Check if a source of this type can connect to a target of the given type.
    ///
    /// The compatibility matrix mirrors the UI's `TYPE_COMPATIBILITY` in
    /// `channel-types.ts`. `Null` is the universal acceptor (replaces old `Any`).
    pub fn is_compatible_with(&self, target: &ChannelType) -> bool {
        if *self == *target || *target == ChannelType::Null || *self == ChannelType::Null {
            return true;
        }
        match target {
            // Widening: int64 accepts int32
            ChannelType::Int64 => matches!(self, ChannelType::Int32),
            // Widening: uint64 accepts uint32
            ChannelType::Uint64 => matches!(self, ChannelType::Uint32),
            // Widening: float64 accepts all numeric types
            ChannelType::Float64 => matches!(
                self,
                ChannelType::Int32
                    | ChannelType::Int64
                    | ChannelType::Uint32
                    | ChannelType::Uint64
                    | ChannelType::Float32
            ),
            // Bytes accepts string (UTF-8 encoding)
            ChannelType::Bytes => matches!(self, ChannelType::String),
            // All other types: self-only (already handled by equality check above)
            _ => false,
        }
    }

    /// All known channel types.
    pub const ALL: &'static [ChannelType] = &[
        ChannelType::String,
        ChannelType::Int32,
        ChannelType::Int64,
        ChannelType::Uint32,
        ChannelType::Uint64,
        ChannelType::Float32,
        ChannelType::Float64,
        ChannelType::Bool,
        ChannelType::Bytes,
        ChannelType::Array,
        ChannelType::Map,
        ChannelType::Null,
    ];
}

impl std::fmt::Display for ChannelType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    #[test]
    fn test_default() {
        assert!(DataType::default().is_null());
    }

    #[test]
    fn test_type_name() {
        assert_eq!(DataType::Int32(0).type_name(), "int32");
        assert_eq!(DataType::Uint32(0).type_name(), "uint32");
        assert_eq!(DataType::Float64(0.0).type_name(), "float64");
    }

    #[test]
    fn test_len() {
        assert_eq!(DataType::String("hi".into()).len(), 2);
        assert_eq!(DataType::Array(vec![]).len(), 0);
        assert_eq!(DataType::Int32(0).len(), 0);
    }

    #[test]
    fn test_to_bool_truthy_falsy() {
        assert!(DataType::Bool(true).to_bool());
        assert!(!DataType::Bool(false).to_bool());
        assert!(DataType::Int32(1).to_bool());
        assert!(!DataType::Int32(0).to_bool());
        assert!(DataType::Uint32(1).to_bool());
        assert!(!DataType::Uint32(0).to_bool());
        assert!(DataType::String("true".into()).to_bool());
        assert!(!DataType::String("false".into()).to_bool());
        assert!(!DataType::Null.to_bool());
    }

    #[test]
    fn test_to_i64() {
        assert_eq!(DataType::Int32(42).to_i64(), Some(42));
        assert_eq!(DataType::Uint32(10).to_i64(), Some(10));
        assert_eq!(DataType::Bool(true).to_i64(), Some(1));
        assert_eq!(DataType::Null.to_i64(), None);
    }

    #[test]
    fn test_to_f64() {
        assert_eq!(DataType::Float64(3.14).to_f64(), Some(3.14));
        assert_eq!(DataType::Int32(5).to_f64(), Some(5.0));
        assert_eq!(DataType::Null.to_f64(), None);
    }

    #[test]
    fn test_to_bytes() {
        assert_eq!(DataType::Bytes(vec![1, 2]).to_bytes(), Some(vec![1, 2]));
        assert_eq!(
            DataType::String("hi".into()).to_bytes(),
            Some(b"hi".to_vec())
        );
        assert_eq!(DataType::Int32(1).to_bytes(), None);
    }

    #[test]
    fn test_json_interop() {
        let json = serde_json::json!({"x": 42, "s": "hi", "b": true});
        let dt = DataType::from_json(json);
        assert_eq!(dt.get("x").unwrap().to_i64(), Some(42));
        let back = dt.to_json();
        assert_eq!(back["s"], "hi");
    }

    #[test]
    fn test_from_small_integers() {
        let dt: DataType = 5i8.into();
        assert!(matches!(dt, DataType::Int32(5)));
        let dt: DataType = 200u8.into();
        assert!(matches!(dt, DataType::Uint32(200)));
    }

    #[test]
    fn test_from_hashmap() {
        let mut hm = HashMap::new();
        hm.insert("k".to_string(), DataType::Int32(1));
        let dt: DataType = hm.into();
        assert!(matches!(dt, DataType::Map(_)));
    }

    #[test]
    fn test_from_byte_slice() {
        let data: &[u8] = &[1, 2, 3];
        let dt: DataType = data.into();
        assert_eq!(dt.as_bytes(), Some(&[1u8, 2, 3][..]));
    }

    #[test]
    fn test_from_option() {
        let dt: DataType = Some(DataType::Int32(5)).into();
        assert!(matches!(dt, DataType::Int32(5)));
        let dt: DataType = None.into();
        assert!(dt.is_null());
    }

    #[test]
    fn test_serialization_with_unsigned() {
        let dt = DataType::Map(BTreeMap::from([
            ("u32".to_string(), DataType::Uint32(42)),
            ("u64".to_string(), DataType::Uint64(999)),
        ]));
        let json = serde_json::to_string(&dt).unwrap();
        let round_trip: DataType = serde_json::from_str(&json).unwrap();
        assert_eq!(dt, round_trip);
    }

    // =========================================================================
    // ChannelType tests
    // =========================================================================

    #[test]
    fn test_channel_type_as_str_and_parse() {
        for ct in ChannelType::ALL {
            let s = ct.as_str();
            let parsed = ChannelType::parse(s).unwrap();
            assert_eq!(*ct, parsed);
            assert_eq!(ct.to_string(), s);
        }
        assert!(ChannelType::parse("unknown").is_none());
    }

    #[test]
    fn test_channel_type_all_has_12_variants() {
        assert_eq!(ChannelType::ALL.len(), 12);
    }

    #[test]
    fn test_channel_type_serde() {
        let json = serde_json::to_string(&ChannelType::String).unwrap();
        assert_eq!(json, "\"string\"");
        let rt: ChannelType = serde_json::from_str("\"array\"").unwrap();
        assert_eq!(rt, ChannelType::Array);
    }

    #[test]
    fn test_channel_type_same_type_compatible() {
        for ct in ChannelType::ALL {
            assert!(
                ct.is_compatible_with(ct),
                "{} should be compatible with itself",
                ct
            );
        }
    }

    #[test]
    fn test_channel_type_null_always_compatible() {
        for ct in ChannelType::ALL {
            assert!(ct.is_compatible_with(&ChannelType::Null), "{} -> null", ct);
            assert!(ChannelType::Null.is_compatible_with(ct), "null -> {}", ct);
        }
    }

    #[test]
    fn test_channel_type_compatibility_full_matrix() {
        use ChannelType::*;
        // Exhaustive 12x12 compatibility matrix.
        // Rows = source type, Columns = target type.
        // true  = source.is_compatible_with(target) should be true.
        // Self->Self always true, anything->Null always true, Null->anything always true.
        // Widening rules: Int32->Int64, Uint32->Uint64, all numeric->Float64, String->Bytes.
        let types = ChannelType::ALL;
        // Expected results for non-trivial pairs (self->self and null handled above)
        #[rustfmt::skip]
        let expected: &[(&ChannelType, &ChannelType, bool)] = &[
            // Int32 source
            (&Int32, &String, false),
            (&Int32, &Int64, true),     // widening
            (&Int32, &Uint32, false),
            (&Int32, &Uint64, false),
            (&Int32, &Float32, false),
            (&Int32, &Float64, true),   // widening
            (&Int32, &Bool, false),
            (&Int32, &Bytes, false),
            (&Int32, &Array, false),
            (&Int32, &Map, false),
            // Int64 source
            (&Int64, &String, false),
            (&Int64, &Int32, false),
            (&Int64, &Uint32, false),
            (&Int64, &Uint64, false),
            (&Int64, &Float32, false),
            (&Int64, &Float64, true),   // widening
            (&Int64, &Bool, false),
            (&Int64, &Bytes, false),
            (&Int64, &Array, false),
            (&Int64, &Map, false),
            // Uint32 source
            (&Uint32, &String, false),
            (&Uint32, &Int32, false),
            (&Uint32, &Int64, false),
            (&Uint32, &Uint64, true),   // widening
            (&Uint32, &Float32, false),
            (&Uint32, &Float64, true),  // widening
            (&Uint32, &Bool, false),
            (&Uint32, &Bytes, false),
            (&Uint32, &Array, false),
            (&Uint32, &Map, false),
            // Uint64 source
            (&Uint64, &String, false),
            (&Uint64, &Int32, false),
            (&Uint64, &Int64, false),
            (&Uint64, &Uint32, false),
            (&Uint64, &Float32, false),
            (&Uint64, &Float64, true),  // widening
            (&Uint64, &Bool, false),
            (&Uint64, &Bytes, false),
            (&Uint64, &Array, false),
            (&Uint64, &Map, false),
            // Float32 source
            (&Float32, &String, false),
            (&Float32, &Int32, false),
            (&Float32, &Int64, false),
            (&Float32, &Uint32, false),
            (&Float32, &Uint64, false),
            (&Float32, &Float64, true), // widening
            (&Float32, &Bool, false),
            (&Float32, &Bytes, false),
            (&Float32, &Array, false),
            (&Float32, &Map, false),
            // Float64 source
            (&Float64, &String, false),
            (&Float64, &Int32, false),
            (&Float64, &Int64, false),
            (&Float64, &Uint32, false),
            (&Float64, &Uint64, false),
            (&Float64, &Float32, false),
            (&Float64, &Bool, false),
            (&Float64, &Bytes, false),
            (&Float64, &Array, false),
            (&Float64, &Map, false),
            // String source
            (&String, &Int32, false),
            (&String, &Int64, false),
            (&String, &Uint32, false),
            (&String, &Uint64, false),
            (&String, &Float32, false),
            (&String, &Float64, false),
            (&String, &Bool, false),
            (&String, &Bytes, true),    // string -> bytes
            (&String, &Array, false),
            (&String, &Map, false),
            // Bool source
            (&Bool, &String, false),
            (&Bool, &Int32, false),
            (&Bool, &Int64, false),
            (&Bool, &Uint32, false),
            (&Bool, &Uint64, false),
            (&Bool, &Float32, false),
            (&Bool, &Float64, false),
            (&Bool, &Bytes, false),
            (&Bool, &Array, false),
            (&Bool, &Map, false),
            // Bytes source
            (&Bytes, &String, false),
            (&Bytes, &Int32, false),
            (&Bytes, &Int64, false),
            (&Bytes, &Uint32, false),
            (&Bytes, &Uint64, false),
            (&Bytes, &Float32, false),
            (&Bytes, &Float64, false),
            (&Bytes, &Bool, false),
            (&Bytes, &Array, false),
            (&Bytes, &Map, false),
            // Array source
            (&Array, &String, false),
            (&Array, &Int32, false),
            (&Array, &Int64, false),
            (&Array, &Uint32, false),
            (&Array, &Uint64, false),
            (&Array, &Float32, false),
            (&Array, &Float64, false),
            (&Array, &Bool, false),
            (&Array, &Bytes, false),
            (&Array, &Map, false),
            // Map source
            (&Map, &String, false),
            (&Map, &Int32, false),
            (&Map, &Int64, false),
            (&Map, &Uint32, false),
            (&Map, &Uint64, false),
            (&Map, &Float32, false),
            (&Map, &Float64, false),
            (&Map, &Bool, false),
            (&Map, &Bytes, false),
            (&Map, &Array, false),
        ];

        // Verify self->self for all types
        for ct in types {
            assert!(ct.is_compatible_with(ct), "{} -> {} should be true", ct, ct);
        }
        // Verify anything->Null and Null->anything
        for ct in types {
            assert!(ct.is_compatible_with(&Null), "{} -> null should be true", ct);
            assert!(Null.is_compatible_with(ct), "null -> {} should be true", ct);
        }
        // Verify all explicit pairs
        for (src, tgt, expect) in expected {
            assert_eq!(
                src.is_compatible_with(tgt),
                *expect,
                "{} -> {} expected {} got {}",
                src,
                tgt,
                expect,
                src.is_compatible_with(tgt)
            );
        }
    }
    // =========================================================================
    // DataType::Future unit tests (Gap #5)
    // =========================================================================

    #[test]
    fn test_future_type_name() {
        let pending = DataType::Future(Box::new(FutureState::Pending));
        let resolved = DataType::Future(Box::new(FutureState::Resolved(Box::new(
            DataType::Int64(1),
        ))));
        let rejected = DataType::Future(Box::new(FutureState::Rejected("err".into())));
        assert_eq!(pending.type_name(), "future");
        assert_eq!(resolved.type_name(), "future");
        assert_eq!(rejected.type_name(), "future");
    }

    #[test]
    fn test_future_is_truthy() {
        // All Future variants are truthy
        assert!(DataType::Future(Box::new(FutureState::Pending)).to_bool());
        assert!(
            DataType::Future(Box::new(FutureState::Resolved(Box::new(DataType::Null)))).to_bool()
        );
        assert!(DataType::Future(Box::new(FutureState::Rejected("x".into()))).to_bool());
    }

    #[test]
    fn test_future_display_pending() {
        let dt = DataType::Future(Box::new(FutureState::Pending));
        assert_eq!(format!("{}", dt), "<future:pending>");
    }

    #[test]
    fn test_future_display_resolved() {
        let dt = DataType::Future(Box::new(FutureState::Resolved(Box::new(DataType::Int64(
            42,
        )))));
        assert_eq!(format!("{}", dt), "<future:resolved(42)>");
    }

    #[test]
    fn test_future_display_rejected() {
        let dt = DataType::Future(Box::new(FutureState::Rejected("timeout".into())));
        assert_eq!(format!("{}", dt), "<future:rejected(timeout)>");
    }

    #[test]
    fn test_future_to_json_pending() {
        let dt = DataType::Future(Box::new(FutureState::Pending));
        let json = dt.to_json();
        assert_eq!(json, serde_json::json!({"state": "pending"}));
    }

    #[test]
    fn test_future_to_json_resolved() {
        let dt = DataType::Future(Box::new(FutureState::Resolved(Box::new(DataType::String(
            "ok".into(),
        )))));
        let json = dt.to_json();
        assert_eq!(
            json,
            serde_json::json!({"state": "resolved", "value": "ok"})
        );
    }

    #[test]
    fn test_future_to_json_rejected() {
        let dt = DataType::Future(Box::new(FutureState::Rejected("bad".into())));
        let json = dt.to_json();
        assert_eq!(
            json,
            serde_json::json!({"state": "rejected", "error": "bad"})
        );
    }

    #[test]
    fn test_future_equality() {
        let a = DataType::Future(Box::new(FutureState::Pending));
        let b = DataType::Future(Box::new(FutureState::Pending));
        assert_eq!(a, b);

        let c = DataType::Future(Box::new(FutureState::Resolved(Box::new(DataType::Int64(
            1,
        )))));
        let d = DataType::Future(Box::new(FutureState::Resolved(Box::new(DataType::Int64(
            1,
        )))));
        assert_eq!(c, d);

        // Different states are not equal
        assert_ne!(a, c);
    }

    #[test]
    fn test_future_not_null() {
        let dt = DataType::Future(Box::new(FutureState::Pending));
        assert!(!dt.is_null());
    }

    // =========================================================================
    // DataType helper method tests
    // =========================================================================

    #[test]
    fn test_datatype_as_str() {
        assert_eq!(DataType::String("hello".into()).as_str(), Some("hello"));
        assert_eq!(DataType::Int64(42).as_str(), None);
        assert_eq!(DataType::Null.as_str(), None);
    }

    #[test]
    fn test_datatype_as_bool() {
        assert_eq!(DataType::Bool(true).as_bool(), Some(true));
        assert_eq!(DataType::Bool(false).as_bool(), Some(false));
        assert_eq!(DataType::String("yes".into()).as_bool(), None);
    }

    #[test]
    fn test_datatype_as_bytes() {
        let bytes = vec![1, 2, 3];
        assert_eq!(
            DataType::Bytes(bytes.clone()).as_bytes(),
            Some(bytes.as_slice())
        );
        assert_eq!(DataType::Null.as_bytes(), None);
    }

    #[test]
    fn test_datatype_as_array() {
        let arr = vec![DataType::Int64(1), DataType::Int64(2)];
        assert_eq!(DataType::Array(arr.clone()).as_array(), Some(&arr));
        assert_eq!(DataType::Null.as_array(), None);
    }

    #[test]
    fn test_datatype_as_map() {
        let mut map = std::collections::BTreeMap::new();
        map.insert("key".to_string(), DataType::Int64(1));
        let dt = DataType::Map(map.clone());
        assert!(dt.as_map().is_some());
        assert_eq!(dt.as_map(), Some(&map));
        assert_eq!(DataType::Null.as_map(), None);
    }

    #[test]
    fn test_datatype_is_null() {
        assert!(DataType::Null.is_null());
        assert!(!DataType::Bool(false).is_null());
        assert!(!DataType::Int64(0).is_null());
    }

    #[test]
    fn test_datatype_is_bytes() {
        assert!(DataType::Bytes(vec![]).is_bytes());
        assert!(!DataType::Null.is_bytes());
        assert!(!DataType::String("".into()).is_bytes());
    }

    #[test]
    fn test_datatype_get() {
        let mut map = std::collections::BTreeMap::new();
        map.insert("name".to_string(), DataType::String("Alice".into()));
        let dt = DataType::Map(map);
        assert_eq!(dt.get("name"), Some(&DataType::String("Alice".into())));
        assert_eq!(dt.get("missing"), None);
        // get on non-map returns None
        assert_eq!(DataType::Int64(42).get("key"), None);
    }

    #[test]
    fn test_datatype_len_extended() {
        assert_eq!(
            DataType::Array(vec![DataType::Null, DataType::Null]).len(),
            2
        );
        assert_eq!(DataType::Bytes(vec![1, 2, 3]).len(), 3);
        let mut map = std::collections::BTreeMap::new();
        map.insert("a".to_string(), DataType::Null);
        assert_eq!(DataType::Map(map).len(), 1);
        assert_eq!(DataType::String("hello".into()).len(), 5);
        assert_eq!(DataType::Int64(42).len(), 0);
    }

    #[test]
    fn test_datatype_is_empty() {
        assert!(DataType::Array(vec![]).is_empty());
        assert!(!DataType::Array(vec![DataType::Null]).is_empty());
        assert!(DataType::String("".into()).is_empty());
        assert!(!DataType::String("x".into()).is_empty());
        assert!(DataType::Bytes(vec![]).is_empty());
        assert!(!DataType::Bytes(vec![1]).is_empty());
        // Non-collection types return 0 from len(), so is_empty() is true
        assert!(DataType::Null.is_empty());
    }

    #[test]
    fn test_datatype_to_i64_extended() {
        assert_eq!(DataType::Int64(100).to_i64(), Some(100));
        assert_eq!(DataType::Uint64(20).to_i64(), Some(20));
        assert_eq!(DataType::Float64(3.14).to_i64(), Some(3));
        assert_eq!(DataType::Float32(2.9).to_i64(), Some(2));
        assert_eq!(DataType::String("123".into()).to_i64(), Some(123));
        assert_eq!(DataType::String("not_a_number".into()).to_i64(), None);
    }

    #[test]
    fn test_datatype_to_f64_extended() {
        assert_eq!(DataType::Float32(1.5).to_f64(), Some(1.5_f32 as f64));
        assert_eq!(DataType::Int64(42).to_f64(), Some(42.0));
        assert_eq!(DataType::Uint32(10).to_f64(), Some(10.0));
        assert_eq!(DataType::Uint64(20).to_f64(), Some(20.0));
        assert_eq!(DataType::Bool(true).to_f64(), Some(1.0));
        assert_eq!(DataType::Bool(false).to_f64(), Some(0.0));
        assert_eq!(DataType::String("3.14".into()).to_f64(), Some(3.14));
        assert_eq!(DataType::String("nope".into()).to_f64(), None);
    }

    #[test]
    fn test_datatype_to_string_lossy() {
        assert_eq!(DataType::String("hello".into()).to_string_lossy(), "hello");
        assert_eq!(DataType::Int64(42).to_string_lossy(), "42");
        assert_eq!(DataType::Bool(true).to_string_lossy(), "true");
        assert_eq!(DataType::Null.to_string_lossy(), "null");
        assert_eq!(DataType::Float64(3.14).to_string_lossy(), "3.14");
    }
}
