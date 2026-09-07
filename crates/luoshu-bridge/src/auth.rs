//! Bearer-token auth for the loopback bridge.

/// Constant-time-ish bearer check. The token is per-run random and the server
/// is loopback-only, but we still avoid early-exit comparisons on principle.
pub fn bearer_token_valid(expected: &str, header_value: Option<&str>) -> bool {
    let Some(value) = header_value else {
        return false;
    };
    let Some(provided) = value.strip_prefix("Bearer ") else {
        return false;
    };
    constant_time_eq(expected.as_bytes(), provided.as_bytes())
}

fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    let mut diff = 0u8;
    for (x, y) in a.iter().zip(b.iter()) {
        diff |= x ^ y;
    }
    diff == 0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_matching_bearer() {
        assert!(bearer_token_valid("abc123", Some("Bearer abc123")));
    }

    #[test]
    fn rejects_wrong_missing_malformed() {
        assert!(!bearer_token_valid("abc123", Some("Bearer abc124")));
        assert!(!bearer_token_valid("abc123", None));
        assert!(!bearer_token_valid("abc123", Some("abc123")));
        assert!(!bearer_token_valid("abc123", Some("Bearer ")));
        assert!(!bearer_token_valid("abc123", Some("Basic abc123")));
        assert!(!bearer_token_valid("abc123", Some("Bearer abc123extra")));
    }
}
