use std::{fs, io, path::Path};

/// 목적지를 덮어쓰지 않고 옮긴다. 같은 filesystem에서는 inode와 시각을 보존한다.
pub(crate) fn move_file(source: &Path, destination: &Path) -> io::Result<()> {
    match fs::hard_link(source, destination) {
        Ok(()) => {}
        Err(e) if e.kind() == io::ErrorKind::CrossesDevices => {
            let mut input = fs::File::open(source)?;
            let metadata = input.metadata()?;
            let mut output = fs::OpenOptions::new().write(true).create_new(true).open(destination)?;
            let copy = (|| {
                io::copy(&mut input, &mut output)?;
                output.set_permissions(metadata.permissions())?;
                output.set_times(fs::FileTimes::new().set_modified(metadata.modified()?).set_accessed(metadata.accessed()?))?;
                output.sync_all()
            })();
            if let Err(e) = copy {
                let _ = fs::remove_file(destination);
                return Err(e);
            }
        }
        Err(e) => return Err(e),
    }
    if let Err(e) = fs::remove_file(source) {
        let _ = fs::remove_file(destination);
        return Err(e);
    }
    Ok(())
}

#[cfg(all(test, target_os = "linux"))]
mod tests {
    use super::move_file;

    #[test]
    fn copies_and_preserves_metadata_when_moving_across_filesystems() {
        use std::os::unix::fs::MetadataExt;

        let source_dir = tempfile::tempdir().unwrap();
        let destination_dir = tempfile::tempdir_in("/dev/shm").unwrap();
        if std::fs::metadata(source_dir.path()).unwrap().dev() == std::fs::metadata(destination_dir.path()).unwrap().dev() {
            return;
        }
        let source = source_dir.path().join("source.bin");
        let destination = destination_dir.path().join("destination.bin");
        std::fs::write(&source, b"cross-device payload").unwrap();
        let modified = std::fs::metadata(&source).unwrap().modified().unwrap();

        move_file(&source, &destination).unwrap();

        assert!(!source.exists());
        assert_eq!(std::fs::read(&destination).unwrap(), b"cross-device payload");
        assert_eq!(std::fs::metadata(&destination).unwrap().modified().unwrap(), modified);
    }

    #[test]
    fn refuses_existing_destination_without_removing_source_or_destination() {
        let dir = tempfile::tempdir().unwrap();
        let source = dir.path().join("source.bin");
        let destination = dir.path().join("destination.bin");
        std::fs::write(&source, b"source").unwrap();
        std::fs::write(&destination, b"destination").unwrap();

        let error = move_file(&source, &destination).unwrap_err();

        assert_eq!(error.kind(), std::io::ErrorKind::AlreadyExists);
        assert_eq!(std::fs::read(&source).unwrap(), b"source");
        assert_eq!(std::fs::read(&destination).unwrap(), b"destination");
    }
}
