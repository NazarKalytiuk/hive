use fake::faker::internet::en::{FreeEmail, IPv4, IPv6, Username};
use fake::faker::lorem::en::{Sentence, Word, Words};
use fake::faker::name::en::{FirstName, LastName, Name};
use fake::faker::phone_number::en::PhoneNumber;
use fake::Fake;
use rand::seq::IndexedRandom;
use rand::{Rng, RngCore};
use uuid::Uuid;

use crate::faker::with_rng;

/// Evaluate a built-in function expression.
/// Returns Some(result) if the expression is a recognized built-in, None otherwise.
pub fn evaluate(expr: &str) -> Option<String> {
    let expr = expr.trim();

    // --- UUIDs ---
    if expr == "$uuid" || expr == "$uuid_v4" {
        return Some(with_rng(|r| build_uuid_v4(r).to_string()));
    }

    if expr == "$uuid_v7" {
        return Some(with_rng(|r| build_uuid_v7(r).to_string()));
    }

    // --- wall-clock helpers (intentionally not seeded) ---
    if expr == "$timestamp" {
        return Some(chrono::Utc::now().timestamp().to_string());
    }

    if expr == "$now_iso" {
        return Some(chrono::Utc::now().to_rfc3339());
    }

    // --- faker corpora ---
    if expr == "$email" {
        return Some(with_rng(|r| FreeEmail().fake_with_rng::<String, _>(r)));
    }

    if expr == "$first_name" {
        return Some(with_rng(|r| FirstName().fake_with_rng::<String, _>(r)));
    }

    if expr == "$last_name" {
        return Some(with_rng(|r| LastName().fake_with_rng::<String, _>(r)));
    }

    if expr == "$name" {
        return Some(with_rng(|r| Name().fake_with_rng::<String, _>(r)));
    }

    if expr == "$username" {
        return Some(with_rng(|r| Username().fake_with_rng::<String, _>(r)));
    }

    if expr == "$phone" {
        return Some(with_rng(|r| PhoneNumber().fake_with_rng::<String, _>(r)));
    }

    if expr == "$word" {
        return Some(with_rng(|r| Word().fake_with_rng::<String, _>(r)));
    }

    if expr == "$sentence" {
        return Some(with_rng(|r| Sentence(4..10).fake_with_rng::<String, _>(r)));
    }

    if expr == "$slug" {
        return Some(with_rng(|r| {
            let parts: Vec<String> = Words(2..4).fake_with_rng(r);
            parts.join("-").to_ascii_lowercase()
        }));
    }

    if expr == "$bool" {
        return Some(with_rng(|r| {
            if r.random::<bool>() { "true" } else { "false" }.to_owned()
        }));
    }

    if expr == "$ipv4" {
        return Some(with_rng(|r| IPv4().fake_with_rng::<String, _>(r)));
    }

    if expr == "$ipv6" {
        return Some(with_rng(|r| IPv6().fake_with_rng::<String, _>(r)));
    }

    // --- parameterized built-ins ---
    if let Some(inner) = strip_func(expr, "$random_hex") {
        let len: usize = inner.parse().ok()?;
        return Some(with_rng(|r| {
            (0..len)
                .map(|_| format!("{:x}", r.random_range(0u8..16)))
                .collect::<String>()
        }));
    }

    if let Some(inner) = strip_func(expr, "$random_int") {
        let parts: Vec<&str> = inner.split(',').map(|s| s.trim()).collect();
        if parts.len() == 2 {
            let min: i64 = parts[0].parse().ok()?;
            let max: i64 = parts[1].parse().ok()?;
            return Some(with_rng(|r| r.random_range(min..=max).to_string()));
        }
        return None;
    }

    if let Some(inner) = strip_func(expr, "$words") {
        let n: usize = inner.trim().parse().ok()?;
        if n == 0 {
            return Some(String::new());
        }
        return Some(with_rng(|r| {
            let parts: Vec<String> = Words(n..n + 1).fake_with_rng(r);
            parts.join(" ")
        }));
    }

    if let Some(inner) = strip_func(expr, "$alpha") {
        let n: usize = inner.trim().parse().ok()?;
        return Some(with_rng(|r| {
            (0..n)
                .map(|_| (b'a' + r.random_range(0u8..26)) as char)
                .collect::<String>()
        }));
    }

    if let Some(inner) = strip_func(expr, "$alnum") {
        let n: usize = inner.trim().parse().ok()?;
        return Some(with_rng(|r| {
            const ALPHABET: &[u8] = b"abcdefghijklmnopqrstuvwxyz0123456789";
            (0..n)
                .map(|_| ALPHABET[r.random_range(0..ALPHABET.len())] as char)
                .collect::<String>()
        }));
    }

    if let Some(inner) = strip_func(expr, "$choice") {
        let options: Vec<&str> = inner.split(',').map(|s| s.trim()).collect();
        if options.is_empty() || options.iter().any(|s| s.is_empty()) {
            return None;
        }
        return Some(with_rng(|r| {
            (*options.choose(r).expect("non-empty")).to_owned()
        }));
    }

    None
}

