mod corpus;

use std::cell::RefCell;
use std::fs;
use std::path::Path;

use doc_reader::pdf::{
    InProcess, PDF_PAGE_LIMIT, PdftotextHooks, Stage, StageError, StageRunner, fast_page_count, find_xref_offset, parse_one_xref_stream, pdf_text_by_pdftotext_with, read_from_obj_stream,
    read_from_pdf,
};

type Answer<'a> = Result<(&'a str, usize), StageError>;
type OwnedAnswer = (Stage, Result<(String, usize), StageError>);

/// 단계별로 준비한 답을 돌려주고 부른 순서를 기록한다.
struct Fake {
    answers: Vec<OwnedAnswer>,
    calls: RefCell<Vec<Stage>>,
}

impl Fake {
    fn new(answers: Vec<(Stage, Answer)>) -> Self {
        Fake { answers: answers.into_iter().map(|(s, r)| (s, r.map(|(t, n)| (t.to_owned(), n)))).collect(), calls: RefCell::new(Vec::new()) }
    }
}

impl StageRunner for Fake {
    fn run(&self, stage: Stage, _path: &Path) -> Result<(String, usize), StageError> {
        self.calls.borrow_mut().push(stage);
        self.answers.iter().find(|(s, _)| *s == stage).map(|(_, r)| r.clone()).unwrap_or(Err(StageError::Failed("없음".into())))
    }
}

fn p() -> &'static Path {
    Path::new("/x/a.pdf")
}

#[test]
fn the_first_usable_stage_wins_and_stops_the_chain() {
    let fake = Fake::new(vec![(Stage::Poppler, Ok(("Hello, 세계!", 7))), (Stage::Lopdf, Ok(("다른 글", 7)))]);
    let read = read_from_pdf(p(), &fake);
    assert_eq!(read.summary, "Hello  세계 ");
    assert_eq!(read.page_count, 7);
    assert_eq!(*fake.calls.borrow(), [Stage::Poppler]);
    assert_eq!(read.records.len(), 1);
    assert_eq!((read.records[0].stage, read.records[0].outcome), (Stage::Poppler, "ok"));
    assert!(read.logs.is_empty());
}

#[test]
fn an_empty_first_stage_on_an_opened_document_means_a_scan_and_stops() {
    let fake = Fake::new(vec![(Stage::Poppler, Ok(("  \n", 12))), (Stage::Lopdf, Ok(("글", 12)))]);
    let read = read_from_pdf(p(), &fake);
    assert_eq!((read.summary.as_str(), read.page_count), ("", 12));
    assert_eq!(*fake.calls.borrow(), [Stage::Poppler]);
    assert_eq!(read.records[0].outcome, "empty");
}

#[test]
fn damaged_text_falls_through_and_the_least_damaged_result_is_kept() {
    let (bytes, _, _) = encoding_rs::EUC_KR.encode("안녕하세요");
    let mojibake: String = bytes.iter().map(|&b| b as char).collect();
    let damaged3 = format!("{mojibake} a {mojibake} b {mojibake}");
    let damaged4 = format!("{mojibake} a {mojibake} b {mojibake} c {mojibake}");
    let fake = Fake::new(vec![(Stage::Poppler, Ok((&damaged4, 3))), (Stage::Lopdf, Ok((&damaged3, 3)))]);
    let read = read_from_pdf(p(), &fake);
    assert_eq!(read.records.iter().map(|r| r.outcome).collect::<Vec<_>>(), ["damaged", "damaged"]);
    assert_eq!(read.summary.matches('a').count(), 1, "구간이 적은 두 번째 결과를 쓴다");
    assert_eq!(read.summary.matches('c').count(), 0);
}

#[test]
fn errors_move_on_and_a_second_stage_can_succeed() {
    let fake = Fake::new(vec![(Stage::Poppler, Err(StageError::Failed("깨짐".into()))), (Stage::Lopdf, Ok(("복구된 글", 4)))]);
    let read = read_from_pdf(p(), &fake);
    assert_eq!((read.summary.as_str(), read.page_count), ("복구된 글", 4));
    assert_eq!(read.records.iter().map(|r| r.outcome).collect::<Vec<_>>(), ["error", "ok"]);
}

