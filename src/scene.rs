//! Version 1 authoring schema. Coordinates are right-handed, Y up.
use crate::{animation::Animated, data::FieldSource, time::FrameRate};
use serde::{Deserialize, Serialize};

pub type Color = [f32; 4];

/// Maximum authored and resolved pixel stroke width supported by path coverage.
/// Output resizing scales pixel strokes, so the limit is checked again per frame.
pub const MAX_STROKE_WIDTH: f32 = 32_768.0;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SceneSpec {
    pub version: u32,
    pub size: [u32; 2],
    pub fps: FrameRate,
    pub duration: f64,
    pub background: Color,
    pub camera: CameraSpec,
    /// Shared local-to-world placement for the composition's world geometry.
    #[serde(default)]
    pub model: TransformSpec,
    #[serde(default)]
    pub assets: Vec<FieldAsset>,
    pub nodes: Vec<NodeSpec>,
}

/// A shared model transform, applied as translation * rotation * uniform scale.
/// Camera values and screen overlays are already in world/canvas coordinates.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TransformSpec {
    #[serde(default = "zero_vector")]
    pub translation: Animated<[f32; 3]>,
    /// Euler XYZ in degrees: Rx * Ry * Rz for column vectors (Z acts first).
    #[serde(default = "zero_vector")]
    pub rotation_degrees: Animated<[f32; 3]>,
    /// Positive uniform scale. Nonuniform scaling is not supported in version 1.
    #[serde(default = "one")]
    pub scale: Animated<f32>,
}

impl Default for TransformSpec {
    fn default() -> Self {
        Self {
            translation: zero_vector(),
            rotation_degrees: zero_vector(),
            scale: one(),
        }
    }
}

fn zero_vector() -> Animated<[f32; 3]> {
    [0.; 3].into()
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CameraSpec {
    /// Canvas pixel rectangle [x, y, width, height], or the full canvas.
    #[serde(default)]
    pub viewport: Option<[f32; 4]>,
    pub eye: Animated<[f32; 3]>,
    pub target: Animated<[f32; 3]>,
    pub up: [f32; 3],
    pub fov_y_degrees: f32,
    pub near: f32,
    pub far: f32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FieldAsset {
    pub id: String,
    pub source: FieldSource,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Palette {
    pub stops: [Color; 5],
    pub range: [f32; 2],
    #[serde(default = "zero")]
    pub shift: Animated<f32>,
}
fn zero() -> Animated<f32> {
    0.0.into()
}
fn one() -> Animated<f32> {
    1.0.into()
}
fn white() -> Color {
    [1.; 4]
}

impl Default for Palette {
    fn default() -> Self {
        Self {
            stops: [
                [0.08, 0.17, 0.31, 1.0],
                [0.12, 0.43, 0.56, 1.0],
                [0.18, 0.72, 0.69, 1.0],
                [0.75, 0.86, 0.49, 1.0],
                [1.0, 0.72, 0.34, 1.0],
            ],
            range: [-1., 1.],
            shift: zero(),
        }
    }
}

#[derive(Debug, Default, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum Occlusion {
    #[default]
    Always,
    /// Hide the entire label when its anchor is behind world geometry.
    Hide,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "kebab-case", deny_unknown_fields)]
pub enum Anchor {
    World {
        position: Animated<[f32; 3]>,
    },
    Field {
        field: String,
        point: [f32; 2],
        height: Animated<f32>,
        #[serde(default)]
        lift: f32,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "kebab-case", deny_unknown_fields)]
pub enum NodeSpec {
    Surface {
        id: String,
        field: String,
        height: Animated<f32>,
        palette: Palette,
        #[serde(default)]
        grid: f32,
    },
    Trace {
        id: String,
        field: String,
        points: Vec<[f32; 2]>,
        height: Animated<f32>,
        #[serde(default)]
        lift: f32,
        width: f32,
        color: Color,
        #[serde(default = "one")]
        reveal: Animated<f32>,
    },
    Polyline {
        id: String,
        points: Vec<[f32; 3]>,
        width: f32,
        color: Color,
        #[serde(default = "one")]
        reveal: Animated<f32>,
    },
    Markers {
        id: String,
        field: String,
        points: Vec<[f32; 2]>,
        height: Animated<f32>,
        #[serde(default)]
        lift: f32,
        radius: f32,
        colors: Vec<Color>,
    },
    Text {
        id: String,
        text: String,
        position: [f32; 2],
        font_size: f32,
        #[serde(default = "white")]
        color: Color,
        #[serde(default = "one")]
        opacity: Animated<f32>,
    },
    Label {
        id: String,
        text: String,
        anchor: Anchor,
        offset: [f32; 2],
        font_size: f32,
        #[serde(default = "white")]
        color: Color,
        #[serde(default)]
        occlusion: Occlusion,
    },
    Legend {
        id: String,
        position: [f32; 2],
        size: [f32; 2],
        palette: Palette,
    },
    GraphicsPlane {
        id: String,
        text: String,
        position: Animated<[f32; 3]>,
        /// World-space basis vectors for one unit of text width and height.
        right: [f32; 3],
        up: [f32; 3],
        width: f32,
        font_size: f32,
        #[serde(default = "white")]
        color: Color,
    },
}

impl NodeSpec {
    pub fn id(&self) -> &str {
        match self {
            Self::Surface { id, .. }
            | Self::Trace { id, .. }
            | Self::Polyline { id, .. }
            | Self::Markers { id, .. }
            | Self::Text { id, .. }
            | Self::Label { id, .. }
            | Self::Legend { id, .. }
            | Self::GraphicsPlane { id, .. } => id,
        }
    }
}

pub fn srgb_to_linear(color: Color) -> Color {
    let channel = |c: f32| {
        if c <= 0.04045 {
            c / 12.92
        } else {
            ((c + 0.055) / 1.055).powf(2.4)
        }
    };
    [
        channel(color[0]),
        channel(color[1]),
        channel(color[2]),
        color[3],
    ]
}
