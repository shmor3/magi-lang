//! Own implementations replacing external crates.
//!
//! Phase 1 of the crate replacement plan — replaces: hex, slug, html-escape,
//! percent-encoding, data-encoding (base32), heck, ordered-float, strsim,
//! crc32fast, glob.

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
}
