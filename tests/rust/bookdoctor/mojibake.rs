//! bookdoctor 인코딩 손상 판정 테스트(계획 Task 13). `classify_text` 관련 테스트는
//! 옛 인코딩 검사 도구 테스트에서 옮기고 단언을 `Verdict`로 바꿨다.
//! Poppler는 `FakeRunner`로 대신하므로 poppler가 없어도 통과한다.

mod common;

use std::fs;

use bookdoctor::cli::Options;
use bookdoctor::mojibake::{self, MIN_HANGUL_RATIO, MOJIBAKE_LIKENESS_THRESHOLD, TextSource, Verdict, classify_text};
use bookdoctor::pdf;
use bookdoctor::scan::scan_file;
use common::*;
use corpus_common::reader::{TEXT_HEAD_CHARS, TEXT_TAIL_CHARS, korean_likeness, read_document_text};
use scan_common::{Category, FileReport, Severity};

fn hanja() -> String {
    HANJA_CLASSIC_UNIT.repeat(20)
}
fn english() -> String {
    ENGLISH_UNIT.repeat(20)
}
fn word_list() -> String {
    KOREAN_WORD_LIST_UNIT.repeat(20)
}

fn opts() -> Options {
    Options { no_poppler: true, ..Options::default() }
}

fn find<'a>(r: &'a FileReport, code: &str) -> Option<&'a scan_common::Finding> {
    r.findings.iter().find(|f| f.code == code)
}

// ---- classify_text (detect.rs에서 이식) ----

#[test]
fn thresholds_keep_the_old_values() {
    assert_eq!((MOJIBAKE_LIKENESS_THRESHOLD, MIN_HANGUL_RATIO), (0.10, 0.10));
}

#[test]
fn classify_text_table() {
    let cases: Vec<(String, Verdict)> = vec![
        (clean_korean(), Verdict::Clean),
        (corrupted(), Verdict::Corrupted),
        (hanja(), Verdict::Undetermined),
        (english(), Verdict::Undetermined),
        (word_list(), Verdict::Clean),
        (String::new(), Verdict::Unreadable),
        ("짧은 글".to_string(), Verdict::Unreadable),
    ];
    for (text, expected) in cases {
        assert_eq!(classify_text(&text).0, expected, "{}", text.chars().take(20).collect::<String>());
    }
}

#[test]
fn hanja_heavy_document_is_never_called_corrupted() {
    assert_eq!(classify_text(&hanja()), (Verdict::Undetermined, None));
}

#[test]
fn classify_returns_likeness_for_decided_verdicts() {
    for text in [clean_korean(), corrupted()] {
        let l = classify_text(&text).1.unwrap();
        assert!((0.0..=1.0).contains(&l));
    }
}

#[test]
fn english_document_with_hangul_noise_is_undetermined() {
    // 한글 비율이 하한(0.10)에 못 미치면 likeness를 잴 수 있어도 판정하지 않는다.
    let text = format!("{}{}", ENGLISH_UNIT.repeat(20).repeat(12), "뷁뾃쎯퉳깗".repeat(12));
    assert!(korean_likeness(&text).is_some());
    assert_eq!(classify_text(&text).0, Verdict::Undetermined);
}

#[test]
fn classify_text_is_undetermined_when_hangul_is_too_sparse_to_measure() {
    let text = format!("{}{}", "가나다라마바사아자차".repeat(4), "x".repeat(300));
    assert!(text.chars().count() >= 200);
    assert_eq!(classify_text(&text), (Verdict::Undetermined, None));
}

#[test]
fn length_boundary_is_200_chars() {
    assert_eq!(classify_text(&"가".repeat(199)).0, Verdict::Unreadable);
    assert_ne!(classify_text(&"가".repeat(200)).0, Verdict::Unreadable);
}

