//! 파일 하나를 읽어 ES 문서로 만든다. 원본: `Loader.read_file`, `detect_file_type`, `_zip_container_type`.

use std::fs::Metadata;
use std::io::Read;
use std::os::unix::fs::MetadataExt;
use std::path::Path;
use std::time::Instant;

use corpus_common::reader::open_zip;
use doc_reader::epub::{EpubPath, read_epub_publisher, read_from_epub};
use doc_reader::pdf::{PDF_STAGE_TIMEOUT, StageRunner, fast_page_count, read_from_pdf};
use doc_reader::text::{read_from_text, reencode_text_file_to_utf8};
use doc_reader::{docx, markup, office, rtf};
use file_time::{StatResult, iso, path_created_time_with_source};
use serde_json::{Map, Value, json};
use stat::Stat;

use crate::config::Config;

/// ZIP 컨테이너를 쓰는 포맷. 확장자가 이미 이 계열이면 매직바이트 검사에서 내용물까지 열지 않는다.
pub const ZIP_BASED_TYPES: [&str; 3] = ["epub", "cbz", "docx"];
const IMAGE_TYPES: [&str; 8] = ["jpg", "jpeg", "png", "gif", "webp", "bmp", "tiff", "svg"];
const SUPPORTED_WITHOUT_TEXT: [&str; 17] = ["txt", "epub", "pdf", "docx", "doc", "hwp", "rtf", "html", "jpg", "jpeg", "png", "gif", "webp", "bmp", "tiff", "svg", "cbz"];

/// 한 파일을 읽은 결과. `item`이 없으면 적재하지 않는다. `messages`는 읽는 동안 남은 경고·오류다.
#[derive(Debug, Clone, Default)]
pub struct ReadOutcome {
    pub item: Option<(i64, Value)>,
    pub messages: Vec<String>,
}

/// ZIP 컨테이너의 실제 종류를 내용물로 판별한다.
pub fn zip_container_type(path: &Path) -> Option<&'static str> {
    let zip = open_zip(path)?;
    let names: Vec<String> = zip.file_names().filter_map(|n| n.ok()).map(|n| n.into_owned()).collect();
    if names.iter().any(|n| n.ends_with(".opf")) || names.iter().any(|n| n == "META-INF/container.xml") {
        return Some("epub");
    }
    if names.iter().any(|n| n.starts_with("word/")) {
        return Some("docx");
    }
    let image_suffixes = [".jpg", ".jpeg", ".png", ".gif", ".webp", ".bmp"];
    let entries: Vec<&String> = names.iter().filter(|n| !n.ends_with('/')).collect();
    if !entries.is_empty() && entries.iter().all(|n| image_suffixes.iter().any(|s| n.to_lowercase().ends_with(s))) {
        return Some("cbz");
    }
    None
}

/// 매직바이트로 실제 포맷을 판별한다. 판별 불가면 `None`.
/// 확장자와 매직바이트가 이미 같은 계열이면 더 파고들지 않아, 정상 파일에서는 앞 4바이트만 읽는다.
pub fn detect_file_type(path: &Path, declared: &str) -> Option<String> {
    detect_file_type_with(path, declared, None)
}

pub type FileInspector<'a> = dyn Fn(&Path) -> Option<&'static str> + 'a;
pub type PageCounter<'a> = dyn Fn(&Path) -> Option<usize> + 'a;
pub type FallbackPageCounter<'a> = dyn Fn(&Path) -> Result<usize, parser_timeout::ParserTimeout> + 'a;

#[derive(Clone, Copy, Default)]
pub struct ReadHooks<'a> {
    pub fast_page_count: Option<&'a PageCounter<'a>>,
    pub fallback_page_count: Option<&'a FallbackPageCounter<'a>>,
    pub inspect: Option<&'a FileInspector<'a>>,
}

