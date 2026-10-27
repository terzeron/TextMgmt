//! 시계, 파일시스템, 외부 명령, HTTP 응답 같은 시스템 능력.
//! 서비스 코드는 구체 타입 대신 이 trait를 받고, 테스트는 대역으로 바꿔 끼운다.
//! 설계: docs/rust-test-seams.md 3.1

use std::fmt;
use std::fs;
use std::io::{self, Read};
use std::os::fd::AsRawFd;
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, OnceLock};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use serde_json::Value;

pub mod poppler;

/// 환경 변수 조회 함수. `loader::Config::from_env`와 같은 모양이다.
pub type EnvFn<'a> = &'a dyn Fn(&str) -> Option<String>;

// ---------------------------------------------------------------------------
// 시계
// ---------------------------------------------------------------------------

pub trait Clock: Send + Sync {
    /// `time.time()`: 유닉스 시각(초).
    fn now(&self) -> f64;
    /// `time.monotonic()`: 되돌아가지 않는 시각(초).
    fn monotonic(&self) -> f64;
    fn sleep(&self, seconds: f64);
}

pub struct SystemClock;

impl Clock for SystemClock {
    fn now(&self) -> f64 {
        SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs_f64()).unwrap_or(0.0)
    }

    fn monotonic(&self) -> f64 {
        static START: OnceLock<Instant> = OnceLock::new();
        START.get_or_init(Instant::now).elapsed().as_secs_f64()
    }

    fn sleep(&self, seconds: f64) {
        // Python time.sleep은 음수에서 예외를 내지만, 여기서는 기다리지 않는 쪽으로 단순화한다.
        if seconds.is_finite() && seconds > 0.0 {
            thread::sleep(Duration::from_secs_f64(seconds));
        }
    }
}

// ---------------------------------------------------------------------------
// 파일시스템
// ---------------------------------------------------------------------------

fn unimplemented_io(what: &str) -> io::Error {
    io::Error::new(io::ErrorKind::Unsupported, format!("FsOps::{what} 미구현"))
}

/// 파일시스템 연산. 모든 메서드의 기본 구현은 `Unsupported` 오류라서, 테스트 대역은 쓰는 메서드만 구현한다.
pub trait FsOps: Send + Sync {
    /// `os.scandir`/`Path.iterdir` 통합. 항목의 전체 경로를 돌려준다.
    fn read_dir(&self, _p: &Path) -> io::Result<Vec<PathBuf>> {
        Err(unimplemented_io("read_dir"))
    }
    fn metadata(&self, _p: &Path) -> io::Result<fs::Metadata> {
        Err(unimplemented_io("metadata"))
    }
    /// `Path.is_file()`: 없는 경로는 오류가 아니라 `false`다.
    fn is_file(&self, _p: &Path) -> io::Result<bool> {
        Err(unimplemented_io("is_file"))
    }
    fn rename(&self, _from: &Path, _to: &Path) -> io::Result<()> {
        Err(unimplemented_io("rename"))
    }
    fn remove_file(&self, _p: &Path) -> io::Result<()> {
        Err(unimplemented_io("remove_file"))
    }
    fn remove_dir(&self, _p: &Path) -> io::Result<()> {
        Err(unimplemented_io("remove_dir"))
    }
    fn remove_dir_all(&self, _p: &Path) -> io::Result<()> {
        Err(unimplemented_io("remove_dir_all"))
    }
    fn canonicalize(&self, _p: &Path) -> io::Result<PathBuf> {
        Err(unimplemented_io("canonicalize"))
    }
    fn same_file(&self, _a: &Path, _b: &Path) -> io::Result<bool> {
        Err(unimplemented_io("same_file"))
    }
    fn open(&self, _p: &Path) -> io::Result<fs::File> {
        Err(unimplemented_io("open"))
    }
    fn write(&self, _p: &Path, _data: &[u8]) -> io::Result<()> {
        Err(unimplemented_io("write"))
    }
    fn disk_free(&self, _dir: &Path) -> io::Result<u64> {
        Err(unimplemented_io("disk_free"))
    }
    /// `os.fsync`.
    fn sync_file(&self, _f: &fs::File) -> io::Result<()> {
        Err(unimplemented_io("sync_file"))
    }
    /// `fcntl.flock(LOCK_EX)`. 논블로킹이면 이미 잠겨 있을 때 `ErrorKind::WouldBlock`(BlockingIOError)이다.
    fn lock_exclusive(&self, _f: &fs::File, _nonblocking: bool) -> io::Result<()> {
        Err(unimplemented_io("lock_exclusive"))
    }
}

