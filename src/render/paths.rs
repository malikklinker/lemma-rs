use crate::{eval::PathDraw, scene::srgb_to_linear};
use bytemuck::{Pod, Zeroable};
use glam::{DVec2, DVec3, DVec4, Mat4};

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
pub struct StrokeVertex {
    pub clip: [f32; 4],
    pub color: [f32; 4],
    pub local: [f32; 2],
    pub shape: [f32; 2],
}

/// Clip centerlines in homogeneous camera space before expanding in pixels.
/// Side planes include the stroke radius so coverage is retained when the
/// centerline itself is just outside the viewport.
fn clip_segment(a: DVec4, b: DVec4, padding: DVec2) -> Option<(DVec4, DVec4)> {
    let planes = [
        DVec4::new(1., 0., 0., 1. + padding.x),
        DVec4::new(-1., 0., 0., 1. + padding.x),
        DVec4::new(0., 1., 0., 1. + padding.y),
        DVec4::new(0., -1., 0., 1. + padding.y),
        DVec4::new(0., 0., 1., 0.),
        DVec4::new(0., 0., -1., 1.),
    ];
    let (mut start, mut end) = (0.0_f64, 1.0_f64);
    for plane in planes {
        let (da, db) = (a.dot(plane), b.dot(plane));
        if da < 0.0 && db < 0.0 {
            return None;
        }
        if (da < 0.0) != (db < 0.0) {
            let t = da / (da - db);
            if da < 0.0 {
                start = start.max(t);
            } else {
                end = end.min(t);
            }
        }
    }
    if start > end {
        return None;
    }
    let (a, b) = (a.lerp(b, start), a.lerp(b, end));
    (a.w > 0.0 && b.w > 0.0).then_some((a, b))
}