#[test]
fn likeness_threshold_boundary_is_exclusive() {
    // 음절 32개 중 1개가 흔한 음절이면 0.03125로 손상이다. 흔한 음절만이면 정상이다.
    let low = format!("{}{}", "이", "쀍".repeat(31)).repeat(8);
    assert_eq!(classify_text(&low), (Verdict::Corrupted, Some(0.03125)));
    assert_eq!(classify_text(&"이".repeat(300)).0, Verdict::Clean);
}

#[test]
fn check_text_rounds_likeness_to_four_places_half_even() {
    // Python round(0.03125, 4)는 0.0312다. 원본 inspect_file 테스트(detect.rs 142행)와 같은 기대값이다.
    let mut r = FileReport::new(std::path::Path::new("t.txt"), None);
    mojibake::check_text(&mut r, &format!("{}{}", "이", "쀍".repeat(31)).repeat(8));
    assert_eq!(r.likeness, Some(0.0312));
    assert_eq!(find(&r, "MOJIBAKE").unwrap().detail, "likeness=0.0312");
}

#[test]
fn check_text_reports_only_corrupted_verdicts() {
    for text in [clean_korean(), hanja(), english(), "짧은 글".to_string()] {
        let mut r = FileReport::new(std::path::Path::new("t.txt"), None);
        mojibake::check_text(&mut r, &text);
        assert!(r.findings.is_empty() && r.likeness.is_none() && !r.incomplete, "{r:?}");
    }
}

// ---- TXT ----

#[test]
fn txt_with_cp949_misread_as_utf8_is_flagged_mojibake() {
    // UTF-8 BOM(EF BB BF)을 CP949로 읽으면 `癤`이 된다. 본문도 같은 방식으로 깨뜨려 UTF-8로 다시 저장한다.
    let mut utf8 = vec![0xEF, 0xBB, 0xBF];
    utf8.extend(clean_korean().repeat(4).into_bytes());
    // `decode`는 BOM을 먼저 떼어 버리므로 BOM을 읽는 `decode_without_bom_handling`을 쓴다.
    let (garbled, _) = encoding_rs::EUC_KR.decode_without_bom_handling(&utf8);
    let garbled: String = garbled.chars().filter(|&c| c != '\u{FFFD}').collect();
    assert!(garbled.starts_with('癤'), "{:?}", garbled.chars().take(4).collect::<Vec<_>>());
    let f = file_with("m.txt", garbled.as_bytes());
    let r = scan_file(f.path(), &opts());
    let d = find(&r, "MOJIBAKE").unwrap_or_else(|| panic!("MOJIBAKE 없음: {r:?}"));
    assert_eq!((d.category, d.severity), (Category::Defect, Severity::High));
    assert!(r.likeness.unwrap() < 0.10, "{:?}", r.likeness);
    assert!(d.detail.starts_with("likeness="));
}

#[test]
fn clean_korean_txt_has_no_mojibake() {
    let f = file_with("c.txt", clean_korean().repeat(4).as_bytes());
    let r = scan_file(f.path(), &opts());
    assert!(find(&r, "MOJIBAKE").is_none() && r.likeness.is_none() && !r.incomplete, "{r:?}");
}

#[test]
fn large_corrupted_txt_is_flagged_from_head_and_tail_samples() {
    // 60000바이트를 넘으면 앞·뒤 표본을 따로 읽어 합친다. 전체가 깨진 큰 파일도 잡혀야 한다.
    let f = file_with("big.txt", corrupted().repeat(60).as_bytes());
    assert!(fs::metadata(f.path()).unwrap().len() > 60_000);
    let r = scan_file(f.path(), &opts());
    assert!(find(&r, "MOJIBAKE").is_some(), "{r:?}");
}

#[test]
fn cp949_txt_is_not_utf8_but_not_mojibake() {
    // 원본은 decode_best로 CP949를 정상 한글로 읽어 clean이다. 새 도구도 MOJIBAKE를 내지 않고 NOT_UTF8만 낸다.
    let text = clean_korean().repeat(4);
    let (bytes, _, _) = encoding_rs::EUC_KR.encode(&text);
    let f = file_with("k.txt", &bytes);
    let r = scan_file(f.path(), &opts());
    assert!(find(&r, "NOT_UTF8").is_some());
    assert!(find(&r, "MOJIBAKE").is_none(), "{r:?}");
}