pub struct RealFs;

impl FsOps for RealFs {
    fn read_dir(&self, p: &Path) -> io::Result<Vec<PathBuf>> {
        fs::read_dir(p)?.map(|e| e.map(|e| e.path())).collect()
    }

    fn metadata(&self, p: &Path) -> io::Result<fs::Metadata> {
        fs::metadata(p)
    }

    fn is_file(&self, p: &Path) -> io::Result<bool> {
        match fs::metadata(p) {
            Ok(m) => Ok(m.is_file()),
            Err(e) if matches!(e.kind(), io::ErrorKind::NotFound | io::ErrorKind::NotADirectory) => Ok(false),
            Err(e) => Err(e),
        }
    }

    fn rename(&self, from: &Path, to: &Path) -> io::Result<()> {
        fs::rename(from, to)
    }

    fn remove_file(&self, p: &Path) -> io::Result<()> {
        fs::remove_file(p)
    }

    fn remove_dir(&self, p: &Path) -> io::Result<()> {
        fs::remove_dir(p)
    }

    fn remove_dir_all(&self, p: &Path) -> io::Result<()> {
        fs::remove_dir_all(p)
    }

    fn canonicalize(&self, p: &Path) -> io::Result<PathBuf> {
        fs::canonicalize(p)
    }

    fn same_file(&self, a: &Path, b: &Path) -> io::Result<bool> {
        let (a, b) = (fs::metadata(a)?, fs::metadata(b)?);
        Ok(a.dev() == b.dev() && a.ino() == b.ino())
    }

    fn open(&self, p: &Path) -> io::Result<fs::File> {
        fs::File::open(p)
    }

    fn write(&self, p: &Path, data: &[u8]) -> io::Result<()> {
        fs::write(p, data)
    }

    fn disk_free(&self, dir: &Path) -> io::Result<u64> {
        use std::os::unix::ffi::OsStrExt;
        let c = std::ffi::CString::new(dir.as_os_str().as_bytes()).map_err(|e| io::Error::new(io::ErrorKind::InvalidInput, e))?;
        let mut st: libc::statvfs = unsafe { std::mem::zeroed() };
        // SAFETY: `c`는 NUL로 끝나는 유효한 C 문자열이고 `st`는 쓸 수 있는 구조체다.
        if unsafe { libc::statvfs(c.as_ptr(), &mut st) } != 0 {
            return Err(io::Error::last_os_error());
        }
        // 플랫폼에 따라 필드 타입이 u32일 수 있어 캐스트를 남긴다.
        #[allow(clippy::unnecessary_cast)]
        Ok(st.f_bavail as u64 * st.f_frsize as u64)
    }

    fn sync_file(&self, f: &fs::File) -> io::Result<()> {
        f.sync_all()
    }

    fn lock_exclusive(&self, f: &fs::File, nonblocking: bool) -> io::Result<()> {
        let op = libc::LOCK_EX | if nonblocking { libc::LOCK_NB } else { 0 };
        // SAFETY: fd는 `f`가 살아 있는 동안 유효하다.
        if unsafe { libc::flock(f.as_raw_fd(), op) } != 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// 외부 명령
// ---------------------------------------------------------------------------

#[derive(Debug)]
pub enum CommandError {
    Timeout,
    NotFound,
    Failed(String),
}

impl fmt::Display for CommandError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            CommandError::Timeout => write!(f, "명령이 시간 상한을 넘었다"),
            CommandError::NotFound => write!(f, "명령을 찾을 수 없다"),
            CommandError::Failed(msg) => write!(f, "명령 실행 실패: {msg}"),
        }
    }
}

