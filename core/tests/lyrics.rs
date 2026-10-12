//! Robustness of the lyrics parser: tag text is untrusted, so arbitrary, mutated and
//! adversarial input must never panic, and the result must always respect the limits.

use anywhere_core::lyrics::{Lyrics, MAX_LINE_CHARS, MAX_LINES};

/// Small deterministic PRNG (xorshift64*), so failures are reproducible from the seed.
struct Rng(u64);
impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        self.0.wrapping_mul(0x2545_f491_4f6c_dd1d)
    }
    fn below(&mut self, n: u64) -> u64 {
        self.next() % n
    }
}

/// The invariants every parse must keep.
fn check(input: &str) {
    let l = Lyrics::parse(input);
    assert!(l.lines.len() <= MAX_LINES);
    for line in &l.lines {
        assert!(line.text.chars().count() <= MAX_LINE_CHARS);
        assert!(!line.text.chars().any(char::is_control));
        assert_eq!(line.text, line.text.trim());
        if !l.synced {
            assert_eq!(line.time_ms, 0);
        }
    }
    if l.synced {
        assert!(l.lines.windows(2).all(|w| w[0].time_ms <= w[1].time_ms));
        // line_at agrees with a linear scan at, between and around every time.
        for probe in l.lines.iter().take(50).flat_map(|x| {
            [
                x.time_ms.saturating_sub(1),
                x.time_ms,
                x.time_ms.saturating_add(1),
            ]
        }) {
            let expect = l.lines.iter().rposition(|x| x.time_ms <= probe);
            assert_eq!(l.line_at(probe), expect);
        }
    } else {
        assert_eq!(l.line_at(1000), None);
    }
}

/// Pieces that look like LRC, to steer random input towards the interesting paths.
const PIECES: &[&str] = &[
    "[",
    "]",
    "<",
    ">",
    ":",
    ".",
    "\n",
    "\r\n",
    "\r",
    " ",
    "\t",
    "[00:01.00]",
    "[59:59.999]",
    "[999:59]",
    "[00:60]",
    "[offset:+",
    "[offset:-99999999999999999999]",
    "[ar:",
    "<00:01.5>",
    "0",
    "9",
    "é",
    "😀",
    "\u{0}",
    "\u{202e}",
    "[00:00:00]",
    "]]]]",
    "[[[[",
    "words",
];

#[test]
fn random_bytes_never_panic() {
    let mut r = Rng(0x5eed_1a7c);
    for _ in 0..3000 {
        let n = r.below(400) as usize;
        let bytes: Vec<u8> = (0..n).map(|_| r.next() as u8).collect();
        check(&String::from_utf8_lossy(&bytes));
    }
}

#[test]
fn lrc_shaped_input_never_panics() {
    let mut r = Rng(0x0dd_ba11);
    for _ in 0..5000 {
        let n = r.below(60) as usize;
        let s: String = (0..n)
            .map(|_| PIECES[r.below(PIECES.len() as u64) as usize])
            .collect();
        check(&s);
    }
}

#[test]
fn mutated_real_lrc_never_panics() {
    let base = "[ar:Someone]\n[offset:+250]\n[00:08.58] I'm in some kind of trouble\n\
                [00:12.23][01:12.23]<00:12.23>Chorus <00:13.00>line\n[00:19.57]\n[02:00]End\n";
    let mut r = Rng(0xfeed_f00d);
    for _ in 0..5000 {
        let mut b = base.as_bytes().to_vec();
        for _ in 0..1 + r.below(6) {
            let i = r.below(b.len() as u64) as usize;
            match r.below(3) {
                0 => b[i] = r.next() as u8,
                1 => {
                    b.remove(i);
                }
                _ => b.insert(i, b"[]:.<>0\n"[r.below(8) as usize]),
            }
            if b.is_empty() {
                break;
            }
        }
        check(&String::from_utf8_lossy(&b));
    }
}

#[test]
fn oversize_input_is_bounded() {
    // Megabytes of times on one line, and of lines: the parse stays within its limits.
    let one_line = "[00:01.00]".repeat(400_000);
    check(&one_line);
    let lines = "[00:01.00]x\n".repeat(400_000);
    check(&lines);
    let plain = format!("{}\n", "y".repeat(5_000_000));
    check(&plain);
}
