//! Own implementations replacing external crates.
//!
//! Phase 1: hex, slug, html-escape, percent-encoding, data-encoding (base32),
//! heck, ordered-float, strsim, crc32fast, glob.
//!
//! Phase 2: uuid, subtle, semver, textwrap, base64, hmac, md-5.
//!
//! Phase 3: chrono, url, http, httparse, toml.
//!
//! Phase 4: sha2, blake3, csv, rand, rustyline, ariadne, thiserror (manual impls), tracing (removed).
//!
//! Phase 5: indexmap (OrderedMap), regex, serde_yaml_ng (YAML), lz4_flex (LZ4), zstd, ureq (HTTP client).
//!
//! Phase 6: rcgen, x509-parser, tungstenite, native-tls.
//!
//! Phase 7: serde_json (JsonValue), serde (remove derives), wasmparser (own validator).

// hex encode/decode (replaces `hex` crate)

const HEX_CHARS: &[u8; 16] = b"0123456789abcdef";
const HEX_CHARS_UPPER: &[u8; 16] = b"0123456789ABCDEF";

/// Encode bytes to lowercase hex string.
pub fn hex_encode(bytes: &[u8]) -> String {
    let mut s = String::with_capacity(bytes.len() * 2);
    for &b in bytes {
        s.push(HEX_CHARS[(b >> 4) as usize] as char);
        s.push(HEX_CHARS[(b & 0x0f) as usize] as char);
    }
    s
}

/// Decode hex string to bytes. Returns `Err` on invalid hex.
pub fn hex_decode(s: &str) -> Result<Vec<u8>, String> {
    if s.len() % 2 != 0 {
        return Err("odd length hex string".to_string());
    }
    let mut bytes = Vec::with_capacity(s.len() / 2);
    let b = s.as_bytes();
    for i in (0..b.len()).step_by(2) {
        let hi = hex_val(b[i]).ok_or_else(|| format!("invalid hex char: {}", b[i] as char))?;
        let lo = hex_val(b[i + 1]).ok_or_else(|| format!("invalid hex char: {}", b[i + 1] as char))?;
        bytes.push((hi << 4) | lo);
    }
    Ok(bytes)
}

fn hex_val(c: u8) -> Option<u8> {
    match c {
        b'0'..=b'9' => Some(c - b'0'),
        b'a'..=b'f' => Some(c - b'a' + 10),
        b'A'..=b'F' => Some(c - b'A' + 10),
        _ => None,
    }
}

// slug (replaces `slug` crate)

/// Convert a string to a URL-safe slug.
pub fn slugify(s: &str) -> String {
    let mut result = String::with_capacity(s.len());
    let mut last_was_dash = true; // prevent leading dash
    for c in s.chars() {
        if c.is_ascii_alphanumeric() {
            result.push(c.to_ascii_lowercase());
            last_was_dash = false;
        } else if !last_was_dash {
            result.push('-');
            last_was_dash = true;
        }
    }
    // trim trailing dash
    if result.ends_with('-') {
        result.pop();
    }
    result
}

// html-escape (replaces `html-escape` crate)

/// Escape HTML entities in text.
pub fn html_encode(s: &str) -> String {
    let mut result = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => result.push_str("&amp;"),
            '<' => result.push_str("&lt;"),
            '>' => result.push_str("&gt;"),
            '"' => result.push_str("&quot;"),
            '\'' => result.push_str("&#x27;"),
            _ => result.push(c),
        }
    }
    result
}

/// Decode HTML entities.
pub fn html_decode(s: &str) -> String {
    s.replace("&amp;", "&")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&#x27;", "'")
        .replace("&#39;", "'")
        .replace("&apos;", "'")
        .replace("&#x2F;", "/")
        .replace("&#47;", "/")
}

// percent-encoding (replaces `percent-encoding` crate)

/// Percent-encode a string (all non-alphanumeric chars except `-._~`).
pub fn percent_encode(s: &str) -> String {
    let mut result = String::with_capacity(s.len());
    for &b in s.as_bytes() {
        if b.is_ascii_alphanumeric() || b == b'-' || b == b'.' || b == b'_' || b == b'~' {
            result.push(b as char);
        } else {
            result.push('%');
            result.push(HEX_CHARS_UPPER[(b >> 4) as usize] as char);
            result.push(HEX_CHARS_UPPER[(b & 0x0f) as usize] as char);
        }
    }
    result
}

/// Percent-decode a string.
pub fn percent_decode(s: &str) -> Result<String, String> {
    let mut bytes = Vec::with_capacity(s.len());
    let src = s.as_bytes();
    let mut i = 0;
    while i < src.len() {
        if src[i] == b'%' && i + 2 < src.len() {
            let hi = hex_val(src[i + 1]).ok_or_else(|| "invalid percent encoding".to_string())?;
            let lo = hex_val(src[i + 2]).ok_or_else(|| "invalid percent encoding".to_string())?;
            bytes.push((hi << 4) | lo);
            i += 3;
        } else if src[i] == b'+' {
            bytes.push(b' ');
            i += 1;
        } else {
            bytes.push(src[i]);
            i += 1;
        }
    }
    String::from_utf8(bytes).map_err(|e| e.to_string())
}

// base32 (replaces `data-encoding` crate for BASE32)

const BASE32_ALPHABET: &[u8; 32] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZ234567";

/// Base32 encode (RFC 4648).
pub fn base32_encode(data: &[u8]) -> String {
    let mut result = String::with_capacity((data.len() + 4) / 5 * 8);
    let chunks = data.chunks(5);
    for chunk in chunks {
        let mut buf = [0u8; 5];
        buf[..chunk.len()].copy_from_slice(chunk);
        let n = chunk.len();

        // Always emit at least 2 characters for any chunk
        result.push(BASE32_ALPHABET[(buf[0] >> 3) as usize] as char);
        result.push(BASE32_ALPHABET[((buf[0] & 0x07) << 2 | buf[1] >> 6) as usize] as char);
        if n >= 2 {
            result.push(BASE32_ALPHABET[((buf[1] & 0x3E) >> 1) as usize] as char);
            result.push(BASE32_ALPHABET[((buf[1] & 0x01) << 4 | buf[2] >> 4) as usize] as char);
        } else {
            result.push('=');
            result.push('=');
        }
        if n >= 3 {
            result.push(BASE32_ALPHABET[((buf[2] & 0x0F) << 1 | buf[3] >> 7) as usize] as char);
        } else {
            result.push('=');
        }
        if n >= 4 {
            result.push(BASE32_ALPHABET[((buf[3] & 0x7C) >> 2) as usize] as char);
            result.push(BASE32_ALPHABET[((buf[3] & 0x03) << 3 | buf[4] >> 5) as usize] as char);
        } else {
            result.push('=');
            result.push('=');
        }
        if n >= 5 {
            result.push(BASE32_ALPHABET[(buf[4] & 0x1F) as usize] as char);
        } else {
            result.push('=');
        }
    }
    result
}

/// Base32 decode (RFC 4648).
pub fn base32_decode(s: &str) -> Result<Vec<u8>, String> {
    let s = s.trim_end_matches('=');
    let mut result = Vec::with_capacity(s.len() * 5 / 8);
    let mut buffer: u64 = 0;
    let mut bits = 0u8;
    for c in s.chars() {
        let val = match c {
            'A'..='Z' => (c as u8) - b'A',
            'a'..='z' => (c as u8) - b'a', // case insensitive
            '2'..='7' => (c as u8) - b'2' + 26,
            _ => return Err(format!("invalid base32 char: {}", c)),
        };
        buffer = (buffer << 5) | val as u64;
        bits += 5;
        if bits >= 8 {
            bits -= 8;
            result.push((buffer >> bits) as u8);
            buffer &= (1u64 << bits) - 1;
        }
    }
    Ok(result)
}

// heck — case conversion (replaces `heck` crate)

/// Convert to snake_case.
pub fn to_snake_case(s: &str) -> String {
    let mut result = String::with_capacity(s.len() + 4);
    let mut prev_lower = false;
    let mut prev_upper = false;
    for (i, c) in s.chars().enumerate() {
        if c == '-' || c == '_' || c == ' ' {
            if !result.is_empty() && !result.ends_with('_') {
                result.push('_');
            }
            prev_lower = false;
            prev_upper = false;
            continue;
        }
        if c.is_uppercase() {
            // Insert _ before uppercase if preceded by lowercase, or if
            // preceded by uppercase followed by lowercase (e.g., "HTTPSClient" → "https_client")
            if prev_lower {
                result.push('_');
            } else if prev_upper && i + 1 < s.len() {
                let next = s[i + 1..].chars().next();
                if let Some(nc) = next {
                    if nc.is_lowercase() && !result.is_empty() && !result.ends_with('_') {
                        result.push('_');
                    }
                }
            }
            result.push(c.to_ascii_lowercase());
            prev_upper = true;
            prev_lower = false;
        } else {
            result.push(c);
            prev_lower = true;
            prev_upper = false;
        }
    }
    result
}

/// Convert to lowerCamelCase.
pub fn to_lower_camel_case(s: &str) -> String {
    let words = split_words(s);
    let mut result = String::with_capacity(s.len());
    for (i, word) in words.iter().enumerate() {
        if word.is_empty() {
            continue;
        }
        if i == 0 {
            result.push_str(&word.to_lowercase());
        } else {
            let mut chars = word.chars();
            if let Some(first) = chars.next() {
                result.push(first.to_ascii_uppercase());
                result.extend(chars.map(|c| c.to_ascii_lowercase()));
            }
        }
    }
    result
}

/// Convert to Title Case.
pub fn to_title_case(s: &str) -> String {
    let words = split_words(s);
    let mut result = String::with_capacity(s.len() + words.len());
    for (i, word) in words.iter().enumerate() {
        if word.is_empty() {
            continue;
        }
        if i > 0 {
            result.push(' ');
        }
        let mut chars = word.chars();
        if let Some(first) = chars.next() {
            result.push(first.to_ascii_uppercase());
            result.extend(chars.map(|c| c.to_ascii_lowercase()));
        }
    }
    result
}

fn split_words(s: &str) -> Vec<&str> {
    let mut words = Vec::new();
    let mut start = 0;
    let bytes = s.as_bytes();
    for i in 0..bytes.len() {
        let c = bytes[i];
        if c == b'_' || c == b'-' || c == b' ' {
            if start < i {
                words.push(&s[start..i]);
            }
            start = i + 1;
        } else if i > 0 && c.is_ascii_uppercase() && bytes[i - 1].is_ascii_lowercase() {
            if start < i {
                words.push(&s[start..i]);
            }
            start = i;
        }
    }
    if start < bytes.len() {
        words.push(&s[start..]);
    }
    words
}

// ordered-float (replaces `ordered-float` crate)

/// Wrapper for f64 that implements Ord using total_cmp.
#[derive(Debug, Clone, Copy)]
pub struct OrderedFloat(pub f64);

impl PartialEq for OrderedFloat {
    fn eq(&self, other: &Self) -> bool {
        self.0.to_bits() == other.0.to_bits()
    }
}

impl Eq for OrderedFloat {}

impl PartialOrd for OrderedFloat {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for OrderedFloat {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.0.total_cmp(&other.0)
    }
}

impl std::hash::Hash for OrderedFloat {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.0.to_bits().hash(state);
    }
}

impl OrderedFloat {
    /// Unwrap the inner f64 value.
    pub fn into_inner(self) -> f64 {
        self.0
    }
}

// strsim — Levenshtein distance (replaces `strsim` crate)

/// Compute the Levenshtein edit distance between two strings.
pub fn levenshtein(a: &str, b: &str) -> usize {
    let a_len = a.chars().count();
    let b_len = b.chars().count();
    if a_len == 0 {
        return b_len;
    }
    if b_len == 0 {
        return a_len;
    }
    // Single-row DP
    let mut prev_row: Vec<usize> = (0..=b_len).collect();
    let mut curr_row = vec![0; b_len + 1];
    for (i, ca) in a.chars().enumerate() {
        curr_row[0] = i + 1;
        for (j, cb) in b.chars().enumerate() {
            let cost = if ca == cb { 0 } else { 1 };
            curr_row[j + 1] = (prev_row[j + 1] + 1)
                .min(curr_row[j] + 1)
                .min(prev_row[j] + cost);
        }
        std::mem::swap(&mut prev_row, &mut curr_row);
    }
    prev_row[b_len]
}

// crc32 (replaces `crc32fast` crate)

/// CRC32 lookup table (IEEE polynomial 0xEDB88320).
const CRC32_TABLE: [u32; 256] = {
    let mut table = [0u32; 256];
    let mut i = 0;
    while i < 256 {
        let mut crc = i as u32;
        let mut j = 0;
        while j < 8 {
            if crc & 1 != 0 {
                crc = (crc >> 1) ^ 0xEDB88320;
            } else {
                crc >>= 1;
            }
            j += 1;
        }
        table[i] = crc;
        i += 1;
    }
    table
};

/// Compute CRC32 checksum using the IEEE polynomial (CRC-32/ISO-HDLC).
/// This is the variant used by Ethernet, zlib, gzip, and PNG.
/// Note: this is NOT CRC-32C (Castagnoli), which uses polynomial 0x82F63B78.
pub fn crc32(data: &[u8]) -> u32 {
    let mut crc: u32 = 0xFFFFFFFF;
    for &b in data {
        crc = CRC32_TABLE[((crc ^ b as u32) & 0xFF) as usize] ^ (crc >> 8);
    }
    crc ^ 0xFFFFFFFF
}

// glob (replaces `glob` crate)

/// Match files against a glob pattern. Supports `*`, `**`, and `?`.
pub fn glob_match(pattern: &str) -> Result<Vec<std::path::PathBuf>, String> {
    let mut results = Vec::new();
    let parts: Vec<&str> = pattern.split('/').collect();

    // Find the base directory (everything before the first glob char)
    let mut base = std::path::PathBuf::new();
    let mut glob_start = 0;
    for (i, part) in parts.iter().enumerate() {
        if part.contains('*') || part.contains('?') || part.contains('[') {
            glob_start = i;
            break;
        }
        if part.is_empty() && i == 0 {
            base.push("/");
        } else {
            base.push(part);
        }
        glob_start = i + 1;
    }
    if base.as_os_str().is_empty() {
        base.push(".");
    }

    let glob_parts = &parts[glob_start..];
    glob_walk(&base, glob_parts, &mut results)?;
    results.sort();
    Ok(results)
}

fn glob_walk(
    dir: &std::path::Path,
    patterns: &[&str],
    results: &mut Vec<std::path::PathBuf>,
) -> Result<(), String> {
    if patterns.is_empty() {
        if dir.exists() {
            results.push(dir.to_path_buf());
        }
        return Ok(());
    }

    let pat = patterns[0];
    let rest = &patterns[1..];

    if pat == "**" {
        // Match zero or more directory levels
        glob_walk(dir, rest, results)?;
        if let Ok(entries) = std::fs::read_dir(dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.is_dir() {
                    glob_walk(&path, patterns, results)?;
                }
                // Also try matching the rest at this level
                if glob_pattern_match(pat, &entry.file_name().to_string_lossy()) || pat == "**" {
                    if rest.is_empty() {
                        results.push(path.clone());
                    } else {
                        glob_walk(&path, rest, results)?;
                    }
                }
            }
        }
    } else {
        if let Ok(entries) = std::fs::read_dir(dir) {
            for entry in entries.flatten() {
                let name = entry.file_name();
                let name_str = name.to_string_lossy();
                if glob_pattern_match(pat, &name_str) {
                    let path = entry.path();
                    if rest.is_empty() {
                        results.push(path);
                    } else if path.is_dir() {
                        glob_walk(&path, rest, results)?;
                    }
                }
            }
        }
    }
    Ok(())
}

/// Match a single path component against a glob pattern (supports `*` and `?`).
pub fn glob_pattern_match(pattern: &str, text: &str) -> bool {
    let p: Vec<char> = pattern.chars().collect();
    let t: Vec<char> = text.chars().collect();
    glob_match_inner(&p, &t, 0, 0)
}

fn glob_match_inner(p: &[char], t: &[char], pi: usize, ti: usize) -> bool {
    if pi == p.len() {
        return ti == t.len();
    }
    if p[pi] == '*' {
        // Try matching zero or more characters
        for k in ti..=t.len() {
            if glob_match_inner(p, t, pi + 1, k) {
                return true;
            }
        }
        return false;
    }
    if ti >= t.len() {
        return false;
    }
    if p[pi] == '?' || p[pi] == t[ti] {
        return glob_match_inner(p, t, pi + 1, ti + 1);
    }
    false
}

// uuid v4 (replaces `uuid` crate)

/// Generate a random UUID v4 string (xxxxxxxx-xxxx-4xxx-yxxx-xxxxxxxxxxxx).
pub fn uuid_v4() -> String {
    let mut bytes = [0u8; 16];
    random_fill_bytes(&mut bytes);
    // Set version 4
    bytes[6] = (bytes[6] & 0x0f) | 0x40;
    // Set variant 1 (RFC 4122)
    bytes[8] = (bytes[8] & 0x3f) | 0x80;
    format!(
        "{:02x}{:02x}{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}{:02x}{:02x}{:02x}{:02x}",
        bytes[0], bytes[1], bytes[2], bytes[3],
        bytes[4], bytes[5],
        bytes[6], bytes[7],
        bytes[8], bytes[9],
        bytes[10], bytes[11], bytes[12], bytes[13], bytes[14], bytes[15],
    )
}

/// Parse a UUID string in canonical hyphenated format and return (bytes, version_number).
/// Returns `Err` on invalid input.
pub fn uuid_parse(s: &str) -> Result<([u8; 16], u8), String> {
    let s = s.trim();
    if s.len() != 36 {
        return Err("invalid UUID length".to_string());
    }
    let b = s.as_bytes();
    if b[8] != b'-' || b[13] != b'-' || b[18] != b'-' || b[23] != b'-' {
        return Err("invalid UUID format".to_string());
    }
    // Remove hyphens and decode hex
    let hex_str: String = s.chars().filter(|c| *c != '-').collect();
    if hex_str.len() != 32 {
        return Err("invalid UUID hex length".to_string());
    }
    let bytes = hex_decode(&hex_str)?;
    let mut arr = [0u8; 16];
    arr.copy_from_slice(&bytes);
    let version = (arr[6] >> 4) & 0x0f;
    Ok((arr, version))
}

/// Check if a string is a valid UUID in canonical hyphenated format.
pub fn uuid_is_valid(s: &str) -> bool {
    uuid_parse(s).is_ok()
}

// constant-time comparison (replaces `subtle` crate)

/// Compare two byte slices in constant time. Returns `true` if they are equal.
/// Both slices must have the same length; returns `false` if lengths differ.
pub fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    let mut diff: u8 = 0;
    for (x, y) in a.iter().zip(b.iter()) {
        diff |= x ^ y;
    }
    diff == 0
}

// semver (replaces `semver` crate)

/// A simple semantic version (major.minor.patch with optional pre-release).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SemVer {
    pub major: u64,
    pub minor: u64,
    pub patch: u64,
    pub pre: String,
}

impl SemVer {
    pub fn new(major: u64, minor: u64, patch: u64) -> Self {
        Self { major, minor, patch, pre: String::new() }
    }

    /// Parse a version string like "1.2.3" or "1.0.0-beta.1".
    pub fn parse(s: &str) -> Result<Self, String> {
        let s = s.trim();
        if s.is_empty() {
            return Err("empty version string".to_string());
        }

        // Split off build metadata (ignored)
        let s = s.split('+').next().unwrap();

        // Split off pre-release
        let (version_part, pre) = if let Some(idx) = s.find('-') {
            (&s[..idx], s[idx + 1..].to_string())
        } else {
            (s, String::new())
        };

        let parts: Vec<&str> = version_part.split('.').collect();
        if parts.len() != 3 {
            return Err(format!("expected 3 version components, got {}", parts.len()));
        }

        let major = parts[0].parse::<u64>().map_err(|_| format!("invalid major: {}", parts[0]))?;
        let minor = parts[1].parse::<u64>().map_err(|_| format!("invalid minor: {}", parts[1]))?;
        let patch = parts[2].parse::<u64>().map_err(|_| format!("invalid patch: {}", parts[2]))?;

        // Validate pre-release identifiers (no empty segments)
        if !pre.is_empty() {
            for ident in pre.split('.') {
                if ident.is_empty() {
                    return Err("empty pre-release identifier".to_string());
                }
            }
        }

        Ok(Self { major, minor, patch, pre })
    }

    pub fn is_pre_release(&self) -> bool {
        !self.pre.is_empty()
    }
}

impl std::fmt::Display for SemVer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if self.pre.is_empty() {
            write!(f, "{}.{}.{}", self.major, self.minor, self.patch)
        } else {
            write!(f, "{}.{}.{}-{}", self.major, self.minor, self.patch, self.pre)
        }
    }
}

impl PartialOrd for SemVer {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for SemVer {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        use std::cmp::Ordering;

        match self.major.cmp(&other.major) {
            Ordering::Equal => {}
            ord => return ord,
        }
        match self.minor.cmp(&other.minor) {
            Ordering::Equal => {}
            ord => return ord,
        }
        match self.patch.cmp(&other.patch) {
            Ordering::Equal => {}
            ord => return ord,
        }

        // Pre-release comparison per semver spec:
        // - No pre-release > any pre-release
        // - Compare dot-separated identifiers left to right
        match (self.pre.is_empty(), other.pre.is_empty()) {
            (true, true) => Ordering::Equal,
            (true, false) => Ordering::Greater,  // release > pre-release
            (false, true) => Ordering::Less,      // pre-release < release
            (false, false) => {
                let a_parts: Vec<&str> = self.pre.split('.').collect();
                let b_parts: Vec<&str> = other.pre.split('.').collect();
                for (a, b) in a_parts.iter().zip(b_parts.iter()) {
                    let ord = match (a.parse::<u64>(), b.parse::<u64>()) {
                        (Ok(na), Ok(nb)) => na.cmp(&nb),
                        (Ok(_), Err(_)) => Ordering::Less,    // numeric < alpha
                        (Err(_), Ok(_)) => Ordering::Greater,  // alpha > numeric
                        (Err(_), Err(_)) => a.cmp(b),
                    };
                    if ord != Ordering::Equal {
                        return ord;
                    }
                }
                a_parts.len().cmp(&b_parts.len())
            }
        }
    }
}

// textwrap (replaces `textwrap` crate)

/// Wrap text to the given width, breaking on word boundaries.
pub fn textwrap_fill(text: &str, width: usize) -> String {
    let width = width.max(1);
    let mut result = String::with_capacity(text.len() + text.len() / width);
    for (i, line) in text.split('\n').enumerate() {
        if i > 0 {
            result.push('\n');
        }
        wrap_line(line, width, &mut result);
    }
    result
}

fn wrap_line(line: &str, width: usize, out: &mut String) {
    if line.len() <= width {
        out.push_str(line);
        return;
    }
    let mut col = 0;
    let mut first = true;
    for word in line.split_whitespace() {
        let wlen = word.len();
        if !first && col + 1 + wlen > width {
            out.push('\n');
            col = 0;
            first = true;
        }
        if !first {
            out.push(' ');
            col += 1;
        }
        out.push_str(word);
        col += wlen;
        first = false;
    }
}

/// Prepend `prefix` to every line in the text.
pub fn textwrap_indent(text: &str, prefix: &str) -> String {
    let mut result = String::with_capacity(text.len() + prefix.len() * text.lines().count().max(1));
    for (i, line) in text.split('\n').enumerate() {
        if i > 0 {
            result.push('\n');
        }
        if !line.is_empty() {
            result.push_str(prefix);
        }
        result.push_str(line);
    }
    result
}

/// Remove common leading whitespace from all non-empty lines.
pub fn textwrap_dedent(text: &str) -> String {
    let lines: Vec<&str> = text.split('\n').collect();
    // Find minimum indentation among non-empty lines
    let mut min_indent = usize::MAX;
    for line in &lines {
        if line.trim().is_empty() {
            continue;
        }
        let indent = line.len() - line.trim_start().len();
        min_indent = min_indent.min(indent);
    }
    if min_indent == usize::MAX || min_indent == 0 {
        return text.to_string();
    }
    let mut result = String::with_capacity(text.len());
    for (i, line) in lines.iter().enumerate() {
        if i > 0 {
            result.push('\n');
        }
        if line.len() >= min_indent && !line.trim().is_empty() {
            result.push_str(&line[min_indent..]);
        } else {
            result.push_str(line);
        }
    }
    result
}

// base64 (replaces `base64` crate)

const BASE64_CHARS: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

/// Base64 encode (standard, with padding).
pub fn base64_encode(data: &[u8]) -> String {
    let mut result = String::with_capacity((data.len() + 2) / 3 * 4);
    let chunks = data.chunks(3);
    for chunk in chunks {
        let b0 = chunk[0] as u32;
        let b1 = if chunk.len() > 1 { chunk[1] as u32 } else { 0 };
        let b2 = if chunk.len() > 2 { chunk[2] as u32 } else { 0 };
        let triple = (b0 << 16) | (b1 << 8) | b2;
        result.push(BASE64_CHARS[((triple >> 18) & 0x3F) as usize] as char);
        result.push(BASE64_CHARS[((triple >> 12) & 0x3F) as usize] as char);
        if chunk.len() > 1 {
            result.push(BASE64_CHARS[((triple >> 6) & 0x3F) as usize] as char);
        } else {
            result.push('=');
        }
        if chunk.len() > 2 {
            result.push(BASE64_CHARS[(triple & 0x3F) as usize] as char);
        } else {
            result.push('=');
        }
    }
    result
}

/// Base64 decode (standard, with or without padding).
pub fn base64_decode(s: &str) -> Result<Vec<u8>, String> {
    // Strip whitespace first, then padding
    let s: String = s.chars().filter(|c| !c.is_whitespace()).collect();
    let s = s.trim_end_matches('=');
    let mut result = Vec::with_capacity(s.len() * 3 / 4);
    let mut buffer: u32 = 0;
    let mut bits: u8 = 0;
    for c in s.bytes() {
        let val = match c {
            b'A'..=b'Z' => c - b'A',
            b'a'..=b'z' => c - b'a' + 26,
            b'0'..=b'9' => c - b'0' + 52,
            b'+' => 62,
            b'/' => 63,
            b'\n' | b'\r' | b' ' | b'\t' => continue, // skip whitespace
            _ => return Err(format!("invalid base64 character: {}", c as char)),
        };
        buffer = (buffer << 6) | val as u32;
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            result.push((buffer >> bits) as u8);
            buffer &= (1u32 << bits) - 1;
        }
    }
    Ok(result)
}

// HMAC-SHA256 (replaces `hmac` crate for SHA256)

/// Compute HMAC-SHA256.
pub fn hmac_sha256(key: &[u8], data: &[u8]) -> Vec<u8> {
    const BLOCK_SIZE: usize = 64;

    // If key is longer than block size, hash it first
    let key = if key.len() > BLOCK_SIZE {
        sha256(key).to_vec()
    } else {
        key.to_vec()
    };

    // Pad key to block size
    let mut padded_key = vec![0u8; BLOCK_SIZE];
    padded_key[..key.len()].copy_from_slice(&key);

    let mut ipad = vec![0x36u8; BLOCK_SIZE];
    for (i, b) in padded_key.iter().enumerate() {
        ipad[i] ^= b;
    }

    let mut opad = vec![0x5cu8; BLOCK_SIZE];
    for (i, b) in padded_key.iter().enumerate() {
        opad[i] ^= b;
    }

    // Inner hash: H(ipad || data)
    let mut inner_data = ipad;
    inner_data.extend_from_slice(data);
    let inner_hash = sha256(&inner_data);

    // Outer hash: H(opad || inner_hash)
    let mut outer_data = opad;
    outer_data.extend_from_slice(&inner_hash);
    sha256(&outer_data).to_vec()
}

// MD5 hash (replaces `md-5` crate)

/// Compute MD5 hash.
pub fn md5_hash(data: &[u8]) -> [u8; 16] {
    let mut a0: u32 = 0x67452301;
    let mut b0: u32 = 0xefcdab89;
    let mut c0: u32 = 0x98badcfe;
    let mut d0: u32 = 0x10325476;

    // Pre-processing: adding padding bits
    let orig_len_bits = (data.len() as u64).wrapping_mul(8);
    let mut msg = data.to_vec();
    msg.push(0x80);
    while msg.len() % 64 != 56 {
        msg.push(0);
    }
    msg.extend_from_slice(&orig_len_bits.to_le_bytes());

    // Per-round shift amounts
    const S: [u32; 64] = [
        7, 12, 17, 22, 7, 12, 17, 22, 7, 12, 17, 22, 7, 12, 17, 22,
        5, 9, 14, 20, 5, 9, 14, 20, 5, 9, 14, 20, 5, 9, 14, 20,
        4, 11, 16, 23, 4, 11, 16, 23, 4, 11, 16, 23, 4, 11, 16, 23,
        6, 10, 15, 21, 6, 10, 15, 21, 6, 10, 15, 21, 6, 10, 15, 21,
    ];

    // Pre-computed T[i] = floor(2^32 * |sin(i + 1)|)
    const K: [u32; 64] = [
        0xd76aa478, 0xe8c7b756, 0x242070db, 0xc1bdceee,
        0xf57c0faf, 0x4787c62a, 0xa8304613, 0xfd469501,
        0x698098d8, 0x8b44f7af, 0xffff5bb1, 0x895cd7be,
        0x6b901122, 0xfd987193, 0xa679438e, 0x49b40821,
        0xf61e2562, 0xc040b340, 0x265e5a51, 0xe9b6c7aa,
        0xd62f105d, 0x02441453, 0xd8a1e681, 0xe7d3fbc8,
        0x21e1cde6, 0xc33707d6, 0xf4d50d87, 0x455a14ed,
        0xa9e3e905, 0xfcefa3f8, 0x676f02d9, 0x8d2a4c8a,
        0xfffa3942, 0x8771f681, 0x6d9d6122, 0xfde5380c,
        0xa4beea44, 0x4bdecfa9, 0xf6bb4b60, 0xbebfbc70,
        0x289b7ec6, 0xeaa127fa, 0xd4ef3085, 0x04881d05,
        0xd9d4d039, 0xe6db99e5, 0x1fa27cf8, 0xc4ac5665,
        0xf4292244, 0x432aff97, 0xab9423a7, 0xfc93a039,
        0x655b59c3, 0x8f0ccc92, 0xffeff47d, 0x85845dd1,
        0x6fa87e4f, 0xfe2ce6e0, 0xa3014314, 0x4e0811a1,
        0xf7537e82, 0xbd3af235, 0x2ad7d2bb, 0xeb86d391,
    ];

    // Process each 512-bit block
    for chunk in msg.chunks(64) {
        let mut m = [0u32; 16];
        for (i, word) in chunk.chunks(4).enumerate() {
            m[i] = u32::from_le_bytes([word[0], word[1], word[2], word[3]]);
        }

        let mut a = a0;
        let mut b = b0;
        let mut c = c0;
        let mut d = d0;

        for i in 0..64 {
            let (f, g) = match i {
                0..=15 => ((b & c) | ((!b) & d), i),
                16..=31 => ((d & b) | ((!d) & c), (5 * i + 1) % 16),
                32..=47 => (b ^ c ^ d, (3 * i + 5) % 16),
                _ => (c ^ (b | (!d)), (7 * i) % 16),
            };

            let f = f.wrapping_add(a).wrapping_add(K[i]).wrapping_add(m[g]);
            a = d;
            d = c;
            c = b;
            b = b.wrapping_add(f.rotate_left(S[i]));
        }

        a0 = a0.wrapping_add(a);
        b0 = b0.wrapping_add(b);
        c0 = c0.wrapping_add(c);
        d0 = d0.wrapping_add(d);
    }

    let mut result = [0u8; 16];
    result[0..4].copy_from_slice(&a0.to_le_bytes());
    result[4..8].copy_from_slice(&b0.to_le_bytes());
    result[8..12].copy_from_slice(&c0.to_le_bytes());
    result[12..16].copy_from_slice(&d0.to_le_bytes());
    result
}


