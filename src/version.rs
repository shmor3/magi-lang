//! MAGI language versioning.
//!
//! Provides semantic versioning for the MAGI language, including version
//! parsing, comparison, and compatibility checking.

use std::fmt;

/// The current MAGI language version.
pub const MAJOR: u32 = 0;
pub const MINOR: u32 = 9;
pub const PATCH: u32 = 0;

/// Pre-release label (empty string for stable releases).
pub const PRE_RELEASE: &str = "";

/// Returns the current MAGI language version as a `Version`.
pub fn current() -> Version {
    Version {
        inner: semver::Version {
            major: MAJOR as u64,
            minor: MINOR as u64,
            patch: PATCH as u64,
            pre: if PRE_RELEASE.is_empty() {
                semver::Prerelease::EMPTY
            } else {
                semver::Prerelease::new(PRE_RELEASE).unwrap_or(semver::Prerelease::EMPTY)
            },
            build: semver::BuildMetadata::EMPTY,
        },
    }
}

/// Returns the current version as a string (e.g., "0.3.0-alpha").
pub fn version_string() -> String {
    current().to_string()
}

/// A semantic version for the MAGI language.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Version {
    inner: semver::Version,
}

impl Version {
    pub fn new(major: u32, minor: u32, patch: u32) -> Self {
        Self {
            inner: semver::Version::new(major as u64, minor as u64, patch as u64),
        }
    }

    pub fn with_pre_release(mut self, pre: &str) -> Self {
        self.inner.pre = semver::Prerelease::new(pre).unwrap_or(semver::Prerelease::EMPTY);
        self
    }

    /// Parse a version string like "0.2.0" or "1.0.0-beta.1".
    pub fn parse(s: &str) -> Result<Self, VersionError> {
        let s = s.strip_prefix('v').unwrap_or(s);
        semver::Version::parse(s)
            .map(|inner| Self { inner })
            .map_err(|_| VersionError::InvalidFormat(s.to_string()))
    }

    /// Check if this version is compatible with another version.
    pub fn is_compatible_with(&self, other: &Version) -> bool {
        if self.inner.major == 0 && other.inner.major == 0 {
            self.inner.minor == other.inner.minor
        } else {
            self.inner.major == other.inner.major
        }
    }

    /// Check if this version satisfies a requirement string.
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
            let base = Version::parse(req)?;
            Ok(self == &base)
        }
    }

    /// Returns the version tuple (major, minor, patch).
    pub fn tuple(&self) -> (u32, u32, u32) {
        (self.inner.major as u32, self.inner.minor as u32, self.inner.patch as u32)
    }

    /// The major version number.
    pub fn major(&self) -> u32 {
        self.inner.major as u32
    }

    /// The minor version number.
    pub fn minor(&self) -> u32 {
        self.inner.minor as u32
    }

    /// The patch version number.
    pub fn patch(&self) -> u32 {
        self.inner.patch as u32
    }

    /// Is this a pre-release version?
    pub fn is_pre_release(&self) -> bool {
        !self.inner.pre.is_empty()
    }

    /// Is this a stable (1.0+) release?
    pub fn is_stable(&self) -> bool {
        self.inner.major >= 1 && self.inner.pre.is_empty()
    }

    /// The pre-release string, if any.
    pub fn pre_release(&self) -> Option<String> {
        if self.inner.pre.is_empty() {
            None
        } else {
            Some(self.inner.pre.to_string())
        }
    }
}

impl fmt::Display for Version {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.inner)
    }
}

impl PartialOrd for Version {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for Version {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.inner.cmp(&other.inner)
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
    Core,
    PatternMatching,
    AsyncAwait,
    PipeOperator,
    StringInterpolation,
    OptionalChaining,
    NullCoalescing,
    Closures,
    Destructuring,
    HigherOrderMethods,
    RangeExpressions,
    Comprehensions,
    Enums,
    Structs,
    RestParams,
    SpreadCalls,
    TryPropagate,
    BlockComments,
    MultilineStrings,
    RawStrings,
    WasmCompilation,
}

impl Feature {
    pub fn since(&self) -> Version {
        match self {
            Feature::Core
            | Feature::PatternMatching
            | Feature::AsyncAwait
            | Feature::PipeOperator
            | Feature::StringInterpolation
            | Feature::OptionalChaining
            | Feature::NullCoalescing
            | Feature::Closures
            | Feature::Destructuring => Version::new(0, 1, 0),

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

    pub fn available_at(&self, version: &Version) -> bool {
        let since = self.since();
        if version.is_pre_release() && version.tuple() == since.tuple() {
            return true;
        }
        version >= &since
    }
}

pub fn available_features() -> Vec<Feature> {
    let v = current();
    all_features()
        .into_iter()
        .filter(|f| f.available_at(&v))
        .collect()
}

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
        assert_eq!(v.major(), 0);
        assert_eq!(v.minor(), 9);
        assert_eq!(v.patch(), 0);
        assert_eq!(v.pre_release(), None);
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
        assert_eq!(v.pre_release(), None);

        let v = Version::parse("0.2.0-beta.1").unwrap();
        assert_eq!(v.tuple(), (0, 2, 0));
        assert_eq!(v.pre_release(), Some("beta.1".to_string()));

        // v-prefix support
        let v = Version::parse("v1.2.3").unwrap();
        assert_eq!(v.tuple(), (1, 2, 3));
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
        // Semver numeric pre-release ordering.
        assert!(
            Version::new(1, 0, 0).with_pre_release("alpha.2")
                > Version::new(1, 0, 0).with_pre_release("alpha.1")
        );
        assert!(
            Version::new(1, 0, 0).with_pre_release("beta")
                > Version::new(1, 0, 0).with_pre_release("alpha")
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
        assert_eq!(s, "0.9.0");
    }

    #[test]
    fn test_version_tuple() {
        let v = Version::new(1, 2, 3);
        assert_eq!(v.tuple(), (1, 2, 3));
    }
}
