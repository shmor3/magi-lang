//! Own implementations replacing external crates.
//!
//! Phase 1: hex, slug, html-escape, percent-encoding, data-encoding (base32),
//! heck, ordered-float, strsim, crc32fast, glob.
//!
//! Phase 2: uuid, subtle, semver, textwrap, base64, hmac, md-5.

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
