//! MAGI language versioning.
//!
//! Provides semantic versioning for the MAGI language, including version
//! parsing, comparison, and compatibility checking.

use std::fmt;

/// The current MAGI language version.
pub const MAJOR: u32 = 0;
pub const MINOR: u32 = 2;
pub const PATCH: u32 = 0;

/// Pre-release label (empty string for stable releases).
pub const PRE_RELEASE: &str = "alpha";

/// Returns the current MAGI language version as a `Version`.
pub fn current() -> Version {
    Version {
        major: MAJOR,
        minor: MINOR,
        patch: PATCH,
        pre_release: if PRE_RELEASE.is_empty() {
            None
        } else {
            Some(PRE_RELEASE.to_string())
        },
    }
}

/// Returns the current version as a string (e.g., "0.2.0-alpha").
pub fn version_string() -> String {
    current().to_string()
}

/// A semantic version for the MAGI language.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Version {
    pub major: u32,
    pub minor: u32,
    pub patch: u32,
    pub pre_release: Option<String>,
}

impl Version {
    pub fn new(major: u32, minor: u32, patch: u32) -> Self {
        Self {
            major,
            minor,
            patch,
            pre_release: None,
        }
    }

    pub fn with_pre_release(mut self, pre: &str) -> Self {
        self.pre_release = Some(pre.to_string());
        self
    }

    /// Parse a version string like "0.2.0" or "1.0.0-beta.1".
    pub fn parse(s: &str) -> Result<Self, VersionError> {
        let (version_part, pre_release) = if let Some((v, pre)) = s.split_once('-') {
            (v, Some(pre.to_string()))
        } else {
            (s, None)
        };

        let parts: Vec<&str> = version_part.split('.').collect();
        if parts.len() != 3 {
            return Err(VersionError::InvalidFormat(s.to_string()));
        }

        let major = parts[0]
            .parse()
            .map_err(|_| VersionError::InvalidFormat(s.to_string()))?;
        let minor = parts[1]
            .parse()
            .map_err(|_| VersionError::InvalidFormat(s.to_string()))?;
        let patch = parts[2]
            .parse()
            .map_err(|_| VersionError::InvalidFormat(s.to_string()))?;

        Ok(Self {
            major,
            minor,
            patch,
            pre_release,
        })
    }

    /// Check if this version is compatible with another version.
    ///
    /// Compatibility rules (semver):
    /// - Major 0: only exact minor matches are compatible (0.2.x compat with 0.2.y)
    /// - Major 1+: same major version is compatible (1.x.y compat with 1.a.b)
    pub fn is_compatible_with(&self, other: &Version) -> bool {
        if self.major == 0 && other.major == 0 {
            // In pre-1.0, minor versions are breaking.
            self.minor == other.minor
        } else {
            self.major == other.major
        }
    }

    /// Check if this version satisfies a requirement string.
    ///
    /// Supported formats:
    /// - `"0.2.0"` — exact match
    /// - `"^0.2.0"` — compatible (caret range)
    /// - `">=0.2.0"` — minimum version
    pub fn satisfies(&self, requirement: &str) -> Result<bool, VersionError> {
        let req = requirement.trim();

        if let Some(rest) = req.strip_prefix('^') {
            let base = Version::parse(rest)?;
            Ok(self.is_compatible_with(&base) && self >= &base)
        } else if let Some(rest) = req.strip_prefix(">=") {
            let base = Version::parse(rest)?;
            Ok(self >= &base)
        } else if let Some(rest) = req.strip_prefix('>') {
            let base = Version::parse(rest)?;
            Ok(self > &base)
        } else if let Some(rest) = req.strip_prefix("<=") {
            let base = Version::parse(rest)?;
            Ok(self <= &base)
        } else if let Some(rest) = req.strip_prefix('<') {
            let base = Version::parse(rest)?;
            Ok(self < &base)
        } else if let Some(rest) = req.strip_prefix('=') {
            let base = Version::parse(rest)?;
            Ok(self == &base)
        } else {
            // Exact match.
            let base = Version::parse(req)?;
            Ok(self == &base)
        }
    }

    /// Returns the version tuple (major, minor, patch).
    pub fn tuple(&self) -> (u32, u32, u32) {
        (self.major, self.minor, self.patch)
    }

    /// Is this a pre-release version?
    pub fn is_pre_release(&self) -> bool {
        self.pre_release.is_some()
    }

    /// Is this a stable (1.0+) release?
    pub fn is_stable(&self) -> bool {
        self.major >= 1 && self.pre_release.is_none()
    }
}

impl fmt::Display for Version {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}.{}.{}", self.major, self.minor, self.patch)?;
        if let Some(pre) = &self.pre_release {
            write!(f, "-{}", pre)?;
        }
        Ok(())
    }
}

