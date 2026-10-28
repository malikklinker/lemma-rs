use anyhow::{Result, ensure};
use serde::{Deserialize, Serialize};

pub trait Interpolate: Clone {
    fn interpolate(&self, other: &Self, t: f32) -> Self;
    fn is_finite(&self) -> bool;
}
impl Interpolate for f32 {
    fn interpolate(&self, other: &Self, t: f32) -> Self {
        (f64::from(*self) * (1.0 - f64::from(t)) + f64::from(*other) * f64::from(t)) as f32
    }
    fn is_finite(&self) -> bool {
        f32::is_finite(*self)
    }
}
impl<const N: usize> Interpolate for [f32; N] {
    fn interpolate(&self, other: &Self, t: f32) -> Self {
        std::array::from_fn(|i| self[i].interpolate(&other[i], t))
    }
    fn is_finite(&self) -> bool {
        self.iter().all(|v| v.is_finite())
    }
}

#[derive(Debug, Default, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Ease {
    #[default]
    Linear,
    Smooth,
    CubicOut,
    Hold,
}

impl Ease {
    fn apply(self, t: f32) -> f32 {
        match self {
            Self::Linear => t,
            Self::Smooth => t * t * (3.0 - 2.0 * t),
            Self::CubicOut => 1.0 - (1.0 - t).powi(3),
            Self::Hold => 0.0,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Keyframe<T> {
    pub time: f64,
    pub value: T,
    /// Easing on the interval beginning at this keyframe.
    #[serde(default)]
    pub ease: Ease,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged, deny_unknown_fields)]
pub enum Animated<T> {
    Constant(T),
    Track { keyframes: Vec<Keyframe<T>> },
}

impl<T: Interpolate> Animated<T> {
    pub fn at(&self, time: f64) -> T {
        match self {
            Self::Constant(value) => value.clone(),
            Self::Track { keyframes } => {
                let end = keyframes.partition_point(|k| k.time <= time);
                if end == 0 {
                    return keyframes[0].value.clone();
                }
                if end == keyframes.len() {
                    return keyframes[end - 1].value.clone();
                }
                let (a, b) = (&keyframes[end - 1], &keyframes[end]);
                let t = ((time - a.time) / (b.time - a.time)) as f32;
                a.value
                    .interpolate(&b.value, a.ease.apply(t.clamp(0.0, 1.0)))
            }
        }
    }

    pub fn validate(&self, name: &str) -> Result<()> {
        match self {
            Self::Constant(value) => ensure!(value.is_finite(), "{name}: value must be finite"),
            Self::Track { keyframes } => {
                ensure!(
                    !keyframes.is_empty(),
                    "{name}: track needs at least one keyframe"
                );
                let mut previous = -1.0;
                for key in keyframes {
                    ensure!(
                        key.time.is_finite() && key.time >= 0.0 && key.time > previous,
                        "{name}: keyframe times must be finite, nonnegative and strictly increasing"
                    );
                    ensure!(
                        key.value.is_finite(),
                        "{name}: keyframe values must be finite"
                    );
                    previous = key.time;
                }
            }
        }
        Ok(())
    }

    pub fn values(&self) -> Vec<&T> {
        match self {
            Self::Constant(v) => vec![v],
            Self::Track { keyframes } => keyframes.iter().map(|k| &k.value).collect(),
        }
    }
}
impl<T> From<T> for Animated<T> {
    fn from(value: T) -> Self {
        Self::Constant(value)
    }
}

pub fn tween<T>(from: T, to: T, start: f64, end: f64) -> Animated<T> {
    Animated::Track {
        keyframes: vec![
            Keyframe {
                time: start,
                value: from,
                ease: Ease::Smooth,
            },
            Keyframe {
                time: end,
                value: to,
                ease: Ease::Linear,
            },
        ],
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn endpoints_clamp_and_hold_steps_at_the_boundary() {
        let a = tween(2.0_f32, 6.0, 1.0, 3.0);
        assert_eq!(a.at(0.0), 2.0);
        assert_eq!(a.at(2.0), 4.0);
        assert_eq!(a.at(3.0), 6.0);
        assert_eq!(a.at(8.0), 6.0);
        let hold = Animated::Track {
            keyframes: vec![
                Keyframe {
                    time: 0.0,
                    value: 1.0_f32,
                    ease: Ease::Hold,
                },
                Keyframe {
                    time: 1.0,
                    value: 9.0,
                    ease: Ease::Linear,
                },
            ],
        };
        assert_eq!(hold.at(0.999), 1.0);
        assert_eq!(hold.at(1.0), 9.0);
    }
    #[test]
    fn ambiguous_tracks_are_rejected() {
        assert!(tween(0.0_f32, 1.0, 1.0, 1.0).validate("node.x").is_err());
        assert!(
            Animated::<f32>::Track { keyframes: vec![] }
                .validate("node.x")
                .is_err()
        );
        assert!(
            serde_json::from_str::<Animated<f32>>(
                r#"{"keyframes":[{"time":0,"value":1}],"unsupported_loop":true}"#
            )
            .is_err()
        );
        let extreme = tween(-f32::MAX, f32::MAX, 0.0, 1.0);
        assert_eq!(extreme.at(0.5), 0.0);
    }
}
