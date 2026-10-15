#![allow(dead_code, unused_variables, unused_assignments)]

//! CELT inverse MDCT primitives.
//!
//! Current scope is a decoder-oriented path for 48 kHz / 20 ms (`LM=3`), with
//! explicit overlap/window processing kept close to libopus flow.

use crate::celt::kiss_fft::{FftPlan, KissFft};

/// Backward MDCT helper for CELT decode path.
#[derive(Debug, Clone)]
pub(crate) struct MdctBackward {
    n: usize,
    overlap: usize,
    fft: KissFft,
    plan: FftPlan,
    trig: Vec<f32>,
}

impl MdctBackward {
    /// Create backward MDCT state for fixed frame size.
    ///
    /// Params: `n` is MDCT size (e.g. 960), `overlap` is overlap length
    /// (e.g. 120).
    /// Returns: initialized state with runtime trig tables.
    pub fn new(n: usize, overlap: usize) -> Self {
        let n2 = n / 2;
        let mut trig = Vec::with_capacity(n2);
        for j in 0..n2 {
            let phase = 2.0_f64 * core::f64::consts::PI * (j as f64 + 0.125_f64) / n as f64;
            trig.push(phase.cos() as f32);
        }
        Self {
            n,
            overlap,
            fft: KissFft::new(n / 4),
            plan: FftPlan::new(n / 4),
            trig,
        }
    }

    /// Return configured MDCT size.
    ///
    /// Params: none.
    /// Returns: MDCT size `N` for this instance.
    pub fn n(&self) -> usize {
        self.n
    }

    /// Return internal FFT size used by MDCT.
    ///
    /// Params: none.
    /// Returns: complex FFT size `N/4`.
    pub fn fft_len(&self) -> usize {
        self.fft.len()
    }

    /// Run `clt_mdct_backward`-style transform.
    ///
    /// Params: `input` contains `N/2` MDCT bins, `window` contains `overlap`
    /// samples, `out` has length at least `N/2 + overlap`.
    /// Returns: `Ok(())` on success, otherwise a static validation error.
    pub fn backward(
        &self,
        input: &[f32],
        window: &[f32],
        out: &mut [f32],
    ) -> Result<(), &'static str> {
        let n = self.n;
        let n2 = n >> 1;
        let n4 = n >> 2;
        let overlap = self.overlap;
        let required = n2 + overlap;
        if input.len() != n2 || out.len() < required {
            return Err("mdct backward length mismatch");
        }
        if window.len() != overlap {
            return Err("mdct backward window length mismatch");
        }


        // Flat interleaved buffer: [re0, im0, re1, im1, ...]
        let mut f2 = vec![0.0f32; n2];
        let t = &self.trig;
        for i in 0..n4 {
            let xp1 = input[2 * i];
            let xp2 = input[n2 - 1 - 2 * i];
            // Match libopus clt_mdct_backward pre-rotation for float path.
            let yr = xp2 * t[i] + xp1 * t[n4 + i];
            let yi = xp1 * t[i] - xp2 * t[n4 + i];
            // Swap slots like libopus FFT-vs-IFFT trick.
            f2[2 * i] = yi;
            f2[2 * i + 1] = yr;
        }

        let mut f2_out = vec![0.0f32; n2];
        self.plan.forward_flat(&f2, &mut f2_out);

        let ov2 = overlap >> 1;
        out[ov2..ov2 + n2].copy_from_slice(&f2_out[..n2]);
        let mut p0 = ov2;
        let mut p1 = ov2 + n2 - 2;
        for i in 0..((n4 + 1) >> 1) {
            // We swap real/imag in reads because we use FFT instead of IFFT.
            let re = out[p0 + 1];
            let im = out[p0];
            let t0 = t[i];
            let t1 = t[n4 + i];
            let yr = re * t0 + im * t1;
            let yi = re * t1 - im * t0;

            // Read yp1 from the original buffer state, like libopus.
            let re2 = out[p1 + 1];
            let im2 = out[p1];
            out[p0] = yr;
            out[p1 + 1] = yi;

            let t2 = t[n4 - i - 1];
            let t3 = t[n2 - i - 1];
            let yr2 = re2 * t2 + im2 * t3;
            let yi2 = re2 * t3 - im2 * t2;
            out[p1] = yr2;
            out[p0 + 1] = yi2;

            p0 += 2;
            p1 -= 2;
        }

        for i in 0..(overlap / 2) {
            let x1 = out[overlap - 1 - i];
            let x2 = out[i];
            out[i] = x2 * window[overlap - 1 - i] - x1 * window[i];
            out[overlap - 1 - i] = x2 * window[i] + x1 * window[overlap - 1 - i];
        }
        Ok(())
    }

    /// Run direct inverse MDCT that returns raw IMDCT output.
    ///
    /// Params: `input` contains `N/2` MDCT bins, `out` has length `N`.
    /// Returns: `Ok(())` on success, otherwise a static validation error.
    pub fn backward_direct(
        &self,
        input: &[f32],
        _window: &[f32],
        out: &mut [f32],
    ) -> Result<(), &'static str> {
        let n = self.n;
        let n2 = n / 2;
        if input.len() != n2 || out.len() != n {
            return Err("mdct backward length mismatch");
        }

        for (i, out_i) in out.iter_mut().enumerate() {
            let mut sum = 0.0f64;
            for (k, &xk) in input.iter().enumerate() {
                let phase = core::f64::consts::PI / (n2 as f64)
                    * (i as f64 + 0.5 + n2 as f64 / 2.0)
                    * (k as f64 + 0.5);
                sum += xk as f64 * phase.cos();
            }
            *out_i = sum as f32;
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::MdctBackward;

    #[test]
    fn mdct_backward_validates_lengths() {
        let mdct = MdctBackward::new(960, 120);
        let input = vec![0.0f32; 480];
        let window = vec![1.0f32; 120];
        let mut out = vec![0.0f32; 960];
        assert!(mdct.backward(&input, &window, &mut out).is_ok());
        assert!(mdct.backward(&input[..479], &window, &mut out).is_err());
        assert!(mdct.backward(&input, &window[..119], &mut out).is_err());
    }
}