#[test]
fn timeouts_are_reported_per_file() {
    let fake = Fake::new(vec![(Stage::Poppler, Err(StageError::Timeout)), (Stage::Lopdf, Err(StageError::Timeout)), (Stage::Repair, Err(StageError::Timeout))]);
    let read = read_from_pdf(p(), &fake);
    assert_eq!(read.summary, "");
    assert_eq!(*fake.calls.borrow(), [Stage::Poppler, Stage::Lopdf, Stage::Repair], "쪽을 하나도 못 열면 구조 복구를 시도한다");
    let messages: Vec<&str> = read.logs.iter().map(|(_, m)| m.as_str()).collect();
    assert!(messages.iter().any(|m| m.contains("PDF 파서 타임아웃(60초 초과): pdf_text_poppler")), "{messages:?}");
    assert!(messages.iter().any(|m| m.contains("PDF 구조 복구 타임아웃")));
    assert!(messages.iter().any(|m| m.contains("모든 파서가 타임아웃 또는 오류") && m.contains("pdf_text_repair")), "{messages:?}");
    assert_eq!(read.records.iter().map(|r| r.outcome).collect::<Vec<_>>(), ["timeout", "timeout", "timeout"]);
}

#[test]
fn structure_repair_runs_only_when_no_page_was_opened() {
    let fake = Fake::new(vec![(Stage::Poppler, Err(StageError::Failed("x".into()))), (Stage::Lopdf, Err(StageError::Failed("y".into()))), (Stage::Repair, Ok(("복구 뒤 본문", 9)))]);
    let read = read_from_pdf(p(), &fake);
    assert_eq!((read.summary.as_str(), read.page_count), ("복구 뒤 본문", 9));
    assert!(read.logs.iter().any(|(level, m)| *level == "WARNING" && m.contains("PDF 구조 복구 후 적재 (9쪽)")));

    // 쪽을 열었으면(텍스트가 비어도) 복구하지 않는다.
    let fake = Fake::new(vec![(Stage::Poppler, Err(StageError::Failed("x".into()))), (Stage::Lopdf, Ok(("", 5)))]);
    read_from_pdf(p(), &fake);
    assert!(!fake.calls.borrow().contains(&Stage::Repair));

    // 복구도 실패하면 오류를 남긴다.
    let fake = Fake::new(vec![]);
    let read = read_from_pdf(p(), &fake);
    assert!(read.logs.iter().any(|(_, m)| m.contains("PDF 구조 복구 실패(객체 소실 추정)")));
    assert!(read.logs.iter().filter(|(_, m)| m == "/x/a.pdf").count() >= 2);
}

#[test]
fn damaged_repair_output_is_not_adopted() {
    let (bytes, _, _) = encoding_rs::EUC_KR.encode("안녕하세요");
    let mojibake: String = bytes.iter().map(|&b| b as char).collect();
    let damaged = format!("{mojibake} a {mojibake} b {mojibake}");
    let fake = Fake::new(vec![(Stage::Repair, Ok((&damaged, 2)))]);
    let read = read_from_pdf(p(), &fake);
    assert_eq!(read.summary, "");
    assert_eq!(read.page_count, 2);
    assert_eq!(read.records.last().unwrap().outcome, "damaged");
}

/// 최소 PDF를 만든다. `traditional`이면 xref 표, 아니면 xref 스트림은 쓰지 않고 같은 표를 쓴다.
fn minimal_pdf(pages: usize) -> Vec<u8> {
    let mut objs: Vec<String> = Vec::new();
    let kids: Vec<String> = (0..pages).map(|i| format!("{} 0 R", 3 + i)).collect();
    objs.push("<< /Type /Catalog /Pages 2 0 R >>".into());
    objs.push(format!("<< /Type /Pages /Kids [{}] /Count {pages} >>", kids.join(" ")));
    for _ in 0..pages {
        objs.push("<< /Type /Page /Parent 2 0 R /MediaBox [0 0 100 100] >>".into());
    }
    let mut out = b"%PDF-1.4\n".to_vec();
    let mut offsets = Vec::new();
    for (i, body) in objs.iter().enumerate() {
        offsets.push(out.len());
        out.extend(format!("{} 0 obj\n{body}\nendobj\n", i + 1).bytes());
    }
    let xref_at = out.len();
    out.extend(format!("xref\n0 {}\n0000000000 65535 f \n", objs.len() + 1).bytes());
    for o in &offsets {
        out.extend(format!("{o:010} 00000 n \n").bytes());
    }
    out.extend(format!("trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{xref_at}\n%%EOF\n", objs.len() + 1).bytes());
    out
}