pub fn detect_file_type_with(path: &Path, declared: &str, inspect: Option<&FileInspector<'_>>) -> Option<String> {
    let mut head = [0u8; 4];
    let n = std::fs::File::open(path).ok()?.read(&mut head).ok()?;
    let head = &head[..n];
    if head == b"%PDF" {
        return Some("pdf".into());
    }
    if head == b"PK\x03\x04" {
        if ZIP_BASED_TYPES.contains(&declared) {
            return Some(declared.to_owned());
        }
        return inspect.map_or_else(|| zip_container_type(path).map(str::to_owned), |inspect| inspect(path).map(str::to_owned));
    }
    None
}

/// Python `datetime.isoformat()`: 마이크로초가 0이면 소수부를 생략한다.
pub fn now_iso() -> String {
    iso(chrono::Local::now().naive_local())
}

fn page_count_by_lopdf(path: &Path, messages: &mut Vec<String>) -> usize {
    match lopdf::Document::load(path) {
        Ok(doc) => doc.get_pages().len(),
        Err(e) => {
            messages.push(path.display().to_string());
            messages.push(e.to_string());
            0
        }
    }
}

/// PDF 쪽수만: 경량 방식 먼저, 실패하면 lopdf(원본의 pypdf 대응).
fn page_count_only(path: &Path, messages: &mut Vec<String>, hooks: &ReadHooks<'_>) -> usize {
    let fast = match hooks.fast_page_count {
        Some(count) => count(path),
        None => fast_page_count(path),
    };
    let fallback = fast.or_else(|| match hooks.fallback_page_count {
        Some(count) => match count(path) {
            Ok(n) => Some(n),
            Err(e) => {
                messages.push(format!("PDF 페이지 수 추출 타임아웃({PDF_STAGE_TIMEOUT}초 초과): {e}"));
                None
            }
        },
        None => None,
    });
    fallback.unwrap_or_else(|| page_count_by_lopdf(path, messages))
}

struct Timed<'a> {
    stat: &'a Stat,
    kind: &'static str,
    started: Instant,
}

impl<'a> Timed<'a> {
    fn start(stat: &'a Stat, kind: &'static str) -> Self {
        let _ = stat.add(&format!("{kind}_count"), 1.0);
        Timed { stat, kind, started: Instant::now() }
    }

    fn finish(self) {
        let _ = self.stat.add(&format!("{}_total_time", self.kind), self.started.elapsed().as_secs_f64());
    }
}

/// 파일을 읽어 문서를 만든다. 파일이 아니거나 지원하지 않는 형식이면 `item`이 없다.
pub fn read_file(cfg: &Config, path: &Path, st: Option<&Metadata>, skip_text: bool, path_prefix: Option<&Path>, runner: &dyn StageRunner, stat: &Stat) -> ReadOutcome {
    read_file_with_hooks(cfg, path, st, skip_text, path_prefix, runner, stat, &ReadHooks::default())
}

