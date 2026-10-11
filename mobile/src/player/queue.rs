//! The local play queue (SPEC D14: offline, the app owns the queue): the order tracks play
//! in, shuffle and repeat. Pure: no I/O, no clock, so every rule is unit-tested.
//!
//! Tracks are referred to by their index in the list the app gave; the *play order* is a
//! permutation of those indices (the identity unless shuffled), and the queue's position
//! is an index into the play order.

/// Repeat mode of the local queue.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, uniffi::Enum)]
pub enum LocalRepeat {
    Off,
    /// The whole queue, round again.
    All,
    /// The current track, again and again.
    One,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct PlayQueue {
    order: Vec<usize>,
    pos: usize,
    shuffle: bool,
    repeat: LocalRepeat,
    rng: u64,
}

fn xorshift(s: &mut u64) -> u64 {
    *s ^= *s << 13;
    *s ^= *s >> 7;
    *s ^= *s << 17;
    *s
}

impl PlayQueue {
    /// A queue of `len` tracks starting at list index `start` (clamped). `None` if empty.
    pub(crate) fn new(
        len: usize,
        start: usize,
        shuffle: bool,
        repeat: LocalRepeat,
        seed: u64,
    ) -> Option<PlayQueue> {
        if len == 0 {
            return None;
        }
        let mut q = PlayQueue {
            order: (0..len).collect(),
            pos: start.min(len - 1),
            shuffle: false,
            repeat,
            rng: seed | 1,
        };
        if shuffle {
            q.set_shuffle(true);
        }
        Some(q)
    }

    /// The list index of the current track.
    pub(crate) fn current(&self) -> usize {
        self.order[self.pos]
    }

    /// The current position in the play order.
    pub(crate) fn position(&self) -> usize {
        self.pos
    }

    /// The list index at play-order position `pos`.
    pub(crate) fn index_at(&self, pos: usize) -> Option<usize> {
        self.order.get(pos).copied()
    }

    /// The play order (list indices).
    pub(crate) fn order(&self) -> &[usize] {
        &self.order
    }

    pub(crate) fn shuffle(&self) -> bool {
        self.shuffle
    }

    pub(crate) fn repeat(&self) -> LocalRepeat {
        self.repeat
    }

    /// What plays after position `pos` when it ends by itself (gapless look-ahead).
    pub(crate) fn after(&self, pos: usize) -> Option<usize> {
        let n = self.order.len();
        match self.repeat {
            LocalRepeat::One => Some(pos.min(n - 1)),
            LocalRepeat::All => Some((pos + 1) % n),
            LocalRepeat::Off => (pos + 1 < n).then_some(pos + 1),
        }
    }

    /// The position "next" goes to: the following track (round again under any repeat);
    /// `None` at the end without repeat.
    pub(crate) fn next_pos(&self) -> Option<usize> {
        let n = self.order.len();
        if self.pos + 1 < n {
            Some(self.pos + 1)
        } else if self.repeat != LocalRepeat::Off {
            Some(0)
        } else {
            None
        }
    }

    /// The position "previous" goes to; `None` at the start without repeat (the host then
    /// starts the current track again).
    pub(crate) fn previous_pos(&self) -> Option<usize> {
        if self.pos > 0 {
            Some(self.pos - 1)
        } else if self.repeat != LocalRepeat::Off {
            Some(self.order.len() - 1)
        } else {
            None
        }
    }

    pub(crate) fn set_position(&mut self, pos: usize) {
        self.pos = pos.min(self.order.len() - 1);
    }

    /// Make list index `index` current; false if there is no such track.
    pub(crate) fn jump_to_index(&mut self, index: usize) -> bool {
        match self.order.iter().position(|i| *i == index) {
            Some(p) => {
                self.pos = p;
                true
            }
            None => false,
        }
    }

    /// Shuffle on: the current track stays current, first, and the rest follow in a new
    /// random order. Off: back to list order, at the current track.
    pub(crate) fn set_shuffle(&mut self, on: bool) {
        let cur = self.current();
        if on {
            let mut rest: Vec<usize> = (0..self.order.len()).filter(|i| *i != cur).collect();
            for i in (1..rest.len()).rev() {
                let j = (xorshift(&mut self.rng) % (i as u64 + 1)) as usize;
                rest.swap(i, j);
            }
            self.order = std::iter::once(cur).chain(rest).collect();
            self.pos = 0;
        } else {
            self.order = (0..self.order.len()).collect();
            self.pos = cur;
        }
        self.shuffle = on;
    }

