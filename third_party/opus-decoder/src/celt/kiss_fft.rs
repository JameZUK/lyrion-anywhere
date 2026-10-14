#![allow(dead_code)]

//! Minimal CELT FFT primitives for decoder path.
//!
//! This module intentionally implements only the inverse transform behavior
//! needed by the decoder. It follows the libopus scaling convention where the
//! inverse FFT is unscaled and the forward path applies `1/N`.

use core::f32::consts::PI;

/// Complex number used by CELT FFT/MDCT primitives.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct Complex32 {
    /// Real component.
    pub re: f32,
    /// Imaginary component.
    pub im: f32,
}

impl Complex32 {
    /// Create a complex value from real and imaginary parts.
    ///
    /// Params: `re` is the real component, `im` is the imaginary component.
    /// Returns: a new `Complex32`.
    pub const fn new(re: f32, im: f32) -> Self {
        Self { re, im }
    }
}

/// Inverse-only FFT plan for CELT decoder usage.
#[derive(Debug, Clone)]
pub(crate) struct KissFft {
    nfft: usize,
    inv_twiddles: Vec<Complex32>,
}

impl KissFft {
    /// Build an inverse FFT plan with runtime twiddle generation.
    ///
    /// Params: `nfft` is the FFT size.
    /// Returns: a plan containing precomputed inverse twiddles.
    pub fn new(nfft: usize) -> Self {
        let mut inv_twiddles = Vec::with_capacity(nfft);
        for k in 0..nfft {
            let phase = 2.0 * PI * (k as f32) / (nfft as f32);
            inv_twiddles.push(Complex32::new(phase.cos(), phase.sin()));
        }
        Self { nfft, inv_twiddles }
    }

    /// Return FFT size configured in this plan.
    ///
    /// Params: none.
    /// Returns: number of points in the transform.
    pub fn len(&self) -> usize {
        self.nfft
    }

    /// Compute unscaled inverse FFT.
    ///
    /// Params: `input` is frequency-domain complex spectrum, `output` is
    /// destination time-domain buffer and must have length `self.len()`.
    /// Returns: `Ok(())` on success, otherwise a static validation error.
    pub fn ifft(&self, input: &[Complex32], output: &mut [Complex32]) -> Result<(), &'static str> {
        if input.len() != self.nfft || output.len() != self.nfft {
            return Err("kiss_fft length mismatch");
        }
        for out in output.iter_mut() {
            *out = Complex32::new(0.0, 0.0);
        }
        for (n, out) in output.iter_mut().enumerate() {
            let mut acc_re = 0.0f32;
            let mut acc_im = 0.0f32;
            for (k, xk) in input.iter().enumerate() {
                let tw = self.inv_twiddles[(k * n) % self.nfft];
                acc_re += xk.re * tw.re - xk.im * tw.im;
                acc_im += xk.re * tw.im + xk.im * tw.re;
            }
            *out = Complex32::new(acc_re, acc_im);
        }
        Ok(())
    }

    /// Compute unscaled forward FFT.
    ///
    /// Params: `input` is time-domain complex vector, `output` is destination
    /// frequency-domain buffer and must have length `self.len()`.
    /// Returns: `Ok(())` on success, otherwise a static validation error.
    pub fn fft(&self, input: &[Complex32], output: &mut [Complex32]) -> Result<(), &'static str> {
        if input.len() != self.nfft || output.len() != self.nfft {
            return Err("kiss_fft length mismatch");
        }
        for out in output.iter_mut() {
            *out = Complex32::new(0.0, 0.0);
        }
        for (n, out) in output.iter_mut().enumerate() {
            let mut acc_re = 0.0f32;
            let mut acc_im = 0.0f32;
            for (k, xk) in input.iter().enumerate() {
                let tw = self.inv_twiddles[(k * n) % self.nfft];
                // Conjugate inverse twiddle gives forward phase.
                acc_re += xk.re * tw.re + xk.im * tw.im;
                acc_im += -xk.re * tw.im + xk.im * tw.re;
            }
            *out = Complex32::new(acc_re, acc_im);
        }
        Ok(())
    }
}