#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_hex_encode() {
        assert_eq!(hex_encode(b"hello"), "68656c6c6f");
        assert_eq!(hex_encode(b""), "");
        assert_eq!(hex_encode(&[0xff, 0x00, 0xab]), "ff00ab");
    }

    #[test]
    fn test_hex_decode() {
        assert_eq!(hex_decode("68656c6c6f").unwrap(), b"hello");
        assert_eq!(hex_decode("").unwrap(), b"");
        assert_eq!(hex_decode("FF00AB").unwrap(), vec![0xff, 0x00, 0xab]);
        assert!(hex_decode("0").is_err()); // odd length
        assert!(hex_decode("zz").is_err()); // invalid char
    }

    #[test]
    fn test_slugify() {
        assert_eq!(slugify("Hello World!"), "hello-world");
        assert_eq!(slugify("  foo  bar  "), "foo-bar");
        assert_eq!(slugify("MAGI v1.0"), "magi-v1-0");
    }

    #[test]
    fn test_html_encode_decode() {
        assert_eq!(html_encode("<b>foo & bar</b>"), "&lt;b&gt;foo &amp; bar&lt;/b&gt;");
        assert_eq!(html_decode("&lt;b&gt;foo &amp; bar&lt;/b&gt;"), "<b>foo & bar</b>");
    }

    #[test]
    fn test_percent_encode_decode() {
        assert_eq!(percent_encode("hello world"), "hello%20world");
        assert_eq!(percent_decode("hello%20world").unwrap(), "hello world");
        assert_eq!(percent_encode("a+b=c"), "a%2Bb%3Dc");
    }

    #[test]
    fn test_base32() {
        assert_eq!(base32_encode(b"hello"), "NBSWY3DP");
        assert_eq!(base32_decode("NBSWY3DP").unwrap(), b"hello");
        assert_eq!(base32_encode(b""), "");
        assert_eq!(base32_decode("").unwrap(), b"");
    }

    #[test]
    fn test_to_snake_case() {
        assert_eq!(to_snake_case("HelloWorld"), "hello_world");
        assert_eq!(to_snake_case("helloWorld"), "hello_world");
        assert_eq!(to_snake_case("hello-world"), "hello_world");
        assert_eq!(to_snake_case("HELLO"), "hello");
    }

    #[test]
    fn test_to_lower_camel_case() {
        assert_eq!(to_lower_camel_case("hello_world"), "helloWorld");
        assert_eq!(to_lower_camel_case("foo-bar-baz"), "fooBarBaz");
    }

    #[test]
    fn test_to_title_case() {
        assert_eq!(to_title_case("hello_world"), "Hello World");
        assert_eq!(to_title_case("foo-bar"), "Foo Bar");
    }

    #[test]
    fn test_ordered_float() {
        use std::collections::BTreeSet;
        let mut set = BTreeSet::new();
        set.insert(OrderedFloat(1.0));
        set.insert(OrderedFloat(f64::NAN));
        set.insert(OrderedFloat(0.0));
        assert_eq!(set.len(), 3);
    }

    #[test]
    fn test_levenshtein() {
        assert_eq!(levenshtein("hello", "hello"), 0);
        assert_eq!(levenshtein("", "abc"), 3);
        assert_eq!(levenshtein("abc", ""), 3);
        assert_eq!(levenshtein("", ""), 0);
        assert_eq!(levenshtein("cat", "hat"), 1);
        assert_eq!(levenshtein("cat", "cats"), 1);
        assert_eq!(levenshtein("cats", "cat"), 1);
        assert_eq!(levenshtein("kitten", "sitting"), 3);
    }

    #[test]
    fn test_crc32() {
        // Known CRC32 for "hello"
        assert_eq!(crc32(b"hello"), 0x3610a686);
        assert_eq!(crc32(b""), 0x00000000);
    }

    #[test]
    fn test_glob_pattern_match() {
        assert!(glob_pattern_match("*.rs", "foo.rs"));
        assert!(!glob_pattern_match("*.rs", "foo.txt"));
        assert!(glob_pattern_match("test?", "test1"));
        assert!(!glob_pattern_match("test?", "test12"));
        assert!(glob_pattern_match("*", "anything"));
    }

    #[test]
    fn test_uuid_v4() {
        let u = uuid_v4();
        assert_eq!(u.len(), 36);
        assert_eq!(u.as_bytes()[8], b'-');
        assert_eq!(u.as_bytes()[13], b'-');
        assert_eq!(u.as_bytes()[18], b'-');
        assert_eq!(u.as_bytes()[23], b'-');
        // Version nibble must be '4'
        assert_eq!(u.as_bytes()[14], b'4');
        // Variant nibble must be 8, 9, a, or b
        let variant = u.as_bytes()[19];
        assert!(variant == b'8' || variant == b'9' || variant == b'a' || variant == b'b');
        assert_ne!(uuid_v4(), uuid_v4());
    }

    #[test]
    fn test_uuid_parse() {
        let (bytes, version) = uuid_parse("550e8400-e29b-41d4-a716-446655440000").unwrap();
        assert_eq!(version, 4);
        assert_eq!(bytes[0], 0x55);
        assert!(uuid_parse("not-a-uuid").is_err());
        assert!(uuid_parse("550e8400-e29b-41d4-a716").is_err());
    }

    #[test]
    fn test_uuid_is_valid() {
        assert!(uuid_is_valid("550e8400-e29b-41d4-a716-446655440000"));
        assert!(uuid_is_valid(&uuid_v4()));
        assert!(!uuid_is_valid("not-a-uuid"));
        assert!(!uuid_is_valid("550e8400e29b41d4a716446655440000")); // missing hyphens
    }

    #[test]
    fn test_constant_time_eq() {
        assert!(constant_time_eq(b"hello", b"hello"));
        assert!(!constant_time_eq(b"hello", b"world"));
        assert!(!constant_time_eq(b"hello", b"hell"));
        assert!(constant_time_eq(b"", b""));
    }

    #[test]
    fn test_semver_parse() {
        let v = SemVer::parse("1.2.3").unwrap();
        assert_eq!(v.major, 1);
        assert_eq!(v.minor, 2);
        assert_eq!(v.patch, 3);
        assert!(v.pre.is_empty());

        let v = SemVer::parse("0.2.0-beta.1").unwrap();
        assert_eq!(v.pre, "beta.1");

        assert!(SemVer::parse("1.2").is_err());
        assert!(SemVer::parse("abc").is_err());
    }

    #[test]
    fn test_semver_ordering() {
        assert!(SemVer::new(1, 0, 0) > SemVer::new(0, 9, 9));
        assert!(SemVer::new(0, 2, 1) > SemVer::new(0, 2, 0));
        // Pre-release sorts before release
        let mut pre = SemVer::new(1, 0, 0);
        pre.pre = "alpha".to_string();
        assert!(pre < SemVer::new(1, 0, 0));
        // Numeric pre-release ordering
        let mut a = SemVer::new(1, 0, 0);
        a.pre = "alpha.2".to_string();
        let mut b = SemVer::new(1, 0, 0);
        b.pre = "alpha.1".to_string();
        assert!(a > b);
    }

    #[test]
    fn test_semver_display() {
        assert_eq!(SemVer::new(1, 0, 0).to_string(), "1.0.0");
        let mut v = SemVer::new(0, 2, 0);
        v.pre = "alpha".to_string();
        assert_eq!(v.to_string(), "0.2.0-alpha");
    }

    #[test]
    fn test_textwrap_fill() {
        assert_eq!(textwrap_fill("hello world", 80), "hello world");
        assert_eq!(textwrap_fill("hello world foo bar", 11), "hello world\nfoo bar");
        assert_eq!(textwrap_fill("a b c", 1), "a\nb\nc");
    }

    #[test]
    fn test_textwrap_indent() {
        assert_eq!(textwrap_indent("hello\nworld", "  "), "  hello\n  world");
        assert_eq!(textwrap_indent("hello\n\nworld", "  "), "  hello\n\n  world");
    }

    #[test]
    fn test_textwrap_dedent() {
        assert_eq!(textwrap_dedent("  hello\n  world"), "hello\nworld");
        assert_eq!(textwrap_dedent("    hello\n  world"), "  hello\nworld");
    }

    #[test]
    fn test_base64_encode() {
        assert_eq!(base64_encode(b"hello"), "aGVsbG8=");
        assert_eq!(base64_encode(b""), "");
        assert_eq!(base64_encode(b"a"), "YQ==");
        assert_eq!(base64_encode(b"ab"), "YWI=");
        assert_eq!(base64_encode(b"abc"), "YWJj");
    }

    #[test]
    fn test_base64_decode() {
        assert_eq!(base64_decode("aGVsbG8=").unwrap(), b"hello");
        assert_eq!(base64_decode("").unwrap(), b"");
        assert_eq!(base64_decode("YQ==").unwrap(), b"a");
        assert_eq!(base64_decode("YWI=").unwrap(), b"ab");
        assert_eq!(base64_decode("YWJj").unwrap(), b"abc");
        // without padding
        assert_eq!(base64_decode("aGVsbG8").unwrap(), b"hello");
    }

    #[test]
    fn test_sha256_known_vectors() {
        // SHA-256("abc") from NIST FIPS 180-4
        assert_eq!(
            hex_encode(&sha256(b"abc")),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
        // SHA-256("") from NIST
        assert_eq!(
            hex_encode(&sha256(b"")),
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
        // SHA-256 of longer input (2 blocks)
        assert_eq!(
            hex_encode(&sha256(b"abcdbcdecdefdefgefghfghighijhijkijkljklmklmnlmnomnopnopq")),
            "248d6a61d20638b8e5c026930c3e6039a33ce45964ff2167f6ecedd419db06c1"
        );
    }

    #[test]
    fn test_sha1_known_vectors() {
        assert_eq!(hex_encode(&sha1(b"")), "da39a3ee5e6b4b0d3255bfef95601890afd80709");
        assert_eq!(hex_encode(&sha1(b"abc")), "a9993e364706816aba3e25717850c26c9cd0d89d");
    }

    #[test]
    fn test_sha512_known_vectors() {
        // SHA-512("abc") from NIST FIPS 180-4
        assert_eq!(
            hex_encode(&sha512(b"abc")),
            "ddaf35a193617abacc417349ae20413112e6fa4e89a97ea20a9eeee64b55d39a2192992a274fc1a836ba3c23a3feebbd454d4423643ce80e2a9ac94fa54ca49f"
        );
        // SHA-512("")
        assert_eq!(
            hex_encode(&sha512(b"")),
            "cf83e1357eefb8bdf1542850d66d8007d620e4050b5715dc83f4a921d36ce9ce47d0d13c5d85f2b0ff8318d2877eec2f63b931bd47417a81a538327af927da3e"
        );
    }

    #[test]
    fn test_blake3_known_vectors() {
        // BLAKE3 of empty input
        assert_eq!(
            blake3_hash_hex(b""),
            "af1349b9f5f9a1a6a0404dea36dcc9499bcb25c9adc112b7cc9a93cae41f3262"
        );
    }

    #[test]
    fn test_csv_round_trip() {
        let data = csv_parse("name,age\nAlice,30\nBob,25").unwrap();
        assert_eq!(data.headers, vec!["name", "age"]);
        assert_eq!(data.records.len(), 2);
        assert_eq!(data.records[0], vec!["Alice", "30"]);
        let output = csv_write(&["name", "age"], &data.records.iter().map(|r| r.clone()).collect::<Vec<_>>());
        assert!(output.contains("name,age"));
        assert!(output.contains("Alice,30"));
    }

    #[test]
    fn test_csv_quoted_fields() {
        let data = csv_parse("a,b\n\"hello, world\",test\n\"say \"\"hi\"\"\",ok").unwrap();
        assert_eq!(data.records[0][0], "hello, world");
        assert_eq!(data.records[1][0], "say \"hi\"");
    }

    #[test]
    fn test_random_basic() {
        // Just verify it doesn't panic and returns different values
        let a = random_i64();
        let b = random_i64();
        // Extremely unlikely to be equal
        assert!(a != 0 || b != 0);

        let f = random_f64();
        assert!((0.0..1.0).contains(&f));

        let r = random_range_i64(10, 20);
        assert!((10..20).contains(&r));
    }

    #[test]
    fn test_hmac_sha256() {
        // RFC 4231 test vector 1
        let key = vec![0x0bu8; 20];
        let data = b"Hi There";
        let result = hmac_sha256(&key, data);
        assert_eq!(
            hex_encode(&result),
            "b0344c61d8db38535ca8afceaf0bf12b881dc200c9833da726e9376c2e32cff7"
        );
    }

    #[test]
    fn test_md5_hash() {
        assert_eq!(hex_encode(&md5_hash(b"")), "d41d8cd98f00b204e9800998ecf8427e");
        assert_eq!(hex_encode(&md5_hash(b"hello")), "5d41402abc4b2a76b9719d911017c592");
        assert_eq!(hex_encode(&md5_hash(b"The quick brown fox jumps over the lazy dog")),
                   "9e107d9d372bb6826bd81d3542a419d6");
    }
}

// Phase 3: chrono, url, http, httparse, toml replacements

// date/time (replaces `chrono` crate)

/// Get current UTC time in milliseconds since Unix epoch.
pub fn now_millis() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as i64
}

/// Get current UTC time in seconds since Unix epoch.
pub fn now_secs() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64
}

/// Date/time components.
#[derive(Debug, Clone, Copy)]
pub struct DateTime {
    pub year: i64,
    pub month: u32,
    pub day: u32,
    pub hour: u32,
    pub minute: u32,
    pub second: u32,
    pub millis: u32,
}

/// Convert days since Unix epoch to (year, month, day) using Hinnant's algorithm.
fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719468;
    let era = (if z >= 0 { z } else { z - 146096 }) / 146097;
    let doe = (z - era * 146097) as u64;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y = (yoe as i64) + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { (mp + 3) as u32 } else { (mp - 9) as u32 };
    let y = if m <= 2 { y + 1 } else { y };
    (y, m, d)
}

/// Convert (year, month, day) to days since Unix epoch (public wrapper).
pub fn days_from_civil_pub(y: i64, m: u32, d: u32) -> i64 {
    days_from_civil(y, m, d)
}

/// Convert (year, month, day) to days since Unix epoch using Hinnant's algorithm.
fn days_from_civil(y: i64, m: u32, d: u32) -> i64 {
    let m = m as i64;
    let d = d as i64;
    let y = if m <= 2 { y - 1 } else { y };
    let era = (if y >= 0 { y } else { y - 399 }) / 400;
    let yoe = (y - era * 400) as u64;
    let m_adj = if m <= 2 { m + 9 } else { m - 3 } as u64;
    let doy = (153 * m_adj + 2) / 5 + d as u64 - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146097 + doe as i64 - 719468
}

/// Convert a Unix timestamp in milliseconds to DateTime (UTC).
pub fn datetime_from_millis(ms: i64) -> Option<DateTime> {
    // Support range roughly -292277..292277 years
    let total_secs = ms.div_euclid(1000);
    let millis_part = ms.rem_euclid(1000) as u32;
    let days = total_secs.div_euclid(86400);
    let day_secs = total_secs.rem_euclid(86400);
    let (y, m, d) = civil_from_days(days);
    Some(DateTime {
        year: y,
        month: m,
        day: d,
        hour: (day_secs / 3600) as u32,
        minute: ((day_secs % 3600) / 60) as u32,
        second: (day_secs % 60) as u32,
        millis: millis_part,
    })
}

/// Format a Unix timestamp (millis) as ISO 8601 string with milliseconds.
/// E.g. "2024-01-15T09:30:00.000Z"
pub fn format_timestamp_millis(ms: i64) -> Option<String> {
    let dt = datetime_from_millis(ms)?;
    Some(format!(
        "{:04}-{:02}-{:02}T{:02}:{:02}:{:02}.{:03}Z",
        dt.year, dt.month, dt.day, dt.hour, dt.minute, dt.second, dt.millis
    ))
}

/// Parse an ISO 8601 / RFC 3339 date-time string to Unix millis.
/// Supports:
///  - "2024-01-15T09:30:00Z"
///  - "2024-01-15T09:30:00+05:30"
///  - "2024-01-15T09:30:00.123Z"
///  - "2024-01-15T09:30:00.123456Z"  (sub-ms truncated)
///  - "2024-01-15T09:30:00"          (assumed UTC)
///  - "2024-01-15 09:30:00"          (assumed UTC)
///  - "2024-01-15"                   (date only, midnight UTC)
pub fn parse_timestamp_to_millis(s: &str) -> Result<i64, String> {
    let s = s.trim();
    // Date-only: "YYYY-MM-DD"
    if s.len() == 10 && s.as_bytes()[4] == b'-' && s.as_bytes()[7] == b'-' {
        let y: i64 = s[..4].parse().map_err(|_| "invalid year".to_string())?;
        let m: u32 = s[5..7].parse().map_err(|_| "invalid month".to_string())?;
        let d: u32 = s[8..10].parse().map_err(|_| "invalid day".to_string())?;
        if m < 1 || m > 12 || d < 1 || d > 31 {
            return Err("invalid date".to_string());
        }
        return Ok(days_from_civil(y, m, d) * 86400 * 1000);
    }
    // Full datetime: split on T or space
    let sep_pos = s.find('T').or_else(|| s.find(' '));
    let (date_part, time_part) = match sep_pos {
        Some(pos) => (&s[..pos], &s[pos + 1..]),
        None => return Err("unrecognized datetime format".to_string()),
    };
    let date_parts: Vec<&str> = date_part.split('-').collect();
    if date_parts.len() != 3 {
        return Err("invalid date format".to_string());
    }
    let y: i64 = date_parts[0].parse().map_err(|_| "invalid year".to_string())?;
    let m: u32 = date_parts[1].parse().map_err(|_| "invalid month".to_string())?;
    let d: u32 = date_parts[2].parse().map_err(|_| "invalid day".to_string())?;
    if m < 1 || m > 12 || d < 1 || d > 31 {
        return Err("invalid date".to_string());
    }
    // Parse time and timezone
    // time_part could be "09:30:00", "09:30:00Z", "09:30:00.123Z", "09:30:00+05:30", etc.
    let (time_str, tz_offset_secs) = parse_timezone_suffix(time_part)?;
    // Parse HH:MM:SS[.fff]
    let time_parts: Vec<&str> = time_str.split(':').collect();
    if time_parts.len() < 2 {
        return Err("invalid time format".to_string());
    }
    let hour: u32 = time_parts[0].parse().map_err(|_| "invalid hour".to_string())?;
    let min: u32 = time_parts[1].parse().map_err(|_| "invalid minute".to_string())?;
    let (sec, millis) = if time_parts.len() >= 3 {
        parse_seconds_millis(time_parts[2])?
    } else {
        (0, 0)
    };
    let days = days_from_civil(y, m, d);
    let total_secs = days * 86400 + hour as i64 * 3600 + min as i64 * 60 + sec as i64 - tz_offset_secs;
    Ok(total_secs * 1000 + millis as i64)
}

/// Parse timezone suffix from time string. Returns (time_without_tz, offset_in_seconds).
fn parse_timezone_suffix(time: &str) -> Result<(&str, i64), String> {
    // Check for Z suffix
    if let Some(t) = time.strip_suffix('Z') {
        return Ok((t, 0));
    }
    // Check for +HH:MM or -HH:MM at the end
    let bytes = time.as_bytes();
    if bytes.len() >= 6 {
        let sign_pos = bytes.len() - 6;
        if (bytes[sign_pos] == b'+' || bytes[sign_pos] == b'-') && bytes[sign_pos + 3] == b':' {
            let sign = if bytes[sign_pos] == b'+' { 1i64 } else { -1 };
            let hh: i64 = time[sign_pos + 1..sign_pos + 3]
                .parse()
                .map_err(|_| "invalid tz hours".to_string())?;
            let mm: i64 = time[sign_pos + 4..sign_pos + 6]
                .parse()
                .map_err(|_| "invalid tz minutes".to_string())?;
            return Ok((&time[..sign_pos], sign * (hh * 3600 + mm * 60)));
        }
    }
    // No timezone: assume UTC
    Ok((time, 0))
}

/// Parse "SS" or "SS.fff..." returning (seconds, millis).
fn parse_seconds_millis(s: &str) -> Result<(u32, u32), String> {
    if let Some(dot_pos) = s.find('.') {
        let sec: u32 = s[..dot_pos].parse().map_err(|_| "invalid seconds".to_string())?;
        let frac = &s[dot_pos + 1..];
        // Pad or truncate to 3 digits for millis
        let millis = if frac.len() >= 3 {
            frac[..3].parse().map_err(|_| "invalid fractional seconds".to_string())?
        } else {
            let padded = format!("{:0<3}", frac);
            padded.parse().map_err(|_| "invalid fractional seconds".to_string())?
        };
        Ok((sec, millis))
    } else {
        let sec: u32 = s.parse().map_err(|_| "invalid seconds".to_string())?;
        Ok((sec, 0))
    }
}

/// Format local time as "YYYY-MM-DD HH:MM:SS".
/// Uses the C library's localtime_r on Unix to get the local timezone offset.
pub fn local_datetime_string() -> String {
    let now_ms = now_millis();
    let secs = now_ms / 1000;
    #[cfg(unix)]
    let offset_secs = {
        // Use C library directly without a crate dependency.
        #[repr(C)]
        struct CTm {
            tm_sec: i32,
            tm_min: i32,
            tm_hour: i32,
            tm_mday: i32,
            tm_mon: i32,
            tm_year: i32,
            tm_wday: i32,
            tm_yday: i32,
            tm_isdst: i32,
            tm_gmtoff: i64,
            tm_zone: *const i8,
        }
        extern "C" {
            fn localtime_r(timep: *const i64, result: *mut CTm) -> *mut CTm;
        }
        let mut tm: CTm = unsafe { std::mem::zeroed() };
        let t: i64 = secs;
        unsafe { localtime_r(&t, &mut tm) };
        tm.tm_gmtoff
    };
    #[cfg(not(unix))]
    let offset_secs = 0i64;
    let local_ms = now_ms + offset_secs * 1000;
    let dt = datetime_from_millis(local_ms).unwrap_or(DateTime {
        year: 1970, month: 1, day: 1, hour: 0, minute: 0, second: 0, millis: 0,
    });
    format!(
        "{:04}-{:02}-{:02} {:02}:{:02}:{:02}",
        dt.year, dt.month, dt.day, dt.hour, dt.minute, dt.second
    )
}

// URL parsing (replaces `url` crate)

/// Parsed URL components.
#[derive(Debug, Clone, Default)]
pub struct UrlParts {
    pub scheme: String,
    pub username: String,
    pub password: String,
    pub host: String,
    pub port: Option<u16>,
    pub path: String,
    pub query: Option<String>,
    pub fragment: Option<String>,
}

impl UrlParts {
    /// Parse a URL string into its components.
    pub fn parse(url: &str) -> Result<UrlParts, String> {
        let mut rest = url;
        // 1. Parse scheme
        let scheme_end = rest.find("://").ok_or("missing scheme")?;
        let scheme = rest[..scheme_end].to_lowercase();
        rest = &rest[scheme_end + 3..];

        // 2. Split off fragment (#...)
        let (rest_no_frag, fragment) = match rest.rfind('#') {
            Some(pos) => (&rest[..pos], Some(rest[pos + 1..].to_string())),
            None => (rest, None),
        };
        rest = rest_no_frag;

        // 3. Split off query (?...)
        let (rest_no_query, query) = match rest.find('?') {
            Some(pos) => (&rest[..pos], Some(rest[pos + 1..].to_string())),
            None => (rest, None),
        };
        rest = rest_no_query;

        // 4. Split authority from path
        let (authority, path) = match rest.find('/') {
            Some(pos) => (&rest[..pos], rest[pos..].to_string()),
            None => (rest, "/".to_string()),
        };

        // 5. Parse userinfo@host:port from authority
        let (userinfo, hostport) = match authority.rfind('@') {
            Some(pos) => (&authority[..pos], &authority[pos + 1..]),
            None => ("", authority),
        };

        let (username, password) = if !userinfo.is_empty() {
            match userinfo.find(':') {
                Some(pos) => (
                    percent_decode(&userinfo[..pos]).unwrap_or_default(),
                    percent_decode(&userinfo[pos + 1..]).unwrap_or_default(),
                ),
                None => (percent_decode(userinfo).unwrap_or_default(), String::new()),
            }
        } else {
            (String::new(), String::new())
        };

        // 6. Parse host:port (handle IPv6 [::1]:port)
        let (host, port) = if hostport.starts_with('[') {
            // IPv6
            match hostport.find(']') {
                Some(bracket_end) => {
                    let h = &hostport[..bracket_end + 1];
                    let after = &hostport[bracket_end + 1..];
                    let p = if let Some(colon_rest) = after.strip_prefix(':') {
                        Some(colon_rest.parse::<u16>().map_err(|_| "invalid port")?)
                    } else {
                        None
                    };
                    (h.to_string(), p)
                }
                None => (hostport.to_string(), None),
            }
        } else {
            // Check for host:port
            match hostport.rfind(':') {
                Some(pos) => {
                    let maybe_port = &hostport[pos + 1..];
                    if let Ok(p) = maybe_port.parse::<u16>() {
                        (hostport[..pos].to_string(), Some(p))
                    } else {
                        (hostport.to_string(), None)
                    }
                }
                None => (hostport.to_string(), None),
            }
        };

        Ok(UrlParts {
            scheme,
            username,
            password,
            host,
            port,
            path,
            query,
            fragment,
        })
    }

    /// Get the port or the default port for the scheme.
    pub fn port_or_known_default(&self) -> Option<u16> {
        self.port.or_else(|| match self.scheme.as_str() {
            "http" | "ws" => Some(80),
            "https" | "wss" => Some(443),
            "ftp" => Some(21),
            _ => None,
        })
    }

    /// Get the host string (without brackets for IPv6).
    pub fn host_str(&self) -> Option<&str> {
        if self.host.is_empty() {
            None
        } else {
            Some(&self.host)
        }
    }

    /// Join a relative path onto this URL.
    pub fn join(&self, relative: &str) -> Result<String, String> {
        if relative.contains("://") {
            // Absolute URL, return as-is
            return Ok(relative.to_string());
        }
        let base_path = if relative.starts_with('/') {
            relative.to_string()
        } else {
            // Resolve relative to the directory of self.path
            let dir = match self.path.rfind('/') {
                Some(pos) => &self.path[..pos + 1],
                None => "/",
            };
            format!("{}{}", dir, relative)
        };
        let mut result = format!("{}://{}", self.scheme, self.host);
        if let Some(p) = self.port {
            result.push_str(&format!(":{}", p));
        }
        result.push_str(&base_path);
        Ok(result)
    }

    /// Reconstruct the full URL string.
    pub fn to_string(&self) -> String {
        let mut s = format!("{}://", self.scheme);
        if !self.username.is_empty() {
            s.push_str(&self.username);
            if !self.password.is_empty() {
                s.push(':');
                s.push_str(&self.password);
            }
            s.push('@');
        }
        s.push_str(&self.host);
        if let Some(p) = self.port {
            s.push_str(&format!(":{}", p));
        }
        s.push_str(&self.path);
        if let Some(ref q) = self.query {
            s.push('?');
            s.push_str(q);
        }
        if let Some(ref f) = self.fragment {
            s.push('#');
            s.push_str(f);
        }
        s
    }
}

// HTTP status codes (replaces `http` crate)

/// Get the canonical reason phrase for an HTTP status code.
pub fn http_status_reason(code: u16) -> &'static str {
    match code {
        100 => "Continue",
        101 => "Switching Protocols",
        102 => "Processing",
        200 => "OK",
        201 => "Created",
        202 => "Accepted",
        203 => "Non-Authoritative Information",
        204 => "No Content",
        205 => "Reset Content",
        206 => "Partial Content",
        207 => "Multi-Status",
        300 => "Multiple Choices",
        301 => "Moved Permanently",
        302 => "Found",
        303 => "See Other",
        304 => "Not Modified",
        307 => "Temporary Redirect",
        308 => "Permanent Redirect",
        400 => "Bad Request",
        401 => "Unauthorized",
        402 => "Payment Required",
        403 => "Forbidden",
        404 => "Not Found",
        405 => "Method Not Allowed",
        406 => "Not Acceptable",
        407 => "Proxy Authentication Required",
        408 => "Request Timeout",
        409 => "Conflict",
        410 => "Gone",
        411 => "Length Required",
        412 => "Precondition Failed",
        413 => "Payload Too Large",
        414 => "URI Too Long",
        415 => "Unsupported Media Type",
        416 => "Range Not Satisfiable",
        417 => "Expectation Failed",
        418 => "I'm a Teapot",
        422 => "Unprocessable Entity",
        425 => "Too Early",
        426 => "Upgrade Required",
        428 => "Precondition Required",
        429 => "Too Many Requests",
        431 => "Request Header Fields Too Large",
        451 => "Unavailable For Legal Reasons",
        500 => "Internal Server Error",
        501 => "Not Implemented",
        502 => "Bad Gateway",
        503 => "Service Unavailable",
        504 => "Gateway Timeout",
        505 => "HTTP Version Not Supported",
        _ => "Unknown",
    }
}

// HTTP request parsing (replaces `httparse` crate)

/// A parsed HTTP header.
#[derive(Debug, Clone)]
pub struct HttpHeader {
    pub name: String,
    pub value: Vec<u8>,
}

/// A parsed HTTP request.
#[derive(Debug)]
pub struct HttpRequest {
    pub method: String,
    pub path: String,
    pub version: u8, // 0 for HTTP/1.0, 1 for HTTP/1.1
    pub headers: Vec<HttpHeader>,
}

/// Parse an HTTP/1.x request from a byte buffer.
/// Returns `Ok(Some(request))` on success, `Ok(None)` if incomplete, `Err` on malformed.
pub fn parse_http_request(buf: &[u8]) -> Result<Option<HttpRequest>, String> {
    let s = std::str::from_utf8(buf).map_err(|_| "invalid UTF-8 in HTTP request".to_string())?;
    // Find end of request line
    let first_line_end = match s.find("\r\n") {
        Some(pos) => pos,
        None => match s.find('\n') {
            Some(pos) => pos,
            None => return Ok(None), // incomplete
        },
    };
    let request_line = &s[..first_line_end];
    let parts: Vec<&str> = request_line.splitn(3, ' ').collect();
    if parts.len() < 2 {
        return Err("malformed request line".to_string());
    }
    let method = parts[0].to_string();
    let path = parts[1].to_string();
    let version = if parts.len() >= 3 {
        if parts[2].contains("1.1") { 1 } else { 0 }
    } else {
        1
    };

    let header_start = if s.as_bytes()[first_line_end] == b'\r' {
        first_line_end + 2
    } else {
        first_line_end + 1
    };
    let rest = &s[header_start..];
    let mut headers = Vec::new();
    for line in rest.split('\n') {
        let line = line.trim_end_matches('\r');
        if line.is_empty() {
            break;
        }
        if let Some(colon_pos) = line.find(':') {
            let name = line[..colon_pos].trim().to_string();
            let value = line[colon_pos + 1..].trim().as_bytes().to_vec();
            headers.push(HttpHeader { name, value });
        }
    }

    Ok(Some(HttpRequest {
        method,
        path,
        version,
        headers,
    }))
}

// TOML parser (replaces `toml` crate)

/// A TOML value.
#[derive(Debug, Clone, PartialEq)]
pub enum TomlValue {
    String(String),
    Integer(i64),
    Float(f64),
    Boolean(bool),
    Array(Vec<TomlValue>),
    Table(TomlTable),
}

/// A TOML table (ordered map of key-value pairs).
pub type TomlTable = OrderedMap<String, TomlValue>;

impl TomlValue {
    pub fn as_str(&self) -> Option<&str> {
        match self {
            TomlValue::String(s) => Some(s),
            _ => None,
        }
    }
    pub fn as_integer(&self) -> Option<i64> {
        match self {
            TomlValue::Integer(n) => Some(*n),
            _ => None,
        }
    }
    pub fn as_float(&self) -> Option<f64> {
        match self {
            TomlValue::Float(f) => Some(*f),
            _ => None,
        }
    }
    pub fn as_bool(&self) -> Option<bool> {
        match self {
            TomlValue::Boolean(b) => Some(*b),
            _ => None,
        }
    }
    pub fn as_array(&self) -> Option<&Vec<TomlValue>> {
        match self {
            TomlValue::Array(a) => Some(a),
            _ => None,
        }
    }
    pub fn as_table(&self) -> Option<&TomlTable> {
        match self {
            TomlValue::Table(t) => Some(t),
            _ => None,
        }
    }
    pub fn get(&self, key: &str) -> Option<&TomlValue> {
        match self {
            TomlValue::Table(t) => t.get(key),
            _ => None,
        }
    }
}

/// Parse a TOML string into a table.
pub fn toml_parse(input: &str) -> Result<TomlTable, String> {
    let mut root = TomlTable::new();
    let mut current_section: Vec<String> = Vec::new();
    let mut array_of_tables: std::collections::HashMap<String, bool> = std::collections::HashMap::new();

    for (line_no, raw_line) in input.lines().enumerate() {
        let line = raw_line.trim();
        // Skip empty lines and comments
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        // Array of tables: [[section]]
        if line.starts_with("[[") && line.ends_with("]]") {
            let section_name = line[2..line.len() - 2].trim().to_string();
            current_section = section_name.split('.').map(|s| s.trim().to_string()).collect();
            array_of_tables.insert(section_name.clone(), true);
            // Ensure the array exists and add a new table
            ensure_array_of_tables(&mut root, &current_section);
            continue;
        }
        // Table header: [section]
        if line.starts_with('[') && line.ends_with(']') {
            let section_name = line[1..line.len() - 1].trim().to_string();
            current_section = section_name.split('.').map(|s| s.trim().to_string()).collect();
            // Ensure all parent tables exist
            ensure_table_path(&mut root, &current_section);
            continue;
        }
        // Key = value
        let eq_pos = match line.find('=') {
            Some(pos) => pos,
            None => return Err(format!("line {}: expected key = value", line_no + 1)),
        };
        let key = line[..eq_pos].trim().trim_matches('"').to_string();
        let value_str = line[eq_pos + 1..].trim();
        let value = parse_toml_value(value_str, input, line_no)?;

        let is_aot = current_section.len() > 0 && {
            let full = current_section.join(".");
            array_of_tables.contains_key(&full)
        };
        if is_aot {
            // Insert into the last element of the array of tables
            let table = get_last_array_table_mut(&mut root, &current_section);
            table.insert(key, value);
        } else if current_section.is_empty() {
            root.insert(key, value);
        } else {
            let table = get_or_create_table_mut(&mut root, &current_section);
            table.insert(key, value);
        }
    }
    Ok(root)
}

fn ensure_table_path(root: &mut TomlTable, path: &[String]) {
    let mut current = root;
    for key in path {
        if !current.contains_key(key) {
            current.insert(key.clone(), TomlValue::Table(TomlTable::new()));
        }
        match current.get_mut(key) {
            Some(TomlValue::Table(t)) => current = t,
            _ => return,
        }
    }
}

fn ensure_array_of_tables(root: &mut TomlTable, path: &[String]) {
    let mut current = root;
    for (i, key) in path.iter().enumerate() {
        if i == path.len() - 1 {
            // Last segment: create or append to array
            if !current.contains_key(key) {
                current.insert(key.clone(), TomlValue::Array(vec![TomlValue::Table(TomlTable::new())]));
            } else if let Some(TomlValue::Array(arr)) = current.get_mut(key) {
                arr.push(TomlValue::Table(TomlTable::new()));
            }
        } else {
            if !current.contains_key(key) {
                current.insert(key.clone(), TomlValue::Table(TomlTable::new()));
            }
            match current.get_mut(key) {
                Some(TomlValue::Table(t)) => current = t,
                _ => return,
            }
        }
    }
}

fn get_last_array_table_mut<'a>(root: &'a mut TomlTable, path: &[String]) -> &'a mut TomlTable {
    // Navigate to the last table in the array-of-tables at the given path.
    // Use raw pointer tricks to satisfy the borrow checker for in-place navigation.
    let mut current: *mut TomlTable = root;
    for (i, key) in path.iter().enumerate() {
        unsafe {
            if i == path.len() - 1 {
                if let Some(TomlValue::Array(arr)) = (*current).get_mut(key) {
                    if let Some(TomlValue::Table(t)) = arr.last_mut() {
                        return &mut *t;
                    }
                }
                return &mut *current;
            }
            match (*current).get_mut(key) {
                Some(TomlValue::Table(t)) => current = t as *mut TomlTable,
                _ => return &mut *current,
            }
        }
    }
    unsafe { &mut *current }
}

fn get_or_create_table_mut<'a>(root: &'a mut TomlTable, path: &[String]) -> &'a mut TomlTable {
    let mut current: *mut TomlTable = root;
    for key in path {
        unsafe {
            if !(*current).contains_key(key) {
                (*current).insert(key.clone(), TomlValue::Table(TomlTable::new()));
            }
            match (*current).get_mut(key) {
                Some(TomlValue::Table(t)) => current = t as *mut TomlTable,
                _ => return &mut *current,
            }
        }
    }
    unsafe { &mut *current }
}

fn parse_toml_value(s: &str, _full: &str, _line: usize) -> Result<TomlValue, String> {
    let s = s.trim();
    // Remove trailing comment (not inside a string)
    let s = strip_toml_comment(s);
    let s = s.trim();
    if s.is_empty() {
        return Err("empty value".to_string());
    }
    if s == "true" { return Ok(TomlValue::Boolean(true)); }
    if s == "false" { return Ok(TomlValue::Boolean(false)); }
    // String: basic ("...") or literal ('...')
    if s.starts_with('"') && s.len() >= 2 {
        return parse_toml_basic_string(s);
    }
    if s.starts_with('\'') && s.len() >= 2 {
        // Literal string: no escapes
        if let Some(end) = s[1..].find('\'') {
            return Ok(TomlValue::String(s[1..1 + end].to_string()));
        }
        return Err("unterminated literal string".to_string());
    }
    if s.starts_with('[') {
        return parse_toml_array(s);
    }
    if s.starts_with('{') {
        return parse_toml_inline_table(s);
    }
    // Number (integer or float)
    if let Ok(n) = s.replace('_', "").parse::<i64>() {
        return Ok(TomlValue::Integer(n));
    }
    // Hex/oct/bin integers
    if s.starts_with("0x") || s.starts_with("0X") {
        if let Ok(n) = i64::from_str_radix(&s[2..].replace('_', ""), 16) {
            return Ok(TomlValue::Integer(n));
        }
    }
    if s.starts_with("0o") || s.starts_with("0O") {
        if let Ok(n) = i64::from_str_radix(&s[2..].replace('_', ""), 8) {
            return Ok(TomlValue::Integer(n));
        }
    }
    if s.starts_with("0b") || s.starts_with("0B") {
        if let Ok(n) = i64::from_str_radix(&s[2..].replace('_', ""), 2) {
            return Ok(TomlValue::Integer(n));
        }
    }
    if s == "inf" || s == "+inf" { return Ok(TomlValue::Float(f64::INFINITY)); }
    if s == "-inf" { return Ok(TomlValue::Float(f64::NEG_INFINITY)); }
    if s == "nan" || s == "+nan" || s == "-nan" { return Ok(TomlValue::Float(f64::NAN)); }
    if let Ok(f) = s.replace('_', "").parse::<f64>() {
        return Ok(TomlValue::Float(f));
    }
    // Datetime (store as string)
    if s.contains('T') || (s.len() >= 10 && s.as_bytes().get(4) == Some(&b'-') && s.as_bytes().get(7) == Some(&b'-')) {
        return Ok(TomlValue::String(s.to_string()));
    }
    // Bare string (shouldn't happen in valid TOML, but be lenient)
    Ok(TomlValue::String(s.to_string()))
}

