//! Immutable x-fastest, vertex-sampled scalar fields and portable saved results.
use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use std::{
    collections::HashMap,
    fs,
    path::{Path, PathBuf},
    sync::{Arc, Weak},
};

pub const MAX_FIELD_SAMPLES: u64 = 4_194_304;
const SYNTHETIC_PROVIDER: &str = "lemma.synthetic.trigonometric.v1.f32";

/// Execution policy for the numerical sources supported by this version.
#[derive(Debug, Default, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum SourcePolicy {
    #[default]
    OnDemand,
    RequireSaved,
}

#[derive(Debug, Default, Clone, Serialize)]
pub struct SourceCacheStats {
    pub synthetic_computations: u64,
    pub saved_validations: u64,
    pub hits: u64,
    pub entries: usize,
}

/// Shares immutable results across successful scene reloads. Weak entries do not
/// retain numerical fields after their plans and frame packets have been dropped.
#[derive(Default)]
pub struct SourceCache {
    fields: HashMap<String, Weak<ScalarField>>,
    stats: SourceCacheStats,
}

impl SourceCache {
    /// Prime reuse from an already compiled plan, without re-running its source.
    /// Saved sources are still validated from disk on the next resolve call.
    pub fn seed(&mut self, source: &FieldSource, field: Arc<ScalarField>) -> Result<()> {
        let key = match source {
            FieldSource::Synthetic { resolution, bounds } => {
                let request_hash = synthetic_request_hash(*resolution, *bounds);
                ensure!(
                    field.width == resolution[0]
                        && field.height == resolution[1]
                        && field.bounds.map(f32::to_bits) == bounds.map(f32::to_bits),
                    "cannot seed numerical cache: field shape/bounds do not match source"
                );
                ensure!(
                    field.provenance.as_ref().is_some_and(
                        |p| p.provider == SYNTHETIC_PROVIDER && p.request_hash == request_hash
                    ),
                    "cannot seed numerical cache: field provenance does not match source"
                );
                format!("request:{request_hash}")
            }
            FieldSource::Saved { .. } => saved_content_key(&field),
        };
        self.fields.insert(key, Arc::downgrade(&field));
        Ok(())
    }

    pub fn stats(&self) -> SourceCacheStats {
        SourceCacheStats {
            entries: self
                .fields
                .values()
                .filter(|v| v.strong_count() > 0)
                .count(),
            ..self.stats.clone()
        }
    }

    pub fn resolve(
        &mut self,
        source: &FieldSource,
        base: &Path,
        policy: SourcePolicy,
    ) -> Result<Arc<ScalarField>> {
        self.fields.retain(|_, value| value.strong_count() > 0);
        match source {
            FieldSource::Synthetic { resolution, bounds } => {
                ensure!(
                    policy != SourcePolicy::RequireSaved,
                    "require-saved policy rejects synthetic computation; save the field and reference its manifest with kind 'saved'"
                );
                validate_shape(resolution[0], resolution[1], *bounds)?;
                let key = format!("request:{}", synthetic_request_hash(*resolution, *bounds));
                if let Some(field) = self.fields.get(&key).and_then(Weak::upgrade) {
                    self.stats.hits += 1;
                    return Ok(field);
                }
                let field = Arc::new(ScalarField::synthetic(*resolution, *bounds)?);
                self.stats.synthetic_computations += 1;
                self.fields.insert(key, Arc::downgrade(&field));
                Ok(field)
            }
            FieldSource::Saved { manifest } => {
                // A path, mtime, or previously claimed content hash is not a
                // freshness guarantee. Validate the current manifest and bytes
                // on each reload before sharing a previously loaded result.
                let loaded = ScalarField::load_saved(&base.join(manifest))?;
                self.stats.saved_validations += 1;
                let key = saved_content_key(&loaded);
                if let Some(field) = self.fields.get(&key).and_then(Weak::upgrade) {
                    self.stats.hits += 1;
                    return Ok(field);
                }
                let field = Arc::new(loaded);
                self.fields.insert(key, Arc::downgrade(&field));
                Ok(field)
            }
        }
    }
}

