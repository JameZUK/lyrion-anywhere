//! Sliding-window extremes for the limiter and the true-peak detector.

/// Sliding-window extreme (minimum, or maximum via negation) over the last `window`
/// values, with a fixed-capacity monotonic queue: O(1) amortised, no allocation.
#[derive(Debug, Clone)]
pub(crate) struct SlidingMin {
    pub(crate) window: u64,
    idx: Vec<u64>,
    val: Vec<f64>,
    head: usize,
    len: usize,
    count: u64,
}

impl SlidingMin {
    pub(crate) fn new(window: usize) -> Self {
        let cap = window + 1;
        SlidingMin {
            window: window as u64,
            idx: vec![0; cap],
            val: vec![0.0; cap],
            head: 0,
            len: 0,
            count: 0,
        }
    }

    fn cap(&self) -> usize {
        self.idx.len()
    }

    /// Push a value; returns the minimum of the last `window` values pushed.
    #[inline]
    pub(crate) fn push(&mut self, v: f64) -> f64 {
        let i = self.count;
        self.count += 1;
        // Drop larger values from the back: they can never be the minimum again.
        while self.len > 0 {
            let back = (self.head + self.len - 1) % self.cap();
            if self.val[back] >= v {
                self.len -= 1;
            } else {
                break;
            }
        }
        let slot = (self.head + self.len) % self.cap();
        self.idx[slot] = i;
        self.val[slot] = v;
        self.len += 1;
        // Expire the front.
        while self.idx[self.head] + self.window <= i {
            self.head = (self.head + 1) % self.cap();
            self.len -= 1;
        }
        self.val[self.head]
    }

    pub(crate) fn reset(&mut self) {
        self.head = 0;
        self.len = 0;
        self.count = 0;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sliding_min_matches_brute_force() {
        let mut s = SlidingMin::new(5);
        let vals = [
            5.0, 3.0, 4.0, 9.0, 1.0, 7.0, 8.0, 6.0, 6.0, 2.0, 9.0, 9.0, 9.0, 9.0, 9.0,
        ];
        for (i, v) in vals.iter().enumerate() {
            let got = s.push(*v);
            let lo = i.saturating_sub(4);
            let want = vals[lo..=i].iter().cloned().fold(f64::INFINITY, f64::min);
            assert_eq!(got, want, "at {i}");
        }
    }
}