#[allow(clippy::too_many_arguments)]
pub fn read_file_with_hooks(
    cfg: &Config,
    path: &Path,
    st: Option<&Metadata>,
    skip_text: bool,
    path_prefix: Option<&Path>,
    runner: &dyn StageRunner,
    stat: &Stat,
    hooks: &ReadHooks<'_>,
) -> ReadOutcome {
    let mut out = ReadOutcome::default();
    if !path.is_file() {
        return out;
    }
    let owned_st;
    let st = match st {
        Some(s) => s,
        None => match std::fs::metadata(path) {
            Ok(s) => {
                owned_st = s;
                &owned_st
            }
            Err(e) => {
                out.messages.push(format!("{}: {e}", path.display()));
                return out;
            }
        },
    };
    let inode = st.ino() as i64;
    let file_size = st.len();
    let (created, created_source) = match path_created_time_with_source(path, Some(&StatResult::from_metadata(st))) {
        Ok(v) => v,
        Err(e) => {
            out.messages.push(format!("{}: {e}", path.display()));
            return out;
        }
    };
    let prefix = path_prefix.unwrap_or_else(|| cfg.path_prefix(path));
    let Ok(parent_rel) = path.parent().unwrap_or(Path::new("")).strip_prefix(prefix) else {
        out.messages.push(format!("'{}' is not in the subpath of '{}'", path.display(), prefix.display()));
        return out;
    };
    let category = if parent_rel.as_os_str().is_empty() { "_root".to_owned() } else { parent_rel.to_string_lossy().into_owned() };
    let stem = path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
    let (author, title) = split_author_title(&stem);
    let mut file_type = path.extension().map(|e| e.to_string_lossy().to_lowercase()).unwrap_or_default();

    // 확장자가 실제 포맷과 다르면 실제 포맷 기준으로 파서를 고른다.
    if let Some(detected) = detect_file_type_with(path, &file_type, hooks.inspect)
        && detected != file_type
    {
        out.messages.push(format!("확장자와 실제 포맷 불일치: .{file_type} 이지만 {detected}로 처리"));
        file_type = detected;
    }
    let relative_path = path.strip_prefix(prefix).map(|p| p.to_string_lossy().into_owned()).unwrap_or_default();

    if skip_text {
        let mut page_count = 0;
        if file_type == "pdf" {
            page_count = page_count_only(path, &mut out.messages, hooks);
            let _ = stat.add("pdf_count", 1.0);
        }
        if !SUPPORTED_WITHOUT_TEXT.contains(&file_type.as_str()) {
            return out;
        }
        let doc = json!({
            "category": category, "title": title, "author": author, "file_path": relative_path, "file_type": file_type,
            "file_size": file_size, "line_count": 0, "page_count": page_count, "isbn": "", "summary": "",
            "created_time": iso(created), "created_time_source": created_source, "updated_time": now_iso(),
        });
        out.item = Some((inode, doc));
        return out;
    }

    let (mut summary, mut line_count, mut page_count) = (String::new(), 0usize, 0usize);
    let mut raw_content = String::new();
    match file_type.as_str() {
        "txt" => {
            if cfg.reencode {
                let rel = path.strip_prefix(prefix).unwrap_or(Path::new(path.file_name().unwrap_or_default()));
                let r = reencode_text_file_to_utf8(path, cfg.reencode_dry_run, cfg.reencode_backup_dir.as_deref(), rel);
                if r.changed {
                    let _ = stat.add("text_reencoded_count", 1.0);
                    out.messages.push(format!("UTF-8 재인코딩: {}", r.reason));
                } else if r.reason != "이미 UTF-8" && r.reason != "빈 파일" {
                    out.messages.push(format!("UTF-8 재인코딩 건너뜀: {}", r.reason));
                }
            }
            let t = Timed::start(stat, "text");
            let (read, error) = read_from_text(path);
            t.finish();
            out.messages.extend(error);
            (summary, line_count, page_count, raw_content) = (read.summary, read.line_count, read.page_count, read.raw_content);
        }
        "epub" => {
            let started = Instant::now();
            let read = read_from_epub(path);
            let kind = if read.path == EpubPath::Normal { "normal_epub" } else { "zipped_epub" };
            let _ = stat.add(&format!("{kind}_count"), 1.0);
            let _ = stat.add(&format!("{kind}_total_time"), started.elapsed().as_secs_f64());
            out.messages.extend(read.errors);
            (summary, line_count) = (read.summary, read.line_count);
        }
        "pdf" => {
            if cfg.is_comics(path) {
                // 만화: 텍스트 추출은 생략하고 쪽수만 읽는다.
                page_count = page_count_only(path, &mut out.messages, hooks);
                let _ = stat.add("pdf_count", 1.0);
            } else {
                let t = Timed::start(stat, "pdf");
                let read = read_from_pdf(path, runner);
                t.finish();
                for r in &read.records {
                    stat.record_pdf_stage(r.stage.name(), r.seconds, r.outcome);
                }
                out.messages.extend(read.logs.into_iter().map(|(_, m)| m));
                (summary, page_count) = (read.summary, read.page_count);
            }
        }
        "docx" => {
            let t = Timed::start(stat, "docx");
            match docx::read_from_docx(path) {
                Ok((s, l)) => (summary, line_count) = (s, l),
                Err(e) => out.messages.push(format!("can't read docx file '{}': {e}", path.display())),
            }
            t.finish();
        }
        "doc" => {
            let t = Timed::start(stat, "doc");
            let (s, l, errors) = office::read_from_doc(path, &cfg.libreoffice);
            t.finish();
            out.messages.extend(errors);
            (summary, line_count) = (s, l);
        }
        "hwp" => {
            let t = Timed::start(stat, "hwp");
            let (s, l, errors) = office::read_from_hwp(path, &cfg.libreoffice);
            t.finish();
            out.messages.extend(errors);
            (summary, line_count) = (s, l);
        }
        "rtf" => {
            let t = Timed::start(stat, "rtf");
            match std::fs::read(path).map_err(|e| e.to_string()).and_then(|b| String::from_utf8(b).map_err(|e| e.to_string())).and_then(|c| rtf::read_from_rtf_text(&c)) {
                Ok((s, l)) => (summary, line_count) = (s, l),
                Err(e) => {
                    out.messages.push(path.display().to_string());
                    out.messages.push(e);
                }
            }
            t.finish();
        }
        "html" => {
            let t = Timed::start(stat, "html");
            match markup::read_from_html(path) {
                Ok((s, l, p)) => (summary, line_count, page_count) = (s, l, p),
                Err(e) => out.messages.push(format!("can't read html file '{}': {e}", path.display())),
            }
            t.finish();
        }
        t if IMAGE_TYPES.contains(&t) || t == "cbz" => {
            let _ = stat.add("image_count", 1.0);
        }
        _ => return out,
    }

    // ISBN 추출. TXT는 이미 읽은 본문을, PDF는 요약을 다시 쓴다.
    let mut isbn_value = String::new();
    if ["txt", "epub", "pdf", "djvu", "hwp"].contains(&file_type.as_str()) {
        let content = match file_type.as_str() {
            "txt" => Some(raw_content.as_str()),
            "pdf" => Some(summary.as_str()),
            _ => None,
        };
        let values = match runner.worker_executable() {
            Some(executable) => isbn::extract_default_with_worker(path, content, executable),
            None => isbn::extract_default(path, content),
        };
        isbn_value = values.into_iter().next().unwrap_or_default();
    }
    // dc:publisher는 OPF 한 멤버만 풀면 나온다.
    let publisher = if file_type == "epub" { read_epub_publisher(path) } else { String::new() };

    let mut doc = Map::new();
    for (k, v) in [
        ("category", json!(category)),
        ("title", json!(title)),
        ("author", json!(author)),
        ("publisher", json!(publisher)),
        ("file_path", json!(relative_path)),
        ("file_type", json!(file_type)),
        ("file_size", json!(file_size)),
        ("line_count", json!(line_count)),
        ("page_count", json!(page_count)),
        ("isbn", json!(isbn_value)),
        ("summary", json!(summary)),
        ("created_time", json!(iso(created))),
        ("created_time_source", json!(created_source)),
        ("updated_time", json!(now_iso())),
    ] {
        doc.insert(k.to_owned(), v);
    }
    out.item = Some((inode, Value::Object(doc)));
    out
}

/// `^\[(?P<author>[^\]]+)\]\s*(?P<title>.+)$`: 파일명에서 (저자, 제목). 대괄호 저자가 없으면 저자는 빈 문자열이다.
pub fn split_author_title(stem: &str) -> (String, String) {
    if let Some(rest) = stem.strip_prefix('[')
        && let Some(end) = rest.find(']')
    {
        let author = &rest[..end];
        let title = rest[end + 1..].trim_start_matches(|c: char| c.is_whitespace() || ('\u{1c}'..='\u{1f}').contains(&c));
        if !author.is_empty() && !title.is_empty() && !title.contains('\n') && !author.contains('\n') {
            return (author.to_owned(), title.to_owned());
        }
    }
    (String::new(), stem.to_owned())
}
