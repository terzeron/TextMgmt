//! PDF `--sanitize`: 위험한 동작과 스크립트를 지우고 다시 쓴다.
//!
//! 지우는 것: `Launch`·`JavaScript`·`RichMediaExecute` 동작, 그 동작을 가리키는 키(`/A`, `/OpenAction`, `/AA`의 항목, `/Next` 등),
//! `/JS`·`/Win`·`/Mac`·`/Unix`·`/JavaScript`(이름 트리) 키. 가리키는 곳이 없어진 객체는 함께 버린다.
//! 쪽 내용 스트림과 글꼴, 이미지는 건드리지 않는다.
//!
//! 순서: 검사 → 보안 지표(HIGH 이상)가 있는 파일만 대상 → `rebuild` → `verify`(쪽 수·쪽 내용) → 재검사 → 적용.
//! 적용은 EPUB과 같다: 원본을 휴지통 디렉터리에 두고 같은 inode에 덮어쓰고 mtime을 되돌린다.
//! 암호화된 PDF, 참조가 너무 많은 PDF, 너무 큰 PDF는 다시 쓰지 않는다. 디지털 서명은 다시 쓰면 무효가 된다.

use std::collections::BTreeMap;
use std::fs;
use std::io::{self, Write};
use std::path::Path;

use lopdf::{Dictionary, Document, LoadOptions, Object, ObjectId};

use crate::cli::Options;
use crate::pdf::security::{count_raw_references, REF_FILE_BUDGET};
use crate::sanitize::{apply, create_private_temp_ext, has_serious_security, show};
use crate::scan::scan_file;

/// 다시 쓰는 PDF의 크기 상한. lopdf는 객체를 모두 메모리에 올리므로 파일보다 몇 배 큰 메모리를 쓴다.
const MAX_FILE_BYTES: usize = 128 * 1024 * 1024;
/// 읽는 동안 스트림 하나가 풀릴 수 있는 크기 상한(객체 스트림·xref 스트림).
const MAX_LOAD_STREAM_BYTES: usize = 256 * 1024 * 1024;
const MAX_DEPTH: usize = 64;

/// 지우는 동작 이름.
const BAD_ACTIONS: &[&[u8]] = &[b"Launch", b"JavaScript", b"RichMediaExecute"];
/// 값과 관계없이 지우는 키.
const BAD_KEYS: &[&[u8]] = &[b"JS", b"Win", b"Mac", b"Unix", b"JavaScript"];

/// 지운 결과.
pub struct Plan {
    pub bytes: Vec<u8>,
    /// 지운 것: `(위치, 내용)`.
    pub removed: Vec<(String, String)>,
}

fn load_full(data: &[u8]) -> Result<Document, String> {
    let parsed = std::panic::catch_unwind(|| Document::load_mem_with_options(data, LoadOptions { max_decompressed_size: Some(MAX_LOAD_STREAM_BYTES), ..LoadOptions::default() }));
    match parsed {
        Ok(Ok(doc)) => Ok(doc),
        Ok(Err(e)) => Err(format!("PDF를 읽을 수 없다: {e}")),
        Err(_) => Err("PDF를 읽는 중 파서가 멈췄다".to_owned()),
    }
}

fn dict_of(object: &Object) -> Option<&Dictionary> {
    match object {
        Object::Dictionary(d) => Some(d),
        Object::Stream(s) => Some(&s.dict),
        _ => None,
    }
}

/// `/S` 값을 이름으로 푼다(간접 참조도 따라간다).
fn action_name<'a>(doc: &'a Document, d: &'a Dictionary) -> Option<&'a [u8]> {
    match d.get(b"S").ok()? {
        Object::Name(n) => Some(n.as_slice()),
        Object::Reference(id) => match doc.get_object(*id).ok()? {
            Object::Name(n) => Some(n.as_slice()),
            _ => None,
        },
        _ => None,
    }
}

fn bad_action_name(doc: &Document, d: &Dictionary) -> Option<String> {
    action_name(doc, d).filter(|n| BAD_ACTIONS.contains(n)).map(|n| String::from_utf8_lossy(n).into_owned())
}

