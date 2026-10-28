//! Publish complete files atomically, with an atomic no-clobber option.
use anyhow::{Context, Result, ensure};
use std::{
    fs::{self, OpenOptions},
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
};

pub(crate) struct AtomicOutput {
    pub temporary: PathBuf,
    destination: PathBuf,
    overwrite: bool,
}
impl AtomicOutput {
    pub fn new(destination: &Path, overwrite: bool) -> Result<Self> {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        ensure!(
            overwrite || !destination.exists(),
            "{} already exists; pass --overwrite to replace it",
            destination.display()
        );
        if let Some(parent) = destination.parent().filter(|p| !p.as_os_str().is_empty()) {
            fs::create_dir_all(parent)?;
        }
        let extension = destination
            .extension()
            .unwrap_or_default()
            .to_string_lossy();
        let temporary = destination.with_extension(format!(
            "{}.{}.partial.{extension}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temporary)
            .with_context(|| format!("reserve temporary output {}", temporary.display()))?;
        Ok(Self {
            temporary,
            destination: destination.into(),
            overwrite,
        })
    }
    pub fn commit(self) -> Result<()> {
        if self.overwrite {
            fs::rename(&self.temporary, &self.destination)?;
        } else {
            fs::hard_link(&self.temporary, &self.destination).with_context(|| {
                format!(
                    "publish {}; destination must still be absent",
                    self.destination.display()
                )
            })?;
        }
        Ok(())
    }
}
impl Drop for AtomicOutput {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.temporary);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn no_clobber_is_checked_at_publication_and_failure_cleans_up() {
        let dir = std::env::temp_dir().join(format!("lemma-output-test-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        let destination = dir.join("result.mp4");
        let _ = fs::remove_file(&destination);
        let output = AtomicOutput::new(&destination, false).unwrap();
        let temporary = output.temporary.clone();
        fs::write(&temporary, b"new encoded video").unwrap();
        fs::write(&destination, b"created concurrently").unwrap();
        assert!(output.commit().is_err());
        assert_eq!(fs::read(&destination).unwrap(), b"created concurrently");
        assert!(!temporary.exists());
        let output = AtomicOutput::new(&destination, true).unwrap();
        fs::write(&output.temporary, b"complete replacement").unwrap();
        output.commit().unwrap();
        assert_eq!(fs::read(&destination).unwrap(), b"complete replacement");
        fs::remove_dir_all(dir).unwrap();
    }
}
