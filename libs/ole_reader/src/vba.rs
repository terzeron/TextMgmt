//! MS-OVBA: VBA 프로젝트의 압축 해제(2.4.1)와 `dir` 스트림에서 모듈 목록 읽기(2.3.4.2).
//!
//! 입력은 악의적일 수 있다. 해제 결과는 `max_out`바이트를 넘지 못하고, 복사 토큰이 청크 앞을 가리키면 panic이 아니라 오류다.
//! 매크로를 실행하거나 해석하지 않는다. 소스를 텍스트로 꺼내기만 한다.
//!
//! 레코드 ID는 MS-OVBA 2.3.4.2.3에서 확인했다: PROJECTMODULES `0x000F`, MODULENAME `0x0019`, MODULESTREAMNAME `0x001A`
//! (MBCS 이름 + 예약 `0x0032` + 유니코드 이름), MODULEOFFSET `0x0031`(TextOffset u32), MODULE 종결자 `0x002B`.

use crate::{Entry, OleFile};

const CHUNK_OUT: usize = 4096;
/// 한 프로젝트에서 읽는 모듈 수 상한.
const MAX_MODULES: usize = 1000;

/// VBA를 읽다가 생기는 오류.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum VbaError {
    /// 압축 컨테이너 서명(첫 바이트 `0x01`)이 아니다.
    NotCompressed,
    /// 청크가 선언한 길이만큼 데이터가 없다.
    Truncated,
    /// 복사 토큰이 해제한 데이터 앞(청크 시작 앞)을 가리킨다.
    BadOffset,
    /// 해제 결과가 상한을 넘는다.
    OutputLimit,
    /// `dir` 스트림을 읽을 수 없다.
    BadDir(String),
}

/// 모듈 하나.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VbaModule {
    pub name: String,
    pub source: String,
}

/// 읽은 프로젝트. 모듈 하나가 망가져도 나머지는 읽는다. 못 읽은 이유는 `errors`에 남는다.
#[derive(Debug, Clone, Default)]
pub struct VbaProject {
    pub modules: Vec<VbaModule>,
    pub errors: Vec<String>,
}

/// 복사 토큰의 오프셋 비트 수: `max(ceil(log2(청크 안에서 지금까지 해제한 바이트 수)), 4)`, 최대 12.
fn bit_count(done_in_chunk: usize) -> u32 {
    let bits = if done_in_chunk <= 1 { 0 } else { usize::BITS - (done_in_chunk - 1).leading_zeros() };
    bits.clamp(4, 12)
}

/// MS-OVBA 압축 컨테이너를 푼다. `max_out`을 넘으면 `OutputLimit`이다.
pub fn decompress(data: &[u8], max_out: usize) -> Result<Vec<u8>, VbaError> {
    if data.first() != Some(&0x01) {
        return Err(VbaError::NotCompressed);
    }
    let mut out: Vec<u8> = Vec::new();
    let mut pos = 1usize;
    while pos < data.len() {
        let header = u16::from_le_bytes([*data.get(pos).ok_or(VbaError::Truncated)?, *data.get(pos + 1).ok_or(VbaError::Truncated)?]);
        let chunk_end = pos + (header & 0x0FFF) as usize + 3;
        if chunk_end > data.len() {
            return Err(VbaError::Truncated);
        }
        let compressed = header & 0x8000 != 0;
        pos += 2;
        let chunk_start = out.len();
        if !compressed {
            let take = (chunk_end - pos).min(CHUNK_OUT);
            if out.len() + take > max_out {
                return Err(VbaError::OutputLimit);
            }
            out.extend_from_slice(&data[pos..pos + take]);
            pos = chunk_end;
            continue;
        }
        while pos < chunk_end {
            let flags = data[pos];
            pos += 1;
            for bit in 0..8 {
                if pos >= chunk_end {
                    break;
                }
                if flags & (1 << bit) == 0 {
                    if out.len() >= max_out {
                        return Err(VbaError::OutputLimit);
                    }
                    out.push(data[pos]);
                    pos += 1;
                } else {
                    if pos + 2 > chunk_end {
                        return Err(VbaError::Truncated);
                    }
                    let token = u16::from_le_bytes([data[pos], data[pos + 1]]);
                    pos += 2;
                    let done = out.len() - chunk_start;
                    let bits = bit_count(done);
                    let length_mask = 0xFFFFu16 >> bits;
                    let length = (token & length_mask) as usize + 3;
                    let offset = ((token & !length_mask) >> (16 - bits)) as usize + 1;
                    if offset > done {
                        return Err(VbaError::BadOffset);
                    }
                    // 한 청크는 4096바이트를 넘게 해제하지 않는다.
                    let length = length.min(CHUNK_OUT.saturating_sub(done));
                    if out.len() + length > max_out {
                        return Err(VbaError::OutputLimit);
                    }
                    for _ in 0..length {
                        let b = out[out.len() - offset];
                        out.push(b);
                    }
                }
            }
        }
    }
    Ok(out)
}

struct ModuleInfo {
    name: String,
    stream: String,
    offset: usize,
}

fn u16_at(b: &[u8], at: usize) -> Option<u16> {
    b.get(at..at + 2).map(|s| u16::from_le_bytes([s[0], s[1]]))
}