/// 인라인 사전이 지울 동작이면 그 이름(`/S`가 이름으로 직접 쓰였을 때만 안다).
fn inline_bad(d: &Dictionary) -> Option<String> {
    match d.get(b"S") {
        Ok(Object::Name(n)) if BAD_ACTIONS.contains(&n.as_slice()) => Some(String::from_utf8_lossy(n).into_owned()),
        _ => None,
    }
}

/// 값이 지울 동작(간접 참조 또는 인라인 사전)이면 그 이름.
fn value_bad_name(value: &Object, bad: &BTreeMap<ObjectId, String>) -> Option<String> {
    match value {
        Object::Reference(id) => bad.get(id).cloned(),
        Object::Dictionary(d) => inline_bad(d),
        _ => None,
    }
}

fn name_text(key: &[u8]) -> String {
    String::from_utf8_lossy(key).chars().filter(|c| !c.is_control()).collect()
}

struct Stripper<'a> {
    bad: &'a BTreeMap<ObjectId, String>,
    removed: &'a mut Vec<(String, String)>,
}

impl Stripper<'_> {
    fn object(&mut self, object: &mut Object, id: ObjectId, depth: usize) {
        if depth > MAX_DEPTH {
            return;
        }
        match object {
            Object::Dictionary(d) => self.dict(d, id, depth),
            Object::Stream(s) => self.dict(&mut s.dict, id, depth),
            Object::Array(items) => {
                let before = items.len();
                items.retain(|item| value_bad_name(item, self.bad).is_none());
                for _ in items.len()..before {
                    self.removed.push((format!("{} {} R", id.0, id.1), "배열의 위험한 동작 항목".to_owned()));
                }
                for item in items.iter_mut() {
                    self.object(item, id, depth + 1);
                }
            }
            _ => {}
        }
    }

    fn dict(&mut self, d: &mut Dictionary, id: ObjectId, depth: usize) {
        let drop: Vec<(Vec<u8>, Option<String>)> = d.iter().filter_map(|(key, value)| {
            let bad_value = value_bad_name(value, self.bad);
            (BAD_KEYS.contains(&key.as_slice()) || bad_value.is_some()).then(|| (key.clone(), bad_value))
        }).collect();
        for (key, action) in drop {
            d.remove(&key);
            let what = match action {
                Some(name) => format!("/{} 제거({name} 동작)", name_text(&key)),
                None => format!("/{} 제거", name_text(&key)),
            };
            self.removed.push((format!("{} {} R", id.0, id.1), what));
        }
        for (_, value) in d.iter_mut() {
            self.object(value, id, depth + 1);
        }
    }
}

/// PDF 바이트에서 위험한 동작과 스크립트를 지워 다시 쓴다.
pub fn rebuild(src: &[u8]) -> Result<Plan, String> {
    if src.len() > MAX_FILE_BYTES {
        return Err("파일이 너무 커서 다시 쓰지 않는다".to_owned());
    }
    if count_raw_references(src) > REF_FILE_BUDGET {
        return Err("참조가 너무 많아 다시 쓰지 않는다".to_owned());
    }
    let mut doc = load_full(src)?;
    if doc.is_encrypted() || doc.trailer.has(b"Encrypt") {
        return Err("암호화된 PDF라 다시 쓰지 않는다".to_owned());
    }
    let bad: BTreeMap<ObjectId, String> = doc.objects.iter().filter_map(|(id, object)| dict_of(object).and_then(|d| bad_action_name(&doc, d)).map(|name| (*id, name))).collect();
    let mut removed = Vec::new();
    {
        let ids: Vec<ObjectId> = doc.objects.keys().copied().collect();
        let mut stripper = Stripper { bad: &bad, removed: &mut removed };
        for id in ids {
            if let Some(object) = doc.objects.get_mut(&id) {
                stripper.object(object, id, 0);
            }
        }
    }
    for (id, name) in &bad {
        doc.objects.remove(id);
        removed.push((format!("{} {} R", id.0, id.1), format!("{name} 동작 객체 삭제")));
    }
    if removed.is_empty() {
        return Ok(Plan { bytes: src.to_vec(), removed });
    }
    doc.prune_objects();
    let mut bytes = Vec::new();
    let uses_xref_streams = matches!(doc.reference_table.cross_reference_type, lopdf::xref::XrefType::CrossReferenceStream);
    let saved = if uses_xref_streams { doc.save_modern(&mut bytes) } else { doc.save_to(&mut bytes) };
    saved.map_err(|e| format!("PDF를 쓸 수 없다: {e}"))?;
    Ok(Plan { bytes, removed })
}

