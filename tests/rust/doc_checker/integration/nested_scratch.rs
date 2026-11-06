//! 중첩 검사의 임시 디렉터리 실패 통합 테스트. 이름 번호(전역 카운터)와 `TMPDIR`은 프로세스 전체가 공유하므로
//! 이 파일에는 테스트를 하나만 두고 단계를 차례로 실행한다.

use std::fs;
use std::io::{self, Write};

use doc_checker::nested_scanner::{ScratchDir, begin, scan_zip_members};
use doc_checker::{CheckOptions, FileReport, Format};

#[test]
fn scratch_directory_failures_end_nested_scanning_with_a_reason() {
    // 1) 처음 100개 이름이 모두 있으면 빈 이름을 찾지 못한다.
    let base = std::env::temp_dir();
    let pid = std::process::id();
    let taken: Vec<_> = (0..100).map(|n| base.join(format!("bookdoctor-{pid}-{n}"))).collect();
    for dir in &taken {
        fs::create_dir(dir).unwrap();
    }
    let error = ScratchDir::new().err().expect("빈 이름이 없다");
    assert_eq!(error.kind(), io::ErrorKind::AlreadyExists);
    for dir in &taken {
        fs::remove_dir(dir).unwrap();
    }

    // 2) 임시 디렉터리를 둘 곳이 없으면 만들기 오류가 그대로 올라온다.
    // SAFETY: 이 프로세스에는 이 테스트 하나뿐이라 환경 변수를 동시에 읽는 스레드가 없다.
    unsafe { std::env::set_var("TMPDIR", "/nonexistent/tmpdir") };
    assert_eq!(ScratchDir::new().err().expect("만들 수 없다").kind(), io::ErrorKind::NotFound);

    // 3) 중첩 멤버를 풀어 검사하려는데 임시 디렉터리가 없으면 사유를 남기고 멈춘다.
    let dir = tempfile::tempdir_in("/dev/shm").unwrap();
    let path = dir.path().join("outer.zip");
    let mut zip = zip::ZipWriter::new(fs::File::create(&path).unwrap());
    zip.start_file("inner.zip", zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored)).unwrap();
    zip.write_all(b"PK\x03\x04 not a real archive").unwrap();
    zip.finish().unwrap();
    let opts = CheckOptions::default();
    let _budget = begin(&opts);
    let mut report = FileReport::new(&path, Some(Format::Zip));

    scan_zip_members(&path, &opts, &mut report);

    assert!(report.incomplete && report.reasons.iter().any(|r| r == "recursion skipped: scratch directory could not be created"), "{:?}", report.reasons);
}
