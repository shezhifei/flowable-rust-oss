//! Unified in-memory SQL `LIKE` matcher for the Flowable Rust port.
//!
//! # Security motivation
//!
//! Before P143, several crates carried independent copies of LIKE matching:
//! recursive backtracking (exponential worst case on dense `%` patterns), full
//! O(n×m) DP matrices (multi-GB allocation on long inputs), or linear
//! double-pointer byte walkers. Any authenticated REST caller could supply
//! `*Like` query parameters and trigger stack overflow, CPU exhaustion, or
//! allocator abort.
//!
//! This module is the **single** authoritative implementation. All other crates
//! (engine, cmmn-engine, dmn-engine, form-service, event-registry-service, rest)
//! must delegate here. Semantics match the former REST P142d helper:
//! rolling-array O(value length) space DP, 512-character caps, char-based
//! (Unicode scalar) matching.

/// Max Unicode scalar count for pattern or value. Either side over this cap
/// yields a non-match (`false`) without allocating a large DP table.
pub const MAX_SQL_LIKE_LEN: usize = 512;

/// SQL-LIKE style match (`%` any sequence, `_` one char, other chars literal).
///
/// Case-sensitive. Operates on Unicode scalars (`char`), not bytes — so `_`
/// matches one character (including multi-byte UTF-8 such as `中`). Callers
/// that need ignore-case lower-case both sides before calling.
///
/// Space is O(value length) via two rolling rows (not O(n×m) full matrix /
/// deep recursion). Oversized pattern or value returns `false`.
pub fn sql_like_matches(pattern: &str, value: &str) -> bool {
    let value: Vec<char> = value.chars().collect();
    let pattern: Vec<char> = pattern.chars().collect();
    if value.len() > MAX_SQL_LIKE_LEN || pattern.len() > MAX_SQL_LIKE_LEN {
        return false;
    }

    let m = value.len();
    // `prev[j]` / `curr[j]`: pattern prefix matches `value[0..j]`.
    let mut prev = vec![false; m + 1];
    let mut curr = vec![false; m + 1];
    prev[0] = true;

    for &p in &pattern {
        curr[0] = p == '%' && prev[0];
        match p {
            '%' => {
                for j in 1..=m {
                    curr[j] = prev[j] || curr[j - 1];
                }
            }
            '_' => {
                for j in 1..=m {
                    curr[j] = prev[j - 1];
                }
            }
            literal => {
                for j in 1..=m {
                    curr[j] = prev[j - 1] && value[j - 1] == literal;
                }
            }
        }
        std::mem::swap(&mut prev, &mut curr);
    }

    prev[m]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn percent_wildcard() {
        assert!(sql_like_matches("%", "hello"));
        assert!(sql_like_matches("h%", "hello"));
        assert!(sql_like_matches("%o", "hello"));
        assert!(sql_like_matches("%ell%", "hello"));
        assert!(sql_like_matches("h%o", "hello"));
        assert!(!sql_like_matches("x%", "hello"));
        assert!(!sql_like_matches("%x", "hello"));
    }

    #[test]
    fn underscore_single_char() {
        assert!(sql_like_matches("_", "a"));
        assert!(sql_like_matches("a_", "ab"));
        assert!(sql_like_matches("_b", "ab"));
        assert!(sql_like_matches("a_c", "abc"));
        assert!(!sql_like_matches("_", "ab"));
        assert!(!sql_like_matches("__", "a"));
        assert!(!sql_like_matches("_", ""));
    }

    #[test]
    fn literal_and_case_sensitive() {
        assert!(sql_like_matches("abc", "abc"));
        assert!(!sql_like_matches("abc", "ab"));
        assert!(!sql_like_matches("ab", "abc"));
        assert!(!sql_like_matches("Abc", "abc"));
        assert!(!sql_like_matches("abc", "Abc"));
        assert!(!sql_like_matches("abc", "abd"));
    }

    #[test]
    fn empty_pattern_and_value() {
        assert!(sql_like_matches("", ""));
        assert!(!sql_like_matches("a", ""));
        assert!(!sql_like_matches("", "a"));
        assert!(sql_like_matches("%", ""));
        assert!(sql_like_matches("%%", ""));
        assert!(!sql_like_matches("_", ""));
    }

    #[test]
    fn oversized_returns_false() {
        let long = "v".repeat(MAX_SQL_LIKE_LEN + 1);
        let long_pattern = "%".repeat(MAX_SQL_LIKE_LEN + 1);
        assert!(!sql_like_matches(&long_pattern, &long));
        assert!(!sql_like_matches("%", &long));
        assert!(!sql_like_matches(&long_pattern, "ok"));
        assert!(!sql_like_matches(&long, "a"));
        assert!(!sql_like_matches("a", &long));
    }

    #[test]
    fn exactly_at_512_boundary_works() {
        let at_cap_v = "a".repeat(MAX_SQL_LIKE_LEN);
        let at_cap_p = "%".repeat(MAX_SQL_LIKE_LEN);
        assert!(sql_like_matches(&at_cap_p, &at_cap_v));
        assert!(sql_like_matches("%", &at_cap_v));
        assert!(sql_like_matches(&at_cap_v, &at_cap_v));
    }

    #[test]
    fn multibyte_utf8_matched_as_char() {
        assert!(sql_like_matches("_", "中"));
        assert!(sql_like_matches("中_", "中文"));
        assert!(!sql_like_matches("_", "中文"));
        assert!(sql_like_matches("%文", "中文"));
        assert!(sql_like_matches("中%", "中文"));
    }

    #[test]
    fn dense_percent_pattern_does_not_explode() {
        // Dense `%` patterns must stay O(n×m) time / O(m) space — result pin only.
        assert!(sql_like_matches("%%%%", "ab"));
        assert!(sql_like_matches("%a%b%", "xaybz"));
        assert!(!sql_like_matches("%a%b%", "xbyaz"));
        assert!(sql_like_matches("%%%%%%%%%%", "short"));
    }
}
