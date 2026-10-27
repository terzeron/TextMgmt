//! 통계 카운터 테스트 (tests/test_stat.py 이식).

use std::sync::Arc;
use std::thread;

use stat::{Stat, StatError};

fn printed(s: &Stat) -> String {
    let mut out = Vec::new();
    s.print(&mut out).unwrap();
    String::from_utf8(out).unwrap()
}

#[test]
fn divide_handles_zero() {
    assert_eq!(Stat::divide(10.0, 2.0), 5.0);
    assert_eq!(Stat::divide(10.0, 0.0), 0.0);
}

#[test]
fn print_lists_every_section_with_python_formats() {
    let s = Stat::new();
    s.set_value("text_count", 1.0).unwrap();
    s.set_value("text_total_time", 0.5).unwrap();
    s.set_value("pdf_count", 2.0).unwrap();
    s.set_value("pdf_total_time", 1.0).unwrap();
    s.set_value("index_count", 3.0).unwrap();
    s.set_value("index_total_time", 1.5).unwrap();
    let out = printed(&s);
    assert!(out.starts_with("[Stat]\n"));
    assert!(out.contains("text:    0000001 / 0.5000\n"), "{out}");
    assert!(out.contains("pdf:     0000002 / 0.5000\n"), "{out}");
    assert!(out.contains("total:   0000003 / 0.5000\n"), "{out}");
    for label in ["epub:    0000000 / 0.0000", "epub(z): 0000000 / 0.0000", "html:    ", "docx:    ", "doc:     ", "hwp:     ", "rtf:     ", "image:   "] {
        assert!(out.contains(label), "{label}: {out}");
    }
    // 재인코딩이 0건이면 그 줄은 없다.
    assert!(!out.contains("재인코딩"));
}

#[test]
fn print_shows_the_reencoded_count_only_when_nonzero() {
    let s = Stat::new();
    s.add("text_reencoded_count", 4.0).unwrap();
    assert!(printed(&s).contains("  └ UTF-8 재인코딩: 4건\n"));
}

#[test]
fn pdf_stages_are_sorted_and_count_outcomes() {
    let s = Stat::new();
    s.record_pdf_stage("pdfium", 0.5, "ok");
    s.record_pdf_stage("pdfium", 1.5, "timeout");
    s.record_pdf_stage("isbn_pdfium", 0.25, "empty");
    s.record_pdf_stage("pypdf", 0.0, "damaged");
    s.record_pdf_stage("pypdf", 0.0, "error");
    let out = printed(&s);
    let lines: Vec<&str> = out.lines().filter(|l| l.starts_with("  - ")).collect();
    assert_eq!(lines.len(), 3);
    assert!(lines[0].starts_with("  - isbn_pdfium"), "{lines:?}");
    assert!(lines[1].starts_with("  - pdfium"));
    assert!(lines[2].starts_with("  - pypdf"));
    assert_eq!(lines[1], format!("  - {:<24} 0000002 / 1.0000 / ok=1 empty=0 damaged=0 timeout=1 error=0", "pdfium"));
    assert!(lines[2].ends_with("ok=0 empty=0 damaged=1 timeout=0 error=1"));
}

#[test]
fn add_and_set_value_work_by_python_attribute_name() {
    let s = Stat::new();
    s.add("text_count", 1.0).unwrap();
    s.add("text_count", 2.0).unwrap();
    assert_eq!(s.get("text_count").unwrap(), 3.0);
    s.set_value("text_count", 0.0).unwrap();
    assert_eq!(s.get("text_count").unwrap(), 0.0);
    assert_eq!(s.get("html_total_time").unwrap(), 0.0);
}

#[test]
fn add_requires_an_existing_counter_attribute() {
    let s = Stat::new();
    assert_eq!(s.add("nope", 1.0), Err(StatError::UnknownField("nope".into())));
    assert!(s.get("nope").is_err());
    assert_eq!(StatError::UnknownField("x".into()).to_string(), "알 수 없는 통계 항목: x");
}

#[test]
fn set_value_creates_python_style_custom_counter_attributes() {
    let s = Stat::new();
    s.set_value("custom_metric", 2.5).unwrap();
    assert_eq!(s.get("custom_metric").unwrap(), 2.5);
    s.add("custom_metric", 1.5).unwrap();
    assert_eq!(s.get("custom_metric").unwrap(), 4.0);
}

#[test]
fn add_is_thread_safe() {
    let s = Arc::new(Stat::new());
    let handles: Vec<_> = (0..2)
        .map(|_| {
            let s = Arc::clone(&s);
            thread::spawn(move || {
                for _ in 0..1000 {
                    s.add("text_count", 1.0).unwrap();
                }
            })
        })
        .collect();
    for h in handles {
        h.join().unwrap();
    }
    assert_eq!(s.get("text_count").unwrap(), 2000.0);
}

#[test]
fn the_global_instance_is_shared_like_the_python_class_attributes() {
    let before = Stat::global().get("image_count").unwrap();
    Stat::global().add("image_count", 1.0).unwrap();
    assert_eq!(Stat::global().get("image_count").unwrap(), before + 1.0);
}

#[test]
fn print_to_a_failing_writer_reports_the_error() {
    struct Broken;
    impl std::io::Write for Broken {
        fn write(&mut self, _: &[u8]) -> std::io::Result<usize> {
            Err(std::io::Error::other("closed"))
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
    assert!(Stat::new().print(&mut Broken).is_err());
}

#[test]
fn default_is_an_empty_counter_set() {
    assert_eq!(Stat::default().get("text_count").unwrap(), 0.0);
}

#[test]
fn a_writer_that_fails_midway_reports_the_error_from_every_section() {
    // 출력 길이를 하나씩 늘려 가며 쓰다 실패시켜, 어느 줄에서 실패해도 오류가 올라오는지 본다.
    struct FailAfter(usize);
    impl std::io::Write for FailAfter {
        fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
            if self.0 < buf.len() {
                return Err(std::io::Error::other("full"));
            }
            self.0 -= buf.len();
            Ok(buf.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
    let s = Stat::new();
    s.add("text_reencoded_count", 1.0).unwrap();
    s.record_pdf_stage("pdfium", 0.1, "ok");
    let total = printed(&s).len();
    for limit in (0..total).step_by(7) {
        assert!(s.print(&mut FailAfter(limit)).is_err(), "{limit}");
    }
    assert!(s.print(&mut FailAfter(total)).is_ok());
}