/// Forward FFT plan on flat interleaved complex buffers, mixed radix (Lyrion Anywhere patch).
///
/// Replaces the direct DFT the MDCT used, which was O(n²) with a `sin_cos` per term and made
/// decoding run at only a few times real time. The twiddles are built once per size; each
/// stage is a generic radix-`p` butterfly, as kissfft's `kf_bfly_generic`, so any size works
/// (CELT needs 60, 120, 240 and 480).
#[derive(Debug, Clone)]
pub(crate) struct FftPlan {
    n: usize,
    /// `(p, m)` pairs: radix `p` over `m` points, outermost first.
    factors: Vec<(usize, usize)>,
    /// `exp(-2πi k / n)`.
    twiddles: Vec<Complex32>,
}

impl FftPlan {
    pub fn new(n: usize) -> Self {
        let mut factors = Vec::new();
        let mut rest = n.max(1);
        while rest > 1 {
            let p = [4, 2, 3, 5]
                .into_iter()
                .find(|p| rest % p == 0)
                .unwrap_or_else(|| (6..=rest).find(|p| rest % p == 0).unwrap_or(rest));
            rest /= p;
            factors.push((p, rest));
        }
        let twiddles = (0..n)
            .map(|k| {
                let phase = -2.0 * core::f64::consts::PI * (k as f64) / (n as f64);
                Complex32::new(phase.cos() as f32, phase.sin() as f32)
            })
            .collect();
        Self {
            n,
            factors,
            twiddles,
        }
    }

    pub fn len(&self) -> usize {
        self.n
    }

    /// Unscaled forward transform of `n` complex points; the same result as
    /// [`flat_fft_forward`] (to rounding).
    pub fn forward_flat(&self, input: &[f32], output: &mut [f32]) {
        let n = self.n;
        assert!(input.len() >= 2 * n && output.len() >= 2 * n);
        if n == 0 {
            return;
        }
        let src: Vec<Complex32> = (0..n)
            .map(|j| Complex32::new(input[2 * j], input[2 * j + 1]))
            .collect();
        let mut out = vec![Complex32::new(0.0, 0.0); n];
        let mut scratch = Vec::new();
        self.work(&mut out, &src, 0, 1, 0, &mut scratch);
        for (k, c) in out.iter().enumerate() {
            output[2 * k] = c.re;
            output[2 * k + 1] = c.im;
        }
    }

    /// Decimation in time: `out` gets the transform of `src[start + i * stride]`.
    fn work(
        &self,
        out: &mut [Complex32],
        src: &[Complex32],
        start: usize,
        stride: usize,
        stage: usize,
        scratch: &mut Vec<Complex32>,
    ) {
        let Some(&(p, m)) = self.factors.get(stage) else {
            out[0] = src[start];
            return;
        };
        for q in 0..p {
            self.work(
                &mut out[q * m..(q + 1) * m],
                src,
                start + q * stride,
                stride * p,
                stage + 1,
                scratch,
            );
        }
        let n = self.n;
        // This stage's transforms have p * m points, so its twiddles step by n / (p * m).
        let fstride = stride;
        scratch.clear();
        scratch.resize(p, Complex32::new(0.0, 0.0));
        for u in 0..m {
            for q in 0..p {
                let x = out[u + q * m];
                // Twiddle the q-th sub-transform's u-th point by w^(q u).
                let t = self.twiddles[(fstride * q * u) % n];
                scratch[q] = Complex32::new(x.re * t.re - x.im * t.im, x.re * t.im + x.im * t.re);
            }
            for k in 0..p {
                let mut acc = scratch[0];
                for q in 1..p {
                    let t = self.twiddles[(fstride * m * q * k) % n];
                    let x = scratch[q];
                    acc.re += x.re * t.re - x.im * t.im;
                    acc.im += x.re * t.im + x.im * t.re;
                }
                out[u + k * m] = acc;
            }
        }
    }
}

/// Compute forward DFT on flat interleaved complex buffers.
///
/// Params: `input`/`output` are `[re0, im0, re1, im1, ...]` and `n` is the
/// number of complex samples.
/// Returns: nothing; writes the forward transform to `output`.
pub(crate) fn flat_fft_forward(input: &[f32], output: &mut [f32], n: usize) {
    assert!(input.len() >= 2 * n && output.len() >= 2 * n);
    for k in 0..n {
        let mut sum_re = 0.0f64;
        let mut sum_im = 0.0f64;
        for j in 0..n {
            let angle = -2.0 * PI as f64 * (k as f64) * (j as f64) / (n as f64);
            let (sin_a, cos_a) = angle.sin_cos();
            let re = input[2 * j] as f64;
            let im = input[2 * j + 1] as f64;
            sum_re += re * cos_a - im * sin_a;
            sum_im += re * sin_a + im * cos_a;
        }
        output[2 * k] = sum_re as f32;
        output[2 * k + 1] = sum_im as f32;
    }
}

