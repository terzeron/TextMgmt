//! `FileHeader` 스트림(256바이트, 명세 표 3).

/// 서명 32바이트 + 버전 4 + 속성 4 + 속성2 4 + EncryptVersion 4 + KOGL 1 + 예약 207.
pub const HEADER_LEN: usize = 256;
/// 서명은 `"HWP Document File"`이고 32바이트 칸의 나머지는 채움이다.
const SIGNATURE: &[u8] = b"HWP Document File";

/// 읽은 `FileHeader`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FileHeader {
    pub version: u32,
    pub flags: u32,
    pub flags2: u32,
    pub encrypt_version: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HeaderError {
    TooShort(usize),
    BadSignature,
}

impl HeaderError {
    pub fn detail(&self) -> String {
        match self {
            HeaderError::TooShort(len) => format!("FileHeader is {len} bytes (expected {HEADER_LEN})"),
            HeaderError::BadSignature => "FileHeader does not start with \"HWP Document File\"".to_string(),
        }
    }
}

fn u32_at(b: &[u8], at: usize) -> u32 {
    b.get(at..at + 4).map_or(0, |s| u32::from_le_bytes([s[0], s[1], s[2], s[3]]))
}

/// `FileHeader` 바이트를 읽는다. 256바이트 미만이거나 서명이 다르면 오류다.
pub fn parse(bytes: &[u8]) -> Result<FileHeader, HeaderError> {
    if bytes.len() < HEADER_LEN {
        return Err(HeaderError::TooShort(bytes.len()));
    }
    if !bytes.starts_with(SIGNATURE) {
        return Err(HeaderError::BadSignature);
    }
    Ok(FileHeader { version: u32_at(bytes, 32), flags: u32_at(bytes, 36), flags2: u32_at(bytes, 40), encrypt_version: u32_at(bytes, 44) })
}

impl FileHeader {
    fn bit(&self, n: u32) -> bool {
        self.flags & (1 << n) != 0
    }

    pub fn compressed(&self) -> bool {
        self.bit(0)
    }

    pub fn encrypted(&self) -> bool {
        self.bit(1)
    }

    pub fn distributable(&self) -> bool {
        self.bit(2)
    }

    pub fn has_scripts(&self) -> bool {
        self.bit(3)
    }

    pub fn drm(&self) -> bool {
        self.bit(4)
    }

    pub fn cert_encrypted(&self) -> bool {
        self.bit(8)
    }

    pub fn cert_drm(&self) -> bool {
        self.bit(10)
    }

    /// 본문을 읽을 수 없게 하는 설정이 있으면 미완료 사유를 돌려준다. `encrypted: `로 시작해 cyan으로 표시된다.
    pub fn encryption_reason(&self) -> Option<&'static str> {
        if self.encrypted() {
            Some("encrypted: HWP password (FileHeader bit1)")
        } else if self.drm() {
            Some("encrypted: HWP DRM document (FileHeader bit4)")
        } else if self.cert_encrypted() {
            Some("encrypted: HWP certificate encryption (FileHeader bit8)")
        } else if self.cert_drm() {
            Some("encrypted: HWP certificate DRM (FileHeader bit10)")
        } else {
            None
        }
    }
}
