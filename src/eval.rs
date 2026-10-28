use crate::{
    animation::Animated,
    data::{ScalarField, SourceCache, SourcePolicy},
    scene::{Anchor, Color, FieldAsset, MAX_STROKE_WIDTH, NodeSpec, Occlusion, Palette, SceneSpec},
    time::Sample,
};
use anyhow::{Context, Result, bail, ensure};
use glam::{EulerRot, Mat4, Quat, Vec3};
use serde::Serialize;
use std::{
    collections::{BTreeMap, HashSet},
    io::Read,
    path::Path,
    sync::Arc,
};

pub struct CompiledPlan {
    spec: SceneSpec,
    fields: BTreeMap<String, Arc<ScalarField>>,
    /// Static numerical samples, in node order. Trace/marker entries have one
    /// value per point; a field-anchored label has one. Other entries are empty.
    /// These stay separate from animation so seeking only maps cached values.
    field_samples: Vec<Vec<f32>>,
}

#[derive(Debug, Clone)]
pub struct FramePacket {
    pub sample: Sample,
    pub size: [u32; 2],
    pub background: Color,
    pub view_projection: Mat4,
    pub eye: Vec3,
    pub viewport: [f32; 4],
    pub surfaces: Vec<SurfaceDraw>,
    pub paths: Vec<PathDraw>,
    pub markers: Vec<MarkerDraw>,
    pub graphics: Vec<GraphicDraw>,
    pub legends: Vec<LegendDraw>,
}
#[derive(Debug, Clone)]
pub struct SurfaceDraw {
    pub id: String,
    pub field: Arc<ScalarField>,
    pub model: Mat4,
    pub height: f32,
    pub palette: Palette,
    pub shift: f32,
    pub grid: f32,
}
#[derive(Debug, Clone, Serialize)]
pub struct PathDraw {
    pub id: String,
    pub points: Vec<[f32; 3]>,
    pub width: f32,
    pub color: Color,
    pub reveal: f32,
}
#[derive(Debug, Clone, Serialize)]
pub struct MarkerDraw {
    pub position: [f32; 3],
    pub radius: f32,
    pub color: Color,
}
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub enum Placement {
    Screen {
        position: [f32; 2],
    },
    Projected {
        anchor: [f32; 3],
        offset: [f32; 2],
        occlusion: Occlusion,
    },
    World {
        position: [f32; 3],
        right: [f32; 3],
        up: [f32; 3],
        width: f32,
    },
}
#[derive(Debug, Clone, Serialize)]
pub struct GraphicDraw {
    pub id: String,
    pub text: String,
    pub font_size: f32,
    pub color: Color,
    pub placement: Placement,
}
#[derive(Debug, Clone, Serialize)]
pub struct LegendDraw {
    pub position: [f32; 2],
    pub size: [f32; 2],
    pub palette: Palette,
    pub shift: f32,
}

fn finite(values: &[f32], name: &str) -> Result<()> {
    ensure!(
        values.iter().all(|v| v.is_finite()),
        "{name}: values must be finite"
    );
    Ok(())
}
fn color(value: Color, name: &str) -> Result<()> {
    ensure!(
        value
            .iter()
            .all(|v| v.is_finite() && (0.0..=1.0).contains(v)),
        "{name}: color components must be in [0, 1]"
    );
    Ok(())
}
fn palette(value: &Palette, name: &str) -> Result<()> {
    for stop in value.stops {
        color(stop, name)?;
    }
    ensure!(
        value.stops.iter().all(|c| c[3] == 1.0),
        "{name}: palette stops must be opaque"
    );
    finite(&value.range, name)?;
    ensure!(
        value.range[0] < value.range[1],
        "{name}: palette range must increase"
    );
    ensure!(
        (value.range[1] - value.range[0]).is_finite(),
        "{name}: palette range span must be finite in f32"
    );
    value.shift.validate(&format!("{name}.shift"))
}
fn positive(value: f32, name: &str) -> Result<()> {
    ensure!(
        value.is_finite() && value > 0.0,
        "{name}: must be finite and positive"
    );
    Ok(())
}
fn stroke_width(value: f32, name: &str) -> Result<()> {
    ensure!(
        value.is_finite() && value > 0.0 && value <= MAX_STROKE_WIDTH,
        "{name}: stroke width must be finite and in (0, {MAX_STROKE_WIDTH}] pixels; reduce width or output scale"
    );
    Ok(())
}
fn unit_animation(value: &Animated<f32>, name: &str) -> Result<()> {
    value.validate(name)?;
    ensure!(
        value.values().iter().all(|v| (0.0..=1.0).contains(*v)),
        "{name}: values must be in [0, 1]"
    );
    Ok(())
}

