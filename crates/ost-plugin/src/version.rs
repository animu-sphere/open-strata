// SPDX-License-Identifier: Apache-2.0
//! Minimal version-range checking for compatibility assertions (Level 1).
//!
//! The harness compares a concrete runtime version (e.g. OpenUSD `24.11`)
//! against a plugin's tolerated range (e.g. `>=24.11,<25.0`). OpenStrata pins
//! its dependency tree to a fixed edition, so rather than pull in `semver` we
//! implement just the comma-separated comparator-list subset the manifests use.
//!
//! A version is a dotted sequence of integers; missing trailing components are
//! treated as zero, so `24.11` and `24.11.0` compare equal. Non-numeric or
//! empty input is rejected rather than guessed at.

/// Parse a dotted version into numeric components. `None` if any component is
/// not a non-negative integer, or the string is empty.
fn parse(version: &str) -> Option<Vec<u64>> {
    let v = version.trim();
    if v.is_empty() {
        return None;
    }
    v.split('.').map(|p| p.trim().parse::<u64>().ok()).collect()
}

/// Compare two parsed versions, zero-padding the shorter one.
fn cmp(a: &[u64], b: &[u64]) -> std::cmp::Ordering {
    let n = a.len().max(b.len());
    for i in 0..n {
        let x = a.get(i).copied().unwrap_or(0);
        let y = b.get(i).copied().unwrap_or(0);
        match x.cmp(&y) {
            std::cmp::Ordering::Equal => continue,
            other => return other,
        }
    }
    std::cmp::Ordering::Equal
}

/// Why a range check could not be decided.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RangeError {
    /// The concrete version did not parse as a dotted integer version.
    BadVersion(String),
    /// A comparator in the range was malformed (e.g. `>=` with no number).
    BadRange(String),
}

impl std::fmt::Display for RangeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            RangeError::BadVersion(v) => write!(f, "unparseable version '{v}'"),
            RangeError::BadRange(r) => write!(f, "unparseable version range '{r}'"),
        }
    }
}

/// Does `version` satisfy `range` (a comma-separated comparator list)?
///
/// Supported comparators: `>=`, `<=`, `>`, `<`, `=`/`==`, or a bare version
/// (treated as `=`). All comparators must hold (logical AND).
pub fn satisfies(version: &str, range: &str) -> Result<bool, RangeError> {
    let ver = parse(version).ok_or_else(|| RangeError::BadVersion(version.to_string()))?;

    for part in range.split(',') {
        let part = part.trim();
        if part.is_empty() {
            continue;
        }
        let (op, rhs) = split_op(part);
        let bound = parse(rhs).ok_or_else(|| RangeError::BadRange(range.to_string()))?;
        let ord = cmp(&ver, &bound);
        use std::cmp::Ordering::*;
        let ok = match op {
            ">=" => ord != Less,
            "<=" => ord != Greater,
            ">" => ord == Greater,
            "<" => ord == Less,
            "=" | "==" => ord == Equal,
            _ => return Err(RangeError::BadRange(range.to_string())),
        };
        if !ok {
            return Ok(false);
        }
    }
    Ok(true)
}

/// Split a comparator into its operator and version operand. A bare version
/// (no leading operator) is treated as `=`.
fn split_op(part: &str) -> (&str, &str) {
    for op in [">=", "<=", "==", ">", "<", "="] {
        if let Some(rest) = part.strip_prefix(op) {
            return (op, rest.trim());
        }
    }
    ("=", part)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn range_with_two_bounds() {
        assert_eq!(satisfies("24.11", ">=24.11,<25.0"), Ok(true));
        assert_eq!(satisfies("24.08", ">=24.11,<25.0"), Ok(false));
        assert_eq!(satisfies("25.0", ">=24.11,<25.0"), Ok(false));
        assert_eq!(satisfies("24.11.3", ">=24.11,<25.0"), Ok(true));
    }

    #[test]
    fn zero_padding_makes_minor_match() {
        // 25 == 25.0 == 25.0.0
        assert_eq!(satisfies("25", "<25.0"), Ok(false));
        assert_eq!(satisfies("25", ">=25.0.0"), Ok(true));
    }

    #[test]
    fn bare_version_is_equality() {
        assert_eq!(satisfies("1.39", "1.39"), Ok(true));
        assert_eq!(satisfies("1.39.1", "1.39"), Ok(false));
    }

    #[test]
    fn bad_input_is_an_error_not_a_false_pass() {
        assert!(matches!(
            satisfies("twentyfour", ">=24.11"),
            Err(RangeError::BadVersion(_))
        ));
        assert!(matches!(
            satisfies("24.11", ">=abc"),
            Err(RangeError::BadRange(_))
        ));
    }
}