pub fn tessellate(paths: &[PathDraw], matrix: Mat4, size: [f32; 2]) -> Vec<StrokeVertex> {
    let viewport = DVec2::from_array(size.map(f64::from));
    let matrix = matrix.as_dmat4();
    let point = |p: [f32; 3]| DVec3::from_array(p.map(f64::from));
    let mut vertices = Vec::new();
    for path in paths {
        // Finite f32 coordinates can have distances whose squared intermediates
        // overflow f32. Keep arc length and interpolation in f64, then upload
        // only the bounded, camera-clipped screen geometry.
        let total: f64 = path
            .points
            .windows(2)
            .map(|p| point(p[0]).distance(point(p[1])))
            .sum();
        let mut remaining = total * f64::from(path.reveal.clamp(0.0, 1.0));
        if remaining <= 0.0 {
            continue;
        }
        let color = srgb_to_linear(path.color);
        let half_width = f64::from(path.width) * 0.5;
        let extent = half_width + 1.0;
        let padding = DVec2::splat(extent) / viewport * 2.0;
        for points in path.points.windows(2) {
            let (world_a, mut world_b) = (point(points[0]), point(points[1]));
            let world_length = world_a.distance(world_b);
            if world_length == 0.0 {
                continue;
            }
            if remaining < world_length {
                world_b = world_a.lerp(world_b, remaining / world_length);
            }
            remaining -= world_length;
            if let Some((a, b)) = clip_segment(
                matrix * world_a.extend(1.0),
                matrix * world_b.extend(1.0),
                padding,
            ) {
                let pa = a.truncate().truncate() / a.w * viewport * 0.5;
                let pb = b.truncate().truncate() / b.w * viewport * 0.5;
                let delta = pb - pa;
                let length = delta.length();
                if length > 1e-4 {
                    let direction = delta / length;
                    let normal = DVec2::new(-direction.y, direction.x);
                    let make = |endpoint: DVec4, x: f64, y: f64, local_x: f64| {
                        let shift = (direction * x + normal * y) * 2.0 / viewport;
                        // Stroke attributes use screen-linear interpolation.
                        // Normalizing clip W to one preserves projected depth
                        // and prevents large world magnitudes from overflowing
                        // the expanded homogeneous coordinates on upload.
                        let ndc = endpoint.truncate() / endpoint.w;
                        StrokeVertex {
                            clip: [
                                (ndc.x + shift.x) as f32,
                                (ndc.y + shift.y) as f32,
                                ndc.z as f32,
                                1.0,
                            ],
                            color,
                            local: [local_x as f32, y as f32],
                            shape: [length as f32, half_width as f32],
                        }
                    };
                    let quad = [
                        make(a, -extent, -extent, -extent),
                        make(b, extent, -extent, length + extent),
                        make(b, extent, extent, length + extent),
                        make(a, -extent, extent, -extent),
                    ];
                    for i in [0, 1, 2, 0, 2, 3] {
                        vertices.push(quad[i]);
                    }
                }
            }
            if remaining <= 0.0 {
                break;
            }
        }
    }
    vertices
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn near_plane_clipping_prevents_exploding_strokes() {
        let path = PathDraw {
            id: "near".into(),
            points: vec![[0., 0., 1.], [0.4, 0.3, -2.]],
            width: 4.,
            color: [1.; 4],
            reveal: 1.,
        };
        let matrix = Mat4::perspective_rh(1.0, 1.0, 0.1, 10.0);
        let vertices = tessellate(&[path], matrix, [500., 500.]);
        assert_eq!(vertices.len(), 6);
        assert!(vertices.iter().all(|v| v.clip.iter().all(|n| n.is_finite())
            && v.clip[3] > 0.0
            && v.clip[2] >= -1e-6));
    }
    #[test]
    fn repeated_points_and_zero_reveal_are_safe() {
        let mut path = PathDraw {
            id: "empty".into(),
            points: vec![[0., 0., 0.5]; 2],
            width: 2.,
            color: [1.; 4],
            reveal: 1.,
        };
        assert!(tessellate(&[path.clone()], Mat4::IDENTITY, [100., 100.]).is_empty());
        path.points.push([0.5, 0.5, 0.5]);
        path.reveal = 0.0;
        assert!(tessellate(&[path], Mat4::IDENTITY, [100., 100.]).is_empty());
    }

    #[test]
    fn stroke_coverage_outside_centerline_frustum_is_preserved() {
        for axis in 0..2 {
            for side in [-1.0, 1.0] {
                let mut points = [[-0.5, 0.0, 0.5], [0.5, 0.0, 0.5]];
                for (i, point) in points.iter_mut().enumerate() {
                    point[axis] = side * 1.04;
                    point[1 - axis] = if i == 0 { -0.5 } else { 0.5 };
                }
                let mut path = PathDraw {
                    id: "edge".into(),
                    points: points.to_vec(),
                    width: 8.0,
                    color: [1.0; 4],
                    reveal: 1.0,
                };
                assert_eq!(
                    tessellate(&[path.clone()], Mat4::IDENTITY, [100.0, 100.0]).len(),
                    6,
                    "stroke centered two pixels beyond edge still covers the viewport"
                );
                for point in &mut path.points {
                    point[axis] = side * 1.2;
                }
                assert!(
                    tessellate(&[path], Mat4::IDENTITY, [100.0, 100.0]).is_empty(),
                    "fully offscreen strokes should still be culled"
                );
            }
        }
    }

    #[test]
    fn reveal_uses_finite_arc_lengths_at_large_world_coordinates() {
        let mut path = PathDraw {
            id: "large".into(),
            points: vec![[-1e20, 0.0, -1e20], [1e20, 0.0, -1e20]],
            width: 8.0,
            color: [1.0; 4],
            reveal: 0.25,
        };
        let matrix = Mat4::perspective_rh(120.0_f32.to_radians(), 1.0, 0.1, 1e22);
        let quarter = tessellate(&[path.clone()], matrix, [256.0, 256.0]);
        assert_eq!(quarter.len(), 6);
        assert!(quarter.iter().all(|v| v.clip[0] < 0.0));
        path.reveal = 0.75;
        let three_quarters = tessellate(&[path], matrix, [256.0, 256.0]);
        assert_eq!(three_quarters.len(), 6);
        assert!(three_quarters.iter().any(|v| v.clip[0] > 0.0));
        assert!(quarter.iter().chain(&three_quarters).all(|v| {
            v.clip
                .iter()
                .chain(&v.local)
                .chain(&v.shape)
                .all(|n| n.is_finite())
        }));
    }

    #[test]
    fn positive_camera_depths_below_one_microunit_remain_visible() {
        let path = PathDraw {
            id: "small".into(),
            points: vec![[-2e-8, 0.0, -1e-7], [2e-8, 0.0, -1e-7]],
            width: 4.0,
            color: [1.0; 4],
            reveal: 1.0,
        };
        let matrix = Mat4::perspective_rh(1.0, 1.0, 1e-8, 1.0);
        let vertices = tessellate(&[path], matrix, [256.0, 256.0]);
        assert_eq!(vertices.len(), 6);
        assert!(
            vertices
                .iter()
                .all(|v| v.clip.iter().all(|n| n.is_finite()))
        );
    }
}