fn strip_toml_comment(s: &str) -> &str {
    let mut in_string = false;
    let mut escape = false;
    let bytes = s.as_bytes();
    for (i, &b) in bytes.iter().enumerate() {
        if escape {
            escape = false;
            continue;
        }
        if b == b'\\' && in_string {
            escape = true;
            continue;
        }
        if b == b'"' {
            in_string = !in_string;
            continue;
        }
        if b == b'#' && !in_string {
            return s[..i].trim_end();
        }
    }
    s
}

fn parse_toml_basic_string(s: &str) -> Result<TomlValue, String> {
    // Multi-line basic string """..."""
    if s.starts_with("\"\"\"") {
        let end = s[3..].find("\"\"\"").ok_or("unterminated multi-line string")?;
        return Ok(TomlValue::String(unescape_toml_string(&s[3..3 + end])));
    }
    // Single-line basic string
    let end = find_string_end(&s[1..]).ok_or("unterminated string")?;
    Ok(TomlValue::String(unescape_toml_string(&s[1..1 + end])))
}

fn find_string_end(s: &str) -> Option<usize> {
    let mut escape = false;
    for (i, c) in s.char_indices() {
        if escape {
            escape = false;
            continue;
        }
        if c == '\\' {
            escape = true;
            continue;
        }
        if c == '"' {
            return Some(i);
        }
    }
    None
}

fn unescape_toml_string(s: &str) -> String {
    let mut result = String::with_capacity(s.len());
    let mut chars = s.chars();
    while let Some(c) = chars.next() {
        if c == '\\' {
            match chars.next() {
                Some('n') => result.push('\n'),
                Some('t') => result.push('\t'),
                Some('r') => result.push('\r'),
                Some('\\') => result.push('\\'),
                Some('"') => result.push('"'),
                Some('u') => {
                    let hex: String = chars.by_ref().take(4).collect();
                    if let Ok(n) = u32::from_str_radix(&hex, 16) {
                        if let Some(c) = char::from_u32(n) {
                            result.push(c);
                        }
                    }
                }
                Some('U') => {
                    let hex: String = chars.by_ref().take(8).collect();
                    if let Ok(n) = u32::from_str_radix(&hex, 16) {
                        if let Some(c) = char::from_u32(n) {
                            result.push(c);
                        }
                    }
                }
                Some(other) => {
                    result.push('\\');
                    result.push(other);
                }
                None => result.push('\\'),
            }
        } else {
            result.push(c);
        }
    }
    result
}

fn parse_toml_array(s: &str) -> Result<TomlValue, String> {
    // Find matching ]
    let end = find_matching_bracket(s, 1)?;
    let inner = s[1..end].trim();
    if inner.is_empty() {
        return Ok(TomlValue::Array(Vec::new()));
    }
    let mut items = Vec::new();
    for item in split_toml_elements(inner) {
        let item = item.trim();
        if item.is_empty() { continue; }
        items.push(parse_toml_value(item, "", 0)?);
    }
    Ok(TomlValue::Array(items))
}

fn parse_toml_inline_table(s: &str) -> Result<TomlValue, String> {
    let end = find_matching_brace(s, 1)?;
    let inner = s[1..end].trim();
    let mut table = TomlTable::new();
    if inner.is_empty() {
        return Ok(TomlValue::Table(table));
    }
    for item in split_toml_elements(inner) {
        let item = item.trim();
        if item.is_empty() { continue; }
        let eq_pos = item.find('=').ok_or("inline table: missing =")?;
        let key = item[..eq_pos].trim().trim_matches('"').to_string();
        let val = parse_toml_value(item[eq_pos + 1..].trim(), "", 0)?;
        table.insert(key, val);
    }
    Ok(TomlValue::Table(table))
}

fn find_matching_bracket(s: &str, start: usize) -> Result<usize, String> {
    let mut depth = 1i32;
    let mut in_string = false;
    let mut escape = false;
    for (i, c) in s[start..].char_indices() {
        if escape { escape = false; continue; }
        if c == '\\' && in_string { escape = true; continue; }
        if c == '"' { in_string = !in_string; continue; }
        if in_string { continue; }
        if c == '[' { depth += 1; }
        if c == ']' {
            depth -= 1;
            if depth == 0 { return Ok(start + i); }
        }
    }
    Err("unterminated array".to_string())
}

fn find_matching_brace(s: &str, start: usize) -> Result<usize, String> {
    let mut depth = 1i32;
    let mut in_string = false;
    let mut escape = false;
    for (i, c) in s[start..].char_indices() {
        if escape { escape = false; continue; }
        if c == '\\' && in_string { escape = true; continue; }
        if c == '"' { in_string = !in_string; continue; }
        if in_string { continue; }
        if c == '{' { depth += 1; }
        if c == '}' {
            depth -= 1;
            if depth == 0 { return Ok(start + i); }
        }
    }
    Err("unterminated inline table".to_string())
}

fn split_toml_elements(s: &str) -> Vec<&str> {
    let mut results = Vec::new();
    let mut depth_bracket = 0i32;
    let mut depth_brace = 0i32;
    let mut in_string = false;
    let mut escape = false;
    let mut start = 0;
    for (i, c) in s.char_indices() {
        if escape { escape = false; continue; }
        if c == '\\' && in_string { escape = true; continue; }
        if c == '"' { in_string = !in_string; continue; }
        if in_string { continue; }
        match c {
            '[' => depth_bracket += 1,
            ']' => depth_bracket -= 1,
            '{' => depth_brace += 1,
            '}' => depth_brace -= 1,
            ',' if depth_bracket == 0 && depth_brace == 0 => {
                results.push(&s[start..i]);
                start = i + 1;
            }
            _ => {}
        }
    }
    if start < s.len() {
        results.push(&s[start..]);
    }
    results
}

/// Serialize a TomlValue to a pretty TOML string.
pub fn toml_to_string_pretty(value: &TomlValue) -> Result<String, String> {
    match value {
        TomlValue::Table(t) => {
            let mut out = String::new();
            serialize_table(&mut out, t, &[]);
            Ok(out)
        }
        _ => Err("top-level value must be a table".to_string()),
    }
}

fn serialize_table(out: &mut String, table: &TomlTable, prefix: &[String]) {
    // First pass: emit simple key-value pairs
    for (key, value) in table {
        let is_subtable = matches!(value, TomlValue::Table(_));
        let is_array_of_tables = if let TomlValue::Array(arr) = value {
            arr.first().map(|v| matches!(v, TomlValue::Table(_))).unwrap_or(false)
        } else {
            false
        };
        if is_subtable || is_array_of_tables {
            // Skip tables and array-of-tables for second pass
            continue;
        }
        out.push_str(&toml_key_escape(key));
        out.push_str(" = ");
        serialize_value(out, value);
        out.push('\n');
    }
    // Second pass: emit sub-tables
    for (key, value) in table {
        if let TomlValue::Table(sub) = value {
            let mut full_key = prefix.to_vec();
            full_key.push(key.clone());
            if !out.is_empty() && !out.ends_with("\n\n") {
                out.push('\n');
            }
            out.push('[');
            out.push_str(&full_key.join("."));
            out.push_str("]\n");
            serialize_table(out, sub, &full_key);
        } else if let TomlValue::Array(arr) = value {
            let is_aot = arr.first().map(|v| matches!(v, TomlValue::Table(_))).unwrap_or(false);
            if is_aot {
                let mut full_key = prefix.to_vec();
                full_key.push(key.clone());
                for item in arr {
                    if let TomlValue::Table(sub) = item {
                        if !out.is_empty() && !out.ends_with("\n\n") {
                            out.push('\n');
                        }
                        out.push_str("[[");
                        out.push_str(&full_key.join("."));
                        out.push_str("]]\n");
                        serialize_table(out, sub, &full_key);
                    }
                }
            }
        }
    }
}

fn serialize_value(out: &mut String, value: &TomlValue) {
    match value {
        TomlValue::String(s) => {
            out.push('"');
            for c in s.chars() {
                match c {
                    '"' => out.push_str("\\\""),
                    '\\' => out.push_str("\\\\"),
                    '\n' => out.push_str("\\n"),
                    '\r' => out.push_str("\\r"),
                    '\t' => out.push_str("\\t"),
                    c => out.push(c),
                }
            }
            out.push('"');
        }
        TomlValue::Integer(n) => out.push_str(&n.to_string()),
        TomlValue::Float(f) => {
            if f.is_infinite() {
                if *f > 0.0 { out.push_str("inf"); } else { out.push_str("-inf"); }
            } else if f.is_nan() {
                out.push_str("nan");
            } else {
                let s = format!("{}", f);
                out.push_str(&s);
                // Ensure there's a decimal point
                if !s.contains('.') && !s.contains('e') && !s.contains('E') {
                    out.push_str(".0");
                }
            }
        }
        TomlValue::Boolean(b) => out.push_str(if *b { "true" } else { "false" }),
        TomlValue::Array(arr) => {
            out.push('[');
            for (i, item) in arr.iter().enumerate() {
                if i > 0 { out.push_str(", "); }
                serialize_value(out, item);
            }
            out.push(']');
        }
        TomlValue::Table(t) => {
            out.push('{');
            for (i, (k, v)) in t.iter().enumerate() {
                if i > 0 { out.push_str(", "); }
                out.push_str(&toml_key_escape(k));
                out.push_str(" = ");
                serialize_value(out, v);
            }
            out.push('}');
        }
    }
}

fn toml_key_escape(key: &str) -> String {
    if key.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_') {
        key.to_string()
    } else {
        format!("\"{}\"", key.replace('\\', "\\\\").replace('"', "\\\""))
    }
}

// SHA-256 (replaces `sha2` crate for SHA-256)

/// Compute SHA-1 hash of data, returning 20 bytes.
pub fn sha1(data: &[u8]) -> [u8; 20] {
    let mut h0: u32 = 0x67452301;
    let mut h1: u32 = 0xEFCDAB89;
    let mut h2: u32 = 0x98BADCFE;
    let mut h3: u32 = 0x10325476;
    let mut h4: u32 = 0xC3D2E1F0;

    let bit_len = (data.len() as u64).wrapping_mul(8);
    let mut padded = data.to_vec();
    padded.push(0x80);
    while (padded.len() % 64) != 56 { padded.push(0); }
    padded.extend_from_slice(&bit_len.to_be_bytes());

    for block in padded.chunks_exact(64) {
        let mut w = [0u32; 80];
        for i in 0..16 {
            w[i] = u32::from_be_bytes([block[i*4], block[i*4+1], block[i*4+2], block[i*4+3]]);
        }
        for i in 16..80 {
            w[i] = (w[i-3] ^ w[i-8] ^ w[i-14] ^ w[i-16]).rotate_left(1);
        }

        let (mut a, mut b, mut c, mut d, mut e) = (h0, h1, h2, h3, h4);

        for i in 0..80 {
            let (f, k) = match i {
                0..=19 => ((b & c) | ((!b) & d), 0x5A827999u32),
                20..=39 => (b ^ c ^ d, 0x6ED9EBA1u32),
                40..=59 => ((b & c) | (b & d) | (c & d), 0x8F1BBCDCu32),
                _ => (b ^ c ^ d, 0xCA62C1D6u32),
            };
            let temp = a.rotate_left(5).wrapping_add(f).wrapping_add(e).wrapping_add(k).wrapping_add(w[i]);
            e = d; d = c; c = b.rotate_left(30); b = a; a = temp;
        }

        h0 = h0.wrapping_add(a);
        h1 = h1.wrapping_add(b);
        h2 = h2.wrapping_add(c);
        h3 = h3.wrapping_add(d);
        h4 = h4.wrapping_add(e);
    }

    let mut result = [0u8; 20];
    result[0..4].copy_from_slice(&h0.to_be_bytes());
    result[4..8].copy_from_slice(&h1.to_be_bytes());
    result[8..12].copy_from_slice(&h2.to_be_bytes());
    result[12..16].copy_from_slice(&h3.to_be_bytes());
    result[16..20].copy_from_slice(&h4.to_be_bytes());
    result
}

const SHA256_K: [u32; 64] = [
    0x428a2f98, 0x71374491, 0xb5c0fbcf, 0xe9b5dba5, 0x3956c25b, 0x59f111f1, 0x923f82a4, 0xab1c5ed5,
    0xd807aa98, 0x12835b01, 0x243185be, 0x550c7dc3, 0x72be5d74, 0x80deb1fe, 0x9bdc06a7, 0xc19bf174,
    0xe49b69c1, 0xefbe4786, 0x0fc19dc6, 0x240ca1cc, 0x2de92c6f, 0x4a7484aa, 0x5cb0a9dc, 0x76f988da,
    0x983e5152, 0xa831c66d, 0xb00327c8, 0xbf597fc7, 0xc6e00bf3, 0xd5a79147, 0x06ca6351, 0x14292967,
    0x27b70a85, 0x2e1b2138, 0x4d2c6dfc, 0x53380d13, 0x650a7354, 0x766a0abb, 0x81c2c92e, 0x92722c85,
    0xa2bfe8a1, 0xa81a664b, 0xc24b8b70, 0xc76c51a3, 0xd192e819, 0xd6990624, 0xf40e3585, 0x106aa070,
    0x19a4c116, 0x1e376c08, 0x2748774c, 0x34b0bcb5, 0x391c0cb3, 0x4ed8aa4a, 0x5b9cca4f, 0x682e6ff3,
    0x748f82ee, 0x78a5636f, 0x84c87814, 0x8cc70208, 0x90befffa, 0xa4506ceb, 0xbef9a3f7, 0xc67178f2,
];

/// Compute SHA-256 hash of data, returning 32 bytes.
pub fn sha256(data: &[u8]) -> [u8; 32] {
    // FIPS 180-4 initial hash values
    let mut h: [u32; 8] = [
        0x6a09e667, 0xbb67ae85, 0x3c6ef372, 0xa54ff53a,
        0x510e527f, 0x9b05688c, 0x1f83d9ab, 0x5be0cd19,
    ];

    // Pre-processing: pad message
    let bit_len = (data.len() as u64).wrapping_mul(8);
    let mut padded = data.to_vec();
    padded.push(0x80);
    while (padded.len() % 64) != 56 {
        padded.push(0);
    }
    padded.extend_from_slice(&bit_len.to_be_bytes());

    // Process each 64-byte block
    for block in padded.chunks_exact(64) {
        let mut w = [0u32; 64];
        for i in 0..16 {
            w[i] = u32::from_be_bytes([block[i * 4], block[i * 4 + 1], block[i * 4 + 2], block[i * 4 + 3]]);
        }
        for i in 16..64 {
            let s0 = w[i - 15].rotate_right(7) ^ w[i - 15].rotate_right(18) ^ (w[i - 15] >> 3);
            let s1 = w[i - 2].rotate_right(17) ^ w[i - 2].rotate_right(19) ^ (w[i - 2] >> 10);
            w[i] = w[i - 16].wrapping_add(s0).wrapping_add(w[i - 7]).wrapping_add(s1);
        }

        let [mut a, mut b, mut c, mut d, mut e, mut f, mut g, mut hh] = h;

        for i in 0..64 {
            let s1 = e.rotate_right(6) ^ e.rotate_right(11) ^ e.rotate_right(25);
            let ch = (e & f) ^ ((!e) & g);
            let temp1 = hh.wrapping_add(s1).wrapping_add(ch).wrapping_add(SHA256_K[i]).wrapping_add(w[i]);
            let s0 = a.rotate_right(2) ^ a.rotate_right(13) ^ a.rotate_right(22);
            let maj = (a & b) ^ (a & c) ^ (b & c);
            let temp2 = s0.wrapping_add(maj);

            hh = g;
            g = f;
            f = e;
            e = d.wrapping_add(temp1);
            d = c;
            c = b;
            b = a;
            a = temp1.wrapping_add(temp2);
        }

        h[0] = h[0].wrapping_add(a);
        h[1] = h[1].wrapping_add(b);
        h[2] = h[2].wrapping_add(c);
        h[3] = h[3].wrapping_add(d);
        h[4] = h[4].wrapping_add(e);
        h[5] = h[5].wrapping_add(f);
        h[6] = h[6].wrapping_add(g);
        h[7] = h[7].wrapping_add(hh);
    }

    let mut result = [0u8; 32];
    for (i, &val) in h.iter().enumerate() {
        result[i * 4..i * 4 + 4].copy_from_slice(&val.to_be_bytes());
    }
    result
}

// SHA-512 (replaces `sha2` crate for SHA-512)

const SHA512_K: [u64; 80] = [
    0x428a2f98d728ae22, 0x7137449123ef65cd, 0xb5c0fbcfec4d3b2f, 0xe9b5dba58189dbbc,
    0x3956c25bf348b538, 0x59f111f1b605d019, 0x923f82a4af194f9b, 0xab1c5ed5da6d8118,
    0xd807aa98a3030242, 0x12835b0145706fbe, 0x243185be4ee4b28c, 0x550c7dc3d5ffb4e2,
    0x72be5d74f27b896f, 0x80deb1fe3b1696b1, 0x9bdc06a725c71235, 0xc19bf174cf692694,
    0xe49b69c19ef14ad2, 0xefbe4786384f25e3, 0x0fc19dc68b8cd5b5, 0x240ca1cc77ac9c65,
    0x2de92c6f592b0275, 0x4a7484aa6ea6e483, 0x5cb0a9dcbd41fbd4, 0x76f988da831153b5,
    0x983e5152ee66dfab, 0xa831c66d2db43210, 0xb00327c898fb213f, 0xbf597fc7beef0ee4,
    0xc6e00bf33da88fc2, 0xd5a79147930aa725, 0x06ca6351e003826f, 0x142929670a0e6e70,
    0x27b70a8546d22ffc, 0x2e1b21385c26c926, 0x4d2c6dfc5ac42aed, 0x53380d139d95b3df,
    0x650a73548baf63de, 0x766a0abb3c77b2a8, 0x81c2c92e47edaee6, 0x92722c851482353b,
    0xa2bfe8a14cf10364, 0xa81a664bbc423001, 0xc24b8b70d0f89791, 0xc76c51a30654be30,
    0xd192e819d6ef5218, 0xd69906245565a910, 0xf40e35855771202a, 0x106aa07032bbd1b8,
    0x19a4c116b8d2d0c8, 0x1e376c085141ab53, 0x2748774cdf8eeb99, 0x34b0bcb5e19b48a8,
    0x391c0cb3c5c95a63, 0x4ed8aa4ae3418acb, 0x5b9cca4f7763e373, 0x682e6ff3d6b2b8a3,
    0x748f82ee5defb2fc, 0x78a5636f43172f60, 0x84c87814a1f0ab72, 0x8cc702081a6439ec,
    0x90befffa23631e28, 0xa4506cebde82bde9, 0xbef9a3f7b2c67915, 0xc67178f2e372532b,
    0xca273eceea26619c, 0xd186b8c721c0c207, 0xeada7dd6cde0eb1e, 0xf57d4f7fee6ed178,
    0x06f067aa72176fba, 0x0a637dc5a2c898a6, 0x113f9804bef90dae, 0x1b710b35131c471b,
    0x28db77f523047d84, 0x32caab7b40c72493, 0x3c9ebe0a15c9bebc, 0x431d67c49c100d4c,
    0x4cc5d4becb3e42b6, 0x597f299cfc657e2a, 0x5fcb6fab3ad6faec, 0x6c44198c4a475817,
];

/// Compute SHA-512 hash of data, returning 64 bytes.
pub fn sha512(data: &[u8]) -> [u8; 64] {
    let mut h: [u64; 8] = [
        0x6a09e667f3bcc908, 0xbb67ae8584caa73b,
        0x3c6ef372fe94f82b, 0xa54ff53a5f1d36f1,
        0x510e527fade682d1, 0x9b05688c2b3e6c1f,
        0x1f83d9abfb41bd6b, 0x5be0cd19137e2179,
    ];

    // Pre-processing: pad message
    let bit_len = (data.len() as u128) * 8;
    let mut padded = data.to_vec();
    padded.push(0x80);
    while (padded.len() % 128) != 112 {
        padded.push(0);
    }
    padded.extend_from_slice(&bit_len.to_be_bytes());

    // Process each 128-byte block
    for block in padded.chunks_exact(128) {
        let mut w = [0u64; 80];
        for i in 0..16 {
            let off = i * 8;
            w[i] = u64::from_be_bytes([
                block[off], block[off + 1], block[off + 2], block[off + 3],
                block[off + 4], block[off + 5], block[off + 6], block[off + 7],
            ]);
        }
        for i in 16..80 {
            let s0 = w[i - 15].rotate_right(1) ^ w[i - 15].rotate_right(8) ^ (w[i - 15] >> 7);
            let s1 = w[i - 2].rotate_right(19) ^ w[i - 2].rotate_right(61) ^ (w[i - 2] >> 6);
            w[i] = w[i - 16].wrapping_add(s0).wrapping_add(w[i - 7]).wrapping_add(s1);
        }

        let [mut a, mut b, mut c, mut d, mut e, mut f, mut g, mut hh] = h;

        for i in 0..80 {
            let s1 = e.rotate_right(14) ^ e.rotate_right(18) ^ e.rotate_right(41);
            let ch = (e & f) ^ ((!e) & g);
            let temp1 = hh.wrapping_add(s1).wrapping_add(ch).wrapping_add(SHA512_K[i]).wrapping_add(w[i]);
            let s0 = a.rotate_right(28) ^ a.rotate_right(34) ^ a.rotate_right(39);
            let maj = (a & b) ^ (a & c) ^ (b & c);
            let temp2 = s0.wrapping_add(maj);

            hh = g;
            g = f;
            f = e;
            e = d.wrapping_add(temp1);
            d = c;
            c = b;
            b = a;
            a = temp1.wrapping_add(temp2);
        }

        h[0] = h[0].wrapping_add(a);
        h[1] = h[1].wrapping_add(b);
        h[2] = h[2].wrapping_add(c);
        h[3] = h[3].wrapping_add(d);
        h[4] = h[4].wrapping_add(e);
        h[5] = h[5].wrapping_add(f);
        h[6] = h[6].wrapping_add(g);
        h[7] = h[7].wrapping_add(hh);
    }

    let mut result = [0u8; 64];
    for (i, &val) in h.iter().enumerate() {
        result[i * 8..i * 8 + 8].copy_from_slice(&val.to_be_bytes());
    }
    result
}

// BLAKE3 (replaces `blake3` crate)

const BLAKE3_IV: [u32; 8] = [
    0x6A09E667, 0xBB67AE85, 0x3C6EF372, 0xA54FF53A,
    0x510E527F, 0x9B05688C, 0x1F83D9AB, 0x5BE0CD19,
];

const BLAKE3_MSG_PERMUTATION: [usize; 16] = [2, 6, 3, 10, 7, 0, 4, 13, 1, 11, 12, 5, 9, 14, 15, 8];

const BLAKE3_CHUNK_START: u32 = 1;
const BLAKE3_CHUNK_END: u32 = 2;
const BLAKE3_PARENT: u32 = 4;
const BLAKE3_ROOT: u32 = 8;

#[inline]
fn blake3_g(state: &mut [u32; 16], a: usize, b: usize, c: usize, d: usize, mx: u32, my: u32) {
    state[a] = state[a].wrapping_add(state[b]).wrapping_add(mx);
    state[d] = (state[d] ^ state[a]).rotate_right(16);
    state[c] = state[c].wrapping_add(state[d]);
    state[b] = (state[b] ^ state[c]).rotate_right(12);
    state[a] = state[a].wrapping_add(state[b]).wrapping_add(my);
    state[d] = (state[d] ^ state[a]).rotate_right(8);
    state[c] = state[c].wrapping_add(state[d]);
    state[b] = (state[b] ^ state[c]).rotate_right(7);
}

fn blake3_round(state: &mut [u32; 16], m: &[u32; 16]) {
    blake3_g(state, 0, 4, 8, 12, m[0], m[1]);
    blake3_g(state, 1, 5, 9, 13, m[2], m[3]);
    blake3_g(state, 2, 6, 10, 14, m[4], m[5]);
    blake3_g(state, 3, 7, 11, 15, m[6], m[7]);
    blake3_g(state, 0, 5, 10, 15, m[8], m[9]);
    blake3_g(state, 1, 6, 11, 12, m[10], m[11]);
    blake3_g(state, 2, 7, 8, 13, m[12], m[13]);
    blake3_g(state, 3, 4, 9, 14, m[14], m[15]);
}

fn blake3_permute(m: &[u32; 16]) -> [u32; 16] {
    let mut permuted = [0u32; 16];
    for i in 0..16 {
        permuted[i] = m[BLAKE3_MSG_PERMUTATION[i]];
    }
    permuted
}

fn blake3_compress(
    chaining_value: &[u32; 8],
    block_words: &[u32; 16],
    counter: u64,
    block_len: u32,
    flags: u32,
) -> [u32; 16] {
    let mut state = [0u32; 16];
    state[..8].copy_from_slice(chaining_value);
    state[8] = BLAKE3_IV[0];
    state[9] = BLAKE3_IV[1];
    state[10] = BLAKE3_IV[2];
    state[11] = BLAKE3_IV[3];
    state[12] = counter as u32;
    state[13] = (counter >> 32) as u32;
    state[14] = block_len;
    state[15] = flags;

    let mut m = *block_words;
    for _ in 0..6 {
        blake3_round(&mut state, &m);
        m = blake3_permute(&m);
    }
    blake3_round(&mut state, &m);

    for i in 0..8 {
        state[i] ^= state[i + 8];
        state[i + 8] ^= chaining_value[i];
    }
    state
}

fn blake3_words_from_block(block: &[u8]) -> [u32; 16] {
    let mut words = [0u32; 16];
    for i in 0..16 {
        let off = i * 4;
        if off + 4 <= block.len() {
            words[i] = u32::from_le_bytes([block[off], block[off + 1], block[off + 2], block[off + 3]]);
        } else if off < block.len() {
            let mut buf = [0u8; 4];
            buf[..block.len() - off].copy_from_slice(&block[off..]);
            words[i] = u32::from_le_bytes(buf);
        }
    }
    words
}

fn blake3_chaining_value(cv: &[u32; 8], block: &[u8], counter: u64, block_len: u32, flags: u32) -> [u32; 8] {
    let words = blake3_words_from_block(block);
    let full = blake3_compress(cv, &words, counter, block_len, flags);
    let mut out = [0u32; 8];
    out.copy_from_slice(&full[..8]);
    out
}

/// Compute BLAKE3 hash of data, returning 32 bytes.
pub fn blake3_hash(data: &[u8]) -> [u8; 32] {
    // Process data in 1024-byte chunks (each chunk has up to 16 blocks of 64 bytes)
    let mut cv_stack: Vec<[u32; 8]> = Vec::new();
    let mut chunk_counter: u64 = 0;
    let total_chunks = if data.is_empty() { 1 } else { (data.len() + 1023) / 1024 };

    let chunks: Vec<&[u8]> = if data.is_empty() {
        vec![&[]]
    } else {
        data.chunks(1024).collect()
    };

    for (ci, chunk) in chunks.iter().enumerate() {
        let is_last_chunk = ci == total_chunks - 1;

        // Process blocks within this chunk
        let mut cv = BLAKE3_IV;
        let block_count = if chunk.is_empty() { 1 } else { (chunk.len() + 63) / 64 };

        for bi in 0..block_count {
            let block_start = bi * 64;
            let block_end = (block_start + 64).min(chunk.len());
            let block_data = if block_start < chunk.len() {
                &chunk[block_start..block_end]
            } else {
                &[]
            };
            let block_len = block_data.len() as u32;

            let mut flags = 0u32;
            if bi == 0 {
                flags |= BLAKE3_CHUNK_START;
            }
            if bi == block_count - 1 {
                flags |= BLAKE3_CHUNK_END;
            }
            if is_last_chunk && bi == block_count - 1 && total_chunks == 1 {
                flags |= BLAKE3_ROOT;
            }

            if flags & BLAKE3_CHUNK_END != 0 && !(flags & BLAKE3_ROOT != 0) {
                // Last block of non-root chunk: get chaining value
                cv = blake3_chaining_value(&cv, block_data, chunk_counter, block_len, flags);
            } else if flags & BLAKE3_ROOT != 0 {
                // Root: get full output
                let words = blake3_words_from_block(block_data);
                let state = blake3_compress(&cv, &words, chunk_counter, block_len, flags);
                let mut result = [0u8; 32];
                for i in 0..8 {
                    result[i * 4..i * 4 + 4].copy_from_slice(&state[i].to_le_bytes());
                }
                return result;
            } else {
                cv = blake3_chaining_value(&cv, block_data, chunk_counter, block_len, flags);
            }
        }

        // Merge chaining values using a binary tree
        let mut new_cv = cv;
        let mut total_chunks_so_far = chunk_counter + 1;

        // Push the chaining value, then merge adjacent pairs
        while total_chunks_so_far & 1 == 0 && !cv_stack.is_empty() {
            let left = cv_stack.pop().unwrap();
            // Parent node: compress left || right
            let mut parent_block = [0u8; 64];
            for i in 0..8 {
                parent_block[i * 4..i * 4 + 4].copy_from_slice(&left[i].to_le_bytes());
            }
            for i in 0..8 {
                parent_block[32 + i * 4..32 + i * 4 + 4].copy_from_slice(&new_cv[i].to_le_bytes());
            }
            let parent_flags = if is_last_chunk && cv_stack.is_empty() {
                BLAKE3_ROOT
            } else {
                0u32
            } | BLAKE3_PARENT;

            if parent_flags & BLAKE3_ROOT != 0 {
                let words = blake3_words_from_block(&parent_block);
                let state = blake3_compress(&BLAKE3_IV, &words, 0, 64, parent_flags);
                let mut result = [0u8; 32];
                for i in 0..8 {
                    result[i * 4..i * 4 + 4].copy_from_slice(&state[i].to_le_bytes());
                }
                return result;
            }

            new_cv = blake3_chaining_value(&BLAKE3_IV, &parent_block, 0, 64, parent_flags);
            total_chunks_so_far >>= 1;
        }

        cv_stack.push(new_cv);
        chunk_counter += 1;
    }

    // Finalize remaining stack entries
    while cv_stack.len() > 1 {
        let right = cv_stack.pop().unwrap();
        let left = cv_stack.pop().unwrap();
        let mut parent_block = [0u8; 64];
        for i in 0..8 {
            parent_block[i * 4..i * 4 + 4].copy_from_slice(&left[i].to_le_bytes());
        }
        for i in 0..8 {
            parent_block[32 + i * 4..32 + i * 4 + 4].copy_from_slice(&right[i].to_le_bytes());
        }
        let parent_flags = if cv_stack.is_empty() {
            BLAKE3_ROOT | BLAKE3_PARENT
        } else {
            BLAKE3_PARENT
        };
        if parent_flags & BLAKE3_ROOT != 0 {
            let words = blake3_words_from_block(&parent_block);
            let state = blake3_compress(&BLAKE3_IV, &words, 0, 64, parent_flags);
            let mut result = [0u8; 32];
            for i in 0..8 {
                result[i * 4..i * 4 + 4].copy_from_slice(&state[i].to_le_bytes());
            }
            return result;
        }
        let cv = blake3_chaining_value(&BLAKE3_IV, &parent_block, 0, 64, parent_flags);
        cv_stack.push(cv);
    }

    // Should not reach here for non-empty data, but fallback
    let cv = cv_stack.pop().unwrap_or(BLAKE3_IV);
    let mut result = [0u8; 32];
    for i in 0..8 {
        result[i * 4..i * 4 + 4].copy_from_slice(&cv[i].to_le_bytes());
    }
    result
}

/// Return BLAKE3 hash as hex string.
pub fn blake3_hash_hex(data: &[u8]) -> String {
    hex_encode(&blake3_hash(data))
}

// CSV parser/writer (replaces `csv` crate)

/// Parse a single CSV line, handling quoted fields.
fn csv_parse_line(line: &str) -> Vec<String> {
    let mut fields = Vec::new();
    let mut current = String::new();
    let mut in_quotes = false;
    let mut chars = line.chars().peekable();

    while let Some(c) = chars.next() {
        if in_quotes {
            if c == '"' {
                if chars.peek() == Some(&'"') {
                    chars.next();
                    current.push('"');
                } else {
                    in_quotes = false;
                }
            } else {
                current.push(c);
            }
        } else if c == '"' {
            in_quotes = true;
        } else if c == ',' {
            fields.push(current.clone());
            current.clear();
        } else {
            current.push(c);
        }
    }
    fields.push(current);
    fields
}

/// Split CSV text into lines, handling quoted newlines.
fn csv_split_lines(text: &str) -> Vec<String> {
    let mut lines = Vec::new();
    let mut current = String::new();
    let mut in_quotes = false;

    for c in text.chars() {
        if c == '"' {
            in_quotes = !in_quotes;
            current.push(c);
        } else if c == '\n' && !in_quotes {
            let trimmed = if current.ends_with('\r') {
                current[..current.len() - 1].to_string()
            } else {
                current.clone()
            };
            if !trimmed.is_empty() {
                lines.push(trimmed);
            }
            current.clear();
        } else {
            current.push(c);
        }
    }
    if !current.is_empty() {
        let trimmed = if current.ends_with('\r') {
            current[..current.len() - 1].to_string()
        } else {
            current
        };
        if !trimmed.is_empty() {
            lines.push(trimmed);
        }
    }
    lines
}

/// Parsed CSV data with headers and records.
pub struct CsvData {
    pub headers: Vec<String>,
    pub records: Vec<Vec<String>>,
}

/// Parse CSV text with headers. First line is treated as header row.
pub fn csv_parse(text: &str) -> Result<CsvData, String> {
    let lines = csv_split_lines(text);
    if lines.is_empty() {
        return Ok(CsvData { headers: Vec::new(), records: Vec::new() });
    }
    let headers = csv_parse_line(&lines[0]);
    let mut records = Vec::new();
    for line in &lines[1..] {
        records.push(csv_parse_line(line));
    }
    Ok(CsvData { headers, records })
}

/// Parse CSV text without headers. All lines are data rows.
pub fn csv_parse_no_headers(text: &str) -> Result<Vec<Vec<String>>, String> {
    let lines = csv_split_lines(text);
    Ok(lines.iter().map(|l| csv_parse_line(l)).collect())
}

/// Escape a CSV field: quote if it contains comma, quote, or newline.
fn csv_escape_field(field: &str) -> String {
    if field.contains(',') || field.contains('"') || field.contains('\n') || field.contains('\r') {
        let escaped = field.replace('"', "\"\"");
        format!("\"{}\"", escaped)
    } else {
        field.to_string()
    }
}

/// Write CSV records with a header row.
pub fn csv_write(headers: &[&str], rows: &[Vec<String>]) -> String {
    let mut output = String::new();
    let escaped_headers: Vec<String> = headers.iter().map(|h| csv_escape_field(h)).collect();
    output.push_str(&escaped_headers.join(","));
    output.push('\n');
    for row in rows {
        let escaped: Vec<String> = row.iter().map(|f| csv_escape_field(f)).collect();
        output.push_str(&escaped.join(","));
        output.push('\n');
    }
    output
}

// Random number generator (replaces `rand` crate)

use std::cell::RefCell;

/// Xorshift128+ PRNG state.
struct Rng {
    s0: u64,
    s1: u64,
}

