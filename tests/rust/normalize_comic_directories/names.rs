//! 이름 해석 함수 테스트. 기대값은 Python 원본(`utils/normalize_comic_directories.py`)이 만든 `cases.json`이다.

use std::fs;
use std::path::Path;

use normalize_comic_directories::names::*;
use serde_json::Value;

fn cases() -> Value {
    serde_json::from_str(include_str!("cases.json")).unwrap()
}

fn s(v: &Value) -> &str {
    v.as_str().unwrap()
}

fn opt(v: &Value) -> Option<&str> {
    v.as_str()
}

#[test]
fn parse_filename_matches_python() {
    for case in cases()["parse_filename"].as_array().unwrap() {
        let expected = case[1].as_array().map(|a| (s(&a[0]).to_owned(), opt(&a[1]).map(str::to_owned)));
        assert_eq!(parse_filename(Path::new(s(&case[0]))), expected, "{}", case[0]);
    }
}

#[test]
fn directory_title_and_author_match_python() {
    let c = cases();
    for case in c["directory_title"].as_array().unwrap() {
        assert_eq!(directory_title(s(&case[0])), s(&case[1]), "{}", case[0]);
    }
    for case in c["directory_author"].as_array().unwrap() {
        assert_eq!(directory_author(s(&case[0])).as_deref(), opt(&case[1]), "{}", case[0]);
    }
}

#[test]
fn korean_author_matches_python() {
    for case in cases()["korean_author"].as_array().unwrap() {
        assert_eq!(korean_author(opt(&case[0])).as_deref(), opt(&case[1]), "{}", case[0]);
    }
}

#[test]
fn title_keys_bases_variants_and_safe_names_match_python() {
    let c = cases();
    for case in c["title_key"].as_array().unwrap() {
        assert_eq!(title_key(s(&case[0])), s(&case[1]), "{}", case[0]);
    }
    for case in c["base"].as_array().unwrap() {
        assert_eq!(bookstore_title_base(s(&case[0])), s(&case[1]), "{}", case[0]);
    }
    for case in c["variants"].as_array().unwrap() {
        let mut got: Vec<_> = title_variants(s(&case[0])).into_iter().collect();
        got.sort();
        let expected: Vec<_> = case[1].as_array().unwrap().iter().map(|v| s(v).to_owned()).collect();
        assert_eq!(got, expected, "{}", case[0]);
    }
    for case in c["safe"].as_array().unwrap() {
        assert_eq!(filesystem_safe_name(s(&case[0])).as_deref(), opt(&case[1]), "{}", case[0]);
    }
    for case in c["equiv"].as_array().unwrap() {
        assert_eq!(titles_equivalent(s(&case[0]), s(&case[1])), case[2].as_bool().unwrap(), "{} {}", case[0], case[1]);
    }
    for case in c["matches"].as_array().unwrap() {
        assert_eq!(bookstore_title_matches(s(&case[0]), s(&case[1])), case[2].as_bool().unwrap(), "{} {}", case[0], case[1]);
    }
}

fn make_dir(root: &Path, name: &str, entries: &[Value]) -> std::path::PathBuf {
    let dir = root.join(name);
    fs::create_dir(&dir).unwrap();
    for e in entries {
        let e = s(e);
        if e.contains('.') {
            fs::write(dir.join(e), b"x").unwrap();
        } else {
            fs::create_dir(dir.join(e)).unwrap();
        }
    }
    dir
}

#[test]
fn normalized_directory_metadata_matches_python() {
    for case in cases()["metadata"].as_array().unwrap() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = make_dir(tmp.path(), s(&case[0]), case[1].as_array().unwrap());
        assert_eq!(normalized_directory_metadata(&dir).as_deref(), opt(&case[2]), "{} {}", case[0], case[1]);
    }
}

#[test]
fn infer_identity_matches_python() {
    for case in cases()["identity"].as_array().unwrap() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = make_dir(tmp.path(), s(&case[0]), case[1].as_array().unwrap());
        let got = infer_identity(&dir);
        let e = case[2].as_array().unwrap();
        assert_eq!((got.title.as_deref(), got.author.as_deref()), (opt(&e[0]), opt(&e[1])), "{} {}", case[0], case[1]);
        assert!((got.confidence - e[2].as_f64().unwrap()).abs() < 1e-5, "{} {}: {}", case[0], case[1], got.confidence);
        assert_eq!(got.file_count as i64, e[3].as_i64().unwrap());
        assert_eq!(got.failure_status.as_deref(), opt(&e[4]), "{} {}", case[0], case[1]);
        assert_eq!(got.failure_reason, s(&e[5]));
    }
}

#[test]
fn infer_identity_ignores_symlinks_and_other_extensions() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path().join("제목");
    fs::create_dir(&dir).unwrap();
    fs::write(dir.join("제목 1권.pdf"), b"x").unwrap();
    fs::write(dir.join("제목 2권.txt"), b"x").unwrap();
    std::os::unix::fs::symlink(dir.join("제목 1권.pdf"), dir.join("제목 3권.pdf")).unwrap();
    let got = infer_identity(&dir);
    assert_eq!(got.failure_status.as_deref(), Some("SKIP_INSUFFICIENT_FILES"));
    assert_eq!(got.file_count, 1);
}

#[test]
fn local_author_references_and_matching() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    for dir in ["5_완결/[홍길동] 원피스 1-10권", "5_완결/[만화] 무시 1-2권", "5_완결/[Hong] 영문 1-3권", "5_완결/[김철수] 나루토 완결", "2_미완/[이몽룡] 나루토 외전 1-2권", "2_미완/제목만"]
    {
        fs::create_dir_all(root.join(dir)).unwrap();
    }
    fs::write(root.join("5_완결/파일.txt"), b"x").unwrap();
    std::os::unix::fs::symlink(root.join("5_완결"), root.join("link")).unwrap();
    let refs = local_author_references(root);
    assert_eq!(matching_local_author("원피스", &refs).as_deref(), Some("홍길동"));
    assert_eq!(matching_local_author("원피", &refs).as_deref(), Some("홍길동"), "접두 일치");
    assert_eq!(matching_local_author("나루토", &refs).as_deref(), Some("김철수"), "정확히 일치하는 저자가 하나면 접두 후보는 보지 않는다");
    assert_eq!(matching_local_author("나루", &refs), None, "접두 후보의 저자가 둘이면 모호하다");
    assert_eq!(matching_local_author("없는책", &refs), None);
    assert_eq!(matching_local_author("a", &refs), None, "한 글자 키는 접두 일치를 쓰지 않는다");
}

#[test]
fn matched_numbers_and_existing_metadata() {
    assert_eq!(existing_metadata("제목 (1-5권 완결)"), "(1-5권 완결)");
    assert_eq!(existing_metadata("제목"), "");
    assert_eq!(existing_metadata("제목 완결 외전"), "완결 외전");
}
