use super::*;
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};

struct TempDir(PathBuf);

impl TempDir {
    fn new() -> Self {
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        let path = std::env::temp_dir().join(format!(
            "pyferrite-detect-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir(&path).unwrap();
        TempDir(path)
    }

    fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn zip_header(name: &[u8]) -> Vec<u8> {
    let mut bytes = vec![0; 30];
    bytes[..4].copy_from_slice(b"PK\x03\x04");
    bytes[26..28].copy_from_slice(&(name.len() as u16).to_le_bytes());
    bytes.extend(name);
    bytes
}

#[test]
fn byte_sniffer_recognizes_magic_and_zip_member_types() {
    assert_eq!(detect_from_bytes(b"\x93NUMPY\x01"), Some(Format::Npy));
    assert_eq!(detect_from_bytes(b"\x89HDF\r\n\x1a\n"), Some(Format::Hdf5));
    assert_eq!(detect_from_bytes(b"PK\x03\x04"), Some(Format::Npz));
    assert_eq!(detect_from_bytes(&zip_header(b"weights.npy")), Some(Format::Npz));
    assert_eq!(detect_from_bytes(&zip_header(b"archive/data.pkl")), Some(Format::Pt));
    assert_eq!(detect_from_bytes(&zip_header(b"other.bin")), Some(Format::Pt));

    let mut truncated_name = zip_header(b"x");
    truncated_name[26..28].copy_from_slice(&100u16.to_le_bytes());
    assert_eq!(detect_from_bytes(&truncated_name), Some(Format::Pt));
}

#[test]
fn byte_sniffer_recognizes_compression_and_pickle_openers() {
    assert_eq!(detect_from_bytes(b"ZFpayload"), Some(Format::Joblib));
    assert_eq!(detect_from_bytes(b"\x28\xb5\x2f\xfdpayload"), Some(Format::Joblib));
    assert_eq!(detect_from_bytes(b"\x80\x05pickle"), Some(Format::Pickle));
    assert_eq!(detect_from_bytes(b"(protocol-zero"), Some(Format::Pickle));
    assert_eq!(detect_from_bytes(b"]protocol-zero"), Some(Format::Pickle));
    assert_eq!(detect_from_bytes(b"unknown"), None);
    assert_eq!(detect_from_bytes(b""), None);
}

#[test]
fn path_sniffer_prefers_unambiguous_magic_then_extension_for_pickle() {
    let dir = TempDir::new();
    let npy = dir.path().join("renamed.bin");
    std::fs::write(&npy, b"\x93NUMPY\x01").unwrap();
    assert_eq!(detect_from_path(&npy).unwrap(), Format::Npy);

    let joblib = dir.path().join("model.joblib");
    std::fs::write(&joblib, b"\x80\x05pickle").unwrap();
    assert_eq!(detect_from_path(&joblib).unwrap(), Format::Joblib);

    let pickle = dir.path().join("model.pickle");
    std::fs::write(&pickle, b"\x80\x05pickle").unwrap();
    assert_eq!(detect_from_path(&pickle).unwrap(), Format::Pickle);

    let pickle_without_known_extension = dir.path().join("model.unknown");
    std::fs::write(&pickle_without_known_extension, b"\x80\x05pickle").unwrap();
    assert_eq!(detect_from_path(&pickle_without_known_extension).unwrap(), Format::Pickle);

    let extension_only = dir.path().join("array.NPY");
    std::fs::write(&extension_only, b"unknown").unwrap();
    assert_eq!(detect_from_path(&extension_only).unwrap(), Format::Npy);
}

#[test]
fn path_sniffer_reports_open_and_unknown_format_errors() {
    let dir = TempDir::new();
    assert!(detect_from_path(&dir.path().join("missing.npy")).is_err());

    let unknown = dir.path().join("file.unknown");
    std::fs::write(&unknown, b"unknown").unwrap();
    assert!(detect_from_path(&unknown)
        .unwrap_err()
        .to_string()
        .contains("cannot determine format"));
}