impl PartialOrd for Version {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for Version {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.major
            .cmp(&other.major)
            .then(self.minor.cmp(&other.minor))
            .then(self.patch.cmp(&other.patch))
            .then_with(|| match (&self.pre_release, &other.pre_release) {
                (None, None) => std::cmp::Ordering::Equal,
                (Some(_), None) => std::cmp::Ordering::Less, // pre-release < release
                (None, Some(_)) => std::cmp::Ordering::Greater,
                (Some(a), Some(b)) => a.cmp(b),
            })
    }
}

/// Errors related to version parsing.
#[derive(Debug, Clone, thiserror::Error)]
pub enum VersionError {
    #[error("invalid version format: {0}")]
    InvalidFormat(String),
}

/// Language feature flags — tracks which features are available at each version.
#[derive(Debug, Clone)]
pub struct FeatureSet {
    /// Minimum version that supports this feature set.
    pub since: Version,
    pub features: Vec<Feature>,
}

/// Individual language features with their introduction version.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Feature {
    /// Core language (variables, functions, loops, conditionals).
    Core,
    /// Pattern matching with match expressions.
    PatternMatching,
    /// Async/await support.
    AsyncAwait,
    /// Pipe operator (|>).
    PipeOperator,
    /// String interpolation (f"...").
    StringInterpolation,
    /// Optional chaining (?.).
    OptionalChaining,
    /// Null coalescing (??).
    NullCoalescing,
    /// Closures/lambdas (|x| expr).
    Closures,
    /// Destructuring (let [a, b] = ...).
    Destructuring,
    /// Higher-order array/map methods (.map, .filter, .reduce).
    HigherOrderMethods,
    /// Range expressions (0..10, 0..=10).
    RangeExpressions,
    /// List/map comprehensions ([x for x in arr]).
    Comprehensions,
    /// Enum types.
    Enums,
    /// Struct types.
    Structs,
    /// Rest parameters (...args).
    RestParams,
    /// Spread in function calls (f(...args)).
    SpreadCalls,
    /// Try-propagate operator (?).
    TryPropagate,
    /// Block comments (/* */).
    BlockComments,
    /// Multiline strings (""" """).
    MultilineStrings,
    /// Raw strings (r"...").
    RawStrings,
    /// WASM compilation target.
    WasmCompilation,
}

impl Feature {
    /// The version at which this feature was introduced.
    pub fn since(&self) -> Version {
        match self {
            // v0.1.0 — initial release
            Feature::Core
            | Feature::PatternMatching
            | Feature::AsyncAwait
            | Feature::PipeOperator
            | Feature::StringInterpolation
            | Feature::OptionalChaining
            | Feature::NullCoalescing
            | Feature::Closures
            | Feature::Destructuring => Version::new(0, 1, 0),

            // v0.2.0 — comprehensive improvements
            Feature::HigherOrderMethods
            | Feature::RangeExpressions
            | Feature::Comprehensions
            | Feature::Enums
            | Feature::Structs
            | Feature::RestParams
            | Feature::SpreadCalls
            | Feature::TryPropagate
            | Feature::BlockComments
            | Feature::MultilineStrings
            | Feature::RawStrings
            | Feature::WasmCompilation => Version::new(0, 2, 0),
        }
    }

    /// Check if this feature is available at a given version.
    /// Pre-release versions of a release are considered to include that release's features.
    pub fn available_at(&self, version: &Version) -> bool {
        let since = self.since();
        // A pre-release like 0.2.0-alpha includes 0.2.0 features.
        if version.is_pre_release() && version.tuple() == since.tuple() {
            return true;
        }
        version >= &since
    }
}

/// Get all features available at the current language version.
pub fn available_features() -> Vec<Feature> {
    let v = current();
    all_features()
        .into_iter()
        .filter(|f| f.available_at(&v))
        .collect()
}