impl std::error::Error for CommandError {}

#[derive(Debug, Clone, Default)]
pub struct CommandOutput {
    pub returncode: i32,
    pub stdout: Vec<u8>,
    pub stderr: Vec<u8>,
}

pub trait CommandRunner: Send + Sync {
    /// `subprocess.run(argv, capture_output=True, timeout=..)`. 시간 상한을 넘으면 프로세스를 죽이고 `Timeout`이다.
    fn run(&self, argv: &[String], timeout: Option<Duration>) -> Result<CommandOutput, CommandError>;
    /// `shutil.which`.
    fn which(&self, program: &str) -> Option<PathBuf>;
}

pub struct SystemRunner;

fn drain<R: Read + Send + 'static>(mut r: R) -> thread::JoinHandle<Vec<u8>> {
    thread::spawn(move || {
        let mut buf = Vec::new();
        let _ = r.read_to_end(&mut buf);
        buf
    })
}

impl CommandRunner for SystemRunner {
    fn run(&self, argv: &[String], timeout: Option<Duration>) -> Result<CommandOutput, CommandError> {
        let (program, args) = argv.split_first().ok_or_else(|| CommandError::Failed("빈 argv".into()))?;
        let mut child = Command::new(program)
            .args(args)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|e| if e.kind() == io::ErrorKind::NotFound { CommandError::NotFound } else { CommandError::Failed(e.to_string()) })?;
        // 파이프가 가득 차서 자식이 멈추지 않게 별도 스레드에서 비운다.
        let out = drain(child.stdout.take().expect("stdout 파이프"));
        let err = drain(child.stderr.take().expect("stderr 파이프"));
        let deadline = timeout.map(|t| Instant::now() + t);
        let status = loop {
            match child.try_wait() {
                Ok(Some(status)) => break status,
                Ok(None) => {}
                Err(e) => return Err(CommandError::Failed(e.to_string())),
            }
            if deadline.is_some_and(|d| Instant::now() >= d) {
                let _ = child.kill();
                let _ = child.wait();
                let _ = (out.join(), err.join());
                return Err(CommandError::Timeout);
            }
            thread::sleep(Duration::from_millis(5));
        };
        let returncode = {
            use std::os::unix::process::ExitStatusExt;
            // Python은 시그널로 죽으면 음수 시그널 번호를 돌려준다.
            status.code().unwrap_or_else(|| -status.signal().unwrap_or(1))
        };
        Ok(CommandOutput { returncode, stdout: out.join().unwrap_or_default(), stderr: err.join().unwrap_or_default() })
    }

    fn which(&self, program: &str) -> Option<PathBuf> {
        use std::os::unix::fs::PermissionsExt;
        let is_exec = |p: &Path| fs::metadata(p).map(|m| m.is_file() && m.permissions().mode() & 0o111 != 0).unwrap_or(false);
        if program.contains('/') {
            let p = PathBuf::from(program);
            return is_exec(&p).then_some(p);
        }
        std::env::split_paths(&std::env::var_os("PATH")?).map(|d| d.join(program)).find(|p| is_exec(p))
    }
}

/// 시간 상한 안에 끝나지 않았다.
#[derive(Debug)]
pub struct TimedOut;

/// 타임아웃 때 kill/wait 호출을 검증해야 하는 자식 프로세스(epubcheck 등).
#[async_trait::async_trait]
pub trait ChildProcess: Send {
    async fn communicate(&mut self, timeout: Duration) -> Result<(Vec<u8>, Vec<u8>), TimedOut>;
    fn returncode(&self) -> Option<i32>;
    fn kill(&mut self);
    async fn wait(&mut self);
}

pub trait ProcessSpawner: Send + Sync {
    fn spawn(&self, argv: &[String]) -> io::Result<Box<dyn ChildProcess>>;
}

// ---------------------------------------------------------------------------
// HTTP
// ---------------------------------------------------------------------------

#[derive(Debug)]
pub enum HttpError {
    Timeout,
    Connection(String),
    Other(String),
}