/// Build a UUID v4 from the given RNG so seeded runs stay reproducible.
///
/// `Uuid::new_v4` reads from `getrandom` internally, which bypasses
/// Tarn's seeded RNG. Constructing the UUID from an RNG-filled byte
/// buffer routes through the same seed as every other built-in.
fn build_uuid_v4(r: &mut dyn RngCore) -> Uuid {
    let mut bytes = [0u8; 16];
    r.fill_bytes(&mut bytes);
    uuid::Builder::from_random_bytes(bytes).into_uuid()
}

/// Build a UUID v7 using a wall-clock millisecond timestamp and
/// RNG-sourced random bytes. The timestamp stays wall-clock even when
/// seeded (see `faker` module docs); only the random portion is
/// determined by the seed, which is enough to keep v7 valid and
/// sortable while matching the spirit of the determinism contract.
fn build_uuid_v7(r: &mut dyn RngCore) -> Uuid {
    let ts_millis = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64;
    let mut rand_bytes = [0u8; 10];
    r.fill_bytes(&mut rand_bytes);
    uuid::Builder::from_unix_timestamp_millis(ts_millis, &rand_bytes).into_uuid()
}

/// Extract the arguments from a function call like "$func_name(args)".
fn strip_func<'a>(expr: &'a str, prefix: &str) -> Option<&'a str> {
    let rest = expr.strip_prefix(prefix)?;
    let rest = rest.strip_prefix('(')?;
    let rest = rest.strip_suffix(')')?;
    Some(rest)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::faker;

    fn reset() {
        faker::reset_for_test(None);
    }

    #[test]
    fn uuid_generates_valid_uuid() {
        reset();
        let result = evaluate("$uuid").unwrap();
        assert_eq!(result.len(), 36);
        assert!(result.contains('-'));
        assert!(Uuid::parse_str(&result).is_ok());
    }

    #[test]
    fn timestamp_returns_number() {
        let result = evaluate("$timestamp").unwrap();
        let ts: i64 = result.parse().unwrap();
        assert!(ts > 1_000_000_000);
    }

    #[test]
    fn now_iso_returns_valid_datetime() {
        let result = evaluate("$now_iso").unwrap();
        assert!(result.contains('T'));
        assert!(result.contains('+') || result.contains('Z'));
    }

    #[test]
    fn random_hex_correct_length() {
        reset();
        let result = evaluate("$random_hex(8)").unwrap();
        assert_eq!(result.len(), 8);
        assert!(result.chars().all(|c| c.is_ascii_hexdigit()));
    }

    #[test]
    fn random_hex_different_lengths() {
        reset();
        let r4 = evaluate("$random_hex(4)").unwrap();
        assert_eq!(r4.len(), 4);
        let r16 = evaluate("$random_hex(16)").unwrap();
        assert_eq!(r16.len(), 16);
    }

    #[test]
    fn random_int_in_range() {
        reset();
        for _ in 0..100 {
            let result = evaluate("$random_int(1, 10)").unwrap();
            let val: i64 = result.parse().unwrap();
            assert!((1..=10).contains(&val));
        }
    }

    #[test]
    fn random_int_negative_range() {
        reset();
        for _ in 0..50 {
            let result = evaluate("$random_int(-5, 5)").unwrap();
            let val: i64 = result.parse().unwrap();
            assert!((-5..=5).contains(&val));
        }
    }

    #[test]
    fn random_int_single_value() {
        let result = evaluate("$random_int(42, 42)").unwrap();
        assert_eq!(result, "42");
    }

    #[test]
    fn unknown_builtin_returns_none() {
        assert!(evaluate("$unknown").is_none());
        assert!(evaluate("$not_a_function(1)").is_none());
        assert!(evaluate("plain text").is_none());
    }

    #[test]
    fn random_hex_invalid_arg() {
        assert!(evaluate("$random_hex(abc)").is_none());
    }

    #[test]
    fn random_int_wrong_arg_count() {
        assert!(evaluate("$random_int(1)").is_none());
        assert!(evaluate("$random_int(1, 2, 3)").is_none());
    }

    #[test]
    fn strip_func_helper() {
        assert_eq!(strip_func("$random_hex(8)", "$random_hex"), Some("8"));
        assert_eq!(
            strip_func("$random_int(1, 10)", "$random_int"),
            Some("1, 10")
        );
        assert_eq!(strip_func("$other(x)", "$random_hex"), None);
        assert_eq!(strip_func("$random_hex", "$random_hex"), None);
    }

    #[test]
    fn uuid_generates_unique_values() {
        reset();
        let a = evaluate("$uuid").unwrap();
        let b = evaluate("$uuid").unwrap();
        assert_ne!(a, b);
    }

    #[test]
    fn uuid_v4_generates_v4() {
        reset();
        let result = evaluate("$uuid_v4").unwrap();
        let parsed = Uuid::parse_str(&result).unwrap();
        assert_eq!(parsed.get_version_num(), 4);
    }

    #[test]
    fn uuid_v7_generates_v7() {
        reset();
        let result = evaluate("$uuid_v7").unwrap();
        let parsed = Uuid::parse_str(&result).unwrap();
        assert_eq!(parsed.get_version_num(), 7);
    }

    #[test]
    fn email_is_syntactically_valid() {
        reset();
        let email = evaluate("$email").unwrap();
        assert!(email.contains('@'), "expected `@` in {email}");
        let parts: Vec<_> = email.split('@').collect();
        assert_eq!(parts.len(), 2);
        assert!(!parts[0].is_empty());
        assert!(parts[1].contains('.'));
    }

    #[test]
    fn names_are_non_empty_strings() {
        reset();
        for builtin in ["$first_name", "$last_name", "$name", "$username"] {
            let v = evaluate(builtin).unwrap();
            assert!(!v.is_empty(), "empty result for {builtin}");
        }
    }

    #[test]
    fn phone_is_non_empty() {
        reset();
        assert!(!evaluate("$phone").unwrap().is_empty());
    }

    #[test]
    fn word_is_single_token() {
        reset();
        let w = evaluate("$word").unwrap();
        assert!(!w.is_empty());
        assert!(!w.contains(' '), "expected single word, got {w}");
    }

    #[test]
    fn sentence_has_multiple_words() {
        reset();
        let s = evaluate("$sentence").unwrap();
        assert!(s.split_whitespace().count() >= 3);
    }

    #[test]
    fn slug_is_hyphen_joined_lowercase() {
        reset();
        let s = evaluate("$slug").unwrap();
        assert!(s.contains('-'));
        assert_eq!(s, s.to_ascii_lowercase());
        assert!(!s.contains(' '));
    }

    #[test]
    fn bool_is_true_or_false() {
        reset();
        for _ in 0..10 {
            let b = evaluate("$bool").unwrap();
            assert!(b == "true" || b == "false");
        }
    }

    #[test]
    fn ipv4_is_parseable() {
        use std::net::Ipv4Addr;
        reset();
        let ip = evaluate("$ipv4").unwrap();
        ip.parse::<Ipv4Addr>().unwrap();
    }

    #[test]
    fn ipv6_is_parseable() {
        use std::net::Ipv6Addr;
        reset();
        let ip = evaluate("$ipv6").unwrap();
        ip.parse::<Ipv6Addr>().unwrap();
    }

    #[test]
    fn words_n_has_n_tokens() {
        reset();
        let w = evaluate("$words(5)").unwrap();
        assert_eq!(w.split_whitespace().count(), 5);
    }

    #[test]
    fn alpha_n_is_lowercase_letters() {
        reset();
        let a = evaluate("$alpha(12)").unwrap();
        assert_eq!(a.len(), 12);
        assert!(a.chars().all(|c| c.is_ascii_lowercase()));
    }

    #[test]
    fn alnum_n_is_alphanumeric() {
        reset();
        let a = evaluate("$alnum(16)").unwrap();
        assert_eq!(a.len(), 16);
        assert!(a.chars().all(|c| c.is_ascii_alphanumeric()));
    }

    #[test]
    fn choice_picks_one_option() {
        reset();
        for _ in 0..20 {
            let c = evaluate("$choice(red, green, blue)").unwrap();
            assert!(matches!(c.as_str(), "red" | "green" | "blue"));
        }
    }

    #[test]
    fn choice_rejects_empty_option() {
        assert!(evaluate("$choice(a, , c)").is_none());
    }

    #[test]
    fn seeded_mode_is_deterministic_for_rng_builtins() {
        // Timestamps / now_iso / the uuid_v7 timestamp prefix are
        // wall-clock by design, so we don't include them here.
        for builtin in [
            "$uuid",
            "$uuid_v4",
            "$random_hex(16)",
            "$random_int(0, 1000000)",
            "$email",
            "$first_name",
            "$last_name",
            "$name",
            "$username",
            "$phone",
            "$word",
            "$words(4)",
            "$sentence",
            "$slug",
            "$alpha(12)",
            "$alnum(16)",
            "$choice(red, green, blue)",
            "$bool",
            "$ipv4",
            "$ipv6",
        ] {
            faker::reset_for_test(Some(42));
            let a = evaluate(builtin).unwrap();
            faker::reset_for_test(Some(42));
            let b = evaluate(builtin).unwrap();
            assert_eq!(a, b, "non-deterministic under seed: {builtin}");
        }
        faker::reset_for_test(None);
    }
}