impl CompiledPlan {
    /// Validated authoring input. Change a clone and compile a new plan to edit it.
    pub fn spec(&self) -> &SceneSpec {
        &self.spec
    }

    pub fn fields(&self) -> &BTreeMap<String, Arc<ScalarField>> {
        &self.fields
    }

    pub fn compile(spec: SceneSpec, base: &Path) -> Result<Self> {
        Self::compile_with_cache(spec, base, &mut SourceCache::default())
    }

    pub fn compile_with_cache(
        spec: SceneSpec,
        base: &Path,
        cache: &mut SourceCache,
    ) -> Result<Self> {
        Self::compile_with_policy(spec, base, cache, SourcePolicy::OnDemand)
    }

    pub fn compile_with_policy(
        spec: SceneSpec,
        base: &Path,
        cache: &mut SourceCache,
        policy: SourcePolicy,
    ) -> Result<Self> {
        ensure!(
            spec.version == 1,
            "unsupported SceneSpec version {}; expected 1",
            spec.version
        );
        ensure!(
            spec.size.iter().all(|n| *n > 0 && *n <= 8192),
            "scene size must be between 1 and 8192 pixels per axis"
        );
        ensure!(
            spec.duration.is_finite() && spec.duration > 0.0,
            "scene duration must be finite and positive"
        );
        color(spec.background, "background")?;
        ensure!(
            spec.background[3] == 1.0,
            "background must be opaque for SDR output"
        );
        spec.model.translation.validate("model.translation")?;
        spec.model
            .rotation_degrees
            .validate("model.rotation_degrees")?;
        spec.model.scale.validate("model.scale")?;
        ensure!(
            spec.model.scale.values().iter().all(|scale| **scale > 0.0),
            "model.scale: values must be positive; only uniform scaling is supported"
        );
        let camera = &spec.camera;
        if let Some([x, y, w, h]) = camera.viewport {
            finite(&[x, y, w, h], "camera.viewport")?;
            ensure!(
                x >= 0.0
                    && y >= 0.0
                    && w >= 1.0
                    && h >= 1.0
                    && x + w <= spec.size[0] as f32
                    && y + h <= spec.size[1] as f32,
                "camera.viewport must be a positive rectangle within the canvas"
            );
        }
        camera.eye.validate("camera.eye")?;
        camera.target.validate("camera.target")?;
        finite(&camera.up, "camera.up")?;
        ensure!(
            Vec3::from(camera.up).length_squared() > 1e-8,
            "camera.up must be nonzero"
        );
        ensure!(
            camera.fov_y_degrees.is_finite() && (1.0..179.0).contains(&camera.fov_y_degrees),
            "camera field of view must be in (1, 179) degrees"
        );
        ensure!(
            camera.near.is_finite()
                && camera.far.is_finite()
                && camera.near > 0.0
                && camera.far > camera.near,
            "camera clipping planes require 0 < near < far"
        );
        let mut ids = HashSet::new();
        let mut fields = BTreeMap::new();
        for asset in &spec.assets {
            ensure!(
                !asset.id.is_empty() && ids.insert(asset.id.clone()),
                "empty or duplicate asset ID: {}",
                asset.id
            );
            let data = cache
                .resolve(&asset.source, base, policy)
                .with_context(|| format!("asset {}", asset.id))?;
            fields.insert(asset.id.clone(), data);
        }
        ids.clear();
        let field = |id: &str| -> Result<&Arc<ScalarField>> {
            fields
                .get(id)
                .with_context(|| format!("unknown field asset {id:?}"))
        };
        let points = |asset: &str, pts: &[[f32; 2]], node: &str| -> Result<Vec<f32>> {
            let f = field(asset)?;
            ensure!(
                !pts.is_empty() && pts.len() <= 1_000_000,
                "{node}: point count must be in 1..=1000000"
            );
            ensure!(
                pts.iter().all(|p| f.contains(*p)),
                "{node}: points must be finite and within field {asset:?}"
            );
            Ok(pts.iter().map(|point| f.sample(*point)).collect())
        };
        let text = |s: &str, size: f32, c: Color, node: &str| -> Result<()> {
            ensure!(
                !s.is_empty() && s.chars().count() <= 4096,
                "{node}: text length must be in 1..=4096"
            );
            ensure!(
                !s.contains(['\n', '\r', '\t']),
                "{node}: only single-line text is currently supported"
            );
            ensure!(
                size.is_finite() && (1.0..=256.0).contains(&size),
                "{node}: font size must be in 1..=256"
            );
            color(c, node)?;
            crate::text::validate(s, size).with_context(|| format!("node {node:?}"))
        };
        let mut field_samples = Vec::with_capacity(spec.nodes.len());
        for node in &spec.nodes {
            let id = node.id();
            ensure!(
                !id.is_empty() && ids.insert(id.to_owned()),
                "empty or duplicate node ID: {id}"
            );
            let mut samples = Vec::new();
            let result: Result<()> = (|| {
                match node {
                    NodeSpec::Surface {
                        field: f,
                        height,
                        palette: p,
                        grid,
                        ..
                    } => {
                        field(f)?;
                        height.validate("height")?;
                        palette(p, "palette")?;
                        ensure!(
                            grid.is_finite() && (0.0..=1.0).contains(grid),
                            "grid must be in [0, 1]"
                        );
                    }
                    NodeSpec::Trace {
                        field: f,
                        points: pts,
                        height,
                        lift,
                        width,
                        color: c,
                        reveal,
                        ..
                    } => {
                        samples = points(f, pts, id)?;
                        ensure!(pts.len() >= 2, "trace needs at least two points");
                        height.validate("height")?;
                        finite(&[*lift], "lift")?;
                        stroke_width(*width, "width")?;
                        color(*c, "color")?;
                        ensure!(
                            c[3] == 1.0,
                            "translucent traces are not supported; use an opaque color"
                        );
                        unit_animation(reveal, "reveal")?;
                    }
                    NodeSpec::Polyline {
                        points,
                        width,
                        color: c,
                        reveal,
                        ..
                    } => {
                        ensure!(
                            points.len() >= 2 && points.len() <= 1_000_000,
                            "polyline needs 2..=1000000 points"
                        );
                        for p in points {
                            finite(p, "points")?;
                        }
                        stroke_width(*width, "width")?;
                        color(*c, "color")?;
                        ensure!(
                            c[3] == 1.0,
                            "translucent polylines are not supported; use an opaque color"
                        );
                        unit_animation(reveal, "reveal")?;
                    }
                    NodeSpec::Markers {
                        field: f,
                        points: pts,
                        height,
                        lift,
                        radius,
                        colors,
                        ..
                    } => {
                        samples = points(f, pts, id)?;
                        height.validate("height")?;
                        finite(&[*lift], "lift")?;
                        positive(*radius, "radius")?;
                        ensure!(
                            colors.len() == 1 || colors.len() == pts.len(),
                            "markers require one color or one per point"
                        );
                        for c in colors {
                            color(*c, "colors")?;
                            ensure!(c[3] == 1.0, "markers currently require opaque colors");
                        }
                    }
                    NodeSpec::Text {
                        text: s,
                        position,
                        font_size,
                        color,
                        opacity,
                        ..
                    } => {
                        text(s, *font_size, *color, id)?;
                        finite(position, "position")?;
                        unit_animation(opacity, "opacity")?;
                    }
                    NodeSpec::Label {
                        text: s,
                        anchor,
                        offset,
                        font_size,
                        color,
                        ..
                    } => {
                        text(s, *font_size, *color, id)?;
                        finite(offset, "offset")?;
                        match anchor {
                            Anchor::World { position } => position.validate("anchor.position")?,
                            Anchor::Field {
                                field: f,
                                point,
                                height,
                                lift,
                            } => {
                                samples = points(f, &[*point], id)?;
                                height.validate("anchor.height")?;
                                finite(&[*lift], "anchor.lift")?;
                            }
                        }
                    }
                    NodeSpec::Legend {
                        position,
                        size,
                        palette: p,
                        ..
                    } => {
                        finite(position, "position")?;
                        for n in size {
                            positive(*n, "size")?;
                        }
                        palette(p, "palette")?;
                    }
                    NodeSpec::GraphicsPlane {
                        text: s,
                        position,
                        right,
                        up,
                        width,
                        font_size,
                        color,
                        ..
                    } => {
                        text(s, *font_size, *color, id)?;
                        position.validate("position")?;
                        finite(right, "right")?;
                        finite(up, "up")?;
                        positive(*width, "width")?;
                        ensure!(
                            Vec3::from(*right).cross(Vec3::from(*up)).length_squared() > 1e-8,
                            "graphics plane basis must be independent"
                        );
                    }
                }
                Ok(())
            })();
            result.with_context(|| format!("node {id:?}"))?;
            field_samples.push(samples);
        }
        Ok(Self {
            spec,
            fields,
            field_samples,
        })
    }