fn saved_content_key(field: &ScalarField) -> String {
    match &field.provenance {
        Some(p) => format!(
            "content:{}:{}:{}",
            field.content_hash, p.provider, p.request_hash
        ),
        None => format!("content:{}:unspecified", field.content_hash),
    }
}

fn synthetic_request_hash(resolution: [u32; 2], bounds: [f32; 4]) -> String {
    let mut hash = blake3::Hasher::new();
    hash.update(SYNTHETIC_PROVIDER.as_bytes());
    hash.update(&resolution[0].to_le_bytes());
    hash.update(&resolution[1].to_le_bytes());
    for value in bounds {
        hash.update(&value.to_le_bytes());
    }
    hash.finalize().to_hex().to_string()
}

/// Optional provenance for the built-in demonstrator, not a foreign-provider ABI.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FieldProvenance {
    pub provider: String,
    pub request_hash: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "kebab-case", deny_unknown_fields)]
pub enum FieldSource {
    Synthetic {
        resolution: [u32; 2],
        bounds: [f32; 4],
    },
    Saved {
        manifest: PathBuf,
    },
}

/// Validated immutable numerical data. Construct a new field to change samples
/// so its range and content identity are always updated together.
///
/// ```compile_fail
/// let mut field = lemma::data::ScalarField::new(2, 2, [0., 1., 0., 1.], vec![0.; 4]).unwrap();
/// field.values()[0] = 1.0;
/// ```
#[derive(Debug, Clone)]
pub struct ScalarField {
    pub(crate) width: u32,
    pub(crate) height: u32,
    /// [x_min, x_max, z_min, z_max], inclusive vertex coordinates.
    pub(crate) bounds: [f32; 4],
    pub(crate) values: Arc<[f32]>,
    pub(crate) content_hash: String,
    pub(crate) range: [f32; 2],
    pub(crate) provenance: Option<FieldProvenance>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FieldManifest {
    pub version: u32,
    pub dtype: String,
    pub shape: [u32; 2],
    pub byte_strides: [u64; 2],
    pub axes: [String; 2],
    pub sampling: String,
    pub bounds: [f32; 4],
    pub values_file: PathBuf,
    pub content_hash: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provenance: Option<FieldProvenance>,
}

fn validate_shape(width: u32, height: u32, bounds: [f32; 4]) -> Result<usize> {
    ensure!(
        width >= 2 && height >= 2,
        "field dimensions must each be at least 2"
    );
    let count = u64::from(width) * u64::from(height);
    ensure!(
        count <= MAX_FIELD_SAMPLES,
        "field exceeds {MAX_FIELD_SAMPLES} samples"
    );
    ensure!(
        bounds.iter().all(|x| x.is_finite()) && bounds[0] < bounds[1] && bounds[2] < bounds[3],
        "field bounds must be finite, increasing [x_min, x_max, z_min, z_max]"
    );
    let spans = [bounds[1] - bounds[0], bounds[3] - bounds[2]];
    let spacing = [
        spans[0] / (width - 1) as f32,
        spans[1] / (height - 1) as f32,
    ];
    ensure!(
        spans
            .into_iter()
            .chain(spacing)
            .all(|value| value.is_finite() && value > 0.0),
        "field bounds span and grid spacing must be finite and positive in f32; domain overflows or grid spacing underflows"
    );
    Ok(count as usize)
}

fn coordinate(min: f32, max: f32, index: u32, count: u32) -> f32 {
    let t = f64::from(index) / f64::from(count - 1);
    (f64::from(min) * (1.0 - t) + f64::from(max) * t) as f32
}

impl ScalarField {
    pub fn width(&self) -> u32 {
        self.width
    }
    pub fn height(&self) -> u32 {
        self.height
    }
    pub fn bounds(&self) -> [f32; 4] {
        self.bounds
    }
    pub fn values(&self) -> &[f32] {
        &self.values
    }
    pub fn content_hash(&self) -> &str {
        &self.content_hash
    }
    pub fn range(&self) -> [f32; 2] {
        self.range
    }
    pub fn provenance(&self) -> Option<&FieldProvenance> {
        self.provenance.as_ref()
    }

