//! Wall-clock checks for tests that must not flake on a slow or busy machine.
//!
//! An absolute time limit fails whenever the machine is slower than the one it was set on.
//! These compare runs on inputs of two sizes instead, so the machine's speed cancels out,
//! and take the fastest of several runs, because other work on the machine only ever
//! slows a run down. A fixed allowance absorbs timer noise on runs of a few microseconds.

use std::time::{Duration, Instant};

/// Runs of each size; the fastest counts.
const RUNS: usize = 3;
/// Added to the limit, for scheduling and timer noise on very short runs.
const NOISE: Duration = Duration::from_millis(20);

/// The fastest of [`RUNS`] runs of `f`.
pub fn fastest(mut f: impl FnMut()) -> Duration {
    (0..RUNS)
        .map(|_| {
            let start = Instant::now();
            f();
            start.elapsed()
        })
        .min()
        .unwrap_or_default()
}

/// Assert that `work(n)` takes time at most linear in `n`: on an input `factor` times the
/// size of `small`, it may take at most twice `factor` times as long. Work that is
/// quadratic in `n` takes `factor`² times as long, so with `factor` 4 or more it fails.
pub fn assert_linear(what: &str, small: usize, factor: usize, mut work: impl FnMut(usize)) {
    let t_small = fastest(|| work(small));
    let t_big = fastest(|| work(small * factor));
    let limit = t_small * (2 * factor) as u32 + NOISE;
    assert!(
        t_big <= limit,
        "{what}: {} bytes took {t_small:?}, {} bytes took {t_big:?} (more than {limit:?}): \
         the work grows faster than the input",
        small,
        small * factor
    );
}