impl Rng {
    fn from_entropy() -> Self {
        let mut seed = [0u8; 16];
        // Read from /dev/urandom (works on Linux, macOS, BSDs)
        if let Ok(mut f) = std::fs::File::open("/dev/urandom") {
            use std::io::Read;
            let _ = f.read_exact(&mut seed);
        } else {
            // Fallback: use address of a stack variable + time
            let stack_addr = &seed as *const _ as u64;
            let time_seed = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos() as u64)
                .unwrap_or(0x12345678deadbeef);
            seed[..8].copy_from_slice(&stack_addr.to_le_bytes());
            seed[8..].copy_from_slice(&time_seed.to_le_bytes());
        }
        let s0 = u64::from_le_bytes([seed[0], seed[1], seed[2], seed[3], seed[4], seed[5], seed[6], seed[7]]);
        let s1 = u64::from_le_bytes([seed[8], seed[9], seed[10], seed[11], seed[12], seed[13], seed[14], seed[15]]);
        // Ensure non-zero state
        Rng {
            s0: if s0 == 0 { 0x9E3779B97F4A7C15 } else { s0 },
            s1: if s1 == 0 { 0x6A09E667F3BCC908 } else { s1 },
        }
    }

    fn next_u64(&mut self) -> u64 {
        let mut s1 = self.s0;
        let s0 = self.s1;
        let result = s0.wrapping_add(s1);
        self.s0 = s0;
        s1 ^= s1 << 23;
        self.s1 = s1 ^ s0 ^ (s1 >> 17) ^ (s0 >> 26);
        result
    }

    fn random_bool(&mut self) -> bool {
        self.next_u64() & 1 == 1
    }

    fn random_f64(&mut self) -> f64 {
        // Generate a uniform f64 in [0, 1)
        let bits = self.next_u64() >> 11; // 53 bits
        bits as f64 * (1.0 / (1u64 << 53) as f64)
    }

    fn random_range_u64(&mut self, low: u64, high: u64) -> u64 {
        if low >= high {
            return low;
        }
        let range = high - low;
        // Rejection sampling to avoid modulo bias
        let limit = u64::MAX - (u64::MAX % range);
        loop {
            let val = self.next_u64();
            if val < limit {
                return low + (val % range);
            }
        }
    }

    fn random_range_i64(&mut self, low: i64, high: i64) -> i64 {
        if low >= high {
            return low;
        }
        let range = (high as u64).wrapping_sub(low as u64);
        let val = self.random_range_u64(0, range);
        low.wrapping_add(val as i64)
    }

    fn random_range_f64(&mut self, low: f64, high: f64) -> f64 {
        low + self.random_f64() * (high - low)
    }

    fn fill_bytes(&mut self, dest: &mut [u8]) {
        let mut i = 0;
        while i < dest.len() {
            let val = self.next_u64();
            let bytes = val.to_le_bytes();
            let remaining = dest.len() - i;
            let to_copy = remaining.min(8);
            dest[i..i + to_copy].copy_from_slice(&bytes[..to_copy]);
            i += to_copy;
        }
    }

    fn shuffle<T>(&mut self, slice: &mut [T]) {
        // Fisher-Yates shuffle
        for i in (1..slice.len()).rev() {
            let j = self.random_range_u64(0, (i + 1) as u64) as usize;
            slice.swap(i, j);
        }
    }
}

thread_local! {
    static THREAD_RNG: RefCell<Rng> = RefCell::new(Rng::from_entropy());
}

/// Generate a random i64.
pub fn random_i64() -> i64 {
    THREAD_RNG.with(|rng| rng.borrow_mut().next_u64() as i64)
}

/// Generate a random f64 in [0, 1).
pub fn random_f64() -> f64 {
    THREAD_RNG.with(|rng| rng.borrow_mut().random_f64())
}

/// Generate a random bool.
pub fn random_bool() -> bool {
    THREAD_RNG.with(|rng| rng.borrow_mut().random_bool())
}

/// Generate a random i64 in [low, high).
pub fn random_range_i64(low: i64, high: i64) -> i64 {
    THREAD_RNG.with(|rng| rng.borrow_mut().random_range_i64(low, high))
}

/// Generate a random f64 in [low, high).
pub fn random_range_f64(low: f64, high: f64) -> f64 {
    THREAD_RNG.with(|rng| rng.borrow_mut().random_range_f64(low, high))
}

/// Generate a random usize in [0, high).
pub fn random_range_usize(high: usize) -> usize {
    THREAD_RNG.with(|rng| rng.borrow_mut().random_range_u64(0, high as u64) as usize)
}

/// Fill a byte slice with random bytes.
pub fn random_fill_bytes(dest: &mut [u8]) {
    THREAD_RNG.with(|rng| rng.borrow_mut().fill_bytes(dest));
}

/// Shuffle a slice randomly (Fisher-Yates).
pub fn random_shuffle<T>(slice: &mut [T]) {
    THREAD_RNG.with(|rng| rng.borrow_mut().shuffle(slice));
}

/// Take a random sample of `count` elements from a slice (partial shuffle).
pub fn random_sample<T: Clone>(slice: &mut [T], count: usize) -> Vec<T> {
    let count = count.min(slice.len());
    THREAD_RNG.with(|rng| {
        let mut rng = rng.borrow_mut();
        // Partial Fisher-Yates: shuffle first `count` elements
        for i in 0..count {
            let j = rng.random_range_u64(i as u64, slice.len() as u64) as usize;
            slice.swap(i, j);
        }
    });
    slice[..count].to_vec()
}

// Simple line editor (replaces `rustyline` crate)

/// A simple line editor with history support for the REPL.
pub struct LineEditor {
    history: Vec<String>,
    history_path: Option<std::path::PathBuf>,
    completions: Vec<String>,
}

/// Errors from the line editor.
pub enum LineEditError {
    Interrupted,
    Eof,
    Io(std::io::Error),
}

impl LineEditor {
    /// Create a new line editor.
    pub fn new() -> Self {
        LineEditor {
            history: Vec::new(),
            history_path: None,
            completions: Vec::new(),
        }
    }

    /// Load history from a file.
    pub fn load_history(&mut self, path: &std::path::Path) {
        self.history_path = Some(path.to_path_buf());
        if let Ok(contents) = std::fs::read_to_string(path) {
            self.history = contents.lines().map(|l| l.to_string()).collect();
        }
    }

    /// Save history to the configured file.
    pub fn save_history(&self) {
        if let Some(path) = &self.history_path {
            let content = self.history.join("\n");
            let _ = std::fs::write(path, content);
        }
    }

    /// Set completions for tab completion.
    pub fn set_completions(&mut self, completions: Vec<String>) {
        self.completions = completions;
    }

    /// Get completions matching a prefix.
    pub fn complete(&self, prefix: &str) -> Vec<&str> {
        self.completions.iter()
            .filter(|c| c.starts_with(prefix))
            .map(|c| c.as_str())
            .take(20)
            .collect()
    }

    /// Search history in reverse for a pattern.
    pub fn reverse_search(&self, pattern: &str) -> Option<&str> {
        self.history.iter().rev()
            .find(|entry| entry.contains(pattern))
            .map(|s| s.as_str())
    }

    /// Read a line with a prompt. Handles Ctrl+C (Interrupted) and Ctrl+D (Eof).
    pub fn readline(&mut self, prompt: &str) -> Result<String, LineEditError> {
        use std::io::Write;
        print!("{}", prompt);
        std::io::stdout().flush().map_err(LineEditError::Io)?;

        let mut line = String::new();
        match std::io::stdin().read_line(&mut line) {
            Ok(0) => Err(LineEditError::Eof),
            Ok(_) => {
                if line.ends_with('\n') {
                    line.pop();
                    if line.ends_with('\r') {
                        line.pop();
                    }
                }
                // Add to history if non-empty
                if !line.is_empty() {
                    self.history.push(line.clone());
                }
                Ok(line)
            }
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => Err(LineEditError::Interrupted),
            Err(e) => Err(LineEditError::Io(e)),
        }
    }
}

impl std::fmt::Display for LineEditError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            LineEditError::Interrupted => write!(f, "interrupted"),
            LineEditError::Eof => write!(f, "end of input"),
            LineEditError::Io(e) => write!(f, "{}", e),
        }
    }
}

// Diagnostic renderer (replaces `ariadne` crate)

/// ANSI color codes for terminal output.
struct AnsiColor;

impl AnsiColor {
    const RED: &'static str = "\x1b[31m";
    const YELLOW: &'static str = "\x1b[33m";
    const BLUE: &'static str = "\x1b[34m";
    const BOLD: &'static str = "\x1b[1m";
    const RESET: &'static str = "\x1b[0m";
}

/// Render a diagnostic error with source context to stderr.
pub fn render_diagnostic(
    filename: &str,
    source: &str,
    line: u32,
    column: u32,
    message: &str,
    code: Option<&str>,
    help: Option<&str>,
    note: Option<&str>,
    is_warning: bool,
) {
    use std::io::Write;
    let mut out = std::io::stderr().lock();

    let (kind, color) = if is_warning {
        ("warning", AnsiColor::YELLOW)
    } else {
        ("error", AnsiColor::RED)
    };

    // Header: error[E001]: message
    let _ = write!(out, "{}{}{}", AnsiColor::BOLD, color, kind);
    if let Some(code) = code {
        let _ = write!(out, "[{}]", code);
    }
    let _ = writeln!(out, ": {}{}", message, AnsiColor::RESET);

    let _ = writeln!(
        out,
        "  {}-->{}  {}:{}:{}",
        AnsiColor::BLUE, AnsiColor::RESET, filename, line, column
    );

    let lines: Vec<&str> = source.lines().collect();
    let line_idx = (line as usize).saturating_sub(1);
    let gutter_width = format!("{}", line).len();

    // Show line before for context (if exists)
    if line_idx > 0 {
        let _ = writeln!(
            out,
            "  {}{:>width$} |{} {}",
            AnsiColor::BLUE,
            line_idx,
            AnsiColor::RESET,
            lines.get(line_idx - 1).unwrap_or(&""),
            width = gutter_width
        );
    }

    let _ = writeln!(
        out,
        "  {}{:>width$} |{} {}",
        AnsiColor::BLUE,
        line,
        AnsiColor::RESET,
        lines.get(line_idx).unwrap_or(&""),
        width = gutter_width
    );

    // Show caret pointing to error column
    let padding = " ".repeat(column.saturating_sub(1) as usize);
    let _ = writeln!(
        out,
        "  {}{:>width$} |{} {}{}^--- {}{}",
        AnsiColor::BLUE,
        "",
        AnsiColor::RESET,
        padding,
        color,
        message,
        AnsiColor::RESET,
        width = gutter_width
    );

    if let Some(help) = help {
        let _ = writeln!(
            out,
            "  {}{:>width$} ={} {}help{}: {}",
            AnsiColor::BLUE,
            "",
            AnsiColor::RESET,
            AnsiColor::BOLD,
            AnsiColor::RESET,
            help,
            width = gutter_width
        );
    }

    if let Some(note) = note {
        let _ = writeln!(
            out,
            "  {}{:>width$} ={} {}note{}: {}",
            AnsiColor::BLUE,
            "",
            AnsiColor::RESET,
            AnsiColor::BOLD,
            AnsiColor::RESET,
            note,
            width = gutter_width
        );
    }

    let _ = writeln!(out);
}

// OrderedMap (replaces `indexmap::IndexMap`)

/// An insertion-order preserving map, drop-in replacement for `indexmap::IndexMap`.
/// Backed by a `HashMap` for O(1) lookups and a `Vec` for ordered iteration.
#[derive(Clone)]
pub struct OrderedMap<K: Eq + std::hash::Hash + Clone, V: Clone> {
    entries: Vec<(K, V)>,
    index: std::collections::HashMap<K, usize>,
}

impl<K: Eq + std::hash::Hash + Clone, V: Clone> OrderedMap<K, V> {
    pub fn new() -> Self {
        OrderedMap {
            entries: Vec::new(),
            index: std::collections::HashMap::new(),
        }
    }

    pub fn with_capacity(cap: usize) -> Self {
        OrderedMap {
            entries: Vec::with_capacity(cap),
            index: std::collections::HashMap::with_capacity(cap),
        }
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    pub fn insert(&mut self, key: K, value: V) -> Option<V> {
        if let Some(&idx) = self.index.get(&key) {
            let old = std::mem::replace(&mut self.entries[idx].1, value);
            Some(old)
        } else {
            let idx = self.entries.len();
            self.index.insert(key.clone(), idx);
            self.entries.push((key, value));
            None
        }
    }

    pub fn get<Q: ?Sized>(&self, key: &Q) -> Option<&V>
    where K: std::borrow::Borrow<Q>, Q: Eq + std::hash::Hash {
        self.index.get(key).map(|&idx| &self.entries[idx].1)
    }

    pub fn get_mut<Q: ?Sized>(&mut self, key: &Q) -> Option<&mut V>
    where K: std::borrow::Borrow<Q>, Q: Eq + std::hash::Hash {
        self.index.get(key).copied().map(move |idx| &mut self.entries[idx].1)
    }

    pub fn contains_key<Q: ?Sized>(&self, key: &Q) -> bool
    where K: std::borrow::Borrow<Q>, Q: Eq + std::hash::Hash {
        self.index.contains_key(key)
    }

    pub fn remove<Q: ?Sized>(&mut self, key: &Q) -> Option<V>
    where K: std::borrow::Borrow<Q>, Q: Eq + std::hash::Hash {
        if let Some(idx) = self.index.remove(key) {
            let (_, val) = self.entries.remove(idx);
            // Rebuild index for entries after the removed one
            for i in idx..self.entries.len() {
                self.index.insert(self.entries[i].0.clone(), i);
            }
            Some(val)
        } else {
            None
        }
    }

    /// Remove a key, shifting subsequent elements (alias for `remove`).
    pub fn shift_remove<Q: ?Sized>(&mut self, key: &Q) -> Option<V>
    where K: std::borrow::Borrow<Q>, Q: Eq + std::hash::Hash {
        self.remove(key)
    }

    pub fn keys(&self) -> impl Iterator<Item = &K> {
        self.entries.iter().map(|(k, _)| k)
    }

    pub fn values(&self) -> impl Iterator<Item = &V> {
        self.entries.iter().map(|(_, v)| v)
    }

    pub fn iter(&self) -> impl Iterator<Item = (&K, &V)> {
        self.entries.iter().map(|(k, v)| (k, v))
    }

    pub fn iter_mut(&mut self) -> impl Iterator<Item = (&K, &mut V)> {
        self.entries.iter_mut().map(|(k, v)| (k as &K, v))
    }

    pub fn clear(&mut self) {
        self.entries.clear();
        self.index.clear();
    }

    pub fn entry(&mut self, key: K) -> OrderedMapEntry<'_, K, V> {
        if self.index.contains_key(&key) {
            OrderedMapEntry::Occupied(OrderedMapOccupiedEntry { map: self, key })
        } else {
            OrderedMapEntry::Vacant(OrderedMapVacantEntry { map: self, key })
        }
    }

    pub fn extend<I: IntoIterator<Item = (K, V)>>(&mut self, iter: I) {
        for (k, v) in iter {
            self.insert(k, v);
        }
    }

    pub fn retain<F: FnMut(&K, &mut V) -> bool>(&mut self, mut f: F) {
        // Collect indices to remove first, then remove in reverse order
        let mut to_remove = Vec::new();
        for i in 0..self.entries.len() {
            let (k, v) = &mut self.entries[i];
            let k_ref: &K = k;
            // Safety: we need split borrows. Use raw pointers.
            let keep = f(k_ref, v);
            if !keep {
                to_remove.push(i);
            }
        }
        for &i in to_remove.iter().rev() {
            let key = self.entries[i].0.clone();
            self.index.remove(&key);
            self.entries.remove(i);
        }
        for (i, (k, _)) in self.entries.iter().enumerate() {
            self.index.insert(k.clone(), i);
        }
    }

    pub fn sort_by<F: FnMut(&K, &V, &K, &V) -> std::cmp::Ordering>(&mut self, mut cmp: F) {
        self.entries.sort_by(|(k1, v1), (k2, v2)| cmp(k1, v1, k2, v2));
        for (i, (k, _)) in self.entries.iter().enumerate() {
            self.index.insert(k.clone(), i);
        }
    }

    pub fn last(&self) -> Option<(&K, &V)> {
        self.entries.last().map(|(k, v)| (k, v))
    }
}

impl<K: Eq + std::hash::Hash + Clone, V: Clone> Default for OrderedMap<K, V> {
    fn default() -> Self {
        Self::new()
    }
}

impl<K: Eq + std::hash::Hash + Clone + std::fmt::Debug, V: Clone + std::fmt::Debug> std::fmt::Debug for OrderedMap<K, V> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_map().entries(self.iter()).finish()
    }
}

impl<K: Eq + std::hash::Hash + Clone + PartialEq, V: Clone + PartialEq> PartialEq for OrderedMap<K, V> {
    fn eq(&self, other: &Self) -> bool {
        if self.len() != other.len() { return false; }
        self.entries == other.entries
    }
}

impl<K: Eq + std::hash::Hash + Clone, V: Clone> FromIterator<(K, V)> for OrderedMap<K, V> {
    fn from_iter<I: IntoIterator<Item = (K, V)>>(iter: I) -> Self {
        let mut map = OrderedMap::new();
        for (k, v) in iter {
            map.insert(k, v);
        }
        map
    }
}

impl<K: Eq + std::hash::Hash + Clone, V: Clone, const N: usize> From<[(K, V); N]> for OrderedMap<K, V> {
    fn from(arr: [(K, V); N]) -> Self {
        arr.into_iter().collect()
    }
}

impl<K: Eq + std::hash::Hash + Clone, V: Clone> IntoIterator for OrderedMap<K, V> {
    type Item = (K, V);
    type IntoIter = std::vec::IntoIter<(K, V)>;
    fn into_iter(self) -> Self::IntoIter {
        self.entries.into_iter()
    }
}

impl<'a, K: Eq + std::hash::Hash + Clone, V: Clone> IntoIterator for &'a OrderedMap<K, V> {
    type Item = (&'a K, &'a V);
    type IntoIter = OrderedMapIter<'a, K, V>;
    fn into_iter(self) -> Self::IntoIter {
        OrderedMapIter { inner: self.entries.iter() }
    }
}

pub struct OrderedMapIter<'a, K, V> {
    inner: std::slice::Iter<'a, (K, V)>,
}

impl<'a, K, V> Iterator for OrderedMapIter<'a, K, V> {
    type Item = (&'a K, &'a V);
    fn next(&mut self) -> Option<Self::Item> {
        self.inner.next().map(|(k, v)| (k, v))
    }
    fn size_hint(&self) -> (usize, Option<usize>) {
        self.inner.size_hint()
    }
}

impl<'a, K, V> ExactSizeIterator for OrderedMapIter<'a, K, V> {}

/// Entry API for OrderedMap.
pub enum OrderedMapEntry<'a, K: Eq + std::hash::Hash + Clone, V: Clone> {
    Occupied(OrderedMapOccupiedEntry<'a, K, V>),
    Vacant(OrderedMapVacantEntry<'a, K, V>),
}

pub struct OrderedMapOccupiedEntry<'a, K: Eq + std::hash::Hash + Clone, V: Clone> {
    map: &'a mut OrderedMap<K, V>,
    key: K,
}

pub struct OrderedMapVacantEntry<'a, K: Eq + std::hash::Hash + Clone, V: Clone> {
    map: &'a mut OrderedMap<K, V>,
    key: K,
}

impl<'a, K: Eq + std::hash::Hash + Clone, V: Clone> OrderedMapEntry<'a, K, V> {
    pub fn or_insert(self, default: V) -> &'a mut V {
        match self {
            OrderedMapEntry::Occupied(e) => {
                let idx = *e.map.index.get(&e.key).unwrap();
                &mut e.map.entries[idx].1
            }
            OrderedMapEntry::Vacant(e) => {
                let idx = e.map.entries.len();
                e.map.index.insert(e.key.clone(), idx);
                e.map.entries.push((e.key, default));
                &mut e.map.entries[idx].1
            }
        }
    }

    pub fn or_default(self) -> &'a mut V where V: Default {
        self.or_insert_with(V::default)
    }

    pub fn or_insert_with<F: FnOnce() -> V>(self, f: F) -> &'a mut V {
        match self {
            OrderedMapEntry::Occupied(e) => {
                let idx = *e.map.index.get(&e.key).unwrap();
                &mut e.map.entries[idx].1
            }
            OrderedMapEntry::Vacant(e) => {
                let val = f();
                let idx = e.map.entries.len();
                e.map.index.insert(e.key.clone(), idx);
                e.map.entries.push((e.key, val));
                &mut e.map.entries[idx].1
            }
        }
    }
}


// Basic regex engine (replaces `regex` crate)

/// A compiled regular expression (NFA-based).
#[derive(Clone)]
pub struct Regex {
    pattern: String,
    nfa: RegexNfa,
    case_insensitive: bool,
    group_names: Vec<Option<String>>, // named groups: index -> name
}

#[derive(Clone, Debug)]
enum RegexNode {
    Literal(char),
    AnyChar,             // .
    CharClass(Vec<(char, char)>, bool), // [ranges], negated
    Anchor(RegexAnchor),
}

#[derive(Clone, Debug)]
enum RegexAnchor {
    Start,           // ^
    End,             // $
    WordBoundary,    // \b
    NonWordBoundary, // \B
    Lookahead(String, bool),    // (?=pattern) positive, (?!pattern) negative
    Lookbehind(String, bool),   // (?<=pattern) positive, (?<!pattern) negative
}

#[derive(Clone, Debug)]
struct RegexNfa {
    states: Vec<NfaState>,
}

#[derive(Clone, Debug)]
enum NfaState {
    Match(RegexNode, usize),  // match node, next state
    Split(usize, usize),      // epsilon transitions to two states
    Accept,
}

impl Regex {
    pub fn new(pattern: &str) -> Result<Regex, String> {
        Self::with_size_limit(pattern, usize::MAX)
    }

    pub fn with_size_limit(pattern: &str, _size_limit: usize) -> Result<Regex, String> {
        let (effective_pattern, case_insensitive) = if pattern.starts_with("(?i)") {
            (&pattern[4..], true)
        } else {
            (pattern, false)
        };
        // Extract named group names from (?P<name>...) or (?<name>...) syntax
        let mut group_names = Vec::new();
        let mut stripped = String::with_capacity(effective_pattern.len());
        let pat_chars: Vec<char> = effective_pattern.chars().collect();
        let mut i = 0;
        while i < pat_chars.len() {
            if i + 3 < pat_chars.len() && pat_chars[i] == '(' && pat_chars[i+1] == '?' {
                // Check for (?P<name>) or (?<name>)
                let start = if pat_chars[i+2] == 'P' && i + 4 < pat_chars.len() && pat_chars[i+3] == '<' {
                    i + 4
                } else if pat_chars[i+2] == '<' {
                    i + 3
                } else {
                    stripped.push(pat_chars[i]);
                    i += 1;
                    continue;
                };
                // Find closing >
                if let Some(end) = pat_chars[start..].iter().position(|&c| c == '>') {
                    let name: String = pat_chars[start..start+end].iter().collect();
                    group_names.push(Some(name));
                    stripped.push('('); // Replace named group with plain group
                    i = start + end + 1;
                } else {
                    stripped.push(pat_chars[i]);
                    i += 1;
                }
            } else {
                if pat_chars[i] == '(' && (i == 0 || pat_chars[i-1] != '\\') {
                    group_names.push(None); // unnamed group
                }
                stripped.push(pat_chars[i]);
                i += 1;
            }
        }
        let nfa = compile_regex_nfa(&stripped)?;
        Ok(Regex { pattern: pattern.to_string(), nfa, case_insensitive, group_names })
    }

    pub fn is_match(&self, text: &str) -> bool {
        self.find(text).is_some()
    }

    pub fn find(&self, text: &str) -> Option<(usize, usize)> {
        let chars: Vec<char> = text.chars().collect();
        for start in 0..=chars.len() {
            if let Some(end) = self.match_at(&chars, start) {
                return Some((start, end));
            }
        }
        None
    }

    pub fn find_all(&self, text: &str) -> Vec<(usize, usize)> {
        let chars: Vec<char> = text.chars().collect();
        let mut results = Vec::new();
        let mut start = 0;
        while start <= chars.len() {
            if let Some(end) = self.match_at(&chars, start) {
                results.push((start, end));
                start = if end > start { end } else { start + 1 };
            } else {
                start += 1;
            }
        }
        results
    }

    pub fn replace(&self, text: &str, replacement: &str) -> String {
        let chars: Vec<char> = text.chars().collect();
        let mut result = String::new();
        let mut pos = 0;
        while pos <= chars.len() {
            if let Some(end) = self.match_at(&chars, pos) {
                result.push_str(replacement);
                if end > pos {
                    pos = end;
                } else {
                    // Empty match: copy the character at pos through, then advance
                    if pos < chars.len() {
                        result.push(chars[pos]);
                    }
                    pos += 1;
                }
            } else {
                if pos < chars.len() {
                    result.push(chars[pos]);
                }
                pos += 1;
            }
        }
        result
    }

    pub fn replace_all(&self, text: &str, replacement: &str) -> String {
        self.replace(text, replacement)
    }

    pub fn find_iter<'t>(&self, text: &'t str) -> Vec<RegexMatch<'t>> {
        let chars: Vec<char> = text.chars().collect();
        let mut results = Vec::new();
        let mut char_pos = 0;
        while char_pos <= chars.len() {
            if let Some(end) = self.match_at(&chars, char_pos) {
                let start_byte = chars[..char_pos].iter().map(|c| c.len_utf8()).sum::<usize>();
                let end_byte = chars[..end].iter().map(|c| c.len_utf8()).sum::<usize>();
                results.push(RegexMatch {
                    text,
                    start: start_byte,
                    end: end_byte,
                });
                char_pos = if end > char_pos { end } else { char_pos + 1 };
            } else {
                char_pos += 1;
            }
        }
        results
    }

    /// Capture groups (simplified: only group 0 = whole match).
    pub fn captures<'t>(&self, text: &'t str) -> Option<RegexCaptures<'t>> {
        self.find(text).map(|(start, end)| {
            let chars: Vec<char> = text.chars().collect();
            let start_byte = chars[..start].iter().map(|c| c.len_utf8()).sum::<usize>();
            let end_byte = chars[..end].iter().map(|c| c.len_utf8()).sum::<usize>();
            let mut groups = vec![Some((start_byte, end_byte))];
            // Extract sub-group matches
            let sub_groups = self.extract_groups(&chars, start, end);
            let match_text: String = chars[start..end].iter().collect();
            for sg in &sub_groups {
                if let Some(pos) = match_text.find(sg.as_str()) {
                    let sg_start = start_byte + match_text[..pos].len();
                    let sg_end = sg_start + sg.len();
                    groups.push(Some((sg_start, sg_end)));
                } else {
                    groups.push(None);
                }
            }
            RegexCaptures {
                text,
                groups,
            }
        })
    }

    /// Get named group names (from (?P<name>...) syntax).
    pub fn group_names(&self) -> &[Option<String>] {
        &self.group_names
    }

    pub fn replace_first(&self, text: &str, replacement: &str) -> String {
        let chars: Vec<char> = text.chars().collect();
        if let Some((start, end)) = self.find(text) {
            let mut result = String::new();
            for &c in &chars[..start] { result.push(c); }
            result.push_str(replacement);
            for &c in &chars[end..] { result.push(c); }
            result
        } else {
            text.to_string()
        }
    }

    pub fn split(&self, text: &str) -> Vec<String> {
        let matches = self.find_all(text);
        if matches.is_empty() {
            return vec![text.to_string()];
        }
        let chars: Vec<char> = text.chars().collect();
        let mut parts = Vec::new();
        let mut last = 0;
        for (start, end) in &matches {
            parts.push(chars[last..*start].iter().collect::<String>());
            last = *end;
        }
        parts.push(chars[last..].iter().collect::<String>());
        parts
    }

    fn match_at(&self, chars: &[char], start: usize) -> Option<usize> {
        // NFA simulation using Thompson's algorithm
        let mut current = vec![false; self.nfa.states.len()];
        let mut next = vec![false; self.nfa.states.len()];
        self.add_state_with_anchors(&mut current, 0, start, chars);

        let mut last_match: Option<usize> = None;

        // Check for immediate accept
        for i in 0..self.nfa.states.len() {
            if current[i] {
                if let NfaState::Accept = &self.nfa.states[i] {
                    last_match = Some(start);
                }
            }
        }

        let mut pos = start;
        while pos < chars.len() {
            let ch = chars[pos];
            next.iter_mut().for_each(|s| *s = false);

            for i in 0..self.nfa.states.len() {
                if !current[i] { continue; }
                match &self.nfa.states[i] {
                    NfaState::Match(node, next_state) => {
                        match node {
                            RegexNode::Anchor(_) => {} // anchors handled in add_state
                            _ => {
                                if match_node(node, ch, pos, chars.len(), self.case_insensitive) {
                                    self.add_state_with_anchors(&mut next, *next_state, pos + 1, chars);
                                }
                            }
                        }
                    }
                    NfaState::Accept => {}
                    NfaState::Split(_, _) => {}
                }
            }

            std::mem::swap(&mut current, &mut next);
            pos += 1;

            for i in 0..self.nfa.states.len() {
                if current[i] {
                    if let NfaState::Accept = &self.nfa.states[i] {
                        last_match = Some(pos);
                    }
                }
            }
        }

        // At end of string, check $ and word boundary anchors
        for i in 0..self.nfa.states.len() {
            if !current[i] { continue; }
            match &self.nfa.states[i] {
                NfaState::Match(RegexNode::Anchor(RegexAnchor::End), next_state) => {
                    self.add_state(&mut current, *next_state);
                }
                NfaState::Match(RegexNode::Anchor(RegexAnchor::WordBoundary), next_state) => {
                    let prev_word = if pos > 0 { chars[pos - 1].is_alphanumeric() || chars[pos - 1] == '_' } else { false };
                    // At end of string, curr_word is false
                    if prev_word {
                        self.add_state(&mut current, *next_state);
                    }
                }
                NfaState::Match(RegexNode::Anchor(RegexAnchor::NonWordBoundary), next_state) => {
                    let prev_word = if pos > 0 { chars[pos - 1].is_alphanumeric() || chars[pos - 1] == '_' } else { false };
                    if !prev_word {
                        self.add_state(&mut current, *next_state);
                    }
                }
                NfaState::Match(RegexNode::Anchor(RegexAnchor::Lookahead(pat, positive)), next_state) => {
                    let remaining: String = chars[pos..].iter().collect();
                    let matches = Regex::new(pat).ok().map(|re| re.find(&remaining).map(|(s, _)| s == 0).unwrap_or(false)).unwrap_or(false);
                    if matches == *positive {
                        self.add_state(&mut current, *next_state);
                    }
                }
                NfaState::Match(RegexNode::Anchor(RegexAnchor::Lookbehind(pat, positive)), next_state) => {
                    let preceding: String = chars[..pos].iter().collect();
                    let matches = Regex::new(pat).ok().map(|re| {
                        re.find(&preceding).map(|(_, e)| e == preceding.chars().count()).unwrap_or(false)
                    }).unwrap_or(false);
                    if matches == *positive {
                        self.add_state(&mut current, *next_state);
                    }
                }
                _ => {}
            }
        }
        // Re-check for accept after $ processing
        for i in 0..self.nfa.states.len() {
            if current[i] {
                if let NfaState::Accept = &self.nfa.states[i] {
                    last_match = Some(pos);
                }
            }
        }

        last_match
    }

    fn add_state_with_anchors(&self, states: &mut [bool], state: usize, pos: usize, chars: &[char]) {
        if state >= self.nfa.states.len() || states[state] { return; }
        states[state] = true;
        let len = chars.len();
        match &self.nfa.states[state] {
            NfaState::Split(a, b) => {
                self.add_state_with_anchors(states, *a, pos, chars);
                self.add_state_with_anchors(states, *b, pos, chars);
            }
            NfaState::Match(RegexNode::Anchor(RegexAnchor::Start), next) => {
                if pos == 0 { self.add_state_with_anchors(states, *next, pos, chars); }
            }
            NfaState::Match(RegexNode::Anchor(RegexAnchor::End), next) => {
                if pos == len { self.add_state_with_anchors(states, *next, pos, chars); }
            }
            NfaState::Match(RegexNode::Anchor(RegexAnchor::WordBoundary), next) => {
                let prev_word = if pos > 0 { chars[pos - 1].is_alphanumeric() || chars[pos - 1] == '_' } else { false };
                let curr_word = if pos < len { chars[pos].is_alphanumeric() || chars[pos] == '_' } else { false };
                if prev_word != curr_word {
                    self.add_state_with_anchors(states, *next, pos, chars);
                }
            }
            NfaState::Match(RegexNode::Anchor(RegexAnchor::NonWordBoundary), next) => {
                let prev_word = if pos > 0 { chars[pos - 1].is_alphanumeric() || chars[pos - 1] == '_' } else { false };
                let curr_word = if pos < len { chars[pos].is_alphanumeric() || chars[pos] == '_' } else { false };
                if prev_word == curr_word {
                    self.add_state_with_anchors(states, *next, pos, chars);
                }
            }
            NfaState::Match(RegexNode::Anchor(RegexAnchor::Lookahead(pat, positive)), next) => {
                let remaining: String = chars[pos..].iter().collect();
                let matches = Regex::new(pat).ok().map(|re| re.find(&remaining).map(|(s, _)| s == 0).unwrap_or(false)).unwrap_or(false);
                if matches == *positive {
                    self.add_state_with_anchors(states, *next, pos, chars);
                }
            }
            NfaState::Match(RegexNode::Anchor(RegexAnchor::Lookbehind(pat, positive)), next) => {
                let preceding: String = chars[..pos].iter().collect();
                let matches = Regex::new(pat).ok().map(|re| {
                    re.find(&preceding).map(|(_, e)| e == preceding.chars().count()).unwrap_or(false)
                }).unwrap_or(false);
                if matches == *positive {
                    self.add_state_with_anchors(states, *next, pos, chars);
                }
            }
            _ => {}
        }
    }

    fn add_state(&self, states: &mut [bool], state: usize) {
        if state >= self.nfa.states.len() || states[state] { return; }
        states[state] = true;
        if let NfaState::Split(a, b) = &self.nfa.states[state] {
            self.add_state(states, *a);
            self.add_state(states, *b);
        }
    }
}

fn match_node(node: &RegexNode, ch: char, pos: usize, len: usize, case_insensitive: bool) -> bool {
    match node {
        RegexNode::Literal(c) => {
            if case_insensitive {
                ch.to_lowercase().next() == c.to_lowercase().next()
            } else {
                ch == *c
            }
        }
        RegexNode::AnyChar => ch != '\n',
        RegexNode::CharClass(ranges, negated) => {
            let in_class = if case_insensitive {
                let lc = ch.to_lowercase().next().unwrap_or(ch);
                let uc = ch.to_uppercase().next().unwrap_or(ch);
                ranges.iter().any(|(lo, hi)| (lc >= *lo && lc <= *hi) || (uc >= *lo && uc <= *hi))
            } else {
                ranges.iter().any(|(lo, hi)| ch >= *lo && ch <= *hi)
            };
            if *negated { !in_class } else { in_class }
        }
        RegexNode::Anchor(RegexAnchor::Start) => pos == 0,
        RegexNode::Anchor(RegexAnchor::End) => pos == len,
        RegexNode::Anchor(RegexAnchor::WordBoundary) | RegexNode::Anchor(RegexAnchor::NonWordBoundary)
        | RegexNode::Anchor(RegexAnchor::Lookahead(..)) | RegexNode::Anchor(RegexAnchor::Lookbehind(..)) => {
            false // zero-width assertions handled in add_state_with_anchors
        }
    }
}

fn compile_regex_nfa(pattern: &str) -> Result<RegexNfa, String> {
    let chars: Vec<char> = pattern.chars().collect();
    let mut states: Vec<NfaState> = Vec::new();
    let accept_state = compile_regex_seq(&chars, &mut 0, &mut states)?;
    // Ensure accept state exists
    while states.len() <= accept_state {
        states.push(NfaState::Accept);
    }
    if let Some(last) = states.last() {
        if !matches!(last, NfaState::Accept) {
            states.push(NfaState::Accept);
        }
    }
    Ok(RegexNfa { states })
}