impl fmt::Display for HttpError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            HttpError::Timeout => write!(f, "HTTP 요청 시간 초과"),
            HttpError::Connection(msg) => write!(f, "HTTP 연결 실패: {msg}"),
            HttpError::Other(msg) => write!(f, "HTTP 오류: {msg}"),
        }
    }
}

impl std::error::Error for HttpError {}

pub trait BodyStream: Send {
    fn next_chunk(&mut self) -> Option<Result<Vec<u8>, HttpError>>;
    /// 여러 번 불러도 안전해야 한다.
    fn close(&mut self);
}

pub struct HttpResponse {
    pub status_code: u16,
    pub headers: Vec<(String, String)>,
    body: Box<dyn BodyStream>,
}

struct ChunkBody {
    chunks: std::vec::IntoIter<Vec<u8>>,
    closed: Arc<AtomicBool>,
}

impl BodyStream for ChunkBody {
    fn next_chunk(&mut self) -> Option<Result<Vec<u8>, HttpError>> {
        self.chunks.next().map(Ok)
    }

    fn close(&mut self) {
        self.closed.store(true, Ordering::SeqCst);
    }
}

impl HttpResponse {
    pub fn new(status_code: u16, headers: Vec<(String, String)>, body: Box<dyn BodyStream>) -> Self {
        Self { status_code, headers, body }
    }

    /// fake 생성용: 본문이 한 덩어리인 응답.
    pub fn from_text(status: u16, text: &str) -> Self {
        Self::from_chunks(status, Vec::new(), vec![text.as_bytes().to_vec()], Arc::new(AtomicBool::new(false)))
    }

    /// fake 생성용: 청크 본문. 본문을 닫으면 `closed`가 `true`가 된다.
    pub fn from_chunks(status: u16, headers: Vec<(String, String)>, chunks: Vec<Vec<u8>>, closed: Arc<AtomicBool>) -> Self {
        Self::new(status, headers, Box::new(ChunkBody { chunks: chunks.into_iter(), closed }))
    }

    pub fn next_chunk(&mut self) -> Option<Result<Vec<u8>, HttpError>> {
        self.body.next_chunk()
    }

    /// 본문 전체를 UTF-8로 읽는다(잘못된 바이트는 대체 문자). 읽은 뒤 본문을 닫는다.
    pub fn text(mut self) -> String {
        let mut buf = Vec::new();
        while let Some(Ok(chunk)) = self.body.next_chunk() {
            buf.extend_from_slice(&chunk);
        }
        self.body.close();
        String::from_utf8_lossy(&buf).into_owned()
    }

    /// 본문을 JSON으로 해석한다. 해석할 수 없으면 `Null`이다.
    pub fn json(self) -> Value {
        serde_json::from_str(&self.text()).unwrap_or(Value::Null)
    }

    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers.iter().find(|(k, _)| k.eq_ignore_ascii_case(name)).map(|(_, v)| v.as_str())
    }

    pub fn is_redirect(&self) -> bool {
        matches!(self.status_code, 301 | 302 | 303 | 307 | 308)
    }

    pub fn location(&self) -> Option<&str> {
        self.header("location")
    }
}

impl Drop for HttpResponse {
    fn drop(&mut self) {
        self.body.close();
    }
}

impl fmt::Debug for HttpResponse {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("HttpResponse").field("status_code", &self.status_code).field("headers", &self.headers).finish_non_exhaustive()
    }
}

/// Per-request network timeouts. `read` bounds response-header and per-read body inactivity.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct HttpTimeout {
    pub connect: Duration,
    pub read: Duration,
}

impl HttpTimeout {
    pub const fn from_secs(connect: u64, read: u64) -> Self {
        Self { connect: Duration::from_secs(connect), read: Duration::from_secs(read) }
    }
}

pub trait HttpSession: Send + Sync {
    fn get(&self, url: &str, params: &[(String, String)]) -> Result<HttpResponse, HttpError>;

    /// Request with explicit timeouts. Implementors may keep the existing `get` behavior.
    fn get_with_timeout(&self, url: &str, params: &[(String, String)], _timeout: HttpTimeout) -> Result<HttpResponse, HttpError> {
        self.get(url, params)
    }
}