    pub fn evaluate(&self, sample: Sample, size: Option<[u32; 2]>) -> Result<FramePacket> {
        let time = sample.seconds();
        let size = size.unwrap_or(self.spec.size);
        ensure!(
            size.iter().all(|n| *n > 0 && *n <= 8192),
            "output dimensions must be in 1..=8192"
        );
        let camera = &self.spec.camera;
        let translation = Vec3::from(self.spec.model.translation.at(time));
        let rotation =
            Vec3::from(self.spec.model.rotation_degrees.at(time)) * (std::f32::consts::PI / 180.0);
        let rotation = Quat::from_euler(EulerRot::XYZ, rotation.x, rotation.y, rotation.z);
        let model_scale = self.spec.model.scale.at(time);
        let model =
            Mat4::from_scale_rotation_translation(Vec3::splat(model_scale), rotation, translation);
        ensure!(model.is_finite(), "model transform overflows at {time}s");
        let eye = Vec3::from(camera.eye.at(time));
        let target = Vec3::from(camera.target.at(time));
        let up = Vec3::from(camera.up).normalize();
        ensure!(
            eye.is_finite() && target.is_finite(),
            "camera values overflowed at {time}s"
        );
        ensure!(
            (target - eye).length_squared() > 1e-8
                && (target - eye).normalize().cross(up).length_squared() > 1e-8,
            "camera eye/target/up form a degenerate view at {time}s"
        );
        let view = Mat4::look_at_rh(eye, target, up);
        // glam's perspective_rh uses the WebGPU/Vulkan [0, 1] depth range.
        let scale = [
            size[0] as f32 / self.spec.size[0] as f32,
            size[1] as f32 / self.spec.size[1] as f32,
        ];
        let rect =
            camera
                .viewport
                .unwrap_or([0., 0., self.spec.size[0] as f32, self.spec.size[1] as f32]);
        let viewport = [
            rect[0] * scale[0],
            rect[1] * scale[1],
            rect[2] * scale[0],
            rect[3] * scale[1],
        ];
        let projection = Mat4::perspective_rh(
            camera.fov_y_degrees.to_radians(),
            viewport[2] / viewport[3],
            camera.near,
            camera.far,
        );
        let view_projection = projection * view;
        ensure!(
            view_projection.is_finite(),
            "camera projection overflows at {time}s; reduce clipping-plane magnitudes or camera coordinates"
        );
        let mut packet = FramePacket {
            sample,
            size,
            background: self.spec.background,
            view_projection,
            eye,
            viewport,
            surfaces: vec![],
            paths: vec![],
            markers: vec![],
            graphics: vec![],
            legends: vec![],
        };
        let field_point = |[x, z]: [f32; 2], value: f32, height: f32, lift: f32| {
            model
                .transform_point3(Vec3::new(x, value * height + lift, z))
                .to_array()
        };
        // Text and pixel-width paths scale uniformly with output height; positions follow the canvas axes.
        let screen = |p: [f32; 2]| [p[0] * scale[0], p[1] * scale[1]];
        for (node, samples) in self.spec.nodes.iter().zip(&self.field_samples) {
            match node {
                NodeSpec::Surface {
                    id,
                    field,
                    height,
                    palette,
                    grid,
                } => packet.surfaces.push(SurfaceDraw {
                    id: id.clone(),
                    field: self.fields[field].clone(),
                    model,
                    height: height.at(time),
                    palette: palette.clone(),
                    shift: palette.shift.at(time),
                    grid: *grid,
                }),
                NodeSpec::Trace {
                    id,
                    field: _,
                    points,
                    height,
                    lift,
                    width,
                    color,
                    reveal,
                } => {
                    let height = height.at(time);
                    packet.paths.push(PathDraw {
                        id: id.clone(),
                        points: points
                            .iter()
                            .zip(samples)
                            .map(|(point, value)| field_point(*point, *value, height, *lift))
                            .collect(),
                        width: *width * scale[1],
                        color: *color,
                        reveal: reveal.at(time),
                    });
                }
                NodeSpec::Polyline {
                    id,
                    points,
                    width,
                    color,
                    reveal,
                } => packet.paths.push(PathDraw {
                    id: id.clone(),
                    points: points
                        .iter()
                        .map(|point| model.transform_point3(Vec3::from(*point)).to_array())
                        .collect(),
                    width: *width * scale[1],
                    color: *color,
                    reveal: reveal.at(time),
                }),
                NodeSpec::Markers {
                    id,
                    field: _,
                    points,
                    height,
                    lift,
                    radius,
                    colors,
                } => {
                    let height = height.at(time);
                    for (i, (point, value)) in points.iter().zip(samples).enumerate() {
                        let position = field_point(*point, *value, height, *lift);
                        finite(
                            &position,
                            &format!("node {id:?}: marker position at {time}s"),
                        )?;
                        let radius = *radius * model_scale;
                        positive(radius, &format!("node {id:?}: scaled marker radius"))?;
                        for component in position {
                            finite(
                                &[component - radius, component + radius],
                                &format!("node {id:?}: marker extent at {time}s"),
                            )?;
                        }
                        // A finite world-space sphere can still overflow the
                        // shader's clip-space arithmetic. Check its bounding
                        // box, which encloses every generated mesh vertex.
                        for x in [position[0] - radius, position[0] + radius] {
                            for y in [position[1] - radius, position[1] + radius] {
                                for z in [position[2] - radius, position[2] + radius] {
                                    ensure!(
                                        (view_projection * Vec3::new(x, y, z).extend(1.0))
                                            .is_finite(),
                                        "node {id:?}: projected marker extent overflows at {time}s; reduce marker radius, model scale or camera magnification"
                                    );
                                }
                            }
                        }
                        packet.markers.push(MarkerDraw {
                            position,
                            radius,
                            color: colors[i % colors.len()],
                        });
                    }
                }
                NodeSpec::Text {
                    id,
                    text,
                    position,
                    font_size,
                    color,
                    opacity,
                } => {
                    let mut c = *color;
                    c[3] *= opacity.at(time);
                    packet.graphics.push(GraphicDraw {
                        id: id.clone(),
                        text: text.clone(),
                        font_size: *font_size * scale[1],
                        color: c,
                        placement: Placement::Screen {
                            position: screen(*position),
                        },
                    });
                }
                NodeSpec::Label {
                    id,
                    text,
                    anchor,
                    offset,
                    font_size,
                    color,
                    occlusion,
                } => {
                    let anchor = match anchor {
                        Anchor::World { position } => model
                            .transform_point3(Vec3::from(position.at(time)))
                            .to_array(),
                        Anchor::Field {
                            field: _,
                            point,
                            height,
                            lift,
                        } => field_point(*point, samples[0], height.at(time), *lift),
                    };
                    packet.graphics.push(GraphicDraw {
                        id: id.clone(),
                        text: text.clone(),
                        font_size: *font_size * scale[1],
                        color: *color,
                        placement: Placement::Projected {
                            anchor,
                            offset: screen(*offset),
                            occlusion: *occlusion,
                        },
                    });
                }
                NodeSpec::Legend {
                    id,
                    position,
                    size,
                    palette,
                } => {
                    let position = screen(*position);
                    let size = screen(*size);
                    finite(&position, &format!("node {id:?}: legend position"))?;
                    finite(&size, &format!("node {id:?}: legend size"))?;
                    finite(
                        &[position[0] + size[0], position[1] + size[1]],
                        &format!("node {id:?}: legend extent"),
                    )?;
                    packet.legends.push(LegendDraw {
                        position,
                        size,
                        palette: palette.clone(),
                        shift: palette.shift.at(time),
                    });
                }
                NodeSpec::GraphicsPlane {
                    id,
                    text,
                    position,
                    right,
                    up,
                    width,
                    font_size,
                    color,
                } => packet.graphics.push(GraphicDraw {
                    id: id.clone(),
                    text: text.clone(),
                    font_size: *font_size,
                    color: *color,
                    placement: Placement::World {
                        position: model
                            .transform_point3(Vec3::from(position.at(time)))
                            .to_array(),
                        right: (rotation * Vec3::from(*right)).to_array(),
                        up: (rotation * Vec3::from(*up)).to_array(),
                        width: *width * model_scale,
                    },
                }),
            }
        }
        for surface in &packet.surfaces {
            finite(
                &[surface.height, surface.shift],
                &format!("node {:?}: resolved surface at {time}s", surface.id),
            )?;
            for value in surface.field.range {
                let height = value * surface.height;
                for x in [surface.field.bounds[0], surface.field.bounds[1]] {
                    for z in [surface.field.bounds[2], surface.field.bounds[3]] {
                        let point = surface.model.transform_point3(Vec3::new(x, height, z));
                        ensure!(
                            point.is_finite() && (view_projection * point.extend(1.0)).is_finite(),
                            "node {:?}: surface height, model transform or projection overflows at {time}s",
                            surface.id
                        );
                    }
                }
            }
        }
        for path in &packet.paths {
            let position_name = format!("node {:?}: resolved path position at {time}s", path.id);
            stroke_width(
                path.width,
                &format!("node {:?}: scaled path width", path.id),
            )?;
            for point in &path.points {
                finite(point, &position_name)?;
                ensure!(
                    (view_projection * Vec3::from(*point).extend(1.0)).is_finite(),
                    "node {:?}: projected path position overflows at {time}s",
                    path.id
                );
            }
        }
        for graphic in &packet.graphics {
            let name = format!("node {:?}", graphic.id);
            let extent = crate::text::extent(&graphic.text, graphic.font_size)
                .with_context(|| name.clone())?;
            match graphic.placement {
                Placement::Screen { position } => {
                    finite(&position, &format!("{name}: scaled screen position"))?
                }
                Placement::Projected { anchor, offset, .. } => {
                    finite(&anchor, &format!("{name}: resolved anchor"))?;
                    finite(&offset, &format!("{name}: scaled offset"))?;
                    ensure!(
                        (view_projection * Vec3::from(anchor).extend(1.0)).is_finite(),
                        "{name}: projected anchor overflows at {time}s"
                    );
                }
                Placement::World {
                    position,
                    right,
                    up,
                    width,
                } => {
                    finite(&position, &format!("{name}: resolved world position"))?;
                    positive(width, &format!("{name}: scaled graphics plane width"))?;
                    let right = Vec3::from(right).try_normalize().with_context(|| {
                        format!("{name}: right basis cannot be normalized in f32")
                    })? * width;
                    let down = -Vec3::from(up)
                        .try_normalize()
                        .with_context(|| format!("{name}: up basis cannot be normalized in f32"))?
                        * width
                        * extent[1] as f32
                        / extent[0] as f32;
                    let position = Vec3::from(position);
                    for point in [
                        position,
                        position + right,
                        position + right + down,
                        position + down,
                    ] {
                        ensure!(
                            point.is_finite() && (view_projection * point.extend(1.0)).is_finite(),
                            "{name}: graphics plane extent or projection overflows at {time}s"
                        );
                    }
                }
            }
        }
        Ok(packet)
    }