#[test]
fn txt_with_a_stray_invalid_byte_keeps_the_old_verdict() {
    // UTF-8이 아닌 파일은 `decode_best`가 가장 한국어다운 디코딩을 고른다. 새 도구는 같은 읽기 함수를 쓰므로
    // 옛 판정(`classify_text(read_document_text(..))`)과 MOJIBAKE 여부가 항상 같다.
    let mut bytes = corrupted().into_bytes();
    bytes.insert(bytes.len() / 2, 0xFF);
    let f = file_with("s.txt", &bytes);
    let r = scan_file(f.path(), &opts());
    assert!(find(&r, "INVALID_UTF8").is_some());
    let old = classify_text(&read_document_text(f.path(), TEXT_HEAD_CHARS, TEXT_TAIL_CHARS)).0;
    assert_eq!(find(&r, "MOJIBAKE").is_some(), old == Verdict::Corrupted, "{r:?}");
}

// ---- EPUB ----

fn epub_report(chapter: &str) -> FileReport {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("b.epub");
    epub_with_chapter(&path, chapter);
    scan_file(&path, &opts())
}

#[test]
fn epub_mojibake_is_flagged() {
    let r = epub_report(&xhtml(&format!("<p>{}</p>", corrupted())));
    assert!(find(&r, "MOJIBAKE").is_some(), "{r:?}");
    assert!(r.likeness.unwrap() < 0.10);
}

#[test]
fn clean_epub_text_has_no_mojibake() {
    let r = epub_report(&xhtml(&format!("<p>{}</p>", clean_korean().repeat(4))));
    assert!(find(&r, "MOJIBAKE").is_none() && !r.reasons.iter().any(|x| x == "no text extracted"), "{r:?}");
}

#[test]
fn no_text_extracted_is_incomplete() {
    // 본문이 50자 미만인 문서는 표본에서 빠져 추출된 텍스트가 없다.
    let r = epub_report(&xhtml("<p>본문</p>"));
    assert!(r.incomplete && r.reasons.iter().any(|x| x == "no text extracted"), "{r:?}");
    assert!(find(&r, "MOJIBAKE").is_none());
}

#[test]
fn short_text_is_neither_a_finding_nor_incomplete() {
    // 50자 이상 200자 미만이면 추출은 됐으나 판독 불가다. 발견도 incomplete도 아니다.
    let r = epub_report(&xhtml(&format!("<p>{}</p>", "소림사 장로가 강호에서 내공을 운기조식하며 화산파 검법을 수련했다. ".repeat(3))));
    assert!(find(&r, "MOJIBAKE").is_none() && !r.reasons.iter().any(|x| x == "no text extracted"), "{r:?}");
}

#[test]
fn non_zip_epub_skips_the_text_check() {
    let f = file_with("n.epub", b"not a zip at all");
    let r = scan_file(f.path(), &opts());
    assert!(r.has("NOT_ZIP"));
    assert!(!r.reasons.iter().any(|x| x == "no text extracted"), "{r:?}");
}

// ---- PDF ----

fn tiny_pdf() -> Vec<u8> {
    let bodies = ["<< /Type /Catalog /Pages 2 0 R >>", "<< /Type /Pages /Kids [3 0 R] /Count 1 >>", "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] >>"];
    let mut out = b"%PDF-1.7\n".to_vec();
    let mut offsets = Vec::new();
    for (i, body) in bodies.iter().enumerate() {
        offsets.push(out.len());
        out.extend(format!("{} 0 obj\n{body}\nendobj\n", i + 1).bytes());
    }
    let xref = out.len();
    out.extend(format!("xref\n0 {}\n0000000000 65535 f \n", bodies.len() + 1).bytes());
    for o in offsets {
        out.extend(format!("{o:010} 00000 n \n").bytes());
    }
    out.extend(format!("trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n", bodies.len() + 1).bytes());
    out
}