    pub(crate) fn set_repeat(&mut self, repeat: LocalRepeat) {
        self.repeat = repeat;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plays_in_order_and_stops_at_the_end() {
        let mut q = PlayQueue::new(3, 1, false, LocalRepeat::Off, 7).unwrap();
        assert_eq!(q.current(), 1);
        assert_eq!(q.after(1), Some(2));
        assert_eq!(q.after(2), None);
        assert_eq!(q.next_pos(), Some(2));
        q.set_position(2);
        assert_eq!(q.next_pos(), None);
        assert_eq!(q.previous_pos(), Some(1));
        q.set_position(0);
        assert_eq!(q.previous_pos(), None);
        assert!(PlayQueue::new(0, 0, false, LocalRepeat::Off, 1).is_none());
        assert_eq!(
            PlayQueue::new(2, 99, false, LocalRepeat::Off, 1)
                .unwrap()
                .current(),
            1
        );
    }

    #[test]
    fn repeat_all_wraps_and_repeat_one_loops_the_track() {
        let mut q = PlayQueue::new(3, 2, false, LocalRepeat::All, 7).unwrap();
        assert_eq!(q.after(2), Some(0));
        assert_eq!(q.next_pos(), Some(0));
        q.set_position(0);
        assert_eq!(q.previous_pos(), Some(2));
        q.set_repeat(LocalRepeat::One);
        assert_eq!(q.after(0), Some(0), "gapless loop of the same track");
        assert_eq!(q.next_pos(), Some(1), "next still moves on");
    }

    #[test]
    fn shuffle_keeps_the_current_track_and_visits_each_once() {
        let mut q = PlayQueue::new(50, 17, false, LocalRepeat::Off, 99).unwrap();
        q.set_shuffle(true);
        assert_eq!(q.current(), 17);
        assert_eq!(q.position(), 0);
        let mut seen: Vec<usize> = q.order().to_vec();
        seen.sort();
        assert_eq!(seen, (0..50).collect::<Vec<_>>());
        assert_ne!(
            q.order(),
            (0..50).collect::<Vec<_>>().as_slice(),
            "actually shuffled"
        );
        q.set_position(10);
        let cur = q.current();
        q.set_shuffle(false);
        assert_eq!(q.current(), cur);
        assert_eq!(q.order(), (0..50).collect::<Vec<_>>().as_slice());
        assert!(q.jump_to_index(3));
        assert_eq!(q.current(), 3);
        assert!(!q.jump_to_index(50));
    }

    /// Random operations never leave the queue inconsistent.
    #[test]
    fn random_operations_keep_invariants() {
        let mut seed = 0x1234_5678_9abc_def1u64;
        for _ in 0..300 {
            let len = (xorshift(&mut seed) % 20 + 1) as usize;
            let start = (xorshift(&mut seed) % 25) as usize;
            let mut q = PlayQueue::new(len, start, seed.is_multiple_of(2), LocalRepeat::Off, seed).unwrap();
            for _ in 0..100 {
                match xorshift(&mut seed) % 7 {
                    0 => q.set_shuffle(xorshift(&mut seed).is_multiple_of(2)),
                    1 => q.set_repeat(
                        [LocalRepeat::Off, LocalRepeat::All, LocalRepeat::One]
                            [(xorshift(&mut seed) % 3) as usize],
                    ),
                    2 => {
                        if let Some(p) = q.next_pos() {
                            q.set_position(p)
                        }
                    }
                    3 => {
                        if let Some(p) = q.previous_pos() {
                            q.set_position(p)
                        }
                    }
                    4 => {
                        q.jump_to_index((xorshift(&mut seed) % (len as u64 + 2)) as usize);
                    }
                    5 => {
                        if let Some(p) = q.after(q.position()) {
                            q.set_position(p)
                        }
                    }
                    _ => q.set_position((xorshift(&mut seed) % 40) as usize),
                }
                assert!(q.position() < len);
                assert!(q.current() < len);
                let mut o = q.order().to_vec();
                o.sort();
                assert_eq!(o, (0..len).collect::<Vec<_>>());
            }
        }
    }
}
