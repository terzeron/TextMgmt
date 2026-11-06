//! 실행 파일 확장자 표. 검사 맥락마다 실행 파일로 보는 확장자가 달라서, 공통 표를 물려받아 넓히는 계층으로 둔다.
//!
//! ```text
//! NATIVE_CORE                 모든 맥락이 실행 파일로 보는 확장자
//! ├─ NATIVE_BINARY            파일 자체가 실행 바이너리인지 볼 때(형식 판별, 디렉터리 순회)
//! └─ ARCHIVE_MEMBER           문서 안의 멤버가 실행 파일인지 볼 때 공통으로 더하는 스크립트·설치 형식
//!    ├─ EPUB_MEMBER           EPUB 멤버
//!    └─ ZIP_MEMBER            ZIP·OOXML·HWP 계열 멤버
//! ```
//!
//! 자식 표는 부모 표의 확장자를 모두 포함한다. 비교는 정확히 같은지 본다. 대소문자를 무시하려면 [`ExecExtensions::contains_ignore_case`]를 쓴다.

/// 실행 파일 확장자 표 하나. 부모 표를 물려받고 자기 확장자를 더한다.
#[derive(Debug)]
pub struct ExecExtensions {
    parent: Option<&'static ExecExtensions>,
    own: &'static [&'static str],
}

impl ExecExtensions {
    /// 이 표나 조상 표에 `ext`가 있는가(정확히 같은지 본다).
    pub fn contains(&self, ext: &str) -> bool {
        self.own.contains(&ext) || self.parent.is_some_and(|p| p.contains(ext))
    }

    /// 대소문자를 무시하고 `contains`.
    pub fn contains_ignore_case(&self, ext: &str) -> bool {
        self.own.iter().any(|e| e.eq_ignore_ascii_case(ext)) || self.parent.is_some_and(|p| p.contains_ignore_case(ext))
    }

    /// 조상 표부터 이 표까지의 확장자 전부.
    pub fn all(&self) -> Vec<&'static str> {
        let mut out = self.parent.map(ExecExtensions::all).unwrap_or_default();
        out.extend_from_slice(self.own);
        out
    }
}

/// 모든 맥락이 실행 파일로 보는 확장자.
pub static NATIVE_CORE: ExecExtensions = ExecExtensions { parent: None, own: &["exe", "dll", "scr", "cpl", "so", "dylib"] };

/// 파일 자체가 실행 바이너리인지 볼 때 쓰는 표. 드라이버·커널 모듈·펌웨어·링커 출력 같은 바이너리 형식을 더한다.
/// 디렉터리를 훑을 때 이 확장자도 모은다.
pub static NATIVE_BINARY: ExecExtensions = ExecExtensions { parent: Some(&NATIVE_CORE), own: &["sys", "ocx", "drv", "efi", "elf", "ko", "bundle", "bin", "out"] };

/// 문서 안의 멤버가 실행 파일인지 볼 때 공통으로 더하는 스크립트·설치·런타임 형식.
pub static ARCHIVE_MEMBER: ExecExtensions = ExecExtensions {
    parent: Some(&NATIVE_CORE),
    own: &[
        "com", "bat", "cmd", "ps1", "psm1", "vbs", "vbe", "jse", "wsf", "wsh", "hta", "msi", "msp", "jar", "lnk", "reg", "sh", "bash", "zsh", "py", "pl", "rb", "php", "apk", "app", "dmg", "run",
        "swf", "class",
    ],
};

/// EPUB 멤버. `bin`을 더한다. `js`는 EPUB이 따로 다루므로 넣지 않는다.
pub static EPUB_MEMBER: ExecExtensions = ExecExtensions { parent: Some(&ARCHIVE_MEMBER), own: &["bin"] };

/// ZIP·문서 패키지 멤버. 데이터로 흔한 `bin`은 넣지 않는다(내용이 실행 파일이면 매직 검사가 잡는다).
/// ZIP·문서 패키지에서 흔한 전달 수단인 `js`, 디스크 이미지, 설치·도움말 형식을 더한다.
pub static ZIP_MEMBER: ExecExtensions =
    ExecExtensions { parent: Some(&ARCHIVE_MEMBER), own: &["js", "pif", "msc", "scf", "url", "iso", "img", "vhd", "vhdx", "chm", "xll", "wsc", "one", "appx", "msix"] };

/// 파일 확장자가 실행 바이너리인가(대소문자 무시).
pub fn is_exec_extension(ext: &str) -> bool {
    NATIVE_BINARY.contains_ignore_case(ext)
}