    pub fn new(width: u32, height: u32, bounds: [f32; 4], values: Vec<f32>) -> Result<Self> {
        let count = validate_shape(width, height, bounds)?;
        ensure!(
            values.len() == count,
            "field needs {count} values, received {}",
            values.len()
        );
        ensure!(
            values.iter().all(|v| v.is_finite()),
            "field contains NaN or infinite values"
        );
        let mut hash = blake3::Hasher::new();
        hash.update(b"lemma.scalar-field.v1.f32le.vertex.xz\0");
        hash.update(&width.to_le_bytes());
        hash.update(&height.to_le_bytes());
        for value in bounds.iter().chain(values.iter()) {
            hash.update(&value.to_le_bytes());
        }
        let range = values
            .iter()
            .fold([f32::INFINITY, f32::NEG_INFINITY], |r, v| {
                [r[0].min(*v), r[1].max(*v)]
            });
        Ok(Self {
            width,
            height,
            bounds,
            values: values.into(),
            content_hash: hash.finalize().to_hex().to_string(),
            range,
            provenance: None,
        })
    }

    pub fn synthetic(resolution: [u32; 2], bounds: [f32; 4]) -> Result<Self> {
        let count = validate_shape(resolution[0], resolution[1], bounds)?;
        let mut values = Vec::with_capacity(count);
        for row in 0..resolution[1] {
            let z = coordinate(bounds[2], bounds[3], row, resolution[1]);
            for col in 0..resolution[0] {
                let x = coordinate(bounds[0], bounds[1], col, resolution[0]);
                let r2 = x * x + z * z;
                values.push(
                    0.8 * (1.4 * x).sin() * (1.2 * z).cos() * (-0.09 * r2).exp()
                        + 0.35 * (2.5 * r2.sqrt()).cos() * (-0.08 * r2).exp(),
                );
            }
        }
        let mut field = Self::new(resolution[0], resolution[1], bounds, values)?;
        field.provenance = Some(FieldProvenance {
            provider: SYNTHETIC_PROVIDER.into(),
            request_hash: synthetic_request_hash(resolution, bounds),
        });
        Ok(field)
    }

    pub fn load_saved(path: &Path) -> Result<Self> {
        ensure!(
            fs::metadata(path)
                .with_context(|| format!("read field manifest {}", path.display()))?
                .len()
                <= 64 * 1024,
            "field manifest exceeds 64 KiB; dense data belongs in its binary payload"
        );
        let metadata: FieldManifest = serde_json::from_slice(
            &fs::read(path).with_context(|| format!("read field manifest {}", path.display()))?,
        )
        .with_context(|| format!("parse field manifest {}", path.display()))?;
        ensure!(
            metadata.version == 1 && metadata.dtype == "f32-le",
            "unsupported field version or dtype"
        );
        ensure!(
            metadata.axes == ["z", "x"] && metadata.sampling == "vertices",
            "unsupported field axes or sampling"
        );
        let [height, width] = metadata.shape;
        let count = validate_shape(width, height, metadata.bounds)?;
        ensure!(
            metadata.byte_strides == [u64::from(width) * 4, 4],
            "field byte strides must describe a contiguous x-fastest array"
        );
        if let Some(provenance) = &metadata.provenance {
            ensure!(
                provenance.provider == SYNTHETIC_PROVIDER,
                "unsupported field provenance provider {:?}",
                provenance.provider
            );
            ensure!(
                provenance.request_hash == synthetic_request_hash([width, height], metadata.bounds),
                "field provenance request hash mismatch"
            );
        }
        let payload = path
            .parent()
            .unwrap_or(Path::new("."))
            .join(&metadata.values_file);
        ensure!(
            fs::metadata(&payload)?.len() == (count * 4) as u64,
            "field payload length does not match its shape"
        );
        let bytes = fs::read(&payload)?;
        ensure!(
            bytes.len() == count * 4,
            "field payload length changed while reading {}",
            payload.display()
        );
        let values = bytes
            .chunks_exact(4)
            .map(|b| f32::from_le_bytes(b.try_into().unwrap()))
            .collect();
        let mut result = Self::new(width, height, metadata.bounds, values)?;
        ensure!(
            result.content_hash == metadata.content_hash,
            "field content hash mismatch: {}",
            payload.display()
        );
        result.provenance = metadata.provenance;
        Ok(result)
    }