fn pdf_report(opts: &Options, runner: &FakeRunner) -> FileReport {
    let f = file_with("t.pdf", &tiny_pdf());
    let mut r = FileReport::new(f.path(), None);
    pdf::scan_pdf_file_with(f.path(), opts, &mut r, runner);
    r
}

fn text_calls(runner: &FakeRunner) -> usize {
    runner.programs().iter().filter(|p| *p == "pdftotext").count()
}

#[test]
fn pdf_text_is_not_extracted_without_the_flag() {
    let runner = FakeRunner::text(&corrupted());
    let r = pdf_report(&Options::default(), &runner);
    assert_eq!(text_calls(&runner), 0, "{:?}", runner.programs());
    assert!(find(&r, "MOJIBAKE").is_none(), "{r:?}");
}

#[test]
fn pdf_text_mojibake_is_flagged_with_the_flag() {
    let on = Options { pdf_text: true, ..Options::default() };
    let r = pdf_report(&on, &FakeRunner::text(&corrupted()));
    assert!(find(&r, "MOJIBAKE").is_some(), "{r:?}");
    assert!(r.likeness.unwrap() < 0.10);

    let r = pdf_report(&on, &FakeRunner::text(&clean_korean().repeat(4)));
    assert!(find(&r, "MOJIBAKE").is_none() && r.likeness.is_none(), "{r:?}");
}

#[test]
fn pdf_text_samples_first_and_last_five_pages() {
    let on = Options { pdf_text: true, ..Options::default() };
    let short = FakeRunner::pages_text(8, &corrupted());
    let r = pdf_report(&on, &short);
    assert!(find(&r, "MOJIBAKE").is_some());
    // 8쪽이면 1-5쪽과 6-8쪽(겹치지 않게)이다. pdfinfo 1회 + pdftotext 2회.
    assert_eq!(text_calls(&short), 2);

    let one = FakeRunner::pages_text(3, &corrupted());
    pdf_report(&on, &one);
    assert_eq!(text_calls(&one), 1);
}

#[test]
fn pdf_text_with_poppler_disabled_is_incomplete_only_when_requested() {
    let on = Options { pdf_text: true, no_poppler: true, ..Options::default() };
    let r = pdf_report(&on, &FakeRunner::text(&corrupted()));
    assert!(r.incomplete && r.reasons.iter().any(|x| x.contains("mojibake")), "{r:?}");
    let r = pdf_report(&opts(), &FakeRunner::text(&corrupted()));
    assert!(!r.reasons.iter().any(|x| x.contains("mojibake")), "{r:?}");
}

#[test]
fn pdf_text_runner_failure_is_incomplete_not_a_finding() {
    let on = Options { pdf_text: true, ..Options::default() };
    let r = pdf_report(&on, &FakeRunner::timeout());
    assert!(r.incomplete && find(&r, "MOJIBAKE").is_none(), "{r:?}");
}

#[test]
fn check_file_on_missing_path_is_incomplete() {
    let dir = tempfile::tempdir().unwrap();
    let mut r = FileReport::new(&dir.path().join("none.txt"), None);
    mojibake::check_file(&dir.path().join("none.txt"), &mut r, TextSource::Txt);
    assert!(r.incomplete);
}

// ---- 확장자가 pdf인 EPUB(내용은 ZIP) ----

fn pdf_named_epub_report(chapter: &str) -> FileReport {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("book.pdf");
    epub_with_chapter(&path, chapter);
    scan_file(&path, &opts())
}

