use crate::assert::types::AssertionResult;
use crate::model::RedirectAssertion;

pub fn assert_redirect(
    expected: &RedirectAssertion,
    actual_url: &str,
    actual_count: u32,
) -> Vec<AssertionResult> {
    let mut results = Vec::new();

    if let Some(expected_url) = expected.url.as_deref() {
        if expected_url == actual_url {
            results.push(AssertionResult::pass(
                "redirect.url",
                expected_url,
                actual_url,
            ));
        } else {
            results.push(AssertionResult::fail(
                "redirect.url",
                expected_url,
                actual_url,
                format!(
                    "Expected final redirect URL '{}', got '{}'",
                    expected_url, actual_url
                ),
            ));
        }
    }

    if let Some(expected_count) = expected.count {
        if expected_count == actual_count {
            results.push(AssertionResult::pass(
                "redirect.count",
                expected_count.to_string(),
                actual_count.to_string(),
            ));
        } else {
            results.push(AssertionResult::fail(
                "redirect.count",
                expected_count.to_string(),
                actual_count.to_string(),
                format!(
                    "Expected redirect count {}, got {}",
                    expected_count, actual_count
                ),
            ));
        }
    }

    results
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn redirect_assertions_pass() {
        let results = assert_redirect(
            &RedirectAssertion {
                url: Some("https://example.com/final".into()),
                count: Some(2),
            },
            "https://example.com/final",
            2,
        );

        assert_eq!(results.len(), 2);
        assert!(results.iter().all(|result| result.passed));
    }

    #[test]
    fn redirect_url_mismatch_fails() {
        let results = assert_redirect(
            &RedirectAssertion {
                url: Some("https://example.com/final".into()),
                count: None,
            },
            "https://example.com/other",
            0,
        );

        assert_eq!(results.len(), 1);
        assert!(!results[0].passed);
        assert_eq!(results[0].assertion, "redirect.url");
    }

    #[test]
    fn redirect_count_mismatch_fails() {
        let results = assert_redirect(
            &RedirectAssertion {
                url: None,
                count: Some(3),
            },
            "https://example.com/final",
            2,
        );

        assert_eq!(results.len(), 1);
        assert!(!results[0].passed);
        assert_eq!(results[0].assertion, "redirect.count");
        assert!(results[0]
            .message
            .contains("Expected redirect count 3, got 2"));
    }
}
