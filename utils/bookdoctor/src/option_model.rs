//! 실행 설정 모델: 하위 명령(`Command`), 실행 옵션(`Options`), 코퍼스 루트 아래의 예약 디렉터리 이름.
//! 명령행 해석(`cli_parser`)이 만들고 각 명령(`file_*`)과 검사 실행(`scan_runner`)이 읽는다.

use std::path::PathBuf;
use std::sync::Arc;

use crate::DEFAULT_LIBRARY_ROOT;
use crate::Severity;
use doc_checker::rule_engine::Compiled;
use doc_checker::{CheckOptions, MAX_TOTAL_MB};

/// 기본 검사 워커 수. /mnt/data는 회전 디스크라 워커를 늘려도 I/O로 막힌다. 2개가 실측상 가장 빨랐다.
/// 메모리 상한이 더 작게 잡으면 그 값을 쓴다.
pub const DEFAULT_WORKERS: usize = 2;

/// 원본을 보관하는 디렉터리 이름(`sanitize`·`repair`). 코퍼스 루트 바로 아래에 있다.
pub const TRASH_DIR_NAME: &str = "0_trash";
/// 인코딩이 손상된 문서를 옮기는 디렉터리 이름(`quarantine`). 코퍼스 루트 바로 아래에 있다.
pub const BROKEN_DIR_NAME: &str = "0_broken";
/// 격리 매니페스트 파일 이름. 격리 디렉터리 안에 둔다.
pub const MANIFEST_NAME: &str = "manifest.json";

/// 하위 명령.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Command {
    /// 진단만 한다. 파일을 바꾸지 않는다.
    #[default]
    Scan,
    /// 보안 지표(HIGH 이상)가 있는 EPUB·PDF를 정리한다.
    Sanitize,
    /// 결함을 고친다: EPUB 본문 XHTML, UTF-16LE로 잘못 저장된 바이너리.
    Repair,
    /// 인코딩이 손상된 TXT·EPUB을 격리 디렉터리로 옮긴다.
    Quarantine,
    /// 격리를 되돌린다.
    Unquarantine,
}

impl Command {
    pub(crate) fn parse(word: &str) -> Option<Self> {
        Some(match word {
            "scan" => Command::Scan,
            "sanitize" => Command::Sanitize,
            "repair" => Command::Repair,
            "quarantine" => Command::Quarantine,
            "unquarantine" => Command::Unquarantine,
            _ => return None,
        })
    }
}

/// 실행 설정. 명령행에서 읽는 것은 `command`, `paths`, `json`, `min_severity`뿐이고 나머지는 고정값이다.
#[derive(Debug, Clone, PartialEq)]
pub struct Options {
    pub command: Command,
    pub paths: Vec<PathBuf>,
    pub json: bool,
    /// 이보다 낮은 심각도의 지표는 출력하지 않는다. 기본은 `High`다.
    pub min_severity: Severity,
    /// 워커 수. 메모리 상한(`effective_workers`)으로 다시 줄어든다.
    pub workers: usize,
    /// 파일 하나에서 읽는 최대 크기(MB).
    pub max_total_mb: u64,
    /// ZIP 계열 멤버를 끝까지 풀어 CRC까지 확인한다. 기본은 멤버마다 앞부분만 풀어 본다.
    pub zip_deep: bool,
    /// 다음 파일을 미리 읽어 디스크 대기와 검사를 겹친다.
    pub prefetch: bool,
    /// MuPDF worker를 부르지 않는다(PDF 본문 표본 추출을 건너뛴다). 테스트용이다.
    pub no_pdf_worker: bool,
    /// ZIP 계열 안의 컨테이너 멤버를 재귀 검사한다.
    pub recurse: bool,
    /// 코퍼스 루트. 휴지통·격리 디렉터리와 매니페스트가 이 아래에 있다.
    pub library_root: PathBuf,
    /// 읽어서 컴파일한 규칙. `execute`가 내장 규칙으로 채운다. 없으면 규칙을 적용하지 않는다(비용 0).
    pub ruleset: Option<Arc<Compiled>>,
}

impl Default for Options {
    fn default() -> Self {
        Options {
            command: Command::Scan,
            paths: Vec::new(),
            json: false,
            min_severity: Severity::High,
            workers: DEFAULT_WORKERS,
            max_total_mb: MAX_TOTAL_MB,
            zip_deep: false,
            prefetch: true,
            no_pdf_worker: false,
            recurse: true,
            library_root: PathBuf::from(DEFAULT_LIBRARY_ROOT),
            ruleset: None,
        }
    }
}

impl Options {
    /// 검사기에 넘기는 검사 설정.
    pub fn check_options(&self) -> CheckOptions {
        CheckOptions { max_total_mb: self.max_total_mb, zip_deep: self.zip_deep, no_pdf_worker: self.no_pdf_worker, recurse: self.recurse, ruleset: self.ruleset.clone() }
    }

    /// `sanitize`·`repair`가 원본을 두는 디렉터리.
    pub fn trash_dir(&self) -> PathBuf {
        self.library_root.join(TRASH_DIR_NAME)
    }

    /// `quarantine`이 파일을 옮기는 디렉터리.
    pub fn broken_dir(&self) -> PathBuf {
        self.library_root.join(BROKEN_DIR_NAME)
    }

    /// 격리 매니페스트 경로.
    pub fn manifest_path(&self) -> PathBuf {
        self.broken_dir().join(MANIFEST_NAME)
    }
}