fn u32_at(b: &[u8], at: usize) -> Option<u32> {
    b.get(at..at + 4).map(|s| u32::from_le_bytes([s[0], s[1], s[2], s[3]]))
}

/// 압축을 푼 `dir` 스트림에서 모듈 목록을 읽는다. PROJECTMODULES 레코드(`0F 00 02 00 00 00`, 뒤에 모듈 수)를 찾아 거기서부터 읽는다.
/// 그 앞의 레코드(참조 등)는 종류가 많고 안쪽 구조가 제각각이라 건너뛴다.
fn parse_modules(dir: &[u8]) -> Result<Vec<ModuleInfo>, VbaError> {
    let start = dir.windows(6).position(|w| w == [0x0F, 0x00, 0x02, 0x00, 0x00, 0x00]).ok_or_else(|| VbaError::BadDir("no PROJECTMODULES record".into()))?;
    let count = u16_at(dir, start + 6).ok_or_else(|| VbaError::BadDir("truncated PROJECTMODULES".into()))? as usize;
    let mut pos = start + 8;
    let mut modules: Vec<ModuleInfo> = Vec::new();
    let mut current: Option<ModuleInfo> = None;
    while pos + 6 <= dir.len() && modules.len() < count.min(MAX_MODULES) {
        let id = u16_at(dir, pos).unwrap_or(0);
        let size = u32_at(dir, pos + 2).unwrap_or(0) as usize;
        let body = pos + 6;
        let Some(data) = dir.get(body..body.saturating_add(size)) else { break };
        pos = body + size;
        match id {
            0x0019 => current = Some(ModuleInfo { name: String::from_utf8_lossy(data).into_owned(), stream: String::new(), offset: 0 }),
            0x001A => {
                // MBCS 이름 뒤에 예약값(u16)과 유니코드 이름(u32 길이 + 바이트)이 이어진다. 유니코드 이름이 있으면 그것을 쓴다.
                let mut stream = String::from_utf8_lossy(data).into_owned();
                if let (Some(_reserved), Some(usize_len)) = (u16_at(dir, pos), u32_at(dir, pos + 2)) {
                    let usize_len = usize_len as usize;
                    if let Some(uni) = dir.get(pos + 6..(pos + 6).saturating_add(usize_len)) {
                        let units: Vec<u16> = uni.chunks_exact(2).map(|c| u16::from_le_bytes([c[0], c[1]])).collect();
                        stream = String::from_utf16_lossy(&units);
                        pos += 6 + usize_len;
                    }
                }
                if let Some(m) = current.as_mut() {
                    m.stream = stream;
                }
            }
            0x001C => {
                // MODULEDOCSTRING도 같은 모양(예약값과 유니코드 문자열)이 뒤따른다.
                if let (Some(_), Some(len)) = (u16_at(dir, pos), u32_at(dir, pos + 2)) {
                    pos = pos.saturating_add(6).saturating_add(len as usize);
                }
            }
            0x0031 => {
                if let (Some(m), Some(off)) = (current.as_mut(), u32_at(data, 0)) {
                    m.offset = off as usize;
                }
            }
            0x002B => {
                if let Some(m) = current.take() {
                    modules.push(m);
                }
            }
            _ => {}
        }
    }
    Ok(modules)
}

/// `vba_root`(예: `"Macros/VBA"`) 아래의 `dir` 스트림을 풀어 모듈을 읽는다. 해제한 바이트의 합은 `max_total`을 넘지 않는다.
pub fn read_project(file: &mut OleFile, vba_root: &str, max_total: u64) -> Result<VbaProject, VbaError> {
    let dir_entry: Entry = file.find(&format!("{vba_root}/dir")).cloned().ok_or_else(|| VbaError::BadDir("no dir stream".into()))?;
    let raw = file.read_stream(&dir_entry).map_err(|e| VbaError::BadDir(format!("{e:?}")))?;
    let mut remaining = usize::try_from(max_total).unwrap_or(usize::MAX);
    let dir = decompress(&raw, remaining)?;
    remaining = remaining.saturating_sub(dir.len());
    let infos = parse_modules(&dir)?;
    let mut project = VbaProject::default();
    for info in infos {
        let Some(entry) = file.find(&format!("{vba_root}/{}", info.stream)).cloned() else {
            project.errors.push(format!("{}: module stream {:?} not found", info.name, info.stream));
            continue;
        };
        let data = match file.read_stream(&entry) {
            Ok(d) => d,
            Err(e) => {
                project.errors.push(format!("{}: {e:?}", info.name));
                continue;
            }
        };
        let Some(tail) = data.get(info.offset..) else {
            project.errors.push(format!("{}: text offset {} is past the end of the stream ({} bytes)", info.name, info.offset, data.len()));
            continue;
        };
        match decompress(tail, remaining) {
            Ok(text) => {
                remaining = remaining.saturating_sub(text.len());
                project.modules.push(VbaModule { name: info.name, source: String::from_utf8_lossy(&text).into_owned() });
            }
            Err(e) => {
                project.errors.push(format!("{}: {e:?}", info.name));
                if e == VbaError::OutputLimit {
                    break;
                }
            }
        }
    }
    Ok(project)
}