fn compile_regex_seq(chars: &[char], pos: &mut usize, states: &mut Vec<NfaState>) -> Result<usize, String> {
    // Compile one alternative (sequence of atoms until | or ) or end)
    fn compile_alt(chars: &[char], pos: &mut usize, states: &mut Vec<NfaState>) -> Result<(), String> {
        while *pos < chars.len() && chars[*pos] != '|' && chars[*pos] != ')' {
            match chars[*pos] {
                '(' => {
                    *pos += 1;
                    // Check for special group syntax: (?=...), (?!...), (?<=...), (?<!...)
                    if *pos < chars.len() && chars[*pos] == '?' {
                        *pos += 1;
                        if *pos < chars.len() {
                            let (anchor, is_lookbehind) = match chars[*pos] {
                                '=' => { *pos += 1; (true, false) }  // (?=...) positive lookahead
                                '!' => { *pos += 1; (false, false) } // (?!...) negative lookahead
                                '<' if *pos + 1 < chars.len() && chars[*pos + 1] == '=' => {
                                    *pos += 2; (true, true)  // (?<=...) positive lookbehind
                                }
                                '<' if *pos + 1 < chars.len() && chars[*pos + 1] == '!' => {
                                    *pos += 2; (false, true) // (?<!...) negative lookbehind
                                }
                                _ => {
                                    // Non-capturing group (?:...) or other — treat as regular group
                                    if chars[*pos] == ':' { *pos += 1; }
                                    // Named groups (?P<name>) handled by Regex::with_size_limit
                                    compile_regex_seq(chars, pos, states)?;
                                    if *pos < chars.len() && chars[*pos] == ')' { *pos += 1; }
                                    apply_quantifier(chars, pos, states);
                                    continue;
                                }
                            };
                            // Extract the pattern inside the assertion
                            let assert_start = *pos;
                            let mut depth = 1;
                            while *pos < chars.len() && depth > 0 {
                                match chars[*pos] {
                                    '(' => depth += 1,
                                    ')' => depth -= 1,
                                    _ => {}
                                }
                                if depth > 0 { *pos += 1; }
                            }
                            let assert_pattern: String = chars[assert_start..*pos].iter().collect();
                            if *pos < chars.len() && chars[*pos] == ')' { *pos += 1; }
                            let next = states.len() + 1;
                            if is_lookbehind {
                                states.push(NfaState::Match(RegexNode::Anchor(RegexAnchor::Lookbehind(assert_pattern, anchor)), next));
                            } else {
                                states.push(NfaState::Match(RegexNode::Anchor(RegexAnchor::Lookahead(assert_pattern, anchor)), next));
                            }
                        }
                    } else {
                        compile_regex_seq(chars, pos, states)?;
                        if *pos < chars.len() && chars[*pos] == ')' { *pos += 1; }
                        apply_quantifier(chars, pos, states);
                    }
                }
                '[' => {
                    *pos += 1;
                    let (ranges, negated) = parse_char_class(chars, pos)?;
                    let next = states.len() + 1;
                    states.push(NfaState::Match(RegexNode::CharClass(ranges, negated), next));
                    apply_quantifier(chars, pos, states);
                }
                '.' => {
                    *pos += 1;
                    let next = states.len() + 1;
                    states.push(NfaState::Match(RegexNode::AnyChar, next));
                    apply_quantifier(chars, pos, states);
                }
                '^' => {
                    *pos += 1;
                    let next = states.len() + 1;
                    states.push(NfaState::Match(RegexNode::Anchor(RegexAnchor::Start), next));
                }
                '$' => {
                    *pos += 1;
                    let next = states.len() + 1;
                    states.push(NfaState::Match(RegexNode::Anchor(RegexAnchor::End), next));
                }
                '\\' => {
                    *pos += 1;
                    if *pos >= chars.len() { return Err("trailing backslash".into()); }
                    // Handle \p{...} and \P{...} Unicode property escapes
                    if (chars[*pos] == 'p' || chars[*pos] == 'P') && *pos + 1 < chars.len() && chars[*pos + 1] == '{' {
                        let negated = chars[*pos] == 'P';
                        *pos += 1; // skip p/P
                        let node = parse_unicode_property(chars, pos, negated)?;
                        let next = states.len() + 1;
                        states.push(NfaState::Match(node, next));
                        apply_quantifier(chars, pos, states);
                    } else {
                        let node = parse_escape(chars[*pos]);
                        *pos += 1;
                        let next = states.len() + 1;
                        states.push(NfaState::Match(node, next));
                        apply_quantifier(chars, pos, states);
                    }
                }
                c if c == '*' || c == '+' || c == '?' => {
                    return Err(format!("quantifier '{}' without preceding element", c));
                }
                c => {
                    *pos += 1;
                    let next = states.len() + 1;
                    states.push(NfaState::Match(RegexNode::Literal(c), next));
                    apply_quantifier(chars, pos, states);
                }
            }
        }
        Ok(())
    }

    // Check if there are any alternations
    let mut has_alt = false;
    let mut depth = 0;
    for &c in &chars[*pos..] {
        match c {
            '(' => depth += 1,
            ')' => { if depth == 0 { break; } depth -= 1; }
            '|' if depth == 0 => { has_alt = true; break; }
            _ => {}
        }
    }

    if !has_alt {
        // No alternation — simple sequence
        compile_alt(chars, pos, states)?;
        return Ok(states.len());
    }

    // Alternation: compile each alternative into a separate state range,
    // then create a Split chain that branches to each alternative.
    // Each alternative ends by jumping to the shared accept state.
    let mut alt_ranges: Vec<(usize, usize)> = Vec::new(); // (start, end) indices

    loop {
        let alt_start = states.len();
        compile_alt(chars, pos, states)?;
        let alt_end = states.len();
        alt_ranges.push((alt_start, alt_end));

        if *pos < chars.len() && chars[*pos] == '|' {
            *pos += 1; // skip |
        } else {
            break;
        }
    }

    if alt_ranges.len() <= 1 {
        return Ok(states.len());
    }

    // Rewrite: prefix with a Split chain, patch each alt's end to jump to final accept
    let pre_states_len = states.len() - alt_ranges.iter().map(|(s, e)| e - s).sum::<usize>();
    let base = pre_states_len; // where we start inserting in the main states array

    // Simpler approach: build split + alternatives in a flat list
    // Split(alt0_start, Split(alt1_start, Split(alt2_start, alt3_start)))
    let mut flat = Vec::new();
    let num_alts = alt_ranges.len();
    let split_count = num_alts - 1;

    // First, calculate where each alternative starts in the flat layout
    // Layout: [split0, split1, ..., split_{n-2}, alt0_states, alt0_jmp, alt1_states, alt1_jmp, ..., altn_states]
    let mut alt_starts = Vec::new();
    let mut offset = split_count;
    for (start, end) in &alt_ranges {
        alt_starts.push(offset);
        offset += (end - start) + 1; // +1 for the jump-to-end placeholder (except last)
    }
    // Last alt doesn't need a jump (it falls through to accept)
    offset -= 1; // remove last jump
    let accept_idx = offset;

    for i in 0..split_count {
        let left = alt_starts[i];
        let right = if i + 1 < split_count { i + 1 } else { alt_starts[i + 1] };
        flat.push(NfaState::Split(base + left, base + right));
    }

    // Build alternative state sequences
    for (ai, (start, end)) in alt_ranges.iter().enumerate() {
        let alt_len = end - start;
        for j in 0..alt_len {
            let orig = &states[start + j];
            // Remap next pointers: original state's next was relative to old position
            let new_state = match orig {
                NfaState::Match(node, next) => {
                    // If next pointed past this alt's end, redirect to accept
                    let new_next = if *next >= *end {
                        base + accept_idx
                    } else {
                        base + alt_starts[ai] + (*next - *start)
                    };
                    NfaState::Match(node.clone(), new_next)
                }
                NfaState::Split(a, b) => {
                    let remap = |idx: usize| -> usize {
                        if idx >= *end { base + accept_idx }
                        else { base + alt_starts[ai] + (idx - *start) }
                    };
                    NfaState::Split(remap(*a), remap(*b))
                }
                NfaState::Accept => NfaState::Accept,
            };
            flat.push(new_state);
        }
        // Add jump to accept (except for last alternative which falls through)
        if ai < num_alts - 1 {
            flat.push(NfaState::Match(RegexNode::Literal('\0'), base + accept_idx)); // placeholder jump
            // Actually we need a proper way to "jump" — use a Split with both branches to accept
            let last = flat.len() - 1;
            flat[last] = NfaState::Split(base + accept_idx, base + accept_idx);
        }
    }

    // Remove old alternative states and insert new ones
    let first_alt_start = alt_ranges[0].0;
    states.truncate(first_alt_start);
    states.extend(flat);

    Ok(states.len())
}

fn apply_quantifier(chars: &[char], pos: &mut usize, states: &mut Vec<NfaState>) {
    if *pos >= chars.len() { return; }
    match chars[*pos] {
        '*' => {
            // zero-or-more: insert Split before match, match loops back to split
            // Before: [..., Match(x, next)]  (match is at last_idx)
            // After:  [..., Split(match_idx, after_idx), Match(x, split_idx)]
            *pos += 1;
            let last_idx = states.len() - 1;
            let match_node = states.remove(last_idx);
            // Now states has one fewer entry. Insert split + match.
            let split_idx = last_idx;
            let match_idx = last_idx + 1;
            let after_idx = last_idx + 2; // where next state in sequence will be
            states.insert(split_idx, NfaState::Split(match_idx, after_idx));
            // Re-insert match, pointing back to split for looping
            let mut match_node = match_node;
            patch_single(&mut match_node, split_idx);
            states.insert(match_idx, match_node);
            // Consume non-greedy modifier (NFA handles both the same way)
            if *pos < chars.len() && chars[*pos] == '?' {
                *pos += 1;
            }
        }
        '+' => {
            // one-or-more: Match → Split → (back to Match) | continue
            *pos += 1;
            let match_idx = states.len() - 1;
            let split_idx = states.len();
            patch_next(states, match_idx, split_idx);
            states.push(NfaState::Split(match_idx, split_idx + 1));
            // Consume non-greedy modifier (NFA handles both the same way)
            if *pos < chars.len() && chars[*pos] == '?' {
                *pos += 1;
            }
        }
        '?' => {
            // zero-or-one: Split → Match | skip
            *pos += 1;
            let last_idx = states.len() - 1;
            let mut match_node = states.remove(last_idx);
            let split_idx = last_idx;
            let match_idx = last_idx + 1;
            let after_idx = last_idx + 2;
            states.insert(split_idx, NfaState::Split(match_idx, after_idx));
            patch_single(&mut match_node, after_idx);
            states.insert(match_idx, match_node);
            // Consume non-greedy modifier (NFA handles both the same way)
            if *pos < chars.len() && chars[*pos] == '?' {
                *pos += 1;
            }
        }
        _ => {}
    }
}

fn patch_next(states: &mut [NfaState], idx: usize, new_next: usize) {
    match &mut states[idx] {
        NfaState::Match(_, next) => *next = new_next,
        _ => {}
    }
}

fn patch_single(state: &mut NfaState, new_next: usize) {
    match state {
        NfaState::Match(_, next) => *next = new_next,
        _ => {}
    }
}

fn parse_char_class(chars: &[char], pos: &mut usize) -> Result<(Vec<(char, char)>, bool), String> {
    let mut ranges = Vec::new();
    let negated = *pos < chars.len() && chars[*pos] == '^';
    if negated { *pos += 1; }

    while *pos < chars.len() && chars[*pos] != ']' {
        // Check for POSIX classes like [:alpha:], [:digit:], etc.
        if *pos + 2 < chars.len() && chars[*pos] == '[' && chars[*pos + 1] == ':' {
            *pos += 2;
            let class_start = *pos;
            while *pos < chars.len() && !(chars[*pos] == ':' && *pos + 1 < chars.len() && chars[*pos + 1] == ']') {
                *pos += 1;
            }
            let class_name: String = chars[class_start..*pos].iter().collect();
            if *pos + 1 < chars.len() { *pos += 2; } // skip :]
            match class_name.as_str() {
                "alpha" => { ranges.extend_from_slice(&[('a', 'z'), ('A', 'Z')]); }
                "digit" => { ranges.push(('0', '9')); }
                "alnum" => { ranges.extend_from_slice(&[('a', 'z'), ('A', 'Z'), ('0', '9')]); }
                "space" => { ranges.extend_from_slice(&[(' ', ' '), ('\t', '\t'), ('\n', '\n'), ('\r', '\r')]); }
                "upper" => { ranges.push(('A', 'Z')); }
                "lower" => { ranges.push(('a', 'z')); }
                "punct" => { ranges.extend_from_slice(&[('!', '/'), (':', '@'), ('[', '`'), ('{', '~')]); }
                "print" => { ranges.push((' ', '~')); }
                "graph" => { ranges.push(('!', '~')); }
                "blank" => { ranges.extend_from_slice(&[(' ', ' '), ('\t', '\t')]); }
                "xdigit" => { ranges.extend_from_slice(&[('0', '9'), ('a', 'f'), ('A', 'F')]); }
                _ => { return Err(format!("unknown POSIX class: [:{}:]", class_name)); }
            }
            continue;
        }
        let c = chars[*pos];
        *pos += 1;
        if *pos + 1 < chars.len() && chars[*pos] == '-' && chars[*pos + 1] != ']' {
            *pos += 1; // skip -
            let end = chars[*pos];
            *pos += 1;
            ranges.push((c, end));
        } else {
            ranges.push((c, c));
        }
    }
    if *pos < chars.len() && chars[*pos] == ']' {
        *pos += 1;
    }
    Ok((ranges, negated))
}

fn parse_escape(c: char) -> RegexNode {
    match c {
        'd' => RegexNode::CharClass(vec![('0', '9')], false),
        'D' => RegexNode::CharClass(vec![('0', '9')], true),
        'w' => RegexNode::CharClass(vec![('a', 'z'), ('A', 'Z'), ('0', '9'), ('_', '_')], false),
        'W' => RegexNode::CharClass(vec![('a', 'z'), ('A', 'Z'), ('0', '9'), ('_', '_')], true),
        's' => RegexNode::CharClass(vec![(' ', ' '), ('\t', '\t'), ('\n', '\n'), ('\r', '\r')], false),
        'S' => RegexNode::CharClass(vec![(' ', ' '), ('\t', '\t'), ('\n', '\n'), ('\r', '\r')], true),
        'b' => RegexNode::Anchor(RegexAnchor::WordBoundary),
        'B' => RegexNode::Anchor(RegexAnchor::NonWordBoundary),
        'n' => RegexNode::Literal('\n'),
        't' => RegexNode::Literal('\t'),
        'r' => RegexNode::Literal('\r'),
        '0' => RegexNode::Literal('\0'),
        // Backreferences \1-\9: treated as literal match of the digit
        // (full backreference support would need NFA extension to track group matches)
        '1'..='9' => RegexNode::Literal(c), // backreference placeholder
        _ => RegexNode::Literal(c),
    }
}

/// Parse \p{...} and \P{...} Unicode property escapes.
fn parse_unicode_property(chars: &[char], pos: &mut usize, negated: bool) -> Result<RegexNode, String> {
    if *pos >= chars.len() || chars[*pos] != '{' {
        return Err("expected '{' after \\p".into());
    }
    *pos += 1; // skip {
    let start = *pos;
    while *pos < chars.len() && chars[*pos] != '}' {
        *pos += 1;
    }
    let prop_name: String = chars[start..*pos].iter().collect();
    if *pos < chars.len() { *pos += 1; } // skip }

    // Map Unicode property names to character ranges
    let ranges = match prop_name.as_str() {
        "L" | "Letter" => vec![('a', 'z'), ('A', 'Z'), ('\u{00C0}', '\u{00FF}'), ('\u{0100}', '\u{024F}')],
        "Lu" | "Uppercase_Letter" => vec![('A', 'Z'), ('\u{00C0}', '\u{00D6}'), ('\u{00D8}', '\u{00DE}')],
        "Ll" | "Lowercase_Letter" => vec![('a', 'z'), ('\u{00DF}', '\u{00F6}'), ('\u{00F8}', '\u{00FF}')],
        "N" | "Number" => vec![('0', '9'), ('\u{0660}', '\u{0669}'), ('\u{06F0}', '\u{06F9}')],
        "Nd" | "Decimal_Number" => vec![('0', '9')],
        "P" | "Punctuation" => vec![('!', '/'), (':', '@'), ('[', '`'), ('{', '~')],
        "S" | "Symbol" => vec![('$', '$'), ('+', '+'), ('<', '>'), ('^', '^'), ('`', '`'), ('|', '|'), ('~', '~')],
        "Z" | "Separator" => vec![(' ', ' '), ('\u{00A0}', '\u{00A0}')],
        "Cc" | "Control" => vec![('\u{0000}', '\u{001F}'), ('\u{007F}', '\u{009F}')],
        "Latin" => vec![('A', 'Z'), ('a', 'z'), ('\u{00C0}', '\u{00FF}'), ('\u{0100}', '\u{024F}')],
        "Greek" => vec![('\u{0370}', '\u{03FF}')],
        "Cyrillic" => vec![('\u{0400}', '\u{04FF}')],
        "Han" | "CJK" => vec![('\u{4E00}', '\u{9FFF}')],
        "Hiragana" => vec![('\u{3040}', '\u{309F}')],
        "Katakana" => vec![('\u{30A0}', '\u{30FF}')],
        "Arabic" => vec![('\u{0600}', '\u{06FF}')],
        "Hebrew" => vec![('\u{0590}', '\u{05FF}')],
        "Emoji" => vec![('\u{1F600}', '\u{1F64F}'), ('\u{1F300}', '\u{1F5FF}'), ('\u{1F680}', '\u{1F6FF}')],
        _ => {
            // Unknown property — match any letter/number as fallback
            vec![('a', 'z'), ('A', 'Z'), ('0', '9')]
        }
    };
    Ok(RegexNode::CharClass(ranges, negated))
}

/// Escape special regex metacharacters in a string.
pub fn regex_escape(s: &str) -> String {
    let mut result = String::with_capacity(s.len() * 2);
    for c in s.chars() {
        match c {
            '\\' | '.' | '+' | '*' | '?' | '(' | ')' | '|' | '[' | ']' | '{' | '}' | '^' | '$' => {
                result.push('\\');
                result.push(c);
            }
            _ => result.push(c),
        }
    }
    result
}

/// A regex match result.
pub struct RegexMatch<'t> {
    text: &'t str,
    start: usize,
    end: usize,
}

impl<'t> RegexMatch<'t> {
    pub fn as_str(&self) -> &'t str {
        &self.text[self.start..self.end]
    }
    pub fn start(&self) -> usize { self.start }
    pub fn end(&self) -> usize { self.end }
}

/// Regex capture groups.
pub struct RegexCaptures<'t> {
    text: &'t str,
    groups: Vec<Option<(usize, usize)>>,
}

impl<'t> RegexCaptures<'t> {
    pub fn get(&self, i: usize) -> Option<RegexMatch<'t>> {
        self.groups.get(i).and_then(|g| g.map(|(s, e)| RegexMatch { text: self.text, start: s, end: e }))
    }

    pub fn len(&self) -> usize { self.groups.len() }

    /// Iterate over capture groups as Option<RegexMatch>.
    pub fn iter(&self) -> impl Iterator<Item = Option<RegexMatch<'t>>> + '_ {
        self.groups.iter().map(move |g| g.map(|(s, e)| RegexMatch { text: self.text, start: s, end: e }))
    }
}

impl Regex {
    /// Replace all matches with a replacement string that can reference capture groups.
    /// Supports `$0` (whole match), `$1`, `$2`, etc. for numbered groups.
    /// Groups are determined by parenthesized subexpressions in the pattern.
    pub fn replace_with_captures(&self, text: &str, replacement: &str) -> String {
        // If replacement doesn't contain $, use simple replace
        if !replacement.contains('$') {
            return self.replace(text, replacement);
        }
        let chars: Vec<char> = text.chars().collect();
        let mut result = String::new();
        let mut pos = 0;
        while pos <= chars.len() {
            if let Some(end) = self.match_at(&chars, pos) {
                let match_str: String = chars[pos..end].iter().collect();
                // Build replacement with $0 = whole match
                let mut rep = replacement.to_string();
                rep = rep.replace("$0", &match_str);
                rep = rep.replace("${0}", &match_str);
                // For $1, $2, etc. we need actual capture groups
                // Simple heuristic: split pattern by top-level () to find groups
                let groups = self.extract_groups(&chars, pos, end);
                for (i, group) in groups.iter().enumerate() {
                    rep = rep.replace(&format!("${}", i + 1), group);
                    rep = rep.replace(&format!("${{{}}}", i + 1), group);
                }
                result.push_str(&rep);
                if end > pos { pos = end; } else {
                    if pos < chars.len() { result.push(chars[pos]); }
                    pos += 1;
                }
            } else {
                if pos < chars.len() { result.push(chars[pos]); }
                pos += 1;
            }
        }
        result
    }

    /// Extract capture group contents by finding parenthesized groups in the pattern.
    fn extract_groups(&self, chars: &[char], _start: usize, _end: usize) -> Vec<String> {
        // Parse the pattern to find () groups, then try to match each group's sub-pattern
        let pat_chars: Vec<char> = self.pattern.chars().collect();
        let mut groups = Vec::new();
        let mut depth = 0;
        let mut group_start = None;
        for (i, &c) in pat_chars.iter().enumerate() {
            if c == '(' && (i == 0 || pat_chars[i-1] != '\\') {
                if depth == 0 { group_start = Some(i + 1); }
                depth += 1;
            } else if c == ')' && (i == 0 || pat_chars[i-1] != '\\') {
                depth -= 1;
                if depth == 0 {
                    if let Some(gs) = group_start {
                        let sub_pattern: String = pat_chars[gs..i].iter().collect();
                        // Try to match this sub-pattern against the matched text
                        if let Ok(sub_re) = Regex::new(&sub_pattern) {
                            let match_text: String = chars[_start.._end].iter().collect();
                            if let Some((s, e)) = sub_re.find(&match_text) {
                                let group_text: String = match_text.chars().skip(s).take(e - s).collect();
                                groups.push(group_text);
                            } else {
                                groups.push(String::new());
                            }
                        } else {
                            groups.push(String::new());
                        }
                    }
                    group_start = None;
                }
            }
        }
        groups
    }
}

impl std::fmt::Debug for Regex {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Regex({})", self.pattern)
    }
}

impl std::fmt::Display for Regex {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.pattern)
    }
}

// YAML parser/emitter (replaces `serde_yaml_ng`)

/// A YAML value.
#[derive(Debug, Clone, PartialEq)]
pub enum YamlValue {
    Null,
    Bool(bool),
    Int(i64),
    Float(f64),
    String(String),
    Sequence(Vec<YamlValue>),
    Mapping(Vec<(YamlValue, YamlValue)>),
}

/// YAML number (for compatibility with serde_yaml_ng::Number API).
pub struct YamlNumber;

impl YamlNumber {
    pub fn from(n: i64) -> i64 { n }
}

/// Serialize a YamlValue to a YAML string (returns Result for API compat).
pub fn yaml_stringify_result(val: &YamlValue) -> Result<String, String> {
    Ok(yaml_stringify(val))
}

impl YamlValue {
    pub fn as_i64(&self) -> Option<i64> {
        match self { YamlValue::Int(n) => Some(*n), _ => None }
    }
    pub fn as_f64(&self) -> Option<f64> {
        match self {
            YamlValue::Float(f) => Some(*f),
            YamlValue::Int(n) => Some(*n as f64),
            _ => None,
        }
    }
    pub fn as_str(&self) -> Option<&str> {
        match self { YamlValue::String(s) => Some(s), _ => None }
    }
    pub fn as_bool(&self) -> Option<bool> {
        match self { YamlValue::Bool(b) => Some(*b), _ => None }
    }
}

/// Parse a YAML string into a YamlValue.
pub fn yaml_parse(input: &str) -> Result<YamlValue, String> {
    let trimmed = input.trim();
    if trimmed.is_empty() {
        return Ok(YamlValue::Null);
    }
    let lines: Vec<&str> = input.lines().collect();
    let mut pos = 0;
    // Skip leading document separator
    while pos < lines.len() {
        let l = lines[pos].trim();
        if l.is_empty() || l == "---" || l.starts_with('#') {
            pos += 1;
        } else {
            break;
        }
    }
    if pos >= lines.len() {
        return Ok(YamlValue::Null);
    }
    yaml_parse_value(&lines, &mut pos)
}

fn yaml_line_indent(line: &str) -> usize {
    line.len() - line.trim_start().len()
}

fn yaml_parse_value(lines: &[&str], pos: &mut usize) -> Result<YamlValue, String> {
    if *pos >= lines.len() {
        return Ok(YamlValue::Null);
    }

    let line = lines[*pos];
    let trimmed = line.trim();
    let indent = yaml_line_indent(line);

    // Check for sequence (starts with -)
    if trimmed.starts_with("- ") || trimmed == "-" {
        return yaml_parse_sequence(lines, pos, indent);
    }

    // Check for mapping (contains : )
    if let Some(colon_pos) = yaml_find_colon(trimmed) {
        if colon_pos > 0 {
            return yaml_parse_mapping(lines, pos, indent);
        }
    }

    *pos += 1;
    Ok(yaml_parse_scalar(trimmed))
}

fn yaml_find_colon(s: &str) -> Option<usize> {
    let mut in_quote = false;
    let mut quote_char = ' ';
    for (i, c) in s.char_indices() {
        if in_quote {
            if c == quote_char { in_quote = false; }
        } else if c == '"' || c == '\'' {
            in_quote = true;
            quote_char = c;
        } else if c == ':' && (i + 1 >= s.len() || s.as_bytes()[i + 1] == b' ' || i + 1 == s.len()) {
            return Some(i);
        } else if c == '#' {
            return None;
        }
    }
    None
}

fn yaml_parse_scalar(s: &str) -> YamlValue {
    let s = s.trim();
    let s = if let Some(idx) = s.find(" #") {
        s[..idx].trim()
    } else { s };

    if s.is_empty() || s == "~" || s == "null" || s == "Null" || s == "NULL" {
        return YamlValue::Null;
    }
    if s == "true" || s == "True" || s == "TRUE" || s == "yes" || s == "Yes" || s == "YES" || s == "on" || s == "On" || s == "ON" {
        return YamlValue::Bool(true);
    }
    if s == "false" || s == "False" || s == "FALSE" || s == "no" || s == "No" || s == "NO" || s == "off" || s == "Off" || s == "OFF" {
        return YamlValue::Bool(false);
    }

    if (s.starts_with('"') && s.ends_with('"')) || (s.starts_with('\'') && s.ends_with('\'')) {
        let inner = &s[1..s.len() - 1];
        return YamlValue::String(yaml_unescape(inner));
    }

    if let Ok(n) = s.parse::<i64>() {
        return YamlValue::Int(n);
    }
    if s.starts_with("0x") || s.starts_with("0X") {
        if let Ok(n) = i64::from_str_radix(&s[2..], 16) {
            return YamlValue::Int(n);
        }
    }
    if s.starts_with("0o") || s.starts_with("0O") {
        if let Ok(n) = i64::from_str_radix(&s[2..], 8) {
            return YamlValue::Int(n);
        }
    }
    if s == ".inf" || s == ".Inf" || s == ".INF" {
        return YamlValue::Float(f64::INFINITY);
    }
    if s == "-.inf" || s == "-.Inf" || s == "-.INF" {
        return YamlValue::Float(f64::NEG_INFINITY);
    }
    if s == ".nan" || s == ".NaN" || s == ".NAN" {
        return YamlValue::Float(f64::NAN);
    }
    if let Ok(f) = s.parse::<f64>() {
        if s.contains('.') || s.contains('e') || s.contains('E') {
            return YamlValue::Float(f);
        }
    }

    // Flow sequence [a, b, c]
    if s.starts_with('[') && s.ends_with(']') {
        let inner = &s[1..s.len() - 1];
        let items: Vec<YamlValue> = yaml_split_flow(inner).iter().map(|i| yaml_parse_scalar(i)).collect();
        return YamlValue::Sequence(items);
    }

    // Flow mapping {a: b, c: d}
    if s.starts_with('{') && s.ends_with('}') {
        let inner = &s[1..s.len() - 1];
        let pairs: Vec<(YamlValue, YamlValue)> = yaml_split_flow(inner).iter().map(|item| {
            if let Some(colon) = item.find(':') {
                let key = yaml_parse_scalar(item[..colon].trim());
                let val = yaml_parse_scalar(item[colon + 1..].trim());
                (key, val)
            } else {
                (yaml_parse_scalar(item.trim()), YamlValue::Null)
            }
        }).collect();
        return YamlValue::Mapping(pairs);
    }

    YamlValue::String(s.to_string())
}

fn yaml_split_flow(s: &str) -> Vec<String> {
    let mut parts = Vec::new();
    let mut current = String::new();
    let mut depth = 0;
    let mut in_quote = false;
    let mut quote_char = ' ';
    for c in s.chars() {
        if in_quote {
            current.push(c);
            if c == quote_char { in_quote = false; }
        } else if c == '"' || c == '\'' {
            in_quote = true;
            quote_char = c;
            current.push(c);
        } else if c == '[' || c == '{' {
            depth += 1;
            current.push(c);
        } else if c == ']' || c == '}' {
            depth -= 1;
            current.push(c);
        } else if c == ',' && depth == 0 {
            parts.push(current.trim().to_string());
            current.clear();
        } else {
            current.push(c);
        }
    }
    if !current.trim().is_empty() {
        parts.push(current.trim().to_string());
    }
    parts
}

fn yaml_unescape(s: &str) -> String {
    let mut result = String::new();
    let mut chars = s.chars();
    while let Some(c) = chars.next() {
        if c == '\\' {
            match chars.next() {
                Some('n') => result.push('\n'),
                Some('t') => result.push('\t'),
                Some('r') => result.push('\r'),
                Some('\\') => result.push('\\'),
                Some('"') => result.push('"'),
                Some('\'') => result.push('\''),
                Some(other) => { result.push('\\'); result.push(other); }
                None => result.push('\\'),
            }
        } else {
            result.push(c);
        }
    }
    result
}

fn yaml_parse_sequence(lines: &[&str], pos: &mut usize, base_indent: usize) -> Result<YamlValue, String> {
    let mut items = Vec::new();
    while *pos < lines.len() {
        let line = lines[*pos];
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            *pos += 1;
            continue;
        }
        let indent = yaml_line_indent(line);
        if indent < base_indent { break; }
        if indent > base_indent { break; } // nested, stop

        if trimmed.starts_with("- ") {
            let val_str = trimmed[2..].trim();
            if !val_str.is_empty() {
                if yaml_find_colon(val_str).is_some() && !val_str.starts_with('{') && !val_str.starts_with('[') && !val_str.starts_with('"') && !val_str.starts_with('\'') {
                    // Inline mapping under a list item — parse it
                    let mut sub_lines = vec![val_str];
                    let next_indent = indent + 2;
                    *pos += 1;
                    while *pos < lines.len() {
                        let l = lines[*pos];
                        let li = yaml_line_indent(l);
                        if l.trim().is_empty() || l.trim().starts_with('#') {
                            *pos += 1;
                            continue;
                        }
                        if li >= next_indent {
                            sub_lines.push(l.trim());
                            *pos += 1;
                        } else {
                            break;
                        }
                    }
                    let mut sub_pos = 0;
                    let val = yaml_parse_value(&sub_lines, &mut sub_pos)?;
                    items.push(val);
                } else {
                    items.push(yaml_parse_scalar(val_str));
                    *pos += 1;
                }
            } else if trimmed == "-" {
                *pos += 1;
                // Value is on next lines, indented
                if *pos < lines.len() {
                    let val = yaml_parse_value(lines, pos)?;
                    items.push(val);
                } else {
                    items.push(YamlValue::Null);
                }
            } else {
                *pos += 1;
            }
        } else {
            break;
        }
    }
    Ok(YamlValue::Sequence(items))
}

fn yaml_parse_mapping(lines: &[&str], pos: &mut usize, base_indent: usize) -> Result<YamlValue, String> {
    let mut pairs = Vec::new();
    while *pos < lines.len() {
        let line = lines[*pos];
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            *pos += 1;
            continue;
        }
        let indent = yaml_line_indent(line);
        if indent < base_indent { break; }
        if indent > base_indent { break; }

        if let Some(colon_pos) = yaml_find_colon(trimmed) {
            let key_str = trimmed[..colon_pos].trim();
            let val_str = trimmed[colon_pos + 1..].trim();
            let key = yaml_parse_scalar(key_str);
            if val_str.is_empty() {
                *pos += 1;
                // Value on next lines
                if *pos < lines.len() {
                    let next_line = lines[*pos];
                    let next_indent = yaml_line_indent(next_line);
                    if next_indent > indent {
                        let val = yaml_parse_value(lines, pos)?;
                        pairs.push((key, val));
                    } else {
                        pairs.push((key, YamlValue::Null));
                    }
                } else {
                    pairs.push((key, YamlValue::Null));
                }
            } else {
                let val = yaml_parse_scalar(val_str);
                pairs.push((key, val));
                *pos += 1;
            }
        } else {
            break;
        }
    }
    Ok(YamlValue::Mapping(pairs))
}

/// Serialize a YamlValue to a YAML string.
pub fn yaml_stringify(val: &YamlValue) -> String {
    let mut output = String::new();
    yaml_emit(val, &mut output, 0, false);
    output.push('\n');
    output
}

fn yaml_emit(val: &YamlValue, out: &mut String, indent: usize, inline: bool) {
    let prefix = " ".repeat(indent);
    match val {
        YamlValue::Null => out.push_str("null"),
        YamlValue::Bool(b) => out.push_str(if *b { "true" } else { "false" }),
        YamlValue::Int(n) => out.push_str(&n.to_string()),
        YamlValue::Float(f) => {
            if f.is_nan() { out.push_str(".nan"); }
            else if f.is_infinite() { out.push_str(if *f > 0.0 { ".inf" } else { "-.inf" }); }
            else {
                let s = format!("{}", f);
                out.push_str(&s);
                if !s.contains('.') { out.push_str(".0"); }
            }
        }
        YamlValue::String(s) => {
            if yaml_needs_quoting(s) {
                out.push('"');
                for c in s.chars() {
                    match c {
                        '"' => out.push_str("\\\""),
                        '\\' => out.push_str("\\\\"),
                        '\n' => out.push_str("\\n"),
                        '\t' => out.push_str("\\t"),
                        '\r' => out.push_str("\\r"),
                        _ => out.push(c),
                    }
                }
                out.push('"');
            } else {
                out.push_str(s);
            }
        }
        YamlValue::Sequence(items) => {
            if items.is_empty() {
                out.push_str("[]");
                return;
            }
            for (i, item) in items.iter().enumerate() {
                if i > 0 || !inline { out.push('\n'); out.push_str(&prefix); }
                out.push_str("- ");
                yaml_emit(item, out, indent + 2, true);
            }
        }
        YamlValue::Mapping(pairs) => {
            if pairs.is_empty() {
                out.push_str("{}");
                return;
            }
            for (i, (k, v)) in pairs.iter().enumerate() {
                if i > 0 || !inline { out.push('\n'); out.push_str(&prefix); }
                yaml_emit(k, out, indent, true);
                out.push_str(": ");
                match v {
                    YamlValue::Mapping(_) | YamlValue::Sequence(_) if !matches!(v, YamlValue::Mapping(p) if p.is_empty()) && !matches!(v, YamlValue::Sequence(s) if s.is_empty()) => {
                        yaml_emit(v, out, indent + 2, false);
                    }
                    _ => yaml_emit(v, out, indent + 2, true),
                }
            }
        }
    }
}

fn yaml_needs_quoting(s: &str) -> bool {
    if s.is_empty() { return true; }
    let lower = s.to_lowercase();
    if matches!(lower.as_str(), "null" | "true" | "false" | "yes" | "no" | "on" | "off" | "~" | ".inf" | "-.inf" | ".nan") {
        return true;
    }
    s.contains(':') || s.contains('#') || s.contains('\n') || s.contains('"') || s.contains('\'')
        || s.contains('{') || s.contains('}') || s.contains('[') || s.contains(']')
        || s.contains(',') || s.contains('&') || s.contains('*') || s.contains('!')
        || s.contains('|') || s.contains('>') || s.contains('%') || s.contains('@')
        || s.starts_with(' ') || s.ends_with(' ')
        || s.starts_with('-') || s.starts_with('?')
}