#[test]
fn fast_page_count_reads_the_count_from_a_traditional_xref() {
    let tmp = tempfile::tempdir().unwrap();
    let path = tmp.path().join("a.pdf");
    fs::write(&path, minimal_pdf(5)).unwrap();
    assert_eq!(fast_page_count(&path), Some(5));
    fs::write(&path, minimal_pdf(1)).unwrap();
    assert_eq!(fast_page_count(&path), Some(1));
}

#[test]
fn fast_page_count_gives_up_on_garbage() {
    let tmp = tempfile::tempdir().unwrap();
    let path = tmp.path().join("g.pdf");
    fs::write(&path, b"%PDF-1.4 no trailer here").unwrap();
    assert_eq!(fast_page_count(&path), None);
    assert_eq!(fast_page_count(&tmp.path().join("none.pdf")), None);
    fs::write(&path, b"").unwrap();
    assert_eq!(fast_page_count(&path), None);
}

#[test]
fn fast_page_count_follows_prev_sections_up_to_the_depth_limit() {
    let tmp = tempfile::tempdir().unwrap();
    let path = tmp.path().join("prev.pdf");
    let pdf = minimal_pdf(4);
    let xref_at = pdf.windows(5).position(|w| w == b"\nxref").unwrap() + 1;
    // /Prev가 자기 자신을 가리켜도 10단계에서 멈추고, 첫 구간에서 읽은 /Root를 그대로 쓴다.
    let looping = String::from_utf8(pdf).unwrap().replace("/Root 1 0 R >>", &format!("/Root 1 0 R /Prev {xref_at} >>"));
    fs::write(&path, looping).unwrap();
    assert_eq!(fast_page_count(&path), Some(4));
}

#[test]
fn xref_tables_accept_cr_line_ends_and_stop_at_a_short_subsection() {
    assert_eq!(find_xref_offset(b"xref\r\n0 2\r0000000000 65535 f \r\n0000000017 00000 n \r\ntrailer", 1), Some(17));
    // 하위 구간 헤더가 말한 항목 수보다 줄이 모자라면 찾지 못한다.
    assert_eq!(find_xref_offset(b"xref\n0 3\n0000000000 65535 f \n", 2), None);
}

fn zlib(data: &[u8]) -> Vec<u8> {
    use std::io::Write;
    let mut enc = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::default());
    enc.write_all(data).unwrap();
    enc.finish().unwrap()
}

/// `bytes`를 파일로 쓰고 연다.
fn file_of(dir: &Path, bytes: &[u8]) -> fs::File {
    let path = dir.join("part.bin");
    fs::write(&path, bytes).unwrap();
    fs::File::open(path).unwrap()
}

#[test]
fn the_last_object_of_a_compressed_object_stream_runs_to_the_stream_end() {
    let tmp = tempfile::tempdir().unwrap();
    let header = b"5 0 ";
    let compressed = zlib(&[&header[..], b"<< /Type /Catalog >>"].concat());
    let mut stream = format!("1 0 obj\n<< /First {} /Length {} /Filter /FlateDecode >>\nstream\n", header.len(), compressed.len()).into_bytes();
    stream.extend(&compressed);
    stream.extend(b"\nendstream");
    assert_eq!(read_from_obj_stream(&mut file_of(tmp.path(), &stream), 0, 5).as_deref(), Some(&b"<< /Type /Catalog >>"[..]));
    // 필터가 없으면 스트림 바이트를 그대로 읽는다.
    let mut plain = format!("1 0 obj\n<< /First {} /Length 24 >>\nstream\n", header.len()).into_bytes();
    plain.extend(b"5 0 << /Type /Catalog >>\nendstream");
    assert_eq!(read_from_obj_stream(&mut file_of(tmp.path(), &plain), 0, 5).as_deref(), Some(&b"<< /Type /Catalog >>"[..]));
}

