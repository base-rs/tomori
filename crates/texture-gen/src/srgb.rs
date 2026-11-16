// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! sRGB's transfer function, computed by arithmetic alone, so that the same linear light gives the same byte on every
//! machine: the platform's `powf` is not specified to the bit, and a texture's digest must be.

const LN_2: f64 = std::f64::consts::LN_2;

/// The natural logarithm of `x`, which is finite and above nothing: its exponent's share, and its mantissa's by the
/// series of `atanh`, within a few units in the last place.
fn ln(x: f64) -> f64 {
    let bits = x.to_bits();
    let mut exponent = ((bits >> 52) & 0x7ff) as i64 - 1023;
    let mut mantissa = f64::from_bits((bits & 0x000f_ffff_ffff_ffff) | 0x3ff0_0000_0000_0000);
    if mantissa > std::f64::consts::SQRT_2 {
        mantissa /= 2.0;
        exponent += 1;
    }
    // ln(m) = 2 atanh(s), |s| at most 0.18, so that twenty terms are past double precision.
    let s = (mantissa - 1.0) / (mantissa + 1.0);
    let square = s * s;
    let (mut term, mut sum) = (s, 0.0);
    for odd in (1..40).step_by(2) {
        sum += term / f64::from(odd);
        term *= square;
    }
    2.0 * sum + exponent as f64 * LN_2
}

/// `e` to the `x`, for `x` within a few hundred of nothing: two to the nearest whole number of halvings, by its bits, and
/// the rest by its series.
fn exp(x: f64) -> f64 {
    let halvings = (x / LN_2).round();
    let rest = x - halvings * LN_2;
    let (mut term, mut sum) = (1.0, 1.0);
    for n in 1..24 {
        term *= rest / f64::from(n);
        sum += term;
    }
    sum * f64::from_bits(((halvings as i64 + 1023) as u64) << 52)
}

/// `x` to the `power`, `x` above nothing.
pub(crate) fn pow(x: f64, power: f64) -> f64 {
    exp(power * ln(x))
}

/// The sRGB byte nearest `linear`, from 0 to 1, as sRGB encodes it.
pub fn encode(linear: f32) -> u8 {
    let linear = f64::from(linear.clamp(0.0, 1.0));
    let encoded = if linear <= 0.003_130_8 {
        linear * 12.92
    } else {
        1.055 * pow(linear, 1.0 / 2.4) - 0.055
    };
    (encoded * 255.0).round().clamp(0.0, 255.0) as u8
}

/// The linear light of each sRGB byte, as a GPU decodes it.
pub fn decode(byte: u8) -> f32 {
    static TABLE: std::sync::OnceLock<[f32; 256]> = std::sync::OnceLock::new();
    TABLE.get_or_init(|| {
        std::array::from_fn(|byte| {
            let encoded = byte as f64 / 255.0;
            (if encoded <= 0.040_45 {
                encoded / 12.92
            } else {
                pow((encoded + 0.055) / 1.055, 2.4)
            }) as f32
        })
    })[usize::from(byte)]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_logarithm_exponential_and_power_agree_with_the_platform_s() {
        for x in [
            1e-6, 0.003, 0.25, 0.5, 0.999, 1.0, 1.414, 1.415, 2.0, 7.5, 1e6,
        ] {
            assert!((ln(x) - x.ln()).abs() < 1e-14 * (1.0 + x.ln().abs()), "{x}");
        }
        for x in [-30.0, -1.0, -0.2, 0.0, 0.3, 1.0, 5.5, 40.0] {
            assert!((exp(x) / x.exp() - 1.0).abs() < 1e-14, "{x}");
        }
        for (x, power) in [(0.5, 2.4), (0.04, 1.0 / 2.4), (0.9, 0.5), (3.0, 3.0)] {
            assert!((pow(x, power) / x.powf(power) - 1.0).abs() < 1e-13);
        }
    }

    #[test]
    fn each_byte_decodes_to_the_light_it_encodes_from() {
        for byte in 0..=255u8 {
            assert_eq!(encode(decode(byte)), byte);
        }
        assert_eq!(
            (encode(-1.0), encode(0.0), encode(1.0), encode(2.0)),
            (0, 0, 255, 255)
        );
        // 128 is about a fifth of the light, and the darkest bytes are linear.
        assert!((decode(128) - 0.2158).abs() < 1e-4);
        assert_eq!(encode(0.2158), 128);
        assert!((decode(10) - 10.0 / 255.0 / 12.92).abs() < 1e-9);
    }
}