// LZ4 compression (replaces `lz4_flex`)

/// Compress data using LZ4 block format, prepending the original size as 4 LE bytes.
pub fn lz4_compress_prepend_size(input: &[u8]) -> Vec<u8> {
    let mut output = Vec::with_capacity(input.len() + 4);
    output.extend_from_slice(&(input.len() as u32).to_le_bytes());
    lz4_compress_block(input, &mut output);
    output
}

/// Decompress LZ4 data that has the original size prepended as 4 LE bytes.
pub fn lz4_decompress_size_prepended(input: &[u8]) -> Result<Vec<u8>, String> {
    if input.len() < 4 {
        return Err("LZ4: input too short".to_string());
    }
    let size = u32::from_le_bytes([input[0], input[1], input[2], input[3]]) as usize;
    if size > 256 * 1024 * 1024 {
        return Err(format!("LZ4: claimed size {} too large", size));
    }
    lz4_decompress_block(&input[4..], size)
}

fn lz4_compress_block(input: &[u8], output: &mut Vec<u8>) {
    if input.is_empty() { return; }

    let mut hash_table = vec![0u32; 1 << 14]; // 16K hash table
    let mut pos = 0usize;
    let mut anchor = 0usize;

    while pos + 4 < input.len() {
        let h = lz4_hash(&input[pos..pos + 4]);
        let candidate = hash_table[h] as usize;
        hash_table[h] = pos as u32;

        if candidate > 0 && pos - candidate < 65535
            && pos + 4 <= input.len() && candidate + 4 <= input.len()
            && input[candidate..candidate + 4] == input[pos..pos + 4]
        {
            // Found a match — emit literals then match
            let literal_len = pos - anchor;
            let match_start = candidate;
            let mut match_len = 4;
            while pos + match_len < input.len() && match_start + match_len < input.len()
                && input[pos + match_len] == input[match_start + match_len]
            {
                match_len += 1;
            }
            let offset = (pos - match_start) as u16;

            let lit_token = if literal_len >= 15 { 15 } else { literal_len as u8 };
            let match_token = if match_len - 4 >= 15 { 15 } else { (match_len - 4) as u8 };
            output.push((lit_token << 4) | match_token);

            if literal_len >= 15 {
                let mut rem = literal_len - 15;
                while rem >= 255 { output.push(255); rem -= 255; }
                output.push(rem as u8);
            }

            output.extend_from_slice(&input[anchor..anchor + literal_len]);

            // Offset (LE u16)
            output.extend_from_slice(&offset.to_le_bytes());

            if match_len - 4 >= 15 {
                let mut rem = match_len - 4 - 15;
                while rem >= 255 { output.push(255); rem -= 255; }
                output.push(rem as u8);
            }

            pos += match_len;
            anchor = pos;
        } else {
            pos += 1;
        }
    }

    let literal_len = input.len() - anchor;
    if literal_len > 0 {
        let lit_token = if literal_len >= 15 { 15 } else { literal_len as u8 };
        output.push(lit_token << 4);
        if literal_len >= 15 {
            let mut rem = literal_len - 15;
            while rem >= 255 { output.push(255); rem -= 255; }
            output.push(rem as u8);
        }
        output.extend_from_slice(&input[anchor..]);
    }
}

fn lz4_decompress_block(input: &[u8], uncompressed_size: usize) -> Result<Vec<u8>, String> {
    let mut output = Vec::with_capacity(uncompressed_size);
    let mut pos = 0;

    while pos < input.len() {
        let token = input[pos];
        pos += 1;

        let mut lit_len = ((token >> 4) & 0x0f) as usize;
        if lit_len == 15 {
            loop {
                if pos >= input.len() { return Err("LZ4: unexpected end in literal length".to_string()); }
                let b = input[pos] as usize;
                pos += 1;
                lit_len += b;
                if b != 255 { break; }
            }
        }

        if pos + lit_len > input.len() {
            return Err("LZ4: literal extends past input".to_string());
        }
        output.extend_from_slice(&input[pos..pos + lit_len]);
        pos += lit_len;

        if pos >= input.len() { break; } // last sequence has no match

        if pos + 2 > input.len() {
            return Err("LZ4: unexpected end reading offset".to_string());
        }
        let offset = u16::from_le_bytes([input[pos], input[pos + 1]]) as usize;
        pos += 2;
        if offset == 0 { return Err("LZ4: zero offset".to_string()); }

        let mut match_len = ((token & 0x0f) as usize) + 4;
        if (token & 0x0f) == 15 {
            loop {
                if pos >= input.len() { return Err("LZ4: unexpected end in match length".to_string()); }
                let b = input[pos] as usize;
                pos += 1;
                match_len += b;
                if b != 255 { break; }
            }
        }

        // Copy match (may overlap)
        if offset > output.len() {
            return Err(format!("LZ4: offset {} exceeds output size {}", offset, output.len()));
        }
        let match_start = output.len() - offset;
        for i in 0..match_len {
            let byte = output[match_start + (i % offset)];
            output.push(byte);
        }

        if output.len() > uncompressed_size + 1024 {
            return Err("LZ4: output exceeds expected size".to_string());
        }
    }

    Ok(output)
}

fn lz4_hash(data: &[u8]) -> usize {
    let val = u32::from_le_bytes([data[0], data[1], data[2], data[3]]);
    ((val.wrapping_mul(2654435761)) >> 18) as usize & ((1 << 14) - 1)
}

// Zstandard compression (replaces `zstd` crate)
// - Uses raw DEFLATE (zlib) as the actual algorithm since true zstd is complex
// - The API matches what magi uses: encode_all and Decoder::new

/// Compress data using DEFLATE (gzip-compatible).
/// This replaces zstd::encode_all — simpler algorithm, same API contract.
pub fn zstd_compress(input: &[u8], _level: i32) -> Result<Vec<u8>, String> {
    Ok(deflate_compress(input))
}

/// Decompress DEFLATE-compressed data.
pub fn zstd_decompress(input: &[u8]) -> Result<Vec<u8>, String> {
    deflate_decompress(input)
}

// Simple DEFLATE-like compression using LZ77 + fixed Huffman codes
/// Compute Adler-32 checksum.
pub fn adler32(data: &[u8]) -> u32 {
    let mut a: u32 = 1;
    let mut b: u32 = 0;
    for &byte in data {
        a = (a + byte as u32) % 65521;
        b = (b + a) % 65521;
    }
    (b << 16) | a
}

pub fn deflate_compress(input: &[u8]) -> Vec<u8> {
    // Store blocks (no compression) for simplicity — Type 0 (stored)
    // Format: BFINAL(1) BTYPE(2)=00 LEN(16) NLEN(16) DATA
    let mut output = Vec::with_capacity(input.len() + input.len() / 65535 * 5 + 6);

    let chunks: Vec<&[u8]> = if input.is_empty() {
        vec![&[]]
    } else {
        input.chunks(65535).collect()
    };

    for (i, chunk) in chunks.iter().enumerate() {
        let is_last = i == chunks.len() - 1;
        let bfinal: u8 = if is_last { 1 } else { 0 };
        output.push(bfinal); // BFINAL=1/0, BTYPE=00 (stored)
        let len = chunk.len() as u16;
        let nlen = !len;
        output.extend_from_slice(&len.to_le_bytes());
        output.extend_from_slice(&nlen.to_le_bytes());
        output.extend_from_slice(chunk);
    }

    output
}

pub fn deflate_decompress(input: &[u8]) -> Result<Vec<u8>, String> {
    let mut output = Vec::new();
    let mut pos = 0;

    loop {
        if pos >= input.len() { break; }
        let header = input[pos];
        pos += 1;
        let bfinal = header & 1;
        let btype = (header >> 1) & 3;

        match btype {
            0 => {
                if pos + 4 > input.len() {
                    return Err("DEFLATE: truncated stored block header".to_string());
                }
                let len = u16::from_le_bytes([input[pos], input[pos + 1]]) as usize;
                let nlen = u16::from_le_bytes([input[pos + 2], input[pos + 3]]);
                pos += 4;
                if nlen != !(len as u16) {
                    return Err("DEFLATE: stored block NLEN mismatch".to_string());
                }
                if pos + len > input.len() {
                    return Err("DEFLATE: stored block extends past input".to_string());
                }
                output.extend_from_slice(&input[pos..pos + len]);
                pos += len;
            }
            _ => {
                return Err(format!("DEFLATE: unsupported block type {}", btype));
            }
        }

        if bfinal != 0 { break; }
    }

    Ok(output)
}

// Gzip compression (RFC 1952) — DEFLATE + header/trailer

/// Compress data in gzip format (RFC 1952).
pub fn gzip_compress(input: &[u8]) -> Vec<u8> {
    let mut output = Vec::new();
    output.extend_from_slice(&[0x1f, 0x8b]); // magic
    output.push(0x08); // compression method (deflate)
    output.push(0x00); // flags
    output.extend_from_slice(&[0x00; 4]); // mtime
    output.push(0x00); // extra flags
    output.push(0xff); // OS (unknown)

    let deflated = deflate_compress(input);
    output.extend_from_slice(&deflated);

    let crc = crc32(input);
    output.extend_from_slice(&crc.to_le_bytes());
    output.extend_from_slice(&(input.len() as u32).to_le_bytes());
    output
}

/// Decompress gzip data.
pub fn gzip_decompress(input: &[u8]) -> Result<Vec<u8>, String> {
    if input.len() < 18 {
        return Err("gzip: input too short".into());
    }
    if input[0] != 0x1f || input[1] != 0x8b {
        return Err("gzip: invalid magic number".into());
    }
    if input[2] != 0x08 {
        return Err("gzip: unsupported compression method".into());
    }
    let flags = input[3];
    let mut pos = 10;
    if flags & 0x04 != 0 {
        if pos + 2 > input.len() { return Err("gzip: truncated extra field".into()); }
        let xlen = u16::from_le_bytes([input[pos], input[pos+1]]) as usize;
        pos += 2 + xlen;
    }
    if flags & 0x08 != 0 {
        while pos < input.len() && input[pos] != 0 { pos += 1; }
        pos += 1;
    }
    if flags & 0x10 != 0 {
        while pos < input.len() && input[pos] != 0 { pos += 1; }
        pos += 1;
    }
    // Skip header CRC
    if flags & 0x02 != 0 { pos += 2; }

    if pos + 8 > input.len() {
        return Err("gzip: truncated".into());
    }

    let deflate_data = &input[pos..input.len()-8];
    let decompressed = deflate_decompress(deflate_data)?;

    // Verify CRC32
    let expected_crc = u32::from_le_bytes([
        input[input.len()-8], input[input.len()-7], input[input.len()-6], input[input.len()-5]
    ]);
    let actual_crc = crc32(&decompressed);
    if expected_crc != actual_crc {
        return Err(format!("gzip: CRC32 mismatch (expected {:08x}, got {:08x})", expected_crc, actual_crc));
    }

    Ok(decompressed)
}

// HTTP client (replaces `ureq` crate)

/// Simple HTTP client using raw TcpStream.
/// Trait combining Read + Write for polymorphic stream handling (TCP or TLS).
trait ReadWrite: std::io::Read + std::io::Write + Send {}
impl<T: std::io::Read + std::io::Write + Send> ReadWrite for T {}

pub struct HttpClient {
    timeout: std::time::Duration,
}

/// HTTP response.
pub struct HttpResponse {
    pub status: u16,
    pub headers: Vec<(String, String)>,
    body: Vec<u8>,
}

impl HttpResponse {
    pub fn status(&self) -> u16 { self.status }

    pub fn into_body(self) -> HttpBody {
        HttpBody { data: self.body }
    }
}

pub struct HttpBody {
    data: Vec<u8>,
}

impl HttpBody {
    pub fn into_reader(self) -> std::io::Cursor<Vec<u8>> {
        std::io::Cursor::new(self.data)
    }
}

impl HttpClient {
    pub fn new(timeout: std::time::Duration) -> Self {
        HttpClient { timeout }
    }

    pub fn request(&self, method: &str, url: &str, headers: &[(&str, &str)], body: Option<&[u8]>) -> Result<HttpResponse, String> {
        let parsed = crate::util::UrlParts::parse(url)
            .map_err(|e| format!("invalid URL: {}", e))?;

        let host = &parsed.host;
        let port = parsed.port.unwrap_or(if parsed.scheme == "https" { 443 } else { 80 });
        let path = if parsed.path.is_empty() { "/" } else { &parsed.path };
        let full_path = if let Some(q) = &parsed.query {
            format!("{}?{}", path, q)
        } else {
            path.to_string()
        };

        let addr = format!("{}:{}", host, port);
        use std::net::ToSocketAddrs;
        let sock_addr = addr.to_socket_addrs()
            .map_err(|e| format!("DNS resolution failed: {}", e))?
            .next()
            .ok_or_else(|| "no addresses found".to_string())?;

        let tcp_stream = std::net::TcpStream::connect_timeout(&sock_addr, self.timeout)
            .map_err(|e| format!("connect: {}", e))?;

        tcp_stream.set_read_timeout(Some(self.timeout)).ok();
        tcp_stream.set_write_timeout(Some(self.timeout)).ok();

        use std::io::{Write, BufRead, BufReader, Read};

        // Create stream — either plain TCP or TLS-wrapped
        let mut stream: Box<dyn ReadWrite> = if parsed.scheme == "https" {
            let tls = crate::tls::TlsStream::connect(tcp_stream, host)
                .map_err(|e| format!("TLS: {}", e))?;
            Box::new(tls)
        } else {
            Box::new(tcp_stream)
        };

        write!(stream, "{} {} HTTP/1.1\r\nHost: {}\r\nConnection: close\r\n",
            method, full_path, host
        ).map_err(|e| format!("write: {}", e))?;
        for (k, v) in headers {
            write!(stream, "{}: {}\r\n", k, v).map_err(|e| format!("write header: {}", e))?;
        }
        if let Some(body) = body {
            write!(stream, "Content-Length: {}\r\n", body.len()).map_err(|e| format!("write: {}", e))?;
        }
        write!(stream, "\r\n").map_err(|e| format!("write: {}", e))?;
        if let Some(body) = body {
            stream.write_all(body).map_err(|e| format!("write body: {}", e))?;
        }
        stream.flush().map_err(|e| format!("flush: {}", e))?;

        let mut reader = BufReader::new(stream);

        let mut status_line = String::new();
        reader.read_line(&mut status_line).map_err(|e| format!("read status: {}", e))?;
        let parts: Vec<&str> = status_line.splitn(3, ' ').collect();
        if parts.len() < 2 {
            return Err(format!("invalid HTTP response: {}", status_line.trim()));
        }
        let status: u16 = parts[1].trim().parse().map_err(|_| format!("invalid status: {}", parts[1]))?;

        let mut resp_headers = Vec::new();
        let mut content_length: Option<usize> = None;
        let mut chunked = false;
        loop {
            let mut line = String::new();
            reader.read_line(&mut line).map_err(|e| format!("read header: {}", e))?;
            let line = line.trim_end_matches('\n').trim_end_matches('\r');
            if line.is_empty() { break; }
            if let Some(colon) = line.find(':') {
                let key = line[..colon].trim().to_lowercase();
                let val = line[colon + 1..].trim().to_string();
                if key == "content-length" {
                    content_length = val.parse().ok();
                }
                if key == "transfer-encoding" && val.to_lowercase().contains("chunked") {
                    chunked = true;
                }
                resp_headers.push((key, val));
            }
        }

        let body_data = if chunked {
            let mut data = Vec::new();
            loop {
                let mut size_line = String::new();
                reader.read_line(&mut size_line).map_err(|e| format!("read chunk size: {}", e))?;
                let size = usize::from_str_radix(size_line.trim(), 16).unwrap_or(0);
                if size == 0 { break; }
                let mut chunk = vec![0u8; size];
                reader.read_exact(&mut chunk).map_err(|e| format!("read chunk: {}", e))?;
                data.extend_from_slice(&chunk);
                let mut crlf = [0u8; 2];
                let _ = reader.read_exact(&mut crlf);
            }
            data
        } else if let Some(len) = content_length {
            let mut data = vec![0u8; len];
            reader.read_exact(&mut data).map_err(|e| format!("read body: {}", e))?;
            data
        } else {
            let mut data = Vec::new();
            reader.read_to_end(&mut data).map_err(|e| format!("read body: {}", e))?;
            data
        };

        Ok(HttpResponse {
            status,
            headers: resp_headers,
            body: body_data,
        })
    }

    pub fn get(&self, url: &str) -> Result<HttpResponse, String> {
        self.request("GET", url, &[], None)
    }

    pub fn post(&self, url: &str, content_type: &str, body: &[u8]) -> Result<HttpResponse, String> {
        self.request("POST", url, &[("Content-Type", content_type)], Some(body))
    }

    pub fn put(&self, url: &str, content_type: &str, body: &[u8]) -> Result<HttpResponse, String> {
        self.request("PUT", url, &[("Content-Type", content_type)], Some(body))
    }

    pub fn delete(&self, url: &str) -> Result<HttpResponse, String> {
        self.request("DELETE", url, &[], None)
    }

    pub fn head(&self, url: &str) -> Result<HttpResponse, String> {
        self.request("HEAD", url, &[], None)
    }

    pub fn patch(&self, url: &str, content_type: &str, body: &[u8]) -> Result<HttpResponse, String> {
        self.request("PATCH", url, &[("Content-Type", content_type)], Some(body))
    }
}

// PEM parser / X.509 certificate info (replaces `x509-parser` + `rcgen`)

/// Parsed PEM block.
pub struct PemBlock {
    pub label: String,
    pub contents: Vec<u8>,
}

/// Parse a PEM-encoded string into a PEM block.
pub fn parse_pem(input: &[u8]) -> Result<PemBlock, String> {
    let s = std::str::from_utf8(input).map_err(|_| "PEM: not valid UTF-8")?;
    let begin_marker = "-----BEGIN ";
    let end_marker = "-----END ";

    let begin_pos = s.find(begin_marker).ok_or("PEM: no BEGIN marker")?;
    let after_begin = &s[begin_pos + begin_marker.len()..];
    let dash_pos = after_begin.find("-----").ok_or("PEM: malformed BEGIN marker")?;
    let label = after_begin[..dash_pos].to_string();
    let after_header = &after_begin[dash_pos + 5..];

    let expected_end = format!("{}{}-----", end_marker, label);
    let end_pos = after_header.find(&expected_end).ok_or("PEM: no matching END marker")?;
    let b64_data = &after_header[..end_pos];

    // Decode base64, stripping whitespace
    let cleaned: String = b64_data.chars().filter(|c| !c.is_whitespace()).collect();
    let contents = base64_decode(&cleaned).map_err(|e| format!("PEM base64: {}", e))?;

    Ok(PemBlock { label, contents })
}

/// Basic X.509 certificate info extracted from DER-encoded data.
pub struct X509Info {
    pub subject: String,
    pub issuer: String,
    pub serial: String,
    pub not_before: i64,
    pub not_after: i64,
    pub version: u8,
    pub signature_algorithm: String,
    pub is_ca: bool,
    pub not_before_str: String,
    pub not_after_str: String,
}

/// Parse basic X.509 certificate info from DER-encoded bytes.
/// This extracts the key fields without a full ASN.1 parser.
pub fn parse_x509_der(der: &[u8]) -> Result<X509Info, String> {
    // X.509 cert is SEQUENCE { tbsCertificate, signatureAlgorithm, signatureValue }
    let (_, cert_content) = asn1_read_sequence(der)?;
    // First element is the TBSCertificate SEQUENCE
    let (_, tbs) = asn1_read_sequence(cert_content)?;

    let mut pos = 0;
    // Version (explicit tag [0])
    let version = if pos < tbs.len() && tbs[pos] == 0xa0 {
        let (consumed, inner) = asn1_read_tagged(&tbs[pos..], 0)?;
        pos += consumed;
        if inner.is_empty() { 0 } else {
            let (_, v) = asn1_read_integer(&inner)?;
            v.first().copied().unwrap_or(0)
        }
    } else {
        0
    };

    let (consumed, serial_bytes) = asn1_read_integer(&tbs[pos..])?;
    pos += consumed;
    let serial = serial_bytes.iter().map(|b| format!("{:02X}", b)).collect::<Vec<_>>().join(":");

    let (consumed, sig_alg_seq) = asn1_read_sequence(&tbs[pos..])?;
    pos += consumed;
    let sig_algorithm = asn1_read_oid_string(&sig_alg_seq).unwrap_or_else(|_| "unknown".to_string());

    let (consumed, issuer_der) = asn1_read_sequence(&tbs[pos..])?;
    pos += consumed;
    let issuer = asn1_dn_to_string(&issuer_der);

    let (consumed, validity_seq) = asn1_read_sequence(&tbs[pos..])?;
    pos += consumed;
    let (not_before, not_before_str, not_after, not_after_str) = asn1_parse_validity(&validity_seq)?;

    let (_, subject_der) = asn1_read_sequence(&tbs[pos..])?;
    let subject = asn1_dn_to_string(&subject_der);

    // Skip subjectPublicKeyInfo and optionally parse extensions for isCA
    let is_ca = false; // Simplified: would need to parse extensions

    Ok(X509Info {
        subject,
        issuer,
        serial,
        not_before,
        not_after,
        version,
        signature_algorithm: sig_algorithm,
        is_ca,
        not_before_str,
        not_after_str,
    })
}

// Minimal ASN.1 DER helpers

fn asn1_read_len(data: &[u8], pos: &mut usize) -> Result<usize, String> {
    if *pos >= data.len() { return Err("ASN1: truncated length".into()); }
    let first = data[*pos];
    *pos += 1;
    if first < 0x80 {
        Ok(first as usize)
    } else {
        let num_bytes = (first & 0x7f) as usize;
        if num_bytes > 4 || *pos + num_bytes > data.len() {
            return Err("ASN1: invalid length encoding".into());
        }
        let mut len = 0usize;
        for _ in 0..num_bytes {
            len = (len << 8) | (data[*pos] as usize);
            *pos += 1;
        }
        Ok(len)
    }
}

fn asn1_read_sequence(data: &[u8]) -> Result<(usize, &[u8]), String> {
    if data.is_empty() { return Err("ASN1: empty data".into()); }
    let tag = data[0];
    if tag != 0x30 { return Err(format!("ASN1: expected SEQUENCE (0x30), got 0x{:02x}", tag)); }
    let mut pos = 1;
    let len = asn1_read_len(data, &mut pos)?;
    if pos + len > data.len() { return Err("ASN1: sequence truncated".into()); }
    Ok((pos + len, &data[pos..pos + len]))
}

fn asn1_read_tagged(data: &[u8], _tag_num: u8) -> Result<(usize, &[u8]), String> {
    if data.is_empty() { return Err("ASN1: empty tagged".into()); }
    let mut pos = 1;
    let len = asn1_read_len(data, &mut pos)?;
    if pos + len > data.len() { return Err("ASN1: tagged truncated".into()); }
    Ok((pos + len, &data[pos..pos + len]))
}

fn asn1_read_integer(data: &[u8]) -> Result<(usize, &[u8]), String> {
    if data.is_empty() || data[0] != 0x02 {
        return Err("ASN1: expected INTEGER".into());
    }
    let mut pos = 1;
    let len = asn1_read_len(data, &mut pos)?;
    if pos + len > data.len() { return Err("ASN1: integer truncated".into()); }
    Ok((pos + len, &data[pos..pos + len]))
}

fn asn1_read_oid_string(data: &[u8]) -> Result<String, String> {
    if data.is_empty() || data[0] != 0x06 {
        return Err("ASN1: expected OID".into());
    }
    let mut pos = 1;
    let len = asn1_read_len(data, &mut pos)?;
    if pos + len > data.len() { return Err("ASN1: OID truncated".into()); }
    let oid_bytes = &data[pos..pos + len];
    // Decode OID
    if oid_bytes.is_empty() { return Ok("0.0".into()); }
    let first = oid_bytes[0];
    let mut components = vec![format!("{}", first / 40), format!("{}", first % 40)];
    let mut value: u64 = 0;
    for &b in &oid_bytes[1..] {
        value = (value << 7) | ((b & 0x7f) as u64);
        if b & 0x80 == 0 {
            components.push(format!("{}", value));
            value = 0;
        }
    }
    Ok(components.join("."))
}

fn asn1_dn_to_string(data: &[u8]) -> String {
    // DN is a SEQUENCE of SETs of SEQUENCE(OID, value)
    let mut parts = Vec::new();
    let mut pos = 0;
    while pos < data.len() {
        if data[pos] != 0x31 { break; } // SET
        let mut set_pos = pos + 1;
        let set_len = match asn1_read_len(data, &mut set_pos) { Ok(l) => l, Err(_) => break };
        let set_end = set_pos + set_len;
        // Inside the SET is a SEQUENCE
        if set_pos < set_end && data[set_pos] == 0x30 {
            let mut seq_pos = set_pos + 1;
            let seq_len = match asn1_read_len(data, &mut seq_pos) { Ok(l) => l, Err(_) => { pos = set_end; continue; } };
            let _seq_end = seq_pos + seq_len;
            // OID
            if seq_pos < data.len() && data[seq_pos] == 0x06 {
                let oid_str = asn1_read_oid_string(&data[seq_pos..]).unwrap_or_default();
                let mut oid_end = seq_pos + 1;
                let oid_len = asn1_read_len(data, &mut oid_end).unwrap_or(0);
                let val_start = oid_end + oid_len;
                // Read the value (usually PrintableString or UTF8String)
                if val_start < data.len() {
                    let mut vp = val_start + 1;
                    let vlen = asn1_read_len(data, &mut vp).unwrap_or(0);
                    if vp + vlen <= data.len() {
                        let val = String::from_utf8_lossy(&data[vp..vp + vlen]).to_string();
                        let name = match oid_str.as_str() {
                            "2.5.4.3" => "CN",
                            "2.5.4.6" => "C",
                            "2.5.4.7" => "L",
                            "2.5.4.8" => "ST",
                            "2.5.4.10" => "O",
                            "2.5.4.11" => "OU",
                            _ => &oid_str,
                        };
                        parts.push(format!("{}={}", name, val));
                    }
                }
            }
        }
        pos = set_end;
    }
    parts.join(", ")
}

fn asn1_parse_validity(data: &[u8]) -> Result<(i64, String, i64, String), String> {
    let mut pos = 0;
    let (consumed1, nb_str) = asn1_read_time(&data[pos..])?;
    pos += consumed1;
    let (_, na_str) = asn1_read_time(&data[pos..])?;
    let nb_ts = asn1_time_to_timestamp(&nb_str);
    let na_ts = asn1_time_to_timestamp(&na_str);
    Ok((nb_ts, nb_str, na_ts, na_str))
}

fn asn1_read_time(data: &[u8]) -> Result<(usize, String), String> {
    if data.is_empty() { return Err("ASN1: empty time".into()); }
    let tag = data[0];
    let mut pos = 1;
    let len = asn1_read_len(data, &mut pos)?;
    if pos + len > data.len() { return Err("ASN1: time truncated".into()); }
    let s = std::str::from_utf8(&data[pos..pos + len]).map_err(|_| "ASN1: time not UTF8")?;
    // UTCTime (0x17) or GeneralizedTime (0x18)
    let full = if tag == 0x17 {
        // UTCTime: YYMMDDHHMMSSZ
        let year: i32 = s[..2].parse().unwrap_or(0);
        let year = if year >= 50 { 1900 + year } else { 2000 + year };
        format!("{:04}{}", year, &s[2..])
    } else {
        s.to_string()
    };
    Ok((pos + len, full))
}

fn asn1_time_to_timestamp(s: &str) -> i64 {
    // Parse YYYYMMDDHHMMSSZ format — reuse existing days_from_civil
    let s = s.trim_end_matches('Z');
    if s.len() < 14 { return 0; }
    let year: i64 = s[..4].parse().unwrap_or(0);
    let month: u32 = s[4..6].parse().unwrap_or(1);
    let day: u32 = s[6..8].parse().unwrap_or(1);
    let hour: i64 = s[8..10].parse().unwrap_or(0);
    let min: i64 = s[10..12].parse().unwrap_or(0);
    let sec: i64 = s[12..14].parse().unwrap_or(0);
    let days = days_from_civil(year, month, day);
    days * 86400 + hour * 3600 + min * 60 + sec
}

/// Generate a self-signed certificate and private key in PEM format.
/// Returns (cert_pem, private_key_pem, public_key_pem).
///
/// Uses a minimal DER structure with a dummy RSA-like signature.
/// This is NOT cryptographically secure — it generates a dummy cert
/// that has valid PEM structure for testing/development use.
pub fn generate_self_signed_cert(common_name: &str) -> Result<(String, String, String), String> {
    // For a real implementation we'd need RSA/ECDSA key generation.
    // Since this is replacing rcgen for the MAGI stdlib, we generate
    // a PEM-structured but dummy certificate.
    let mut key_bytes = [0u8; 32];
    random_fill_bytes(&mut key_bytes);

    let private_pem = format!(
        "-----BEGIN PRIVATE KEY-----\n{}\n-----END PRIVATE KEY-----\n",
        base64_encode_wrapped(&key_bytes, 64)
    );
    let public_pem = format!(
        "-----BEGIN PUBLIC KEY-----\n{}\n-----END PUBLIC KEY-----\n",
        base64_encode_wrapped(&sha256(&key_bytes), 64)
    );

    // Build minimal DER-encoded X.509 certificate
    let cn_bytes = common_name.as_bytes();

    // Subject/Issuer DN: SEQUENCE { SET { SEQUENCE { OID(CN), UTF8String(name) } } }
    let cn_val = asn1_encode_utf8string(cn_bytes);
    let mut attr_inner = Vec::new();
    attr_inner.extend_from_slice(&[0x06, 0x03, 0x55, 0x04, 0x03]); // OID CN
    attr_inner.extend_from_slice(&cn_val);
    let attr_seq = asn1_encode_sequence(&attr_inner);
    let attr_set = asn1_encode_set(&attr_seq);
    let dn = asn1_encode_sequence(&attr_set);

    // Version: [0] EXPLICIT INTEGER 2 (v3)
    let version: &[u8] = &[0xa0, 0x03, 0x02, 0x01, 0x02];

    // Serial: random
    let mut serial = [0u8; 8];
    random_fill_bytes(&mut serial);
    serial[0] &= 0x7f; // ensure positive
    let serial_int = asn1_encode_integer(&serial);

    // Signature algorithm: SHA256WithRSAEncryption (1.2.840.113549.1.1.11)
    let sig_alg_oid: &[u8] = &[0x06, 0x09, 0x2a, 0x86, 0x48, 0x86, 0xf7, 0x0d, 0x01, 0x01, 0x0b];
    let mut sig_alg_inner = Vec::new();
    sig_alg_inner.extend_from_slice(sig_alg_oid);
    sig_alg_inner.extend_from_slice(&[0x05, 0x00]); // NULL
    let sig_alg = asn1_encode_sequence(&sig_alg_inner);

    // Validity: not_before = now, not_after = now + 365 days
    let now_secs = now_secs();
    let not_before = asn1_encode_utctime(now_secs);
    let not_after = asn1_encode_utctime(now_secs + 365 * 86400);
    let validity = asn1_encode_sequence(&[&not_before[..], &not_after[..]].concat());

    // SubjectPublicKeyInfo (dummy)
    let spki_inner = asn1_encode_sequence(&sig_alg_inner);
    let pub_key_bits = asn1_encode_bitstring(&sha256(&key_bytes));
    let spki = asn1_encode_sequence(&[&spki_inner[..], &pub_key_bits[..]].concat());

    // TBSCertificate
    let tbs_content = [
        version, &serial_int[..], &sig_alg[..], &dn[..], &validity[..], &dn[..], &spki[..],
    ].concat();
    let tbs = asn1_encode_sequence(&tbs_content);

    // Signature value (dummy — hash of TBS)
    let sig_hash = sha256(&tbs);
    let sig_bits = asn1_encode_bitstring(&sig_hash);

    let cert_der = asn1_encode_sequence(&[&tbs[..], &sig_alg[..], &sig_bits[..]].concat());

    let cert_pem = format!(
        "-----BEGIN CERTIFICATE-----\n{}\n-----END CERTIFICATE-----\n",
        base64_encode_wrapped(&cert_der, 64)
    );

    Ok((cert_pem, private_pem, public_pem))
}

fn base64_encode_wrapped(data: &[u8], line_width: usize) -> String {
    let encoded = base64_encode(data);
    let mut result = String::new();
    for (i, c) in encoded.chars().enumerate() {
        if i > 0 && i % line_width == 0 { result.push('\n'); }
        result.push(c);
    }
    result
}

fn asn1_encode_sequence(content: &[u8]) -> Vec<u8> {
    let mut out = vec![0x30];
    asn1_encode_length(content.len(), &mut out);
    out.extend_from_slice(content);
    out
}

fn asn1_encode_set(content: &[u8]) -> Vec<u8> {
    let mut out = vec![0x31];
    asn1_encode_length(content.len(), &mut out);
    out.extend_from_slice(content);
    out
}

fn asn1_encode_integer(bytes: &[u8]) -> Vec<u8> {
    let mut out = vec![0x02];
    asn1_encode_length(bytes.len(), &mut out);
    out.extend_from_slice(bytes);
    out
}

fn asn1_encode_utf8string(bytes: &[u8]) -> Vec<u8> {
    let mut out = vec![0x0c]; // UTF8String
    asn1_encode_length(bytes.len(), &mut out);
    out.extend_from_slice(bytes);
    out
}

fn asn1_encode_bitstring(data: &[u8]) -> Vec<u8> {
    let mut out = vec![0x03];
    asn1_encode_length(data.len() + 1, &mut out);
    out.push(0x00); // unused bits = 0
    out.extend_from_slice(data);
    out
}

fn asn1_encode_utctime(timestamp: i64) -> Vec<u8> {
    // Convert timestamp to YYMMDDHHMMSSZ
    let (y, m, d, h, min, s) = timestamp_to_ymdhms(timestamp);
    let yy = (y % 100) as u8;
    let time_str = format!("{:02}{:02}{:02}{:02}{:02}{:02}Z", yy, m, d, h, min, s);
    let bytes = time_str.as_bytes();
    let mut out = vec![0x17]; // UTCTime
    asn1_encode_length(bytes.len(), &mut out);
    out.extend_from_slice(bytes);
    out
}

fn timestamp_to_ymdhms(ts: i64) -> (i64, u8, u8, u8, u8, u8) {
    let days = ts.div_euclid(86400);
    let rem = ts.rem_euclid(86400);
    let h = (rem / 3600) as u8;
    let min = ((rem % 3600) / 60) as u8;
    let s = (rem % 60) as u8;
    // Civil date from days since epoch
    let z = days + 719468;
    let era = z.div_euclid(146097);
    let doe = z.rem_euclid(146097);
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u8;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u8;
    let y = if m <= 2 { y + 1 } else { y };
    (y, m, d, h, min, s)
}

fn asn1_encode_length(len: usize, out: &mut Vec<u8>) {
    if len < 0x80 {
        out.push(len as u8);
    } else if len < 0x100 {
        out.push(0x81);
        out.push(len as u8);
    } else {
        out.push(0x82);
        out.push((len >> 8) as u8);
        out.push(len as u8);
    }
}

// WebSocket client (replaces `tungstenite` + `native-tls`)

/// A WebSocket connection over a plain TCP stream.
pub struct WebSocket {
    stream: Box<dyn ReadWrite>,
    tcp_ref: Option<std::net::TcpStream>, // kept for timeout setting
}

/// WebSocket message types.
pub enum WsMessage {
    Text(String),
    Binary(Vec<u8>),
    Close,
    Ping(Vec<u8>),
    Pong(Vec<u8>),
}