#[test]
fn pdf_named_epub_with_garbled_body_is_flagged_and_complete() {
    let r = pdf_named_epub_report(&xhtml(&format!("<p>{}</p>", corrupted())));
    assert!(find(&r, "MOJIBAKE").is_some(), "{r:?}");
    assert!(!r.reasons.iter().any(|x| x == "no text extracted"), "{r:?}");
    assert_eq!(r.format, Some(scan_common::Format::Epub));
    assert_eq!(r.findings.iter().filter(|f| f.code == "FORMAT_MISMATCH").count(), 1, "{r:?}");
}

#[test]
fn pdf_named_epub_with_clean_body_has_no_mojibake_and_is_complete() {
    let r = pdf_named_epub_report(&xhtml(&format!("<p>{}</p>", clean_korean().repeat(4))));
    assert!(find(&r, "MOJIBAKE").is_none() && !r.incomplete, "{r:?}");
    assert_eq!(r.format, Some(scan_common::Format::Epub));
    assert_eq!(r.findings.iter().filter(|f| f.code == "FORMAT_MISMATCH").count(), 1, "{r:?}");
}

#[test]
fn epub_named_file_keeps_the_old_tool_verdict() {
    // 확장자가 .epub이면 옛 `classify_text(read_document_text(..))`와 판정이 같다.
    for chapter in [corrupted(), clean_korean().repeat(4)] {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("b.epub");
        epub_with_chapter(&path, &xhtml(&format!("<p>{chapter}</p>")));
        let old = classify_text(&read_document_text(&path, TEXT_HEAD_CHARS, TEXT_TAIL_CHARS)).0;
        let r = scan_file(&path, &opts());
        assert_eq!(find(&r, "MOJIBAKE").is_some(), old == Verdict::Corrupted, "{r:?}");
    }
}

#[test]
fn pdf_text_with_unknown_page_count_is_incomplete() {
    let on = Options { pdf_text: true, ..Options::default() };
    let r = pdf_report(&on, &FakeRunner::pages_text(0, "x"));
    assert!(r.reasons.iter().any(|x| x == "pdf text not extracted: page count unknown"), "{r:?}");
    let r = pdf_report(&on, &FakeRunner::fails("boom"));
    assert!(r.reasons.iter().any(|x| x == "pdf text not extracted: page count unknown"), "{r:?}");
    let r = pdf_report(&Options::default(), &FakeRunner::pages_text(0, "x"));
    assert!(!r.reasons.iter().any(|x| x.contains("page count unknown")), "{r:?}");
}

#[test]
fn epub_with_many_large_compressible_members_is_bounded_and_incomplete() {
    // 멤버마다 2MB로 풀리는 문서 40개(압축 후 수십 KB). 표본 추출이 한도 없이 풀면 분 단위가 걸린다(최종 검토 지적).
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("bomb.epub");
    let zeros = "\0".repeat(2 * 1024 * 1024);
    let docs: Vec<(String, &str)> = (0..40).map(|i| (format!("ch{i}.xhtml"), zeros.as_str())).collect();
    let refs: Vec<(&str, &str)> = docs.iter().map(|(a, b)| (a.as_str(), *b)).collect();
    make_zip(&path, &refs);
    let started = std::time::Instant::now();
    let r = scan_file(&path, &Options { max_total_mb: 1, ..Options::default() });
    assert!(started.elapsed() < std::time::Duration::from_secs(5), "{:?}", started.elapsed());
    assert!(r.incomplete, "{r:?}");
    assert!(find(&r, "MOJIBAKE").is_none(), "{r:?}");
}

#[test]
fn mojibake_sampling_is_skipped_when_security_already_reported_the_budget() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("bomb.epub");
    let zeros = "\0".repeat(2 * 1024 * 1024);
    make_zip(&path, &[("a.xhtml", zeros.as_str()), ("b.xhtml", zeros.as_str())]);
    let r = scan_file(&path, &Options { max_total_mb: 1, ..Options::default() });
    assert!(r.reasons.iter().any(|x| x.contains("mojibake sampling skipped")), "{:?}", r.reasons);
}
