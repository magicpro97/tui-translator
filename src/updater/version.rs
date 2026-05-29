use std::fmt;

/// Parsed semantic version used by the updater.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct ParsedVersion {
    /// Major version component.
    pub major: u32,
    /// Minor version component.
    pub minor: u32,
    /// Patch version component.
    pub patch: u32,
}

impl ParsedVersion {
    /// Parse a semantic version from `1.2.3` or `v1.2.3`.
    ///
    /// Pre-release or build suffixes such as `-rc1` and `+build42` are ignored.
    pub fn parse(raw: &str) -> Option<Self> {
        let trimmed = raw.trim();
        let without_prefix = trimmed
            .strip_prefix('v')
            .or_else(|| trimmed.strip_prefix('V'))
            .unwrap_or(trimmed);
        let release_only = without_prefix
            .split_once('-')
            .map_or(without_prefix, |(core, _)| core);
        let release_only = release_only
            .split_once('+')
            .map_or(release_only, |(core, _)| core);

        let mut parts = release_only.split('.');
        let major = parts.next()?.parse().ok()?;
        let minor = parts.next()?.parse().ok()?;
        let patch = parts.next()?.parse().ok()?;
        if parts.next().is_some() {
            return None;
        }

        Some(Self {
            major,
            minor,
            patch,
        })
    }
}

impl fmt::Display for ParsedVersion {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}.{}.{}", self.major, self.minor, self.patch)
    }
}

#[cfg(test)]
mod tests {
    use super::ParsedVersion;

    #[test]
    fn parses_plain_semver() {
        let parsed = ParsedVersion::parse("1.2.3").expect("version should parse");
        assert_eq!(parsed.major, 1);
        assert_eq!(parsed.minor, 2);
        assert_eq!(parsed.patch, 3);
        assert_eq!(parsed.to_string(), "1.2.3");
    }

    #[test]
    fn parses_v_prefixed_semver() {
        let parsed = ParsedVersion::parse("v2.4.6").expect("version should parse");
        assert_eq!(
            parsed,
            ParsedVersion {
                major: 2,
                minor: 4,
                patch: 6
            }
        );
    }

    #[test]
    fn ignores_prerelease_and_build_suffixes() {
        let prerelease = ParsedVersion::parse("v1.2.3-rc.1").expect("prerelease should parse");
        let build = ParsedVersion::parse("1.2.3+build.9").expect("build metadata should parse");
        assert_eq!(
            prerelease,
            ParsedVersion {
                major: 1,
                minor: 2,
                patch: 3
            }
        );
        assert_eq!(
            build,
            ParsedVersion {
                major: 1,
                minor: 2,
                patch: 3
            }
        );
    }

    #[test]
    fn rejects_invalid_versions() {
        assert!(ParsedVersion::parse("1.2").is_none());
        assert!(ParsedVersion::parse("1.2.3.4").is_none());
        assert!(ParsedVersion::parse("release-1.2.3").is_none());
    }

    #[test]
    fn compares_equal_versions() {
        let left = ParsedVersion::parse("v1.2.3").expect("left parses");
        let right = ParsedVersion::parse("1.2.3").expect("right parses");
        assert_eq!(left, right);
    }

    #[test]
    fn compares_older_versions() {
        let older = ParsedVersion::parse("1.2.3").expect("older parses");
        let newer = ParsedVersion::parse("1.2.4").expect("newer parses");
        assert!(older < newer);
    }

    #[test]
    fn compares_newer_versions() {
        let newer = ParsedVersion::parse("2.0.0").expect("newer parses");
        let older = ParsedVersion::parse("1.9.9").expect("older parses");
        assert!(newer > older);
    }
}
