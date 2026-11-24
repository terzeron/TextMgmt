//! PDF 1쪽 앞에 표지 이미지를 넣는다. 서버 API(`/comics/pdf-pages/{id}/delete`의 inserts)와 같은 편집 코드를 쓴다.
//! 원본은 `<root>/0_trash`에 백업하고, 서버와 같은 디스크 캐시(표지·미리보기)를 지운다.
//! ES 색인은 갱신하지 않는다. 실행 뒤 ES의 page_count·file_size를 따로 맞춘다.

use base64::Engine;
use serde_json::json;
use std::os::unix::fs::MetadataExt;
use std::path::Path;

/// 서버 기본 휴지통 이름(`book_manager` cover_finder의 `TRASH_DIRNAME`)과 같다.
const TRASH_DIRNAME: &str = "0_trash";

fn print_usage() {
    eprintln!(
        "PDF 1쪽 앞에 표지 이미지를 넣는다 (원본은 <root>/0_trash에 백업, 표지·미리보기 캐시 삭제).\n\n\
         usage: pdf_insert_cover <pdf> <root> <image_file> [--replace-first]\n\n\
         <root>           서재 루트 (예: /mnt/data/만화). 캐시는 <root>/.cover_cache, <root>/.preview_cache\n\
         --replace-first  기존 1쪽을 지우고 그 자리에 넣는다 (표지 교체)\n\
         출력: PdfEditResult, book_id(inode), file_size, 지운 캐시 목록을 JSON 한 줄로"
    );
}

/// 서버의 `edit_pdf`가 지우는 캐시와 같은 규칙으로 `book_id`의 캐시 파일을 지운다.
fn clear_caches(root: &Path, book_id: u64) -> Vec<String> {
    let mut removed = Vec::new();
    let rules = [(".cover_cache", format!("{book_id}.v"), ".jpg"), (".preview_cache", format!("{book_id}_"), ".pdf")];
    for (directory, prefix, suffix) in &rules {
        let Ok(entries) = std::fs::read_dir(root.join(directory)) else { continue };
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().into_owned();
            if name.starts_with(prefix.as_str()) && name.ends_with(suffix) && std::fs::remove_file(entry.path()).is_ok() {
                removed.push(format!("{directory}/{name}"));
            }
        }
    }
    let single = root.join(".preview_cache").join(format!("{book_id}.pdf"));
    if std::fs::remove_file(&single).is_ok() {
        removed.push(format!(".preview_cache/{book_id}.pdf"));
    }
    removed.sort();
    removed
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let replace = args.iter().any(|a| a == "--replace-first");
    let pos: Vec<&String> = args.iter().filter(|a| !a.starts_with("--")).collect();
    if pos.len() != 3 {
        print_usage();
        std::process::exit(2);
    }
    let (pdf, root, image) = (Path::new(pos[0]), Path::new(pos[1]), Path::new(pos[2]));
    let data = match std::fs::read(image) {
        Ok(data) => base64::engine::general_purpose::STANDARD.encode(data),
        Err(e) => {
            eprintln!("이미지를 읽지 못했습니다: {e}");
            std::process::exit(1);
        }
    };
    let pages: Vec<i64> = if replace { vec![1] } else { vec![] };
    let trash = root.join(TRASH_DIRNAME);
    match book_manager::edit_pdf_file(pdf, &trash, &pages, &[json!({"page": 1, "image_data": data})]) {
        Ok(result) => {
            let meta = std::fs::metadata(pdf).ok();
            let book_id = meta.as_ref().map_or(0, |m| m.ino());
            let removed = if book_id > 0 { clear_caches(root, book_id) } else { Vec::new() };
            println!("{}", json!({"result": result, "book_id": book_id, "file_size": meta.map_or(0, |m| m.len()), "removed_caches": removed}));
        }
        Err(e) => {
            eprintln!("{e}");
            std::process::exit(1);
        }
    }
}
