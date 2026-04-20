//! Faker / RNG seed source for built-in interpolation functions.
//!
//! When `TARN_FAKER_SEED` or `tarn.config.yaml` → `faker.seed` is set,
//! every randomness-backed built-in (`$uuid`, `$uuid_v4`, `$random_hex`,
//! `$random_int`, `$email`, `$first_name`, …) draws from a deterministic
//! per-thread [`StdRng`]. When unset, built-ins fall back to
//! [`rand::rng`] and behave exactly as they did before NAZ-398.
//!
//! Wall-clock built-ins (`$timestamp`, `$now_iso`, and the Unix-ms
//! prefix of `$uuid_v7`) are deliberately *not* frozen — seeding only
//! governs the RNG path.

use std::cell::RefCell;
use std::sync::OnceLock;

use rand::rngs::StdRng;
use rand::{RngCore, SeedableRng};

static FAKER_SEED: OnceLock<Option<u64>> = OnceLock::new();

/// Resolve the faker seed from its two sources. `TARN_FAKER_SEED`
/// overrides the config value. Idempotent — subsequent calls are
/// ignored, so tests and embedders that call it more than once get
/// stable behavior.
pub fn init_seed_from_sources(config_seed: Option<u64>) {
    let env_seed = std::env::var("TARN_FAKER_SEED")
        .ok()
        .and_then(|s| s.trim().parse::<u64>().ok());
    let resolved = env_seed.or(config_seed);
    let _ = FAKER_SEED.set(resolved);
}

/// Current seed, if any. `None` means system RNG.
pub fn active_seed() -> Option<u64> {
    FAKER_SEED.get().copied().flatten()
}

thread_local! {
    static SEEDED_RNG: RefCell<Option<StdRng>> = const { RefCell::new(None) };
}

/// Run `f` with a mutable RNG reference.
///
/// Selection order:
///   1. A thread-local seeded RNG if one has been installed (tests use
///      this to guarantee reproducible state without racing against the
///      process-wide [`OnceLock`]).
///   2. A thread-local RNG seeded from the process-wide seed if one
///      was installed via [`init_seed_from_sources`].
///   3. A fresh [`rand::rng`] handle (non-deterministic, original
///      behavior).
pub fn with_rng<T>(f: impl FnOnce(&mut dyn RngCore) -> T) -> T {
    let has_tls = SEEDED_RNG.with(|cell| cell.borrow().is_some());
    if has_tls {
        return SEEDED_RNG.with(|cell| {
            let mut guard = cell.borrow_mut();
            let rng = guard.as_mut().expect("is_some checked above");
            f(rng)
        });
    }
    match active_seed() {
        Some(seed) => SEEDED_RNG.with(|cell| {
            let mut guard = cell.borrow_mut();
            let rng = guard.get_or_insert_with(|| StdRng::seed_from_u64(seed));
            f(rng)
        }),
        None => {
            let mut rng = rand::rng();
            f(&mut rng)
        }
    }
}

/// Install a per-thread seeded RNG. Intended for tests and embedders
/// that want reproducible output without mutating the process-wide
/// [`OnceLock`]. Passing `None` clears the thread-local so subsequent
/// calls on this thread fall back to [`rand::rng`] (or the process
/// seed, if one is active).
#[cfg(test)]
pub fn install_thread_seed(seed: Option<u64>) {
    SEEDED_RNG.with(|cell| {
        *cell.borrow_mut() = seed.map(StdRng::seed_from_u64);
    });
}

#[cfg(test)]
pub(crate) fn reset_for_test(seed: Option<u64>) {
    install_thread_seed(seed);
}

#[cfg(test)]
mod tests {
    use super::*;
    use rand::Rng;

    #[test]
    fn installed_thread_seed_is_deterministic() {
        install_thread_seed(Some(42));
        let a: u64 = with_rng(|r| r.random());
        install_thread_seed(Some(42));
        let b: u64 = with_rng(|r| r.random());
        assert_eq!(a, b);
        install_thread_seed(None);
    }

    #[test]
    fn different_seeds_yield_different_streams() {
        install_thread_seed(Some(1));
        let a: u64 = with_rng(|r| r.random());
        install_thread_seed(Some(2));
        let b: u64 = with_rng(|r| r.random());
        assert_ne!(a, b);
        install_thread_seed(None);
    }
}