#[cfg(test)]
mod tests {
    use super::{Complex32, FftPlan, KissFft, flat_fft_forward};

    /// The mixed-radix plan matches an exact (f64, true π) DFT for every CELT size and some
    /// awkward ones, at least as closely as the direct DFT it replaced (which used f32 π).
    #[test]
    fn plan_matches_exact_dft() {
        for n in [1, 2, 3, 4, 5, 6, 7, 8, 12, 15, 30, 49, 60, 120, 240, 480] {
            let input: Vec<f32> = (0..2 * n)
                .map(|i| {
                    ((i * 7919 % 113) as f32 / 56.0 - 1.0) * if i % 3 == 0 { 0.5 } else { 1.0 }
                })
                .collect();
            let mut exact = vec![0.0f64; 2 * n];
            for k in 0..n {
                for j in 0..n {
                    let a = -2.0 * core::f64::consts::PI * ((k * j) % n) as f64 / n as f64;
                    let (re, im) = (f64::from(input[2 * j]), f64::from(input[2 * j + 1]));
                    exact[2 * k] += re * a.cos() - im * a.sin();
                    exact[2 * k + 1] += re * a.sin() + im * a.cos();
                }
            }
            let mut old = vec![0.0f32; 2 * n];
            let mut got = vec![0.0f32; 2 * n];
            flat_fft_forward(&input, &mut old, n);
            FftPlan::new(n).forward_flat(&input, &mut got);
            let worst = |v: &[f32]| {
                v.iter()
                    .zip(&exact)
                    .map(|(a, e)| (f64::from(*a) - e).abs())
                    .fold(0.0, f64::max)
            };
            let scale = exact.iter().fold(1.0f64, |a, v| a.max(v.abs()));
            assert!(
                worst(&got) <= 4e-6 * scale,
                "n={n}: off by {} (scale {scale})",
                worst(&got)
            );
            assert!(
                worst(&got) <= worst(&old).max(1e-6 * scale),
                "n={n}: less exact than before"
            );
        }
    }
    use core::f32::consts::PI;

    /// Compute libopus-style forward DFT with `1/N` scaling for tests.
    ///
    /// Params: `input` is time-domain complex vector.
    /// Returns: scaled frequency-domain vector.
    fn forward_scaled(input: &[Complex32]) -> Vec<Complex32> {
        let n = input.len();
        let mut out = vec![Complex32::new(0.0, 0.0); n];
        for (k, yk) in out.iter_mut().enumerate() {
            let mut acc_re = 0.0f32;
            let mut acc_im = 0.0f32;
            for (n_idx, xn) in input.iter().enumerate() {
                let phase = -2.0 * PI * (k as f32) * (n_idx as f32) / (n as f32);
                let c = phase.cos();
                let s = phase.sin();
                acc_re += xn.re * c - xn.im * s;
                acc_im += xn.re * s + xn.im * c;
            }
            *yk = Complex32::new(acc_re / (n as f32), acc_im / (n as f32));
        }
        out
    }

    #[test]
    fn ifft_roundtrip_matches_input() {
        let n = 60usize;
        let fft = KissFft::new(n);
        let mut input = Vec::with_capacity(n);
        for i in 0..n {
            let t = i as f32 / n as f32;
            input.push(Complex32::new(
                (2.0 * PI * 3.0 * t).sin(),
                (2.0 * PI * 5.0 * t).cos(),
            ));
        }
        let freq = forward_scaled(&input);
        let mut recon = vec![Complex32::new(0.0, 0.0); n];
        fft.ifft(&freq, &mut recon).expect("ifft must succeed");
        for (a, b) in input.iter().zip(recon.iter()) {
            assert!(
                (a.re - b.re).abs() < 2e-5,
                "re mismatch: {} vs {}",
                a.re,
                b.re
            );
            assert!(
                (a.im - b.im).abs() < 2e-5,
                "im mismatch: {} vs {}",
                a.im,
                b.im
            );
        }
    }
}
