//! Own implementations replacing external crates.
//!
//! Phase 1: hex, slug, html-escape, percent-encoding, data-encoding (base32),
//! heck, ordered-float, strsim, crc32fast, glob.
//!
//! Phase 2: uuid, subtle, semver, textwrap, base64, hmac, md-5.
//!
//! Phase 3: chrono, url, http, httparse, toml.

// ---------------------------------------------------------------------------
// hex encode/decode (replaces `hex` crate)
// ---------------------------------------------------------------------------

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

// ---------------------------------------------------------------------------
// slug (replaces `slug` crate)
// ---------------------------------------------------------------------------

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

// ---------------------------------------------------------------------------
// html-escape (replaces `html-escape` crate)
// ---------------------------------------------------------------------------

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

// ---------------------------------------------------------------------------
// percent-encoding (replaces `percent-encoding` crate)
// ---------------------------------------------------------------------------

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

// ---------------------------------------------------------------------------
// base32 (replaces `data-encoding` crate for BASE32)
// ---------------------------------------------------------------------------

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

// ---------------------------------------------------------------------------
// heck — case conversion (replaces `heck` crate)
// ---------------------------------------------------------------------------

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

// ---------------------------------------------------------------------------
// ordered-float (replaces `ordered-float` crate)
// ---------------------------------------------------------------------------

/// Wrapper for f64 that implements Ord using total_cmp.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct OrderedFloat(pub f64);

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

// ---------------------------------------------------------------------------
// strsim — Levenshtein distance (replaces `strsim` crate)
// ---------------------------------------------------------------------------

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

// ---------------------------------------------------------------------------
// crc32 (replaces `crc32fast` crate)
// ---------------------------------------------------------------------------

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

/// Compute CRC32 checksum (IEEE/Castagnoli).
pub fn crc32(data: &[u8]) -> u32 {
    let mut crc: u32 = 0xFFFFFFFF;
    for &b in data {
        crc = CRC32_TABLE[((crc ^ b as u32) & 0xFF) as usize] ^ (crc >> 8);
    }
    crc ^ 0xFFFFFFFF
}

// ---------------------------------------------------------------------------
// glob (replaces `glob` crate)
// ---------------------------------------------------------------------------

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

// ---------------------------------------------------------------------------
// uuid v4 (replaces `uuid` crate)
// ---------------------------------------------------------------------------

/// Generate a random UUID v4 string (xxxxxxxx-xxxx-4xxx-yxxx-xxxxxxxxxxxx).
pub fn uuid_v4() -> String {
    use rand::Rng;
    let mut rng = rand::rng();
    let mut bytes = [0u8; 16];
    rng.fill(&mut bytes);
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

// ---------------------------------------------------------------------------
// constant-time comparison (replaces `subtle` crate)
// ---------------------------------------------------------------------------

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

// ---------------------------------------------------------------------------
// semver (replaces `semver` crate)
// ---------------------------------------------------------------------------

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

// ---------------------------------------------------------------------------
// textwrap (replaces `textwrap` crate)
// ---------------------------------------------------------------------------

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

// ---------------------------------------------------------------------------
// base64 (replaces `base64` crate)
// ---------------------------------------------------------------------------

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

// ---------------------------------------------------------------------------
// HMAC-SHA256 (replaces `hmac` crate for SHA256)
// ---------------------------------------------------------------------------

/// Compute HMAC-SHA256.
pub fn hmac_sha256(key: &[u8], data: &[u8]) -> Vec<u8> {
    use sha2::{Sha256, Digest};

    const BLOCK_SIZE: usize = 64;

    // If key is longer than block size, hash it first
    let key = if key.len() > BLOCK_SIZE {
        Sha256::digest(key).to_vec()
    } else {
        key.to_vec()
    };

    // Pad key to block size
    let mut padded_key = vec![0u8; BLOCK_SIZE];
    padded_key[..key.len()].copy_from_slice(&key);

    // Inner padding
    let mut ipad = vec![0x36u8; BLOCK_SIZE];
    for (i, b) in padded_key.iter().enumerate() {
        ipad[i] ^= b;
    }

    // Outer padding
    let mut opad = vec![0x5cu8; BLOCK_SIZE];
    for (i, b) in padded_key.iter().enumerate() {
        opad[i] ^= b;
    }

    // Inner hash: H(ipad || data)
    let mut inner_hasher = Sha256::new();
    inner_hasher.update(&ipad);
    inner_hasher.update(data);
    let inner_hash = inner_hasher.finalize();

    // Outer hash: H(opad || inner_hash)
    let mut outer_hasher = Sha256::new();
    outer_hasher.update(&opad);
    outer_hasher.update(&inner_hash);
    outer_hasher.finalize().to_vec()
}

// ---------------------------------------------------------------------------
// MD5 hash (replaces `md-5` crate)
// ---------------------------------------------------------------------------

/// Compute MD5 hash.
pub fn md5_hash(data: &[u8]) -> [u8; 16] {
    // Initial state
    let mut a0: u32 = 0x67452301;
    let mut b0: u32 = 0xefcdab89;
    let mut c0: u32 = 0x98badcfe;
    let mut d0: u32 = 0x10325476;

    // Pre-processing: adding padding bits
    let orig_len_bits = (data.len() as u64) * 8;
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

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

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
        // Must be unique
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
        // Known MD5 values
        assert_eq!(hex_encode(&md5_hash(b"")), "d41d8cd98f00b204e9800998ecf8427e");
        assert_eq!(hex_encode(&md5_hash(b"hello")), "5d41402abc4b2a76b9719d911017c592");
        assert_eq!(hex_encode(&md5_hash(b"The quick brown fox jumps over the lazy dog")),
                   "9e107d9d372bb6826bd81d3542a419d6");
    }
}

// ===========================================================================
// Phase 3: chrono, url, http, httparse, toml replacements
// ===========================================================================

// ---------------------------------------------------------------------------
// date/time (replaces `chrono` crate)
// ---------------------------------------------------------------------------

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
    // Parse date
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

// ---------------------------------------------------------------------------
// URL parsing (replaces `url` crate)
// ---------------------------------------------------------------------------

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

// ---------------------------------------------------------------------------
// HTTP status codes (replaces `http` crate)
// ---------------------------------------------------------------------------

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

// ---------------------------------------------------------------------------
// HTTP request parsing (replaces `httparse` crate)
// ---------------------------------------------------------------------------

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

    // Parse headers
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

// ---------------------------------------------------------------------------
// TOML parser (replaces `toml` crate)
// ---------------------------------------------------------------------------

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
pub type TomlTable = indexmap::IndexMap<String, TomlValue>;

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

        // Get the table to insert into
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
    // Boolean
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
    // Array
    if s.starts_with('[') {
        return parse_toml_array(s);
    }
    // Inline table
    if s.starts_with('{') {
        return parse_toml_inline_table(s);
    }
    // Number (integer or float)
    // Try integer first
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
    // Float
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
            // Inline table
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