    /// Publish payload first and metadata last; existing content is verified on replay.
    pub fn save(&self, directory: &Path) -> Result<PathBuf> {
        fs::create_dir_all(directory)?;
        let stem = &self.content_hash;
        let values_file = format!("{stem}.f32");
        let manifest_file = directory.join(format!("{stem}.json"));
        let mut bytes = Vec::with_capacity(self.values.len() * 4);
        for value in self.values.iter() {
            bytes.extend_from_slice(&value.to_le_bytes());
        }
        atomic_write(&directory.join(&values_file), &bytes)?;
        let metadata = FieldManifest {
            version: 1,
            dtype: "f32-le".into(),
            shape: [self.height, self.width],
            byte_strides: [u64::from(self.width) * 4, 4],
            axes: ["z".into(), "x".into()],
            sampling: "vertices".into(),
            bounds: self.bounds,
            values_file: values_file.into(),
            content_hash: self.content_hash.clone(),
            provenance: self.provenance.clone(),
        };
        atomic_write(&manifest_file, &serde_json::to_vec_pretty(&metadata)?)?;
        Ok(manifest_file)
    }

    pub fn position(&self, col: u32, row: u32) -> [f32; 2] {
        [
            coordinate(self.bounds[0], self.bounds[1], col, self.width),
            coordinate(self.bounds[2], self.bounds[3], row, self.height),
        ]
    }

    pub fn contains(&self, point: [f32; 2]) -> bool {
        point.iter().all(|x| x.is_finite())
            && point[0] >= self.bounds[0]
            && point[0] <= self.bounds[1]
            && point[1] >= self.bounds[2]
            && point[1] <= self.bounds[3]
    }