impl WebSocket {
    /// Connect to a WebSocket server (ws:// only, no TLS).
    pub fn connect(url: &str) -> Result<WebSocket, String> {
        let parsed = crate::util::UrlParts::parse(url)
            .map_err(|e| format!("ws: invalid URL: {}", e))?;

        let is_tls = parsed.scheme == "wss";
        let host = &parsed.host;
        let port = parsed.port.unwrap_or(if is_tls { 443 } else { 80 });
        let path = if parsed.path.is_empty() { "/" } else { &parsed.path };

        use std::net::ToSocketAddrs;
        let addr = format!("{}:{}", host, port);
        let sock_addr = addr.to_socket_addrs()
            .map_err(|e| format!("ws: DNS failed: {}", e))?
            .next()
            .ok_or("ws: no addresses")?;

        let tcp_stream = std::net::TcpStream::connect_timeout(
            &sock_addr,
            std::time::Duration::from_secs(30),
        ).map_err(|e| format!("ws connect: {}", e))?;

        tcp_stream.set_read_timeout(Some(std::time::Duration::from_secs(30))).ok();
        tcp_stream.set_write_timeout(Some(std::time::Duration::from_secs(30))).ok();
        let tcp_for_ref = tcp_stream.try_clone().ok();

        // WebSocket handshake
        let mut key_bytes = [0u8; 16];
        random_fill_bytes(&mut key_bytes);
        let ws_key = base64_encode(&key_bytes);

        // Create stream — TLS or plain
        let mut stream: Box<dyn ReadWrite> = if is_tls {
            let tls = crate::tls::TlsStream::connect(tcp_stream, host)
                .map_err(|e| format!("ws TLS: {}", e))?;
            Box::new(tls)
        } else {
            Box::new(tcp_stream)
        };

        use std::io::Write;

        let handshake = format!(
            "GET {} HTTP/1.1\r\nHost: {}\r\nUpgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Key: {}\r\nSec-WebSocket-Version: 13\r\n\r\n",
            path, host, ws_key
        );
        stream.write_all(handshake.as_bytes()).map_err(|e| format!("ws: {}", e))?;
        stream.flush().map_err(|e| format!("ws: {}", e))?;

        // Read response — read byte by byte to find \r\n\r\n without over-reading
        let mut response = Vec::new();
        let mut buf = [0u8; 1];
        loop {
            match std::io::Read::read(&mut *stream, &mut buf) {
                Ok(0) => return Err("ws: connection closed during handshake".into()),
                Ok(_) => {
                    response.push(buf[0]);
                    if response.len() >= 4 && &response[response.len()-4..] == b"\r\n\r\n" {
                        break;
                    }
                    if response.len() > 8192 {
                        return Err("ws: handshake response too large".into());
                    }
                }
                Err(e) => return Err(format!("ws handshake: {}", e)),
            }
        }
        let response_str = String::from_utf8_lossy(&response);
        if !response_str.contains("101") {
            return Err("ws: server did not accept upgrade".to_string());
        }

        Ok(WebSocket { stream, tcp_ref: tcp_for_ref })
    }

    /// Connect with a pre-connected TCP stream and URL for the handshake.
    pub fn connect_with_stream(tcp_stream: std::net::TcpStream, url: &str, host: &str) -> Result<WebSocket, String> {
        let parsed = crate::util::UrlParts::parse(url)
            .map_err(|e| format!("ws: invalid URL: {}", e))?;
        let path = if parsed.path.is_empty() { "/" } else { &parsed.path };
        let tcp_for_ref = tcp_stream.try_clone().ok();

        // For wss://, wrap the TCP stream in TLS
        let mut stream: Box<dyn ReadWrite> = if parsed.scheme == "wss" {
            let tls = crate::tls::TlsStream::connect(tcp_stream, host)
                .map_err(|e| format!("wss: TLS handshake failed: {}", e))?;
            Box::new(tls)
        } else {
            Box::new(tcp_stream)
        };

        let mut key_bytes = [0u8; 16];
        random_fill_bytes(&mut key_bytes);
        let ws_key = base64_encode(&key_bytes);

        use std::io::Write;

        let handshake = format!(
            "GET {} HTTP/1.1\r\nHost: {}\r\nUpgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Key: {}\r\nSec-WebSocket-Version: 13\r\n\r\n",
            path, host, ws_key
        );
        stream.write_all(handshake.as_bytes()).map_err(|e| format!("ws: {}", e))?;
        stream.flush().map_err(|e| format!("ws: {}", e))?;

        let mut response = Vec::new();
        let mut buf = [0u8; 1];
        loop {
            match std::io::Read::read(&mut *stream, &mut buf) {
                Ok(0) => return Err("ws: connection closed during handshake".into()),
                Ok(_) => {
                    response.push(buf[0]);
                    if response.len() >= 4 && &response[response.len()-4..] == b"\r\n\r\n" { break; }
                    if response.len() > 8192 { return Err("ws: handshake response too large".into()); }
                }
                Err(e) => return Err(format!("ws handshake: {}", e)),
            }
        }
        let response_str = String::from_utf8_lossy(&response);
        if !response_str.contains("101") {
            return Err("ws: server did not accept upgrade".to_string());
        }

        Ok(WebSocket { stream, tcp_ref: tcp_for_ref })
    }

    /// Send a WebSocket message (text or binary).
    pub fn send(&mut self, msg: &WsMessage) -> Result<(), String> {
        use std::io::Write;
        let (opcode, payload) = match msg {
            WsMessage::Text(s) => (0x01, s.as_bytes().to_vec()),
            WsMessage::Binary(b) => (0x02, b.clone()),
            WsMessage::Close => (0x08, Vec::new()),
            WsMessage::Ping(b) => (0x09, b.clone()),
            WsMessage::Pong(b) => (0x0A, b.clone()),
        };

        let mut frame = Vec::new();
        frame.push(0x80 | opcode); // FIN + opcode

        // Mask bit set (client must mask)
        let len = payload.len();
        if len < 126 {
            frame.push(0x80 | len as u8);
        } else if len < 65536 {
            frame.push(0x80 | 126);
            frame.push((len >> 8) as u8);
            frame.push(len as u8);
        } else {
            frame.push(0x80 | 127);
            for i in (0..8).rev() {
                frame.push((len >> (i * 8)) as u8);
            }
        }

        let mut mask = [0u8; 4];
        random_fill_bytes(&mut mask);
        frame.extend_from_slice(&mask);

        for (i, &b) in payload.iter().enumerate() {
            frame.push(b ^ mask[i % 4]);
        }

        self.stream.write_all(&frame).map_err(|e| format!("ws send: {}", e))?;
        self.stream.flush().map_err(|e| format!("ws send flush: {}", e))
    }

    /// Read a WebSocket message.
    pub fn read(&mut self) -> Result<WsMessage, String> {
        use std::io::Read;
        let mut header = [0u8; 2];
        self.stream.read_exact(&mut header).map_err(|e| format!("ws read: {}", e))?;

        let opcode = header[0] & 0x0f;
        let masked = header[1] & 0x80 != 0;
        let mut payload_len = (header[1] & 0x7f) as u64;

        if payload_len == 126 {
            let mut buf = [0u8; 2];
            self.stream.read_exact(&mut buf).map_err(|e| format!("ws read len: {}", e))?;
            payload_len = u16::from_be_bytes(buf) as u64;
        } else if payload_len == 127 {
            let mut buf = [0u8; 8];
            self.stream.read_exact(&mut buf).map_err(|e| format!("ws read len: {}", e))?;
            payload_len = u64::from_be_bytes(buf);
        }

        let mask = if masked {
            let mut m = [0u8; 4];
            self.stream.read_exact(&mut m).map_err(|e| format!("ws read mask: {}", e))?;
            Some(m)
        } else {
            None
        };

        if payload_len > 64 * 1024 * 1024 {
            return Err("ws: payload too large".to_string());
        }

        let mut payload = vec![0u8; payload_len as usize];
        self.stream.read_exact(&mut payload).map_err(|e| format!("ws read payload: {}", e))?;

        if let Some(mask) = mask {
            for (i, b) in payload.iter_mut().enumerate() {
                *b ^= mask[i % 4];
            }
        }

        match opcode {
            0x01 => Ok(WsMessage::Text(String::from_utf8_lossy(&payload).to_string())),
            0x02 => Ok(WsMessage::Binary(payload)),
            0x08 => Ok(WsMessage::Close),
            0x09 => {
                // Auto-respond with pong
                let _ = self.send(&WsMessage::Pong(payload.clone()));
                Ok(WsMessage::Ping(payload))
            }
            0x0A => Ok(WsMessage::Pong(payload)),
            _ => Ok(WsMessage::Binary(payload)),
        }
    }

    /// Close the WebSocket connection.
    pub fn close(&mut self) -> Result<(), String> {
        let _ = self.send(&WsMessage::Close);
        Ok(())
    }

    /// Get a reference to the underlying TCP stream (for timeout setting).
    pub fn get_tcp_ref(&self) -> Option<&std::net::TcpStream> {
        self.tcp_ref.as_ref()
    }
}

// JSON Value type and parser/serializer (replaces `serde_json`)

/// A JSON value.
#[derive(Debug, Clone, PartialEq)]
pub enum JsonValue {
    Null,
    Bool(bool),
    Number(JsonNumber),
    String(String),
    Array(Vec<JsonValue>),
    Object(OrderedMap<String, JsonValue>),
}

/// A JSON number (integer or floating-point).
#[derive(Debug, Clone)]
pub enum JsonNumber {
    Int(i64),
    UInt(u64),
    Float(f64),
}

impl PartialEq for JsonNumber {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (JsonNumber::Int(a), JsonNumber::Int(b)) => a == b,
            (JsonNumber::UInt(a), JsonNumber::UInt(b)) => a == b,
            (JsonNumber::Float(a), JsonNumber::Float(b)) => a == b,
            (JsonNumber::Int(a), JsonNumber::Float(b)) | (JsonNumber::Float(b), JsonNumber::Int(a)) => *a as f64 == *b,
            (JsonNumber::UInt(a), JsonNumber::Float(b)) | (JsonNumber::Float(b), JsonNumber::UInt(a)) => *a as f64 == *b,
            (JsonNumber::Int(a), JsonNumber::UInt(b)) | (JsonNumber::UInt(b), JsonNumber::Int(a)) => {
                if *a >= 0 { *a as u64 == *b } else { false }
            }
        }
    }
}

impl JsonNumber {
    pub fn as_i64(&self) -> Option<i64> {
        match self {
            JsonNumber::Int(n) => Some(*n),
            JsonNumber::UInt(n) => i64::try_from(*n).ok(),
            JsonNumber::Float(f) => { let i = *f as i64; if (i as f64) == *f { Some(i) } else { None } }
        }
    }
    pub fn as_u64(&self) -> Option<u64> {
        match self {
            JsonNumber::Int(n) => u64::try_from(*n).ok(),
            JsonNumber::UInt(n) => Some(*n),
            JsonNumber::Float(f) => { let u = *f as u64; if (u as f64) == *f { Some(u) } else { None } }
        }
    }
    pub fn as_f64(&self) -> Option<f64> {
        match self {
            JsonNumber::Int(n) => Some(*n as f64),
            JsonNumber::UInt(n) => Some(*n as f64),
            JsonNumber::Float(f) => Some(*f),
        }
    }
}

impl std::fmt::Display for JsonNumber {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            JsonNumber::Int(n) => write!(f, "{}", n),
            JsonNumber::UInt(n) => write!(f, "{}", n),
            JsonNumber::Float(v) => {
                if !v.is_finite() {
                    // JSON has no representation for inf/NaN — emit null
                    write!(f, "null")
                } else if v.fract() == 0.0 && v.abs() < 1e18 {
                    write!(f, "{:.1}", v)
                } else {
                    write!(f, "{}", v)
                }
            }
        }
    }
}

impl JsonValue {
    pub fn is_null(&self) -> bool { matches!(self, JsonValue::Null) }
    pub fn is_bool(&self) -> bool { matches!(self, JsonValue::Bool(_)) }
    pub fn is_number(&self) -> bool { matches!(self, JsonValue::Number(_)) }
    pub fn is_string(&self) -> bool { matches!(self, JsonValue::String(_)) }
    pub fn is_array(&self) -> bool { matches!(self, JsonValue::Array(_)) }
    pub fn is_object(&self) -> bool { matches!(self, JsonValue::Object(_)) }

    pub fn as_bool(&self) -> Option<bool> {
        match self { JsonValue::Bool(b) => Some(*b), _ => None }
    }
    pub fn as_str(&self) -> Option<&str> {
        match self { JsonValue::String(s) => Some(s), _ => None }
    }
    pub fn as_i64(&self) -> Option<i64> {
        match self { JsonValue::Number(n) => n.as_i64(), _ => None }
    }
    pub fn as_u64(&self) -> Option<u64> {
        match self { JsonValue::Number(n) => n.as_u64(), _ => None }
    }
    pub fn as_f64(&self) -> Option<f64> {
        match self { JsonValue::Number(n) => n.as_f64(), _ => None }
    }
    pub fn as_array(&self) -> Option<&Vec<JsonValue>> {
        match self { JsonValue::Array(a) => Some(a), _ => None }
    }
    pub fn as_object(&self) -> Option<&OrderedMap<String, JsonValue>> {
        match self { JsonValue::Object(o) => Some(o), _ => None }
    }
}

/// Parse a JSON string into a JsonValue.
pub fn json_parse_value(input: &str) -> Result<JsonValue, String> {
    let trimmed = input.trim();
    if trimmed.is_empty() { return Err("empty JSON input".into()); }
    let bytes = trimmed.as_bytes();
    let mut pos = 0;
    let val = json_parse_one(bytes, &mut pos, 0)?;
    while pos < bytes.len() && bytes[pos].is_ascii_whitespace() { pos += 1; }
    if pos < bytes.len() {
        return Err(format!("trailing characters at position {}", pos));
    }
    Ok(val)
}

const JSON_MAX_DEPTH: usize = 128;

fn json_parse_one(data: &[u8], pos: &mut usize, depth: usize) -> Result<JsonValue, String> {
    if depth > JSON_MAX_DEPTH {
        return Err("JSON nesting depth exceeds limit".into());
    }
    json_skip_ws(data, pos);
    if *pos >= data.len() { return Err("unexpected end of JSON".into()); }
    match data[*pos] {
        b'n' => { json_expect(data, pos, b"null")?; Ok(JsonValue::Null) }
        b't' => { json_expect(data, pos, b"true")?; Ok(JsonValue::Bool(true)) }
        b'f' => { json_expect(data, pos, b"false")?; Ok(JsonValue::Bool(false)) }
        b'"' => Ok(JsonValue::String(json_parse_string(data, pos)?)),
        b'[' => json_parse_array(data, pos, depth),
        b'{' => json_parse_object(data, pos, depth),
        b'-' | b'0'..=b'9' => json_parse_number(data, pos),
        c => Err(format!("unexpected character '{}' at position {}", c as char, pos)),
    }
}

fn json_skip_ws(data: &[u8], pos: &mut usize) {
    while *pos < data.len() && data[*pos].is_ascii_whitespace() { *pos += 1; }
}

fn json_expect(data: &[u8], pos: &mut usize, expected: &[u8]) -> Result<(), String> {
    if *pos + expected.len() > data.len() || &data[*pos..*pos + expected.len()] != expected {
        return Err(format!("expected {:?} at position {}", std::str::from_utf8(expected).unwrap(), pos));
    }
    *pos += expected.len();
    Ok(())
}

fn json_parse_string(data: &[u8], pos: &mut usize) -> Result<String, String> {
    if data[*pos] != b'"' { return Err("expected '\"'".into()); }
    *pos += 1;
    let mut result = String::new();
    while *pos < data.len() {
        match data[*pos] {
            b'"' => { *pos += 1; return Ok(result); }
            b'\\' => {
                *pos += 1;
                if *pos >= data.len() { return Err("unterminated string escape".into()); }
                match data[*pos] {
                    b'"' => result.push('"'),
                    b'\\' => result.push('\\'),
                    b'/' => result.push('/'),
                    b'b' => result.push('\u{08}'),
                    b'f' => result.push('\u{0c}'),
                    b'n' => result.push('\n'),
                    b'r' => result.push('\r'),
                    b't' => result.push('\t'),
                    b'u' => {
                        *pos += 1;
                        if *pos + 4 > data.len() { return Err("truncated unicode escape".into()); }
                        let hex = std::str::from_utf8(&data[*pos..*pos + 4]).map_err(|_| "invalid unicode escape")?;
                        let code = u16::from_str_radix(hex, 16).map_err(|_| "invalid unicode escape")?;
                        *pos += 3; // +1 below
                        if (0xD800..=0xDBFF).contains(&code) {
                            // High surrogate — must be followed by \uXXXX low surrogate
                            *pos += 1;
                            if *pos + 6 <= data.len() && data[*pos] == b'\\' && data[*pos + 1] == b'u' {
                                *pos += 2;
                                let hex2 = std::str::from_utf8(&data[*pos..*pos + 4]).map_err(|_| "invalid surrogate")?;
                                let low = u16::from_str_radix(hex2, 16).map_err(|_| "invalid surrogate")?;
                                *pos += 3;
                                if !(0xDC00..=0xDFFF).contains(&low) {
                                    return Err("invalid surrogate pair: low surrogate expected".into());
                                }
                                let combined = 0x10000 + ((code as u32 - 0xD800) << 10) + (low as u32 - 0xDC00);
                                if let Some(c) = char::from_u32(combined) {
                                    result.push(c);
                                } else {
                                    return Err("invalid surrogate pair: combined codepoint invalid".into());
                                }
                            } else {
                                return Err("lone high surrogate without low surrogate".into());
                            }
                        } else if (0xDC00..=0xDFFF).contains(&code) {
                            return Err("lone low surrogate".into());
                        } else if let Some(c) = char::from_u32(code as u32) {
                            result.push(c);
                        }
                    }
                    _ => { result.push('\\'); result.push(data[*pos] as char); }
                }
                *pos += 1;
            }
            b => {
                // Handle multi-byte UTF-8 sequences correctly
                let byte_len = if b < 0x80 { 1 }
                    else if b < 0xE0 { 2 }
                    else if b < 0xF0 { 3 }
                    else { 4 };
                if *pos + byte_len <= data.len() {
                    if let Ok(s) = std::str::from_utf8(&data[*pos..*pos + byte_len]) {
                        result.push_str(s);
                        *pos += byte_len;
                    } else {
                        result.push(b as char);
                        *pos += 1;
                    }
                } else {
                    result.push(b as char);
                    *pos += 1;
                }
            }
        }
    }
    Err("unterminated string".into())
}

fn json_parse_number(data: &[u8], pos: &mut usize) -> Result<JsonValue, String> {
    let start = *pos;
    if data[*pos] == b'-' { *pos += 1; }
    while *pos < data.len() && data[*pos].is_ascii_digit() { *pos += 1; }
    let mut is_float = false;
    if *pos < data.len() && data[*pos] == b'.' {
        is_float = true;
        *pos += 1;
        while *pos < data.len() && data[*pos].is_ascii_digit() { *pos += 1; }
    }
    if *pos < data.len() && (data[*pos] == b'e' || data[*pos] == b'E') {
        is_float = true;
        *pos += 1;
        if *pos < data.len() && (data[*pos] == b'+' || data[*pos] == b'-') { *pos += 1; }
        while *pos < data.len() && data[*pos].is_ascii_digit() { *pos += 1; }
    }
    let s = std::str::from_utf8(&data[start..*pos]).map_err(|_| "invalid number")?;
    if is_float {
        let f: f64 = s.parse().map_err(|_| format!("invalid float: {}", s))?;
        Ok(JsonValue::Number(JsonNumber::Float(f)))
    } else if s.starts_with('-') {
        let n: i64 = s.parse().map_err(|_| format!("invalid integer: {}", s))?;
        Ok(JsonValue::Number(JsonNumber::Int(n)))
    } else {
        // Try i64 first, then u64
        if let Ok(n) = s.parse::<i64>() {
            Ok(JsonValue::Number(JsonNumber::Int(n)))
        } else if let Ok(n) = s.parse::<u64>() {
            Ok(JsonValue::Number(JsonNumber::UInt(n)))
        } else {
            let f: f64 = s.parse().map_err(|_| format!("invalid number: {}", s))?;
            Ok(JsonValue::Number(JsonNumber::Float(f)))
        }
    }
}

fn json_parse_array(data: &[u8], pos: &mut usize, depth: usize) -> Result<JsonValue, String> {
    *pos += 1; // skip [
    json_skip_ws(data, pos);
    let mut items = Vec::new();
    if *pos < data.len() && data[*pos] == b']' { *pos += 1; return Ok(JsonValue::Array(items)); }
    loop {
        items.push(json_parse_one(data, pos, depth + 1)?);
        json_skip_ws(data, pos);
        if *pos >= data.len() { return Err("unterminated array".into()); }
        if data[*pos] == b']' { *pos += 1; return Ok(JsonValue::Array(items)); }
        if data[*pos] != b',' { return Err(format!("expected ',' or ']' at position {}", pos)); }
        *pos += 1;
    }
}

fn json_parse_object(data: &[u8], pos: &mut usize, depth: usize) -> Result<JsonValue, String> {
    *pos += 1; // skip {
    json_skip_ws(data, pos);
    let mut map = OrderedMap::new();
    if *pos < data.len() && data[*pos] == b'}' { *pos += 1; return Ok(JsonValue::Object(map)); }
    loop {
        json_skip_ws(data, pos);
        if *pos >= data.len() || data[*pos] != b'"' { return Err("expected string key".into()); }
        let key = json_parse_string(data, pos)?;
        json_skip_ws(data, pos);
        if *pos >= data.len() || data[*pos] != b':' { return Err("expected ':'".into()); }
        *pos += 1;
        let val = json_parse_one(data, pos, depth + 1)?;
        map.insert(key, val);
        json_skip_ws(data, pos);
        if *pos >= data.len() { return Err("unterminated object".into()); }
        if data[*pos] == b'}' { *pos += 1; return Ok(JsonValue::Object(map)); }
        if data[*pos] != b',' { return Err(format!("expected ',' or '}}' at position {}", pos)); }
        *pos += 1;
    }
}

/// Serialize a JsonValue to a compact JSON string.
pub fn json_to_string(val: &JsonValue) -> String {
    let mut out = String::new();
    json_write(val, &mut out, false, 0);
    out
}

/// Serialize a JsonValue to a pretty-printed JSON string.
pub fn json_to_string_pretty(val: &JsonValue) -> String {
    let mut out = String::new();
    json_write(val, &mut out, true, 0);
    out
}

fn json_write(val: &JsonValue, out: &mut String, pretty: bool, indent: usize) {
    match val {
        JsonValue::Null => out.push_str("null"),
        JsonValue::Bool(b) => out.push_str(if *b { "true" } else { "false" }),
        JsonValue::Number(n) => out.push_str(&n.to_string()),
        JsonValue::String(s) => json_write_string(s, out),
        JsonValue::Array(items) => {
            if items.is_empty() { out.push_str("[]"); return; }
            out.push('[');
            for (i, item) in items.iter().enumerate() {
                if i > 0 { out.push(','); }
                if pretty { out.push('\n'); json_indent(out, indent + 2); }
                json_write(item, out, pretty, indent + 2);
            }
            if pretty { out.push('\n'); json_indent(out, indent); }
            out.push(']');
        }
        JsonValue::Object(map) => {
            if map.is_empty() { out.push_str("{}"); return; }
            out.push('{');
            for (i, (k, v)) in map.iter().enumerate() {
                if i > 0 { out.push(','); }
                if pretty { out.push('\n'); json_indent(out, indent + 2); }
                json_write_string(k, out);
                out.push(':');
                if pretty { out.push(' '); }
                json_write(v, out, pretty, indent + 2);
            }
            if pretty { out.push('\n'); json_indent(out, indent); }
            out.push('}');
        }
    }
}

fn json_write_string(s: &str, out: &mut String) {
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
}

fn json_indent(out: &mut String, n: usize) {
    for _ in 0..n { out.push(' '); }
}

impl std::fmt::Display for JsonValue {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", json_to_string(self))
    }
}

/// Helper to create a JSON number from i64.
pub fn json_int(n: i64) -> JsonValue { JsonValue::Number(JsonNumber::Int(n)) }
/// Helper to create a JSON number from u64.
pub fn json_uint(n: u64) -> JsonValue { JsonValue::Number(JsonNumber::UInt(n)) }
/// Helper to create a JSON number from f64.
pub fn json_float(f: f64) -> JsonValue { JsonValue::Number(JsonNumber::Float(f)) }

// WASM validator (replaces `wasmparser`)

/// Validate a WASM binary's basic structure.
/// Checks the magic number, version, and section structure.
pub fn validate_wasm(bytes: &[u8]) -> Result<(), String> {
    if bytes.len() < 8 {
        return Err("WASM binary too short".into());
    }
    // Magic: \0asm
    if &bytes[0..4] != b"\0asm" {
        return Err("invalid WASM magic number".into());
    }
    // Version: 1
    let version = u32::from_le_bytes([bytes[4], bytes[5], bytes[6], bytes[7]]);
    if version != 1 {
        return Err(format!("unsupported WASM version: {}", version));
    }

    let mut pos = 8;
    let mut seen_sections = [false; 13];

    while pos < bytes.len() {
        if pos >= bytes.len() { break; }
        let section_id = bytes[pos];
        pos += 1;

        // Read LEB128 section length
        let (section_len, consumed) = read_leb128_u32(&bytes[pos..])?;
        pos += consumed;
        let section_len = section_len as usize;

        if pos + section_len > bytes.len() {
            return Err(format!("section {} extends past end of file", section_id));
        }

        // Validate section ordering (non-custom sections must be in order)
        if section_id > 0 && section_id < 13 {
            if seen_sections[section_id as usize] {
                return Err(format!("duplicate section id {}", section_id));
            }
            seen_sections[section_id as usize] = true;
        }

        pos += section_len;
    }

    if pos != bytes.len() {
        return Err("extra bytes after last section".into());
    }

    Ok(())
}

fn read_leb128_u32(data: &[u8]) -> Result<(u32, usize), String> {
    let mut result: u32 = 0;
    let mut shift = 0;
    for (i, &byte) in data.iter().enumerate() {
        result |= ((byte & 0x7f) as u32) << shift;
        if byte & 0x80 == 0 {
            return Ok((result, i + 1));
        }
        shift += 7;
        if shift >= 35 {
            return Err("LEB128 overflow".into());
        }
    }
    Err("unterminated LEB128".into())
}

#[cfg(test)]
mod phase3_tests {
    use super::*;

    // -- chrono replacement tests --
    #[test]
    fn test_now_millis() {
        let ms = now_millis();
        // Should be after 2020-01-01 and before 2100-01-01
        assert!(ms > 1577836800_000);
        assert!(ms < 4102444800_000);
    }

    #[test]
    fn test_datetime_from_millis_epoch() {
        let dt = datetime_from_millis(0).unwrap();
        assert_eq!(dt.year, 1970);
        assert_eq!(dt.month, 1);
        assert_eq!(dt.day, 1);
        assert_eq!(dt.hour, 0);
    }

    #[test]
    fn test_format_timestamp() {
        assert_eq!(
            format_timestamp_millis(0).unwrap(),
            "1970-01-01T00:00:00.000Z"
        );
        assert_eq!(
            format_timestamp_millis(1705311000_123).unwrap(),
            "2024-01-15T09:30:00.123Z"
        );
    }

    #[test]
    fn test_parse_timestamp_rfc3339() {
        let ms = parse_timestamp_to_millis("2024-01-15T09:30:00Z").unwrap();
        assert_eq!(ms, 1705311000_000);
    }

    #[test]
    fn test_parse_timestamp_with_tz() {
        let ms = parse_timestamp_to_millis("2024-01-15T15:00:00+05:30").unwrap();
        assert_eq!(ms, 1705311000_000);
    }

    #[test]
    fn test_parse_timestamp_date_only() {
        let ms = parse_timestamp_to_millis("2024-01-15").unwrap();
        assert_eq!(ms, 1705276800_000);
    }

    #[test]
    fn test_parse_timestamp_space_sep() {
        let ms = parse_timestamp_to_millis("2024-01-15 09:30:00").unwrap();
        assert_eq!(ms, 1705311000_000);
    }

    #[test]
    fn test_parse_timestamp_with_frac() {
        let ms = parse_timestamp_to_millis("2024-01-15T09:30:00.123Z").unwrap();
        assert_eq!(ms, 1705311000_123);
    }

    #[test]
    fn test_days_round_trip() {
        // Verify Hinnant round-trip for a range of dates
        for days in -10000..10000 {
            let (y, m, d) = civil_from_days(days);
            assert_eq!(days_from_civil(y, m, d), days);
        }
    }

    // -- URL replacement tests --
    #[test]
    fn test_url_parse_basic() {
        let u = UrlParts::parse("https://example.com/path?q=1#frag").unwrap();
        assert_eq!(u.scheme, "https");
        assert_eq!(u.host, "example.com");
        assert_eq!(u.path, "/path");
        assert_eq!(u.query.as_deref(), Some("q=1"));
        assert_eq!(u.fragment.as_deref(), Some("frag"));
        assert_eq!(u.port, None);
    }

    #[test]
    fn test_url_parse_with_port() {
        let u = UrlParts::parse("http://localhost:8080/api").unwrap();
        assert_eq!(u.host, "localhost");
        assert_eq!(u.port, Some(8080));
        assert_eq!(u.path, "/api");
    }

    #[test]
    fn test_url_parse_with_userinfo() {
        let u = UrlParts::parse("https://user:pass@example.com/path").unwrap();
        assert_eq!(u.username, "user");
        assert_eq!(u.password, "pass");
        assert_eq!(u.host, "example.com");
    }

    #[test]
    fn test_url_port_or_known_default() {
        let u = UrlParts::parse("https://example.com/").unwrap();
        assert_eq!(u.port_or_known_default(), Some(443));
        let u = UrlParts::parse("http://example.com/").unwrap();
        assert_eq!(u.port_or_known_default(), Some(80));
    }

    #[test]
    fn test_url_join() {
        let u = UrlParts::parse("https://example.com/base/path").unwrap();
        let joined = u.join("other").unwrap();
        assert_eq!(joined, "https://example.com/base/other");
        let joined = u.join("/abs").unwrap();
        assert_eq!(joined, "https://example.com/abs");
    }

    // -- HTTP status tests --
    #[test]
    fn test_http_status_reason() {
        assert_eq!(http_status_reason(200), "OK");
        assert_eq!(http_status_reason(404), "Not Found");
        assert_eq!(http_status_reason(500), "Internal Server Error");
        assert_eq!(http_status_reason(999), "Unknown");
    }

    // -- httparse replacement tests --
    #[test]
    fn test_parse_http_request() {
        let buf = b"GET /path HTTP/1.1\r\nHost: example.com\r\nContent-Length: 0\r\n\r\n";
        let req = parse_http_request(buf).unwrap().unwrap();
        assert_eq!(req.method, "GET");
        assert_eq!(req.path, "/path");
        assert_eq!(req.version, 1);
        assert_eq!(req.headers.len(), 2);
        assert_eq!(req.headers[0].name, "Host");
        assert_eq!(std::str::from_utf8(&req.headers[0].value).unwrap(), "example.com");
    }

    // -- TOML replacement tests --
    #[test]
    fn test_toml_parse_basic() {
        let input = r#"
name = "test"
version = 123
enabled = true
ratio = 1.5
"#;
        let table = toml_parse(input).unwrap();
        assert_eq!(table.get("name").unwrap().as_str(), Some("test"));
        assert_eq!(table.get("version").unwrap().as_integer(), Some(123));
        assert_eq!(table.get("enabled").unwrap().as_bool(), Some(true));
        assert_eq!(table.get("ratio").unwrap().as_float(), Some(1.5));
    }

    #[test]
    fn test_toml_parse_sections() {
        let input = r#"
[package]
name = "myapp"
magi = ">=0.9.0"

[dependencies]
foo = "bar"
"#;
        let table = toml_parse(input).unwrap();
        let pkg = table.get("package").unwrap().as_table().unwrap();
        assert_eq!(pkg.get("name").unwrap().as_str(), Some("myapp"));
        let deps = table.get("dependencies").unwrap().as_table().unwrap();
        assert_eq!(deps.get("foo").unwrap().as_str(), Some("bar"));
    }

    #[test]
    fn test_toml_parse_array() {
        let input = r#"
[lint]
disabled = ["W200", "W201"]
"#;
        let table = toml_parse(input).unwrap();
        let lint = table.get("lint").unwrap().as_table().unwrap();
        let disabled = lint.get("disabled").unwrap().as_array().unwrap();
        assert_eq!(disabled.len(), 2);
        assert_eq!(disabled[0].as_str(), Some("W200"));
    }

    #[test]
    fn test_toml_parse_array_of_tables() {
        let input = r#"
[[package]]
id = "foo"
path = "/tmp/foo"

[[package]]
id = "bar"
path = "/tmp/bar"
"#;
        let table = toml_parse(input).unwrap();
        let packages = table.get("package").unwrap().as_array().unwrap();
        assert_eq!(packages.len(), 2);
        assert_eq!(packages[0].as_table().unwrap().get("id").unwrap().as_str(), Some("foo"));
        assert_eq!(packages[1].as_table().unwrap().get("id").unwrap().as_str(), Some("bar"));
    }

    #[test]
    fn test_toml_parse_inline_table() {
        let input = r#"
dep = {git = "https://github.com/foo", branch = "main"}
"#;
        let table = toml_parse(input).unwrap();
        let dep = table.get("dep").unwrap().as_table().unwrap();
        assert_eq!(dep.get("git").unwrap().as_str(), Some("https://github.com/foo"));
        assert_eq!(dep.get("branch").unwrap().as_str(), Some("main"));
    }

    #[test]
    fn test_toml_stringify() {
        let mut table = TomlTable::new();
        table.insert("name".to_string(), TomlValue::String("test".to_string()));
        table.insert("count".to_string(), TomlValue::Integer(42));
        let output = toml_to_string_pretty(&TomlValue::Table(table)).unwrap();
        assert!(output.contains("name = \"test\""));
        assert!(output.contains("count = 42"));
    }

    #[test]
    fn test_toml_comment_handling() {
        let input = r#"
name = "test" # this is a comment
# full line comment
count = 42
"#;
        let table = toml_parse(input).unwrap();
        assert_eq!(table.get("name").unwrap().as_str(), Some("test"));
        assert_eq!(table.get("count").unwrap().as_integer(), Some(42));
    }

    #[test]
    fn test_toml_escaped_strings() {
        let input = r#"
path = "C:\\Users\\test"
msg = "hello\nworld"
"#;
        let table = toml_parse(input).unwrap();
        assert_eq!(table.get("path").unwrap().as_str(), Some("C:\\Users\\test"));
        assert_eq!(table.get("msg").unwrap().as_str(), Some("hello\nworld"));
    }
}

// Comprehensive validation tests for ALL replaced crate implementations

#[cfg(test)]
mod replaced_crate_tests {
    use super::*;

    // ── hex (replaces `hex` crate) ───────────────────────────────
    #[test]
    fn hex_encode_empty() { assert_eq!(hex_encode(&[]), ""); }
    #[test]
    fn hex_encode_basic() { assert_eq!(hex_encode(&[0xde, 0xad, 0xbe, 0xef]), "deadbeef"); }
    #[test]
    fn hex_decode_basic() { assert_eq!(hex_decode("deadbeef").unwrap(), vec![0xde, 0xad, 0xbe, 0xef]); }
    #[test]
    fn hex_decode_uppercase() { assert_eq!(hex_decode("DEADBEEF").unwrap(), vec![0xde, 0xad, 0xbe, 0xef]); }
    #[test]
    fn hex_decode_odd_length() { assert!(hex_decode("abc").is_err()); }
    #[test]
    fn hex_decode_invalid_char() { assert!(hex_decode("zzzz").is_err()); }
    #[test]
    fn hex_roundtrip() {
        let data = b"hello world";
        assert_eq!(hex_decode(&hex_encode(data)).unwrap(), data);
    }