/// Get all defined features.
pub fn all_features() -> Vec<Feature> {
    vec![
        Feature::Core,
        Feature::PatternMatching,
        Feature::AsyncAwait,
        Feature::PipeOperator,
        Feature::StringInterpolation,
        Feature::OptionalChaining,
        Feature::NullCoalescing,
        Feature::Closures,
        Feature::Destructuring,
        Feature::HigherOrderMethods,
        Feature::RangeExpressions,
        Feature::Comprehensions,
        Feature::Enums,
        Feature::Structs,
        Feature::RestParams,
        Feature::SpreadCalls,
        Feature::TryPropagate,
        Feature::BlockComments,
        Feature::MultilineStrings,
        Feature::RawStrings,
        Feature::WasmCompilation,
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_current_version() {
        let v = current();
        assert_eq!(v.major, 0);
        assert_eq!(v.minor, 2);
        assert_eq!(v.patch, 0);
        assert_eq!(v.pre_release, Some("alpha".to_string()));
    }

    #[test]
    fn test_version_display() {
        assert_eq!(Version::new(1, 0, 0).to_string(), "1.0.0");
        assert_eq!(
            Version::new(0, 2, 0).with_pre_release("alpha").to_string(),
            "0.2.0-alpha"
        );
    }

    #[test]
    fn test_version_parse() {
        let v = Version::parse("1.2.3").unwrap();
        assert_eq!(v.tuple(), (1, 2, 3));
        assert_eq!(v.pre_release, None);

        let v = Version::parse("0.2.0-beta.1").unwrap();
        assert_eq!(v.tuple(), (0, 2, 0));
        assert_eq!(v.pre_release, Some("beta.1".to_string()));
    }

    #[test]
    fn test_version_parse_invalid() {
        assert!(Version::parse("1.2").is_err());
        assert!(Version::parse("abc").is_err());
        assert!(Version::parse("1.2.3.4").is_err());
    }

    #[test]
    fn test_version_ordering() {
        assert!(Version::new(1, 0, 0) > Version::new(0, 9, 9));
        assert!(Version::new(0, 2, 0) > Version::new(0, 1, 9));
        assert!(Version::new(0, 2, 1) > Version::new(0, 2, 0));
        // Pre-release sorts before release.
        assert!(
            Version::new(1, 0, 0).with_pre_release("alpha")
                < Version::new(1, 0, 0)
        );
    }

    #[test]
    fn test_compatibility_pre_1() {
        let v020 = Version::new(0, 2, 0);
        let v021 = Version::new(0, 2, 1);
        let v030 = Version::new(0, 3, 0);

        assert!(v020.is_compatible_with(&v021));
        assert!(!v020.is_compatible_with(&v030));
    }

    #[test]
    fn test_compatibility_post_1() {
        let v100 = Version::new(1, 0, 0);
        let v120 = Version::new(1, 2, 0);
        let v200 = Version::new(2, 0, 0);

        assert!(v100.is_compatible_with(&v120));
        assert!(!v100.is_compatible_with(&v200));
    }

    #[test]
    fn test_satisfies_exact() {
        let v = Version::new(0, 2, 0);
        assert!(v.satisfies("0.2.0").unwrap());
        assert!(!v.satisfies("0.2.1").unwrap());
    }

    #[test]
    fn test_satisfies_caret() {
        let v = Version::new(0, 2, 1);
        assert!(v.satisfies("^0.2.0").unwrap());
        assert!(!v.satisfies("^0.3.0").unwrap());

        let v = Version::new(1, 3, 0);
        assert!(v.satisfies("^1.0.0").unwrap());
        assert!(!v.satisfies("^2.0.0").unwrap());
    }

    #[test]
    fn test_satisfies_gte() {
        let v = Version::new(0, 2, 0);
        assert!(v.satisfies(">=0.1.0").unwrap());
        assert!(v.satisfies(">=0.2.0").unwrap());
        assert!(!v.satisfies(">=0.3.0").unwrap());
    }

    #[test]
    fn test_satisfies_lt() {
        let v = Version::new(0, 2, 0);
        assert!(v.satisfies("<0.3.0").unwrap());
        assert!(!v.satisfies("<0.2.0").unwrap());
    }

    #[test]
    fn test_satisfies_lte() {
        let v = Version::new(0, 2, 0);
        assert!(v.satisfies("<=0.2.0").unwrap());
        assert!(v.satisfies("<=0.3.0").unwrap());
        assert!(!v.satisfies("<=0.1.0").unwrap());
    }

    #[test]
    fn test_is_stable() {
        assert!(!Version::new(0, 2, 0).is_stable());
        assert!(Version::new(1, 0, 0).is_stable());
        assert!(!Version::new(1, 0, 0).with_pre_release("rc.1").is_stable());
    }

    #[test]
    fn test_is_pre_release() {
        assert!(!Version::new(1, 0, 0).is_pre_release());
        assert!(Version::new(1, 0, 0).with_pre_release("alpha").is_pre_release());
    }

    #[test]
    fn test_feature_availability() {
        let v010 = Version::new(0, 1, 0);
        let v020 = Version::new(0, 2, 0);
        let v001 = Version::new(0, 0, 1);

        assert!(Feature::Core.available_at(&v010));
        assert!(Feature::Core.available_at(&v020));
        assert!(!Feature::Core.available_at(&v001));

        assert!(!Feature::Enums.available_at(&v010));
        assert!(Feature::Enums.available_at(&v020));
    }

    #[test]
    fn test_available_features() {
        let features = available_features();
        assert!(features.contains(&Feature::Core));
        assert!(features.contains(&Feature::Enums));
        assert!(features.contains(&Feature::WasmCompilation));
    }

    #[test]
    fn test_all_features_count() {
        assert_eq!(all_features().len(), 21);
    }

    #[test]
    fn test_version_string() {
        let s = version_string();
        assert!(s.starts_with("0.2.0"));
    }

    #[test]
    fn test_version_tuple() {
        let v = Version::new(1, 2, 3);
        assert_eq!(v.tuple(), (1, 2, 3));
    }
}
