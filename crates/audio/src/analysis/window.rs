//! Analysis window.
//!
//! Electronic music puts most of its energy in sub-bass. With a Hann window
//! (first sidelobe -31 dB) that energy leaks across the spectrum and fills
//! the empty band above an encoder's low-pass, hiding the very cliff the
//! detector looks for. The minimum 4-term Blackman-Harris window keeps
//! sidelobes below about -92 dB, near the 16-bit noise floor, at the cost of
//! a wider main lobe (about ±4 bins, under 50 Hz at the FFT sizes used here).

/// Coefficients of the minimum 4-term Blackman-Harris window (Harris 1978).
const COEFFICIENTS: [f64; 4] = [0.358_75, 0.488_29, 0.141_28, 0.011_68];

/// Periodic Blackman-Harris window of length `len` (divisor `len`, not
/// `len - 1`), the right form for overlapped spectral averaging.
pub(crate) fn blackman_harris(len: usize) -> Vec<f32> {
    let [a0, a1, a2, a3] = COEFFICIENTS;
    let n = len as f64;
    (0..len)
        .map(|i| {
            let x = std::f64::consts::TAU * i as f64 / n;
            #[allow(
                clippy::cast_possible_truncation,
                reason = "window values are in 0..=1"
            )]
            let value = (a0 - a1 * x.cos() + a2 * (2.0 * x).cos() - a3 * (3.0 * x).cos()) as f32;
            value
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn window_is_periodic_and_peaks_at_the_centre() {
        let w = blackman_harris(8192);
        assert!((w[0] - 6.0e-5).abs() < 1.0e-5, "endpoint {}", w[0]);
        assert!((w[4096] - 1.0).abs() < 1.0e-6);
        for i in 1..4096 {
            assert!((w[i] - w[8192 - i]).abs() < 1.0e-6, "symmetry at {i}");
        }
    }
}