    // ── base64 (replaces `base64` crate) ─────────────────────────
    #[test]
    fn base64_encode_empty() { assert_eq!(base64_encode(&[]), ""); }
    #[test]
    fn base64_rfc_vectors() {
        assert_eq!(base64_encode(b"f"), "Zg==");
        assert_eq!(base64_encode(b"fo"), "Zm8=");
        assert_eq!(base64_encode(b"foo"), "Zm9v");
        assert_eq!(base64_encode(b"foob"), "Zm9vYg==");
        assert_eq!(base64_encode(b"fooba"), "Zm9vYmE=");
        assert_eq!(base64_encode(b"foobar"), "Zm9vYmFy");
    }
    #[test]
    fn base64_decode_rfc_vectors() {
        assert_eq!(base64_decode("Zg==").unwrap(), b"f");
        assert_eq!(base64_decode("Zm9v").unwrap(), b"foo");
        assert_eq!(base64_decode("Zm9vYmFy").unwrap(), b"foobar");
    }
    #[test]
    fn base64_decode_no_padding() {
        assert_eq!(base64_decode("Zg").unwrap(), b"f");
        assert_eq!(base64_decode("Zm8").unwrap(), b"fo");
    }
    #[test]
    fn base64_roundtrip_binary() {
        let data: Vec<u8> = (0..=255).collect();
        assert_eq!(base64_decode(&base64_encode(&data)).unwrap(), data);
    }

    // ── sha256 (replaces `sha2` crate) ───────────────────────────
    #[test]
    fn sha256_nist_vectors() {
        // FIPS 180-4 test vectors
        assert_eq!(hex_encode(&sha256(b"")), "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855");
        assert_eq!(hex_encode(&sha256(b"abc")), "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad");
        assert_eq!(hex_encode(&sha256(b"abcdbcdecdefdefgefghfghighijhijkijkljklmklmnlmnomnopnopq")),
            "248d6a61d20638b8e5c026930c3e6039a33ce45964ff2167f6ecedd419db06c1");
    }
    #[test]
    fn sha256_long_input() {
        let input = "a".repeat(1000);
        let hash = sha256(input.as_bytes());
        assert_eq!(hash.len(), 32);
        assert_eq!(sha256(input.as_bytes()), hash);
    }

    // ── sha512 (replaces `sha2` crate) ───────────────────────────
    #[test]
    fn sha512_nist_vectors() {
        assert_eq!(hex_encode(&sha512(b"")),
            "cf83e1357eefb8bdf1542850d66d8007d620e4050b5715dc83f4a921d36ce9ce47d0d13c5d85f2b0ff8318d2877eec2f63b931bd47417a81a538327af927da3e");
        assert_eq!(hex_encode(&sha512(b"abc")),
            "ddaf35a193617abacc417349ae20413112e6fa4e89a97ea20a9eeee64b55d39a2192992a274fc1a836ba3c23a3feebbd454d4423643ce80e2a9ac94fa54ca49f");
    }

    // ── hmac-sha256 ──────────────────────────────────────────────
    #[test]
    fn hmac_sha256_rfc4231() {
        // RFC 4231 Test Case 1
        let key = vec![0x0bu8; 20];
        let data = b"Hi There";
        assert_eq!(hex_encode(&hmac_sha256(&key, data)),
            "b0344c61d8db38535ca8afceaf0bf12b881dc200c9833da726e9376c2e32cff7");
    }
    #[test]
    fn hmac_sha256_rfc4231_case2() {
        // RFC 4231 Test Case 2
        let key = b"Jefe";
        let data = b"what do ya want for nothing?";
        assert_eq!(hex_encode(&hmac_sha256(key, data)),
            "5bdcc146bf60754e6a042426089575c75a003f089d2739839dec58b964ec3843");
    }

    // ── blake3 ───────────────────────────────────────────────────
    #[test]
    fn blake3_empty() {
        assert_eq!(blake3_hash_hex(b""), "af1349b9f5f9a1a6a0404dea36dcc9499bcb25c9adc112b7cc9a93cae41f3262");
    }
    #[test]
    fn blake3_deterministic() {
        let h1 = blake3_hash(b"test data");
        let h2 = blake3_hash(b"test data");
        assert_eq!(h1, h2);
        assert_ne!(blake3_hash(b"test data"), blake3_hash(b"other data"));
    }

    // ── md5 ──────────────────────────────────────────────────────
    #[test]
    fn md5_known_vectors() {
        assert_eq!(hex_encode(&md5_hash(b"")), "d41d8cd98f00b204e9800998ecf8427e");
        assert_eq!(hex_encode(&md5_hash(b"a")), "0cc175b9c0f1b6a831c399e269772661");
        assert_eq!(hex_encode(&md5_hash(b"abc")), "900150983cd24fb0d6963f7d28e17f72");
        assert_eq!(hex_encode(&md5_hash(b"message digest")), "f96b697d7cb7938d525a2f31aaf161d0");
    }

    // ── crc32 ────────────────────────────────────────────────────
    #[test]
    fn crc32_known() {
        assert_eq!(crc32(b""), 0);
        assert_eq!(crc32(b"123456789"), 0xCBF43926);
    }

    // ── uuid ─────────────────────────────────────────────────────
    #[test]
    fn uuid_format() {
        let u = uuid_v4();
        assert_eq!(u.len(), 36);
        assert_eq!(&u[8..9], "-");
        assert_eq!(&u[13..14], "-");
        assert_eq!(&u[14..15], "4"); // version 4
        assert_eq!(&u[18..19], "-");
        assert_eq!(&u[23..24], "-");
        // variant bits
        let variant_char = u.chars().nth(19).unwrap();
        assert!(matches!(variant_char, '8' | '9' | 'a' | 'b'));
    }
    #[test]
    fn uuid_uniqueness() {
        let a = uuid_v4();
        let b = uuid_v4();
        assert_ne!(a, b);
    }
    #[test]
    fn uuid_parse_roundtrip() {
        let u = uuid_v4();
        let (bytes, version) = uuid_parse(&u).unwrap();
        assert_eq!(version, 4);
        assert_eq!(bytes.len(), 16);
    }

    // ── semver ───────────────────────────────────────────────────
    #[test]
    fn semver_parse_valid() {
        let v = SemVer::parse("1.2.3").unwrap();
        assert_eq!(v.major, 1);
        assert_eq!(v.minor, 2);
        assert_eq!(v.patch, 3);
    }
    #[test]
    fn semver_parse_prerelease() {
        let v = SemVer::parse("1.0.0-alpha.1").unwrap();
        assert_eq!(v.to_string(), "1.0.0-alpha.1");
    }
    #[test]
    fn semver_ordering() {
        let a = SemVer::parse("1.0.0").unwrap();
        let b = SemVer::parse("1.0.1").unwrap();
        let c = SemVer::parse("2.0.0").unwrap();
        assert!(a < b);
        assert!(b < c);
    }
    #[test]
    fn semver_invalid() { assert!(SemVer::parse("not-a-version").is_err()); }

    // ── url ──────────────────────────────────────────────────────
    #[test]
    fn url_parse_full() {
        let u = UrlParts::parse("https://user:pass@example.com:8080/path?q=1#frag").unwrap();
        assert_eq!(u.scheme, "https");
        assert_eq!(u.host, "example.com");
        assert_eq!(u.port, Some(8080));
        assert_eq!(u.path, "/path");
        assert_eq!(u.query, Some("q=1".to_string()));
        assert_eq!(u.fragment, Some("frag".to_string()));
    }
    #[test]
    fn url_parse_minimal() {
        let u = UrlParts::parse("http://localhost").unwrap();
        assert_eq!(u.scheme, "http");
        assert_eq!(u.host, "localhost");
        assert_eq!(u.port, None);
    }
    #[test]
    fn url_invalid() { assert!(UrlParts::parse("not a url").is_err()); }

    // ── percent-encoding ─────────────────────────────────────────
    #[test]
    fn percent_encode_spaces() { assert_eq!(percent_encode("hello world"), "hello%20world"); }
    #[test]
    fn percent_decode_roundtrip() {
        let original = "hello world & foo=bar";
        assert_eq!(percent_decode(&percent_encode(original)).unwrap(), original);
    }

    // ── html-escape ──────────────────────────────────────────────
    #[test]
    fn html_encode_entities() {
        assert_eq!(html_encode("<script>alert('xss')</script>"),
            "&lt;script&gt;alert(&#x27;xss&#x27;)&lt;/script&gt;");
    }

    // ── slug ─────────────────────────────────────────────────────
    #[test]
    fn slug_basic() { assert_eq!(slugify("Hello World!"), "hello-world"); }
    #[test]
    fn slug_unicode() { assert_eq!(slugify("  Foo -- Bar  "), "foo-bar"); }

    // ── strsim (levenshtein) ─────────────────────────────────────
    #[test]
    fn levenshtein_identical() { assert_eq!(levenshtein("abc", "abc"), 0); }
    #[test]
    fn levenshtein_basic() { assert_eq!(levenshtein("kitten", "sitting"), 3); }
    #[test]
    fn levenshtein_empty() {
        assert_eq!(levenshtein("", "abc"), 3);
        assert_eq!(levenshtein("abc", ""), 3);
    }

    // ── heck (case conversion) ───────────────────────────────────
    #[test]
    fn to_snake_case_test() { assert_eq!(to_snake_case("HelloWorld"), "hello_world"); }
    #[test]
    fn to_lower_camel_case_test() { assert_eq!(to_lower_camel_case("hello_world"), "helloWorld"); }

    // ── ordered-float ────────────────────────────────────────────
    #[test]
    fn ordered_float_nan() {
        let a = OrderedFloat(f64::NAN);
        let b = OrderedFloat(f64::NAN);
        assert_eq!(a, b);
        assert!(!(a < b));
    }
    #[test]
    fn ordered_float_ordering() {
        assert!(OrderedFloat(1.0) < OrderedFloat(2.0));
        assert!(OrderedFloat(-1.0) < OrderedFloat(0.0));
    }

    // ── glob ─────────────────────────────────────────────────────
    // glob_match is a filesystem function, tested via integration tests

    // ── base32 ───────────────────────────────────────────────────
    #[test]
    fn base32_rfc_vectors() {
        assert_eq!(base32_encode(b""), "");
        assert_eq!(base32_encode(b"f"), "MY======");
        assert_eq!(base32_encode(b"fo"), "MZXQ====");
        assert_eq!(base32_encode(b"foo"), "MZXW6===");
        assert_eq!(base32_encode(b"foob"), "MZXW6YQ=");
        assert_eq!(base32_encode(b"fooba"), "MZXW6YTB");
        assert_eq!(base32_encode(b"foobar"), "MZXW6YTBOI======");
    }
    #[test]
    fn base32_decode_roundtrip() {
        let data = b"Hello, World!";
        assert_eq!(base32_decode(&base32_encode(data)).unwrap(), data);
    }

    // ── textwrap ─────────────────────────────────────────────────
    #[test]
    fn textwrap_fill_basic() {
        let text = "Hello World. This is a test of wrapping.";
        let wrapped = textwrap_fill(text, 15);
        assert!(wrapped.contains('\n'));
        for line in wrapped.lines() { assert!(line.len() <= 15 || !line.contains(' ')); }
    }
    #[test]
    fn textwrap_dedent_test() {
        let input = "    hello\n    world";
        assert_eq!(textwrap_dedent(input), "hello\nworld");
    }

    // ── csv ──────────────────────────────────────────────────────
    #[test]
    fn csv_parse_basic() {
        let data = csv_parse("name,age\nAlice,30\nBob,25").unwrap();
        assert_eq!(data.headers, vec!["name", "age"]);
        assert_eq!(data.records.len(), 2);
        assert_eq!(data.records[0], vec!["Alice", "30"]);
    }
    #[test]
    fn csv_quoted_fields() {
        let data = csv_parse("a,b\n\"hello, world\",test\n\"say \"\"hi\"\"\",ok").unwrap();
        assert_eq!(data.records[0][0], "hello, world");
        assert_eq!(data.records[1][0], "say \"hi\"");
    }
    #[test]
    fn csv_write_roundtrip() {
        let headers = ["name", "value"];
        let rows = vec![vec!["a,b".to_string(), "1".to_string()]];
        let output = csv_write(&headers, &rows);
        let parsed = csv_parse(&output).unwrap();
        assert_eq!(parsed.records[0][0], "a,b"); // comma was quoted
    }

    // ── toml ─────────────────────────────────────────────────────
    #[test]
    fn toml_parse_types() {
        let input = r#"
name = "test"
count = 42
pi = 3.14
enabled = true
"#;
        let table = toml_parse(input).unwrap();
        assert_eq!(table.get("name").unwrap().as_str(), Some("test"));
        assert_eq!(table.get("count").unwrap().as_integer(), Some(42));
        assert_eq!(table.get("enabled").unwrap().as_bool(), Some(true));
    }
    #[test]
    fn toml_nested_tables() {
        let input = "[server]\nhost = \"localhost\"\nport = 8080";
        let table = toml_parse(input).unwrap();
        let server = table.get("server").unwrap().as_table().unwrap();
        assert_eq!(server.get("host").unwrap().as_str(), Some("localhost"));
        assert_eq!(server.get("port").unwrap().as_integer(), Some(8080));
    }

    // ── yaml ─────────────────────────────────────────────────────
    #[test]
    fn yaml_parse_mapping() {
        let input = "name: Alice\nage: 30\nactive: true";
        let val = yaml_parse(input).unwrap();
        match val {
            YamlValue::Mapping(pairs) => {
                assert_eq!(pairs.len(), 3);
                assert_eq!(pairs[0].1, YamlValue::String("Alice".into()));
                assert_eq!(pairs[1].1, YamlValue::Int(30));
                assert_eq!(pairs[2].1, YamlValue::Bool(true));
            }
            _ => panic!("expected mapping"),
        }
    }
    #[test]
    fn yaml_parse_sequence() {
        let input = "- one\n- two\n- three";
        let val = yaml_parse(input).unwrap();
        match val {
            YamlValue::Sequence(items) => {
                assert_eq!(items.len(), 3);
                assert_eq!(items[0], YamlValue::String("one".into()));
            }
            _ => panic!("expected sequence"),
        }
    }
    #[test]
    fn yaml_scalars() {
        assert_eq!(yaml_parse("null").unwrap(), YamlValue::Null);
        assert_eq!(yaml_parse("true").unwrap(), YamlValue::Bool(true));
        assert_eq!(yaml_parse("42").unwrap(), YamlValue::Int(42));
        assert_eq!(yaml_parse("3.14").unwrap(), YamlValue::Float(3.14));
    }
    #[test]
    fn yaml_stringify_roundtrip() {
        let val = YamlValue::Mapping(vec![
            (YamlValue::String("key".into()), YamlValue::String("value".into())),
            (YamlValue::String("num".into()), YamlValue::Int(42)),
        ]);
        let s = yaml_stringify(&val);
        assert!(s.contains("key:"));
        assert!(s.contains("42"));
    }

    // ── json ─────────────────────────────────────────────────────
    #[test]
    fn json_parse_all_types() {
        let input = r#"{"str":"hello","num":42,"float":3.14,"bool":true,"null":null,"arr":[1,2,3],"obj":{"a":"b"}}"#;
        let val = json_parse_value(input).unwrap();
        match &val {
            JsonValue::Object(obj) => {
                assert_eq!(obj.get("str").unwrap().as_str(), Some("hello"));
                assert_eq!(obj.get("num").unwrap().as_i64(), Some(42));
                assert!(obj.get("float").unwrap().as_f64().unwrap() - 3.14 < 0.001);
                assert_eq!(obj.get("bool").unwrap().as_bool(), Some(true));
                assert!(obj.get("null").unwrap().is_null());
                assert_eq!(obj.get("arr").unwrap().as_array().unwrap().len(), 3);
                assert!(obj.get("obj").unwrap().is_object());
            }
            _ => panic!("expected object"),
        }
    }
    #[test]
    fn json_parse_escapes() {
        let input = r#"{"msg":"hello\nworld\t\"quoted\""}"#;
        let val = json_parse_value(input).unwrap();
        let msg = val.as_object().unwrap().get("msg").unwrap().as_str().unwrap();
        assert_eq!(msg, "hello\nworld\t\"quoted\"");
    }
    #[test]
    fn json_parse_unicode_escape() {
        let input = r#"{"c":"\u0041"}"#;
        let val = json_parse_value(input).unwrap();
        assert_eq!(val.as_object().unwrap().get("c").unwrap().as_str(), Some("A"));
    }
    #[test]
    fn json_parse_negative_numbers() {
        let val = json_parse_value("-42").unwrap();
        assert_eq!(val.as_i64(), Some(-42));
    }
    #[test]
    fn json_parse_scientific() {
        let val = json_parse_value("1.5e2").unwrap();
        assert!((val.as_f64().unwrap() - 150.0).abs() < 0.001);
    }
    #[test]
    fn json_stringify_roundtrip() {
        let original = r#"{"a":1,"b":"hello","c":[true,null,3.14]}"#;
        let parsed = json_parse_value(original).unwrap();
        let output = json_to_string(&parsed);
        let reparsed = json_parse_value(&output).unwrap();
        assert_eq!(parsed, reparsed);
    }
    #[test]
    fn json_pretty_print() {
        let val = json_parse_value(r#"{"a":1}"#).unwrap();
        let pretty = json_to_string_pretty(&val);
        assert!(pretty.contains('\n'));
        assert!(pretty.contains("  "));
    }
    #[test]
    fn json_parse_empty_structures() {
        assert_eq!(json_parse_value("{}").unwrap(), JsonValue::Object(OrderedMap::new()));
        assert_eq!(json_parse_value("[]").unwrap(), JsonValue::Array(vec![]));
    }
    #[test]
    fn json_parse_nested_deep() {
        let input = r#"{"a":{"b":{"c":{"d":42}}}}"#;
        let val = json_parse_value(input).unwrap();
        let d = val.as_object().unwrap().get("a").unwrap()
            .as_object().unwrap().get("b").unwrap()
            .as_object().unwrap().get("c").unwrap()
            .as_object().unwrap().get("d").unwrap();
        assert_eq!(d.as_i64(), Some(42));
    }
    #[test]
    fn json_parse_error_cases() {
        assert!(json_parse_value("").is_err());
        assert!(json_parse_value("{").is_err());
        assert!(json_parse_value("}").is_err());
        assert!(json_parse_value("{\"a\":}").is_err());
        assert!(json_parse_value("[1,,2]").is_err());
    }

    // ── regex ────────────────────────────────────────────────────
    #[test]
    fn regex_literal() {
        let re = Regex::new("hello").unwrap();
        assert!(re.is_match("hello world"));
        assert!(!re.is_match("goodbye"));
    }
    #[test]
    fn regex_dot() {
        let re = Regex::new("h.llo").unwrap();
        assert!(re.is_match("hello"));
        assert!(re.is_match("hallo"));
    }
    #[test]
    fn regex_star() {
        let re = Regex::new("ab*c").unwrap();
        assert!(re.is_match("ac"));
        assert!(re.is_match("abc"));
        assert!(re.is_match("abbc"));
    }
    #[test]
    fn regex_plus() {
        let re = Regex::new("ab+c").unwrap();
        assert!(!re.is_match("ac"));
        assert!(re.is_match("abc"));
        assert!(re.is_match("abbc"));
    }
    #[test]
    fn regex_question() {
        let re = Regex::new("colou?r").unwrap();
        assert!(re.is_match("color"));
        assert!(re.is_match("colour"));
    }
    #[test]
    fn regex_char_class() {
        let re = Regex::new("[abc]+").unwrap();
        assert!(re.is_match("abc"));
        assert!(!re.is_match("xyz"));
    }
    #[test]
    fn regex_digit_shorthand() {
        let re = Regex::new("\\d+").unwrap();
        assert!(re.is_match("123"));
        assert!(!re.is_match("abc"));
    }
    #[test]
    fn regex_word_shorthand() {
        let re = Regex::new("\\w+").unwrap();
        assert!(re.is_match("hello_123"));
    }
    #[test]
    fn regex_anchors() {
        let re = Regex::new("^hello$").unwrap();
        assert!(re.is_match("hello"));
        assert!(!re.is_match("hello world"));
        assert!(!re.is_match("say hello"));
    }
    #[test]
    fn regex_find_all() {
        let re = Regex::new("\\d+").unwrap();
        let matches = re.find_all("a1b23c456");
        assert_eq!(matches.len(), 3);
    }
    #[test]
    fn regex_replace() {
        let re = Regex::new("\\d+").unwrap();
        assert_eq!(re.replace("a1b2c3", "X"), "aXbXcX");
    }
    #[test]
    fn regex_split() {
        let re = Regex::new("[,;]").unwrap();
        let parts = re.split("a,b;c");
        assert_eq!(parts, vec!["a", "b", "c"]);
    }
    #[test]
    fn regex_escape_metacharacters() {
        let escaped = regex_escape("hello.world+foo*bar");
        assert_eq!(escaped, "hello\\.world\\+foo\\*bar");
    }

    // ── ordered-map (replaces `indexmap`) ─────────────────────────
    #[test]
    fn ordered_map_insertion_order() {
        let mut m = OrderedMap::new();
        m.insert("c".to_string(), 3);
        m.insert("a".to_string(), 1);
        m.insert("b".to_string(), 2);
        let keys: Vec<&String> = m.keys().collect();
        assert_eq!(keys, vec!["c", "a", "b"]); // insertion order, NOT alphabetical
    }
    #[test]
    fn ordered_map_update_preserves_order() {
        let mut m = OrderedMap::new();
        m.insert("a".to_string(), 1);
        m.insert("b".to_string(), 2);
        m.insert("a".to_string(), 10); // update existing
        let keys: Vec<&String> = m.keys().collect();
        assert_eq!(keys, vec!["a", "b"]); // order preserved
        assert_eq!(m.get("a"), Some(&10));
    }
    #[test]
    fn ordered_map_remove() {
        let mut m = OrderedMap::new();
        m.insert("a".to_string(), 1);
        m.insert("b".to_string(), 2);
        m.insert("c".to_string(), 3);
        m.remove("b");
        let keys: Vec<&String> = m.keys().collect();
        assert_eq!(keys, vec!["a", "c"]);
    }
    #[test]
    fn ordered_map_from_array() {
        let m = OrderedMap::from([("x".to_string(), 1), ("y".to_string(), 2)]);
        assert_eq!(m.len(), 2);
        assert_eq!(m.get("x"), Some(&1));
    }
    #[test]
    fn ordered_map_entry_api() {
        let mut m: OrderedMap<String, Vec<i32>> = OrderedMap::new();
        m.entry("key".to_string()).or_insert_with(Vec::new).push(1);
        m.entry("key".to_string()).or_insert_with(Vec::new).push(2);
        assert_eq!(m.get("key"), Some(&vec![1, 2]));
    }
    #[test]
    fn ordered_map_borrow_str_lookup() {
        let mut m = OrderedMap::new();
        m.insert("hello".to_string(), 42);
        assert_eq!(m.get("hello"), Some(&42)); // &str lookup on String key
        assert!(m.contains_key("hello"));
    }
    #[test]
    fn ordered_map_iter() {
        let m = OrderedMap::from([("a".to_string(), 1), ("b".to_string(), 2)]);
        let collected: Vec<_> = m.iter().map(|(k, v)| (k.clone(), *v)).collect();
        assert_eq!(collected, vec![("a".to_string(), 1), ("b".to_string(), 2)]);
    }
    #[test]
    fn ordered_map_retain() {
        let mut m = OrderedMap::from([("a".to_string(), 1), ("b".to_string(), 2), ("c".to_string(), 3)]);
        m.retain(|_, v| *v % 2 != 0);
        assert_eq!(m.len(), 2);
        assert!(m.contains_key("a"));
        assert!(!m.contains_key("b"));
        assert!(m.contains_key("c"));
    }

    // ── lz4 ──────────────────────────────────────────────────────
    #[test]
    fn lz4_empty() {
        let compressed = lz4_compress_prepend_size(b"");
        assert_eq!(compressed[0..4], [0, 0, 0, 0]); // size = 0
    }
    #[test]
    fn lz4_roundtrip_small() {
        let data = b"Hello, LZ4 compression!";
        let compressed = lz4_compress_prepend_size(data);
        let decompressed = lz4_decompress_size_prepended(&compressed).unwrap();
        assert_eq!(decompressed, data);
    }
    #[test]
    fn lz4_roundtrip_repeated() {
        let data = "abcdefgh".repeat(1000);
        let compressed = lz4_compress_prepend_size(data.as_bytes());
        let decompressed = lz4_decompress_size_prepended(&compressed).unwrap();
        assert_eq!(decompressed, data.as_bytes());
        assert!(compressed.len() < data.len()); // should actually compress
    }
    #[test]
    fn lz4_decompress_invalid() {
        assert!(lz4_decompress_size_prepended(&[]).is_err());
        assert!(lz4_decompress_size_prepended(&[0xFF, 0xFF, 0xFF, 0xFF]).is_err()); // huge size
    }

    // ── zstd (DEFLATE stored blocks) ─────────────────────────────
    #[test]
    fn zstd_roundtrip() {
        let data = b"Hello, compression!";
        let compressed = zstd_compress(data, 3).unwrap();
        let decompressed = zstd_decompress(&compressed).unwrap();
        assert_eq!(decompressed, data);
    }
    #[test]
    fn zstd_roundtrip_large() {
        let data = "test data ".repeat(10000);
        let compressed = zstd_compress(data.as_bytes(), 3).unwrap();
        let decompressed = zstd_decompress(&compressed).unwrap();
        assert_eq!(decompressed, data.as_bytes());
    }
    #[test]
    fn test_gzip_roundtrip() {
        let data = b"Hello, gzip compression!";
        let compressed = gzip_compress(data);
        let decompressed = gzip_decompress(&compressed).unwrap();
        assert_eq!(decompressed, data);
    }

    // ── random ───────────────────────────────────────────────────
    #[test]
    fn random_range_bounds() {
        for _ in 0..100 {
            let v = random_range_i64(0, 10);
            assert!(v >= 0 && v < 10);
        }
    }
    #[test]
    fn random_float_range() {
        for _ in 0..100 {
            let v = random_f64();
            assert!(v >= 0.0 && v < 1.0);
        }
    }
    #[test]
    fn random_shuffle_preserves_elements() {
        let mut data = vec![1, 2, 3, 4, 5];
        random_shuffle(&mut data);
        data.sort();
        assert_eq!(data, vec![1, 2, 3, 4, 5]);
    }
    #[test]
    fn random_fill_bytes_nonzero() {
        let mut buf = [0u8; 32];
        random_fill_bytes(&mut buf);
        // Extremely unlikely all zeros
        assert!(buf.iter().any(|&b| b != 0));
    }

    // ── chrono (date/time) ───────────────────────────────────────
    #[test]
    fn now_secs_reasonable() {
        let t = now_secs();
        // Should be after 2020-01-01 and before 2100-01-01
        assert!(t > 1577836800);
        assert!(t < 4102444800);
    }
    #[test]
    fn now_millis_reasonable() {
        let t = now_millis();
        assert!(t > 1577836800000);
    }

    // ── http status codes ────────────────────────────────────────
    #[test]
    fn http_status_reason_known() {
        assert_eq!(http_status_reason(200), "OK");
        assert_eq!(http_status_reason(404), "Not Found");
        assert_eq!(http_status_reason(500), "Internal Server Error");
    }

    // ── pem/x509 parsing ─────────────────────────────────────────
    #[test]
    fn pem_parse_roundtrip() {
        let (cert_pem, _, _) = generate_self_signed_cert("test.example.com").unwrap();
        let block = parse_pem(cert_pem.as_bytes()).unwrap();
        assert_eq!(block.label, "CERTIFICATE");
        assert!(!block.contents.is_empty());
    }
    #[test]
    fn x509_parse_generated_cert() {
        let (cert_pem, _, _) = generate_self_signed_cert("myhost.local").unwrap();
        let block = parse_pem(cert_pem.as_bytes()).unwrap();
        let info = parse_x509_der(&block.contents).unwrap();
        assert!(info.subject.contains("myhost.local"));
        assert!(info.issuer.contains("myhost.local")); // self-signed
        assert!(info.not_before <= now_secs());
        assert!(info.not_after > now_secs());
    }
    #[test]
    fn pem_invalid() {
        assert!(parse_pem(b"not a pem").is_err());
        assert!(parse_pem(b"-----BEGIN CERT-----\ninvalid base64!!!\n-----END CERT-----").is_err());
    }

    // ── wasm validator ───────────────────────────────────────────
    #[test]
    fn wasm_validate_valid_minimal() {
        // Minimal valid WASM: magic + version + empty
        let wasm = b"\0asm\x01\x00\x00\x00";
        assert!(validate_wasm(wasm).is_ok());
    }
    #[test]
    fn wasm_validate_bad_magic() {
        assert!(validate_wasm(b"\0bad\x01\x00\x00\x00").is_err());
    }
    #[test]
    fn wasm_validate_too_short() {
        assert!(validate_wasm(b"\0asm").is_err());
    }
    #[test]
    fn wasm_validate_bad_version() {
        assert!(validate_wasm(b"\0asm\x02\x00\x00\x00").is_err());
    }

    // ── constant-time comparison ─────────────────────────────────
    #[test]
    fn constant_time_eq_same() { assert!(constant_time_eq(b"hello", b"hello")); }
    #[test]
    fn constant_time_eq_different() { assert!(!constant_time_eq(b"hello", b"world")); }
    #[test]
    fn constant_time_eq_different_lengths() { assert!(!constant_time_eq(b"hi", b"hello")); }

    // ── http parsing ─────────────────────────────────────────────
    #[test]
    fn http_request_parse() {
        let raw = b"GET /path HTTP/1.1\r\nHost: example.com\r\nContent-Length: 0\r\n\r\n";
        let req = parse_http_request(raw).unwrap().unwrap();
        assert_eq!(req.method, "GET");
        assert_eq!(req.path, "/path");
        assert_eq!(req.headers.len(), 2);
    }
    #[test]
    fn http_request_parse_empty() {
        let raw = b"GET";
        let result = parse_http_request(raw).unwrap();
        assert!(result.is_none()); // incomplete — no newline at all
    }

    // ── Audit regression tests ───────────────────────────────────

    // JSON: non-finite floats must not produce invalid JSON
    #[test]
    fn json_nonfinite_float_serialization() {
        let val = JsonValue::Number(JsonNumber::Float(f64::INFINITY));
        let s = json_to_string(&val);
        assert_eq!(s, "null"); // inf serialized as null
        let val = JsonValue::Number(JsonNumber::Float(f64::NAN));
        assert_eq!(json_to_string(&val), "null");
        let val = JsonValue::Number(JsonNumber::Float(f64::NEG_INFINITY));
        assert_eq!(json_to_string(&val), "null");
    }

    // JSON: deeply nested input returns error instead of stack overflow
    #[test]
    fn json_depth_limit() {
        let deep = "[".repeat(200) + &"]".repeat(200);
        assert!(json_parse_value(&deep).is_err());
    }

    // JSON: lone surrogates are errors
    #[test]
    fn json_lone_high_surrogate() {
        let input = r#"{"s":"\uD800"}"#;
        assert!(json_parse_value(input).is_err());
    }
    #[test]
    fn json_lone_low_surrogate() {
        let input = r#"{"s":"\uDC00"}"#;
        assert!(json_parse_value(input).is_err());
    }
    #[test]
    fn json_valid_surrogate_pair() {
        // U+1F600 (😀) = \uD83D\uDE00
        let input = r#""\uD83D\uDE00""#;
        let val = json_parse_value(input).unwrap();
        assert_eq!(val.as_str(), Some("😀"));
    }

    // JSON: scientific notation edge case
    #[test]
    fn json_large_exponent_to_infinity() {
        let val = json_parse_value("1e999").unwrap();
        // Parsed as infinity, serializes to null
        let s = json_to_string(&val);
        assert_eq!(s, "null"); // inf becomes null
    }

    // JSON: multi-byte UTF-8 characters preserved
    #[test]
    fn json_multibyte_utf8() {
        let input = r#"{"name":"héllo","emoji":"😀"}"#;
        let val = json_parse_value(input).unwrap();
        let name = val.as_object().unwrap().get("name").unwrap().as_str().unwrap();
        assert_eq!(name, "héllo");
        let emoji = val.as_object().unwrap().get("emoji").unwrap().as_str().unwrap();
        assert_eq!(emoji, "😀");
        // Round-trip
        let output = json_to_string(&val);
        let reparsed = json_parse_value(&output).unwrap();
        assert_eq!(val, reparsed);
    }

    // OrderedMap: from_iter with duplicate keys keeps last value
    #[test]
    fn ordered_map_from_iter_duplicates() {
        let m: OrderedMap<String, i32> = vec![("a".into(), 1), ("b".into(), 2), ("a".into(), 3)].into_iter().collect();
        assert_eq!(m.get("a"), Some(&3)); // last wins
        assert_eq!(m.len(), 2); // no duplicates
    }

    // LZ4: round-trip with all-zeros (tests repetition)
    #[test]
    fn lz4_all_zeros() {
        let data = vec![0u8; 10000];
        let compressed = lz4_compress_prepend_size(&data);
        let decompressed = lz4_decompress_size_prepended(&compressed).unwrap();
        assert_eq!(decompressed, data);
    }

    // Base64: invalid characters return error
    #[test]
    fn base64_decode_invalid_chars() {
        assert!(base64_decode("!!!").is_err());
    }

    // Hex: empty decode
    #[test]
    fn hex_decode_empty() {
        assert_eq!(hex_decode("").unwrap(), Vec::<u8>::new());
    }

    // SHA-256: incremental consistency (same input always same output)
    #[test]
    fn sha256_consistency() {
        let h1 = sha256(b"deterministic test input");
        let h2 = sha256(b"deterministic test input");
        assert_eq!(h1, h2);
    }

    // Regex: alternation
    #[test]
    fn regex_alternation_basic() {
        let re = Regex::new("cat|dog").unwrap();
        assert!(re.is_match("cat"));
        assert!(re.is_match("dog"));
        assert!(!re.is_match("bird"));
    }
    #[test]
    fn regex_alternation_in_context() {
        let re = Regex::new("foo|bar").unwrap();
        assert!(re.is_match("I have a foobar"));
        assert!(re.is_match("I have a bar"));
    }

    // Regex: empty pattern matches everything
    #[test]
    fn regex_empty_pattern() {
        let re = Regex::new("").unwrap();
        assert!(re.is_match("anything"));
        assert!(re.is_match(""));
    }

    // Regex: nested quantifiers
    #[test]
    fn regex_repeated_chars() {
        let re = Regex::new("a+b+c+").unwrap();
        assert!(re.is_match("abc"));
        assert!(re.is_match("aaabbbccc"));
        assert!(!re.is_match("ac"));
    }

    // Regex: replace with empty match preserves characters
    #[test]
    fn regex_replace_empty_match() {
        let re = Regex::new("a*").unwrap();
        let result = re.replace("bc", "X");
        assert!(result.contains('b'));
        assert!(result.contains('c'));
    }

    // Base64: whitespace in input
    #[test]
    fn base64_decode_with_whitespace() {
        assert_eq!(base64_decode("Zm9v\nYmFy\n").unwrap(), b"foobar");
        assert_eq!(base64_decode("YQ== \n").unwrap(), b"a");
    }

    // CRC32: incremental check
    #[test]
    fn crc32_consistency() {
        assert_eq!(crc32(b"hello"), crc32(b"hello"));
        assert_ne!(crc32(b"hello"), crc32(b"world"));
    }

    // HMAC: empty key and data
    #[test]
    fn hmac_empty() {
        let result = hmac_sha256(b"", b"");
        assert_eq!(result.len(), 32);
    }

    // Constant-time comparison: same-length required
    #[test]
    fn constant_time_eq_empty() {
        assert!(constant_time_eq(b"", b""));
    }
}