#[test]
fn xref_stream_predictors_need_level_ten_and_default_columns_to_the_entry_width() {
    let tmp = tempfile::tempdir().unwrap();
    // 행마다 필터 바이트 2(위 행 더하기). /Columns가 없으면 /W 합(4)을 쓴다.
    let rows = [2u8, 1, 0, 100, 0, 2, 0, 0, 100, 0];
    let compressed = zlib(&rows);
    let stream = |parms: &str| {
        let mut s = format!("1 0 obj\n<< /W [1 2 1] /Size 2 /Length {} /Filter /FlateDecode /DecodeParms << {parms} >> >>\nstream\n", compressed.len()).into_bytes();
        s.extend(&compressed);
        s.extend(b"\nendstream");
        s
    };
    let parsed = parse_one_xref_stream(&mut file_of(tmp.path(), &stream("/Predictor 12")), 0).unwrap();
    assert_eq!(parsed.entries, [1, 0, 100, 0, 1, 0, 200, 0]);
    // 10 미만 예측기는 되돌리지 않는다.
    let parsed = parse_one_xref_stream(&mut file_of(tmp.path(), &stream("/Predictor 2")), 0).unwrap();
    assert_eq!(parsed.entries, rows);
}

#[test]
fn pdftotext_without_hooks_uses_poppler_in_process() {
    let tmp = tempfile::tempdir().unwrap();
    let path = tmp.path().join("a.pdf");
    fs::write(&path, minimal_pdf(2)).unwrap();
    let hooks = PdftotextHooks { which: None, run: None, path_cache: &std::sync::OnceLock::new() };
    assert_eq!(pdf_text_by_pdftotext_with(&path, &hooks), (String::new(), 2));
    fs::write(&path, b"not a pdf").unwrap();
    assert_eq!(pdf_text_by_pdftotext_with(&path, &hooks), (String::new(), 0));
}

#[test]
fn the_in_process_runner_reads_real_pdfs() {
    let tmp = tempfile::tempdir().unwrap();
    let path = tmp.path().join("a.pdf");
    fs::write(&path, minimal_pdf(3)).unwrap();
    // 글자가 없는 문서도 Poppler가 열면 쪽수를 돌려준다.
    let read = read_from_pdf(&path, &InProcess);
    assert_eq!(read.summary, "");
    assert!(read.page_count == 3 || read.page_count == 0, "{}", read.page_count);
    assert_eq!(PDF_PAGE_LIMIT, 40);
}

#[test]
fn corpus_pdf_has_page_counts_and_extracted_text() {
    let root = corpus::books_root();
    for rec in corpus::records(include_str!("fixtures/corpus_pdf.jsonl")) {
        let path = rec["path"].as_str().unwrap();
        let full_path = root.join(path);
        let fast = fast_page_count(&full_path).map(|count| count as u64);
        assert_eq!(fast, rec["fast"].as_u64(), "fast page count for {path}");
        if rec.get("error").is_some() {
            continue;
        }
        let read = read_from_pdf(&full_path, &InProcess);
        let summary_hash = corpus::fnv1a64(&read.summary);
        let expected_empty = rec["summary_fnv1a64"].as_str() == Some("cbf29ce484222325");
        assert_eq!(summary_hash == "cbf29ce484222325", expected_empty, "text availability for {path}");
        assert_eq!(read.page_count as i64, rec["page_count"].as_i64().unwrap(), "page count for {path}");
    }
}

#[test]
fn mojibake_run_window_matches_the_summary_length() {
    // PDF 추출 실패 판정은 요약에 쓰는 앞부분만 본다.
    assert_eq!(text_decoder::mojibake_counter::RUN_SCAN_CHARS, doc_reader::TEXT_SIZE);
}