    /// Interpolate the same two triangles used to render each grid cell, with
    /// the diagonal from its upper-right to lower-left vertex. Field-attached
    /// geometry therefore follows the rendered surface even on a coarse grid.
    /// Coordinates outside the domain are clamped to its edges.
    pub fn sample(&self, [x, z]: [f32; 2]) -> f32 {
        let u = ((x - self.bounds[0]) / (self.bounds[1] - self.bounds[0])
            * (self.width - 1) as f32)
            .clamp(0.0, (self.width - 1) as f32);
        let v = ((z - self.bounds[2]) / (self.bounds[3] - self.bounds[2])
            * (self.height - 1) as f32)
            .clamp(0.0, (self.height - 1) as f32);
        let (col, row) = (u.floor() as u32, v.floor() as u32);
        let (next_col, next_row) = (
            (col + 1).min(self.width - 1),
            (row + 1).min(self.height - 1),
        );
        let get = |c, r| self.values[(r * self.width + c) as usize];
        let (u, v) = (f64::from(u.fract()), f64::from(v.fract()));
        let value = if u + v <= 1.0 {
            f64::from(get(col, row)) * (1.0 - u - v)
                + f64::from(get(next_col, row)) * u
                + f64::from(get(col, next_row)) * v
        } else {
            f64::from(get(next_col, row)) * (1.0 - v)
                + f64::from(get(col, next_row)) * (1.0 - u)
                + f64::from(get(next_col, next_row)) * (u + v - 1.0)
        };
        value as f32
    }
}

pub fn atomic_write(path: &Path, bytes: &[u8]) -> Result<()> {
    let output = crate::output::AtomicOutput::new(path, true)?;
    fs::write(&output.temporary, bytes)?;
    output.commit()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn field_layout_validation_and_surface_sampling() {
        let f = ScalarField::new(2, 2, [0., 1., 0., 1.], vec![0., 2., 4., 6.]).unwrap();
        assert_eq!(f.sample([0.5, 0.5]), 3.0);
        assert_eq!(f.sample([1., 1.]), 6.0);
        assert!(ScalarField::new(2, 2, [0., 1., 0., 1.], vec![0.]).is_err());
        assert!(ScalarField::new(2, 2, [0., 1., 0., 1.], vec![f32::NAN; 4]).is_err());
        assert!(ScalarField::synthetic([u32::MAX, u32::MAX], [0., 1., 0., 1.]).is_err());
        assert!(ScalarField::new(2, 2, [-f32::MAX, f32::MAX, 0., 1.], vec![0.; 4]).is_err());
        assert!(ScalarField::new(3, 2, [0., f32::from_bits(1), 0., 1.], vec![0.; 6]).is_err());
        let wide = ScalarField::new(4, 2, [0., 3e38, 0., 1.], vec![0.; 8]).unwrap();
        assert_eq!(wide.position(3, 1), [3e38, 1.]);
        assert!(wide.position(2, 1).iter().all(|x| x.is_finite()));
    }
    #[test]
    fn field_sampling_matches_both_mesh_triangles_and_shared_diagonal() {
        // A saddle distinguishes triangle interpolation from a bilinear patch:
        // the rendered diagonal joins the two zero-valued vertices.
        let field = ScalarField::new(2, 2, [0., 1., 0., 1.], vec![4., 0., 0., 8.]).unwrap();
        assert_eq!(field.sample([0.25, 0.25]), 2.0);
        assert_eq!(field.sample([0.75, 0.75]), 4.0);
        assert_eq!(field.sample([0.25, 0.75]), 0.0);
        assert_eq!(field.sample([0.5, 0.5]), 0.0);
        assert_eq!(field.sample([0., 0.]), 4.0);
        assert_eq!(field.sample([1., 1.]), 8.0);
        assert_eq!(field.sample([-1., 0.5]), 2.0);
        assert_eq!(field.sample([2., 0.5]), 4.0);

        let extreme = ScalarField::new(2, 2, [0., 1., 0., 1.], vec![f32::MAX; 4]).unwrap();
        assert_eq!(extreme.sample([0.2, 0.3]), f32::MAX);
        assert_eq!(extreme.sample([0.8, 0.7]), f32::MAX);
    }
    #[test]
    fn saved_data_replays_and_detects_corruption() {
        let dir = std::env::temp_dir().join(format!("lemma-data-test-{}", std::process::id()));
        let field = ScalarField::synthetic([5, 4], [-1., 1., -1., 1.]).unwrap();
        let manifest = field.save(&dir).unwrap();
        let loaded = ScalarField::load_saved(&manifest).unwrap();
        assert_eq!(field.content_hash, loaded.content_hash);
        assert_eq!(field.values, loaded.values);
        fs::write(dir.join(format!("{}.f32", field.content_hash)), [0; 80]).unwrap();
        assert!(ScalarField::load_saved(&manifest).is_err());
        fs::remove_dir_all(dir).unwrap();
    }
    #[test]
    fn concurrent_field_saves_replay_and_leave_no_temporary_files() {
        let dir =
            std::env::temp_dir().join(format!("lemma-concurrent-save-{}", std::process::id()));
        let field = Arc::new(ScalarField::synthetic([17, 13], [-1., 1., -1., 1.]).unwrap());
        let start = Arc::new(std::sync::Barrier::new(4));
        let handles: Vec<_> = (0..4)
            .map(|_| {
                let (field, start, dir) = (field.clone(), start.clone(), dir.clone());
                std::thread::spawn(move || {
                    start.wait();
                    for _ in 0..8 {
                        let manifest = field.save(&dir).unwrap();
                        let replay = ScalarField::load_saved(&manifest).unwrap();
                        assert_eq!(replay.content_hash(), field.content_hash());
                        assert_eq!(replay.values(), field.values());
                        assert_eq!(
                            replay.provenance().unwrap().request_hash,
                            field.provenance().unwrap().request_hash
                        );
                    }
                })
            })
            .collect();
        for handle in handles {
            handle.join().unwrap();
        }
        assert_eq!(fs::read_dir(&dir).unwrap().count(), 2);
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn failed_atomic_publication_cleans_up_reserved_temporary_file() {
        let dir = std::env::temp_dir().join(format!("lemma-failed-save-{}", std::process::id()));
        let destination = dir.join("existing-directory");
        fs::create_dir_all(&destination).unwrap();
        assert!(atomic_write(&destination, b"cannot replace a directory with a file").is_err());
        assert!(destination.is_dir());
        assert_eq!(fs::read_dir(&dir).unwrap().count(), 1);
        fs::remove_dir_all(dir).unwrap();
    }
}
