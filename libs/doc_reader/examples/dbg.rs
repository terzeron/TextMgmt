// 디렉토리의 *.html 파일마다 `document_text_and_lines`의 글을 *.rs.txt로 쓴다.
fn main() {
    let dir = std::env::args().nth(1).unwrap();
    for entry in std::fs::read_dir(&dir).unwrap().flatten() {
        let path = entry.path();
        if path.extension().is_some_and(|e| e == "html") {
            let content = std::fs::read(&path).unwrap();
            let text = doc_reader::epub::document_text_and_lines(&content).map(|r| r.0).unwrap_or_else(|e| format!("ERR {e}"));
            std::fs::write(path.with_extension("rs.txt"), text).unwrap();
        }
    }
}
