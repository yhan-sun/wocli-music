//! Commit a completed download without relying on hard-link support (FAT/exFAT).
use crate::model::{Error, Result};
use std::{
    fs::{self, File, OpenOptions},
    io::{Read, Write},
    path::Path,
    sync::atomic::{AtomicBool, Ordering},
};

fn check_cancelled(cancelled: &AtomicBool) -> Result<()> {
    if cancelled.load(Ordering::Relaxed) {
        Err(Error::coded("cancelled", "已取消"))
    } else {
        Ok(())
    }
}

pub(crate) fn commit(
    source: &Path,
    target: &Path,
    overwrite: bool,
    cancelled: &AtomicBool,
) -> Result<()> {
    commit_with(source, target, overwrite, cancelled, |source, target| {
        // tempfile uses native no-replace rename where supported, including
        // macOS and Windows. Do not let an unsuccessful persist delete staging:
        // it is still needed for the fallback and belongs to the caller.
        let mut path = tempfile::TempPath::try_from_path(source)?;
        path.disable_cleanup(true);
        path.persist_noclobber(target).map_err(|error| error.error)
    })
}

fn commit_with(
    source: &Path,
    target: &Path,
    overwrite: bool,
    cancelled: &AtomicBool,
    publish: impl FnOnce(&Path, &Path) -> std::io::Result<()>,
) -> Result<()> {
    check_cancelled(cancelled)?;
    if overwrite {
        fs::rename(source, target)?;
        return Ok(());
    }
    match publish(source, target) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => Err(error.into()),
        // create_new still enforces no-clobber if another downloader wins the
        // race. The source remains available until the caller removes staging.
        Err(_) => copy_exclusive(File::open(source)?, target, cancelled),
    }
}

fn copy_exclusive(mut source: impl Read, target: &Path, cancelled: &AtomicBool) -> Result<()> {
    check_cancelled(cancelled)?;
    let mut output = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(target)?;
    let result = (|| {
        let mut buffer = vec![0; 1024 * 1024];
        loop {
            check_cancelled(cancelled)?;
            let count = source.read(&mut buffer)?;
            if count == 0 {
                break;
            }
            output.write_all(&buffer[..count])?;
        }
        output.sync_all()?;
        check_cancelled(cancelled)
    })();
    // Close before cleanup, including on Windows. Never leave a partial final
    // file after an I/O error or cancellation during the fallback copy.
    drop(output);
    if result.is_err() {
        let _ = fs::remove_file(target);
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{self, Cursor};

    fn unsupported(_: &Path, _: &Path) -> io::Result<()> {
        Err(io::Error::from(io::ErrorKind::Unsupported))
    }

    #[test]
    fn unsupported_hard_links_copy_all_bytes_without_overwriting_existing_files() {
        let root = tempfile::tempdir().unwrap();
        let source = root.path().join("staged.flac");
        let target = root.path().join("saved.flac");
        let bytes = vec![42; 2 * 1024 * 1024 + 17];
        fs::write(&source, &bytes).unwrap();
        let cancelled = AtomicBool::new(false);
        commit_with(&source, &target, false, &cancelled, unsupported).unwrap();
        assert_eq!(fs::read(&source).unwrap(), bytes);
        assert_eq!(fs::read(&target).unwrap(), bytes);
        fs::write(&source, b"replacement").unwrap();
        assert!(commit_with(&source, &target, false, &cancelled, unsupported).is_err());
        assert_eq!(fs::read(&target).unwrap(), bytes);
        commit(&source, &target, true, &cancelled).unwrap();
        assert_eq!(fs::read(&target).unwrap(), b"replacement");
    }

    #[test]
    fn cancellation_and_read_errors_remove_the_partial_copy() {
        struct CancelAfterRead<'a> {
            cancelled: &'a AtomicBool,
        }
        impl Read for CancelAfterRead<'_> {
            fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
                buffer[0] = 42;
                self.cancelled.store(true, Ordering::Relaxed);
                Ok(1)
            }
        }
        let root = tempfile::tempdir().unwrap();
        let target = root.path().join("saved.flac");
        let cancelled = AtomicBool::new(false);
        let error = copy_exclusive(
            CancelAfterRead {
                cancelled: &cancelled,
            },
            &target,
            &cancelled,
        )
        .unwrap_err();
        assert_eq!(error.code, "cancelled");
        assert!(!target.exists());
        cancelled.store(false, Ordering::Relaxed);
        let reader = Cursor::new(b"partial").chain(io::repeat(0).take(1));
        struct Failing<R>(R);
        impl<R: Read> Read for Failing<R> {
            fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
                let count = self.0.read(buffer)?;
                if count == 0 {
                    Err(io::Error::other("disk read failed"))
                } else {
                    Ok(count)
                }
            }
        }
        assert!(copy_exclusive(Failing(reader), &target, &cancelled).is_err());
        assert!(!target.exists());
    }
}