/// 쪽별 내용 스트림(필터를 푼 뒤의 바이트).
fn page_contents(doc: &Document) -> Vec<Vec<u8>> {
    doc.get_pages().values().map(|id| doc.get_page_content(*id)).collect()
}

/// 다시 쓴 결과가 원본의 쪽을 지키는지 본다: 열리는지, 쪽 수가 같은지, 쪽마다 내용 스트림이 같은지.
pub fn verify(orig: &[u8], new: &[u8]) -> Result<(), String> {
    let (old_doc, new_doc) = (load_full(orig)?, load_full(new)?);
    let (old_pages, new_pages) = (page_contents(&old_doc), page_contents(&new_doc));
    if old_pages.len() != new_pages.len() {
        return Err(format!("쪽 수가 달라졌다: {} → {}", old_pages.len(), new_pages.len()));
    }
    if let Some(i) = old_pages.iter().zip(&new_pages).position(|(a, b)| a != b) {
        return Err(format!("{}쪽 내용이 달라졌다", i + 1));
    }
    Ok(())
}

/// 다시 쓴 결과를 임시 파일에 써서 다시 검사한다. 보안 지표(HIGH 이상)가 남으면 오류다.
fn rescan(bytes: &[u8], opts: &Options) -> Result<(), String> {
    let tmp = create_private_temp_ext(bytes, "pdf").map_err(|e| e.to_string())?;
    let report = scan_file(&tmp, opts);
    let _ = fs::remove_file(&tmp);
    if has_serious_security(&report) {
        Err("재검사에서 보안 지표가 남았다".to_owned())
    } else {
        Ok(())
    }
}

pub(crate) enum Outcome {
    Done,
    Untouched,
    Skipped,
    Failed,
}

/// PDF 하나를 처리한다. `--sanitize`의 EPUB 흐름과 같은 출력 형식을 쓴다.
pub(crate) fn process(path: &Path, shown: &str, opts: &Options, scan_opts: &Options, out: &mut dyn Write) -> io::Result<Outcome> {
    if !has_serious_security(&scan_file(path, scan_opts)) {
        return Ok(Outcome::Untouched);
    }
    let planned = fs::read(path).map_err(|e| e.to_string()).and_then(|src| rebuild(&src).map(|plan| (src, plan)));
    let (src, plan) = match planned {
        Ok(v) => v,
        Err(e) => {
            writeln!(out, "SKIP {shown}: {}", show(&e))?;
            return Ok(Outcome::Skipped);
        }
    };
    if plan.removed.is_empty() {
        writeln!(out, "UNCHANGED {shown}: 지울 요소가 없다(남은 지표는 이번 정리로 해결되지 않는다)")?;
        return Ok(Outcome::Untouched);
    }
    if let Err(e) = verify(&src, &plan.bytes).and_then(|()| rescan(&plan.bytes, scan_opts)) {
        writeln!(out, "SKIP {shown}: 검증 실패: {}", show(&e))?;
        return Ok(Outcome::Skipped);
    }
    let head = if opts.dry_run { "WOULD" } else { "SANITIZED" };
    writeln!(out, "{head} {shown}: {}개 지움", plan.removed.len())?;
    for (location, what) in &plan.removed {
        writeln!(out, "  - 제거 {} ({})", show(location), show(what))?;
    }
    if !opts.dry_run {
        let trash = opts.trash_dir.as_deref().unwrap_or_else(|| Path::new("0_trash"));
        match apply(path, &plan.bytes, trash) {
            Ok(saved) => writeln!(out, "  - 원본 보관 {}", show(&saved.to_string_lossy()))?,
            Err(e) => {
                writeln!(out, "FAIL {shown}: {}", show(&e.to_string()))?;
                return Ok(Outcome::Failed);
            }
        }
    }
    Ok(Outcome::Done)
}
