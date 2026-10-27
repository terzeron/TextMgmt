use std::cell::RefCell;
use std::fs;
use std::path::Path;

use doc_reader::pdf::{InProcess, PDF_PAGE_LIMIT, Stage, StageError, StageRunner, fast_page_count, read_from_pdf};

#[path = "corpus.rs"]
mod corpus;

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
    let fake = Fake::new(vec![(Stage::Pdfium, Ok(("Hello, 세계!", 7))), (Stage::Plumber, Ok(("다른 글", 7)))]);
    let read = read_from_pdf(p(), &fake);
    assert_eq!(read.summary, "Hello  세계 ");
    assert_eq!(read.page_count, 7);
    assert_eq!(*fake.calls.borrow(), [Stage::Pdfium]);
    assert_eq!(read.records.len(), 1);
    assert_eq!((read.records[0].stage, read.records[0].outcome), (Stage::Pdfium, "ok"));
    assert!(read.logs.is_empty());
}

#[test]
fn an_empty_first_stage_on_an_opened_document_means_a_scan_and_stops() {
    let fake = Fake::new(vec![(Stage::Pdfium, Ok(("  \n", 12))), (Stage::Plumber, Ok(("글", 12)))]);
    let read = read_from_pdf(p(), &fake);
    assert_eq!((read.summary.as_str(), read.page_count), ("", 12));
    assert_eq!(*fake.calls.borrow(), [Stage::Pdfium]);
    assert_eq!(read.records[0].outcome, "empty");
}

#[test]
fn damaged_text_falls_through_and_the_least_damaged_result_is_kept() {
    let (bytes, _, _) = encoding_rs::EUC_KR.encode("안녕하세요");
    let mojibake: String = bytes.iter().map(|&b| b as char).collect();
    let damaged3 = format!("{mojibake} a {mojibake} b {mojibake}");
    let damaged4 = format!("{mojibake} a {mojibake} b {mojibake} c {mojibake}");
    let fake = Fake::new(vec![(Stage::Pdfium, Ok((&damaged4, 3))), (Stage::Plumber, Ok((&damaged3, 3)))]);
    let read = read_from_pdf(p(), &fake);
    assert_eq!(read.records.iter().map(|r| r.outcome).collect::<Vec<_>>(), ["damaged", "damaged"]);
    assert_eq!(read.summary.matches('a').count(), 1, "구간이 적은 두 번째 결과를 쓴다");
    assert_eq!(read.summary.matches('c').count(), 0);
}

#[test]
fn errors_move_on_and_a_second_stage_can_succeed() {
    let fake = Fake::new(vec![(Stage::Pdfium, Err(StageError::Failed("깨짐".into()))), (Stage::Plumber, Ok(("복구된 글", 4)))]);
    let read = read_from_pdf(p(), &fake);
    assert_eq!((read.summary.as_str(), read.page_count), ("복구된 글", 4));
    assert_eq!(read.records.iter().map(|r| r.outcome).collect::<Vec<_>>(), ["error", "ok"]);
}

#[test]
fn timeouts_are_reported_per_file() {
    let fake = Fake::new(vec![(Stage::Pdfium, Err(StageError::Timeout)), (Stage::Plumber, Err(StageError::Timeout)), (Stage::Repair, Err(StageError::Timeout))]);
    let read = read_from_pdf(p(), &fake);
    assert_eq!(read.summary, "");
    assert_eq!(*fake.calls.borrow(), [Stage::Pdfium, Stage::Plumber, Stage::Repair], "쪽을 하나도 못 열면 구조 복구를 시도한다");
    let messages: Vec<&str> = read.logs.iter().map(|(_, m)| m.as_str()).collect();
    assert!(messages.iter().any(|m| m.contains("PDF 파서 타임아웃(30초 초과): _pdf_text_by_pypdfium2")), "{messages:?}");
    assert!(messages.iter().any(|m| m.contains("PDF 구조 복구 타임아웃")));
    assert!(messages.iter().any(|m| m.contains("모든 파서가 타임아웃 또는 오류") && m.contains("_pdf_text_by_pikepdf_repair")), "{messages:?}");
    assert_eq!(read.records.iter().map(|r| r.outcome).collect::<Vec<_>>(), ["timeout", "timeout", "timeout"]);
}

#[test]
fn structure_repair_runs_only_when_no_page_was_opened() {
    let fake = Fake::new(vec![(Stage::Pdfium, Err(StageError::Failed("x".into()))), (Stage::Plumber, Err(StageError::Failed("y".into()))), (Stage::Repair, Ok(("복구 뒤 본문", 9)))]);
    let read = read_from_pdf(p(), &fake);
    assert_eq!((read.summary.as_str(), read.page_count), ("복구 뒤 본문", 9));
    assert!(read.logs.iter().any(|(level, m)| *level == "WARNING" && m.contains("PDF 구조 복구 후 적재 (9쪽)")));

    // 쪽을 열었으면(텍스트가 비어도) 복구하지 않는다.
    let fake = Fake::new(vec![(Stage::Pdfium, Err(StageError::Failed("x".into()))), (Stage::Plumber, Ok(("", 5)))]);
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
fn the_in_process_runner_reads_real_pdfs() {
    let tmp = tempfile::tempdir().unwrap();
    let path = tmp.path().join("a.pdf");
    fs::write(&path, minimal_pdf(3)).unwrap();
    // 글자가 없는 문서: pdfium이 열면 쪽수만 남는다. pdfium이 없으면 오류로 넘어가 lopdf가 쪽수를 센다.
    let read = read_from_pdf(&path, &InProcess);
    assert_eq!(read.summary, "");
    assert!(read.page_count == 3 || read.page_count == 0, "{}", read.page_count);
    assert_eq!(PDF_PAGE_LIMIT, 40);
}

#[test]
fn corpus_pdf_matches_python() {
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
        assert_eq!(corpus::fnv1a64(&read.summary), rec["summary_fnv1a64"].as_str().unwrap(), "summary for {path}");
        assert_eq!(read.page_count as i64, rec["page_count"].as_i64().unwrap(), "page count for {path}");
    }
}
