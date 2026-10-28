use anyhow::{Result, bail, ensure};
use serde::{Deserialize, Serialize};
use std::{fmt, str::FromStr};

/// Frames per second, stored as a reduced, positive rational.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "Rate", into = "Rate")]
pub struct FrameRate {
    numerator: u32,
    denominator: u32,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Rate {
    numerator: u32,
    denominator: u32,
}

impl FrameRate {
    pub fn new(numerator: u32, denominator: u32) -> Result<Self> {
        ensure!(
            numerator > 0 && denominator > 0,
            "frame rate must be positive"
        );
        let (mut a, mut b) = (numerator, denominator);
        while b != 0 {
            (a, b) = (b, a % b);
        }
        Ok(Self {
            numerator: numerator / a,
            denominator: denominator / a,
        })
    }

    pub fn numerator(self) -> u32 {
        self.numerator
    }
    pub fn denominator(self) -> u32 {
        self.denominator
    }
    pub fn as_f64(self) -> f64 {
        self.numerator as f64 / self.denominator as f64
    }

    pub fn sample(self, frame: u64) -> Sample {
        Sample { frame, fps: self }
    }

    /// Number of integer samples before an exclusive duration in seconds.
    /// Uses the same conversion as evaluation instead of multiplying by a
    /// rounded floating-point frame rate, which can add a spurious end frame.
    pub fn frame_count(self, duration: f64) -> Result<u64> {
        ensure!(
            duration.is_finite() && duration > 0.0,
            "timeline duration must be finite and positive"
        );
        ensure!(
            duration <= self.sample(u64::MAX).seconds(),
            "timeline duration and frame rate exceed the supported frame range"
        );
        let (mut first, mut end) = (0, u64::MAX);
        while first < end {
            let middle = first + (end - first) / 2;
            if self.sample(middle).seconds() < duration {
                first = middle + 1;
            } else {
                end = middle;
            }
        }
        Ok(first)
    }
}

impl Default for FrameRate {
    fn default() -> Self {
        Self {
            numerator: 60,
            denominator: 1,
        }
    }
}

impl TryFrom<Rate> for FrameRate {
    type Error = anyhow::Error;
    fn try_from(rate: Rate) -> Result<Self> {
        Self::new(rate.numerator, rate.denominator)
    }
}
impl From<FrameRate> for Rate {
    fn from(rate: FrameRate) -> Self {
        Self {
            numerator: rate.numerator,
            denominator: rate.denominator,
        }
    }
}
impl fmt::Display for FrameRate {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.denominator == 1 {
            write!(f, "{}", self.numerator)
        } else {
            write!(f, "{}/{}", self.numerator, self.denominator)
        }
    }
}
impl FromStr for FrameRate {
    type Err = anyhow::Error;
    fn from_str(value: &str) -> Result<Self> {
        let parts: Vec<_> = value.split('/').collect();
        match parts.as_slice() {
            [n] => Self::new(n.parse()?, 1),
            [n, d] => Self::new(n.parse()?, d.parse()?),
            _ => bail!("expected a frame rate such as 60 or 30000/1001"),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct Sample {
    pub frame: u64,
    pub fps: FrameRate,
}

impl Sample {
    /// No accumulated clock or dependency on previously evaluated frames.
    pub fn seconds(self) -> f64 {
        let numerator = u128::from(self.frame) * u128::from(self.fps.denominator);
        numerator as f64 / f64::from(self.fps.numerator)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn rational_samples_do_not_accumulate_rounding() {
        let fps: FrameRate = "60000/2002".parse().unwrap();
        assert_eq!(fps.to_string(), "30000/1001");
        assert_eq!(fps.sample(30_000).seconds(), 1001.0);
        assert!("60/0".parse::<FrameRate>().is_err());
        assert!(serde_json::from_str::<FrameRate>(r#"{"numerator":0,"denominator":1}"#).is_err());
    }

    #[test]
    fn exclusive_duration_agrees_with_explicit_sample_endpoints() {
        let fps = FrameRate::new(30_000, 1001).unwrap();
        assert_eq!(fps.frame_count(1001.0).unwrap(), 30_000);
        for frame in [1, 3, 100, 479, 30_000, 1_000_001] {
            let duration = fps.sample(frame).seconds();
            assert_eq!(fps.frame_count(duration).unwrap(), frame);
            let just_after = f64::from_bits(duration.to_bits() + 1);
            assert_eq!(fps.frame_count(just_after).unwrap(), frame + 1);
        }
        assert_eq!(FrameRate::default().frame_count(8.0).unwrap(), 480);
        assert_eq!(FrameRate::new(30, 1).unwrap().frame_count(0.1).unwrap(), 3);
        assert_eq!(fps.frame_count(f64::from_bits(1)).unwrap(), 1);
        for invalid in [0.0, -1.0, f64::NAN, f64::INFINITY, f64::MAX] {
            assert!(fps.frame_count(invalid).is_err());
        }
    }
}
