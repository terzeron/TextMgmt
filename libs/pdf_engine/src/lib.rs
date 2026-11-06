//! PDF 처리 능력. lopdf로 열기·쪽 추출(`pdf_page_reader`)과, MuPDF를 별도 프로세스(`tm-pdf-worker`)에서
//! 30초·512MiB 상한으로 돌리는 worker 본체(`native_pdf`)와 호출 adapter(`pdf_worker_client`)를 둔다.
//!
//! 본문 해석·판정·표지 처리는 하지 않는다. 그건 `doc_reader`, `doc_checker`, `cover_manager`가 맡는다.

mod native_pdf;
pub mod pdf_page_reader;
pub mod pdf_worker_client;
