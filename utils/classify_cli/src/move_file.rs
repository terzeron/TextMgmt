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