    pub fn inspect(&self, packet: &FramePacket) -> serde_json::Value {
        serde_json::json!({
            "sample": packet.sample, "seconds": packet.sample.seconds(), "size": packet.size,
            "background": packet.background,
            "camera": {"eye": packet.eye.to_array(), "view_projection": packet.view_projection.to_cols_array(), "viewport":packet.viewport},
            "assets": self.fields.iter().map(|(id,f)| serde_json::json!({"id":id,"shape":[f.height,f.width],"content_hash":f.content_hash,"range":f.range})).collect::<Vec<_>>(),
            "surfaces":packet.surfaces.iter().map(|s| serde_json::json!({"id":s.id,"field":s.field.content_hash,"model":s.model.to_cols_array(),"height":s.height,"palette":s.palette,"palette_shift":s.shift,"grid":s.grid})).collect::<Vec<_>>(),
            "paths":packet.paths,
            "markers":packet.markers,
            "graphics":packet.graphics,
            "legends":packet.legends
        })
    }
}

pub fn read_scene(path: &Path) -> Result<SceneSpec> {
    let mut bytes = Vec::new();
    std::fs::File::open(path)
        .with_context(|| format!("read scene {}", path.display()))?
        .take(64 * 1024 * 1024 + 1)
        .read_to_end(&mut bytes)?;
    if bytes.len() > 64 * 1024 * 1024 {
        bail!("scene JSON exceeds 64 MiB; use binary assets for dense data");
    }
    let value = serde_json::from_slice(&bytes)
        .with_context(|| format!("parse scene {}", path.display()))?;
    parse_scene(value).with_context(|| format!("parse scene {}", path.display()))
}

/// Parse the authoring boundary with stable IDs attached to malformed nodes and
/// assets, including unknown fields rejected before semantic compilation.
pub fn parse_scene(scene: serde_json::Value) -> Result<SceneSpec> {
    if let Some(nodes) = scene.get("nodes").and_then(serde_json::Value::as_array) {
        for (index, node) in nodes.iter().enumerate() {
            serde_json::from_value::<NodeSpec>(node.clone()).map_err(|error| {
                anyhow::anyhow!(
                    "node {}: {error}",
                    node.get("id").unwrap_or(&serde_json::json!(index))
                )
            })?;
        }
    }
    if let Some(assets) = scene.get("assets").and_then(serde_json::Value::as_array) {
        for (index, asset) in assets.iter().enumerate() {
            serde_json::from_value::<FieldAsset>(asset.clone()).map_err(|error| {
                anyhow::anyhow!(
                    "asset {}: {error}",
                    asset.get("id").unwrap_or(&serde_json::json!(index))
                )
            })?;
        }
    }
    Ok(serde_json::from_value(scene)?)
}
