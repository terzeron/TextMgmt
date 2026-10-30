//! OLE 복합 파일(CFB, MS-CFB)을 관대하게 읽는 파서.
//!
//! 정상 파서는 손상된 구조를 오류로 거부한다. 이 crate는 읽을 수 있는 만큼 읽고, 파일이 어떻게 비정상인지는 [`Anomaly`] 목록으로 돌려준다.
//! 악의적인 입력을 가정한다: 모든 인덱스는 `get`으로 읽고, 섹터 체인은 방문 표시로 순환을 끊고, 할당량은 [`Limits`]와 파일 크기로 막는다.
//! panic하지 않는다. 디렉터리는 형제·자식 트리를 믿지 않고 항목을 선형으로 읽는다(트리는 부모 경로를 알아내는 데만 쓰고, 순환은 끊는다).
//!
//! 특수 섹터 값은 MS-CFB 2.3(FAT 섹터)을 따른다: 일반 섹터 번호는 `0`부터 `0xFFFFFFF9`까지이고,
//! `MAXREGSECT = 0xFFFFFFFA`, `DIFSECT = 0xFFFFFFFC`, `FATSECT = 0xFFFFFFFD`, `ENDOFCHAIN = 0xFFFFFFFE`, `FREESECT = 0xFFFFFFFF`다.
//! 헤더·디렉터리 항목의 필드 위치는 MS-CFB 2.2와 2.6.1을 따른다(`docs/superpowers/plans/2026-10-30-bookdoctor-p2.md`의 표).

use std::collections::HashSet;
use std::fs::File;
use std::io::Read;
use std::path::Path;

const SIGNATURE: [u8; 8] = [0xD0, 0xCF, 0x11, 0xE0, 0xA1, 0xB1, 0x1A, 0xE1];
const HEADER_LEN: usize = 512;
const ENTRY_LEN: usize = 128;
/// 일반 섹터 번호의 최댓값(`MAXREGSECT`)이다. 이보다 큰 값은 특수 값이다.
const MAXREGSECT: u32 = 0xFFFF_FFFA;
const ENDOFCHAIN: u32 = 0xFFFF_FFFE;
const FREESECT: u32 = 0xFFFF_FFFF;
const NOSTREAM: u32 = 0xFFFF_FFFF;
/// 미니 스트림 한도: 이보다 작은 스트림은 미니 FAT·미니 스트림에 있다.
const MINI_CUTOFF: u64 = 4096;
const MINI_SECTOR: usize = 64;

/// 읽기 상한.
#[derive(Debug, Clone, Copy)]
pub struct Limits {
    /// 스트림 하나에서 읽는 최대 바이트.
    pub max_stream: u64,
    /// 파일 전체 크기와 읽는 바이트의 합 상한.
    pub max_total: u64,
    /// 디렉터리 항목 수 상한.
    pub max_entries: usize,
}

impl Default for Limits {
    fn default() -> Self {
        Limits { max_stream: 64 * 1024 * 1024, max_total: 256 * 1024 * 1024, max_entries: 100_000 }
    }
}

/// 항목의 종류.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EntryKind {
    Unallocated,
    Storage,
    Stream,
    Root,
    /// 명세가 정하지 않은 값.
    Invalid(u8),
}

/// 디렉터리 항목 하나.
#[derive(Debug, Clone)]
pub struct Entry {
    pub id: u32,
    pub name: String,
    pub kind: EntryKind,
    /// 첫 섹터(스트림), 루트는 미니 스트림의 첫 섹터.
    pub start: u32,
    /// 선언한 크기. 주버전 3이면 상위 32비트를 무시한다.
    pub size: u64,
    pub clsid: [u8; 16],
    /// 부모 저장소의 항목 ID. 트리를 따라가지 못한 항목은 `None`이다.
    pub parent: Option<u32>,
}

/// 파일이 비정상인 곳. 읽을 수 있는 만큼 읽은 뒤에 모은다.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Anomaly {}

/// 읽을 수 없는 경우.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OleError {
    /// OLE 시그니처가 아니다.
    NotOle,
    /// 시그니처는 맞지만 헤더(512바이트)보다 작다.
    TooSmall,
    Unreadable(String),
    /// 상한을 넘는다.
    Limit(String),
}

/// 열린 OLE 복합 파일.
pub struct OleFile {
    data: Vec<u8>,
    sector_size: usize,
    major: u16,
    fat: Vec<u32>,
    minifat: Vec<u32>,
    mini_stream: Vec<u8>,
    entries: Vec<Entry>,
    /// 항목마다 [왼쪽, 오른쪽, 자식] 형제 ID(원본 그대로).
    siblings: Vec<[u32; 3]>,
    anomalies: Vec<Anomaly>,
    limits: Limits,
    spent: u64,
}

fn u16_at(b: &[u8], at: usize) -> Option<u16> {
    b.get(at..at + 2).map(|s| u16::from_le_bytes([s[0], s[1]]))
}

fn u32_at(b: &[u8], at: usize) -> Option<u32> {
    b.get(at..at + 4).map(|s| u32::from_le_bytes([s[0], s[1], s[2], s[3]]))
}

fn u64_at(b: &[u8], at: usize) -> Option<u64> {
    b.get(at..at + 8).map(|s| u64::from_le_bytes([s[0], s[1], s[2], s[3], s[4], s[5], s[6], s[7]]))
}

impl OleFile {
    /// 파일을 읽어 연다. 파일 크기가 `limits.max_total`을 넘으면 읽지 않고 `Limit` 오류다.
    pub fn open(path: &Path, limits: &Limits) -> Result<OleFile, OleError> {
        let mut file = File::open(path).map_err(|e| OleError::Unreadable(e.to_string()))?;
        let len = file.metadata().map_err(|e| OleError::Unreadable(e.to_string()))?.len();
        if len > limits.max_total {
            return Err(OleError::Limit(format!("file is {len} bytes (max {})", limits.max_total)));
        }
        let mut data = Vec::with_capacity(len as usize);
        (&mut file).take(limits.max_total + 1).read_to_end(&mut data).map_err(|e| OleError::Unreadable(e.to_string()))?;
        OleFile::from_bytes(data, limits)
    }

    /// 메모리의 바이트로 연다.
    pub fn from_bytes(data: Vec<u8>, limits: &Limits) -> Result<OleFile, OleError> {
        if data.len() < SIGNATURE.len() || data[..8] != SIGNATURE {
            return Err(OleError::NotOle);
        }
        if data.len() < HEADER_LEN {
            return Err(OleError::TooSmall);
        }
        if data.len() as u64 > limits.max_total {
            return Err(OleError::Limit(format!("file is {} bytes (max {})", data.len(), limits.max_total)));
        }
        let major = u16_at(&data, 26).unwrap_or(3);
        let shift = u16_at(&data, 30).unwrap_or(9);
        // 명세는 주버전 3이면 9, 4이면 12다. 어긋나면 주버전에 맞춰 읽는다.
        let sector_size = match (major, shift) {
            (3, 9) | (4, 12) => 1usize << shift,
            (4, _) => 4096,
            _ => 512,
        };
        let mut file = OleFile { data, sector_size, major, fat: Vec::new(), minifat: Vec::new(), mini_stream: Vec::new(), entries: Vec::new(), siblings: Vec::new(), anomalies: Vec::new(), limits: *limits, spent: 0 };
        file.read_fat();
        file.read_directory();
        file.read_minifat();
        file.link_parents();
        if file.entries.is_empty() {
            return Err(OleError::Unreadable("no directory entries could be read".into()));
        }
        Ok(file)
    }

    /// 파일에 들어 있는 섹터 수(헤더 섹터 제외).
    fn sector_count(&self) -> usize {
        (self.data.len() / self.sector_size).saturating_sub(1)
    }

    /// 섹터 하나의 바이트. 범위를 벗어나거나 잘렸으면 `None`이다.
    fn sector(&self, id: u32) -> Option<&[u8]> {
        if id > MAXREGSECT {
            return None;
        }
        let start = (id as usize).checked_add(1)?.checked_mul(self.sector_size)?;
        self.data.get(start..start.checked_add(self.sector_size)?)
    }

    fn read_fat(&mut self) {
        let per = self.sector_size / 4;
        let max_sectors = self.sector_count();
        let mut fat_sectors: Vec<u32> = Vec::new();
        for i in 0..109 {
            if let Some(s) = u32_at(&self.data, 76 + i * 4)
                && s <= MAXREGSECT
            {
                fat_sectors.push(s);
            }
        }
        // DIFAT 사슬: 각 섹터의 마지막 항목이 다음 DIFAT 섹터다.
        let mut next = u32_at(&self.data, 68).unwrap_or(ENDOFCHAIN);
        let mut seen = HashSet::new();
        while next <= MAXREGSECT && seen.len() < max_sectors && seen.insert(next) {
            let Some(sector) = self.sector(next) else { break };
            for k in 0..per.saturating_sub(1) {
                if let Some(s) = u32_at(sector, k * 4)
                    && s <= MAXREGSECT
                {
                    fat_sectors.push(s);
                }
            }
            next = u32_at(sector, (per - 1) * 4).unwrap_or(ENDOFCHAIN);
        }
        let mut seen_fat = HashSet::new();
        let mut table = Vec::new();
        for s in fat_sectors {
            if !seen_fat.insert(s) || seen_fat.len() > max_sectors {
                continue;
            }
            if let Some(sector) = self.sector(s) {
                for k in 0..per {
                    table.push(u32_at(sector, k * 4).unwrap_or(FREESECT));
                }
            }
        }
        self.fat = table;
    }

    /// FAT를 따라 섹터 번호 사슬을 모은다. 순환은 방문 표시로, 길이는 파일의 섹터 수로 끊는다.
    fn chain(&self, table: &[u32], start: u32) -> Vec<u32> {
        let mut out = Vec::new();
        let mut seen = HashSet::new();
        let mut cur = start;
        let limit = table.len().min(self.sector_count().max(table.len().min(1 << 20)));
        while cur <= MAXREGSECT && out.len() <= limit && seen.insert(cur) {
            out.push(cur);
            cur = table.get(cur as usize).copied().unwrap_or(ENDOFCHAIN);
        }
        out
    }

    fn read_directory(&mut self) {
        let start = u32_at(&self.data, 48).unwrap_or(ENDOFCHAIN);
        let chain = self.chain(&self.fat, start);
        let mut raw = Vec::new();
        for s in chain {
            let Some(sector) = self.sector(s) else { break };
            raw.extend_from_slice(sector);
            if raw.len() / ENTRY_LEN > self.limits.max_entries {
                break;
            }
        }
        let count = (raw.len() / ENTRY_LEN).min(self.limits.max_entries);
        for id in 0..count {
            let e = &raw[id * ENTRY_LEN..(id + 1) * ENTRY_LEN];
            let name_len = (u16_at(e, 64).unwrap_or(0) as usize).min(64) & !1;
            let units: Vec<u16> = (0..name_len / 2).map(|i| u16::from_le_bytes([e[i * 2], e[i * 2 + 1]])).collect();
            let units = units.split(|&u| u == 0).next().unwrap_or(&[]);
            let kind = match e[66] {
                0 => EntryKind::Unallocated,
                1 => EntryKind::Storage,
                2 => EntryKind::Stream,
                5 => EntryKind::Root,
                other => EntryKind::Invalid(other),
            };
            let raw_size = u64_at(e, 120).unwrap_or(0);
            let size = if self.major == 3 { raw_size & 0xFFFF_FFFF } else { raw_size };
            let mut clsid = [0u8; 16];
            clsid.copy_from_slice(&e[80..96]);
            self.entries.push(Entry { id: id as u32, name: String::from_utf16_lossy(units), kind, start: u32_at(e, 116).unwrap_or(ENDOFCHAIN), size, clsid, parent: None });
            self.siblings.push([u32_at(e, 68).unwrap_or(NOSTREAM), u32_at(e, 72).unwrap_or(NOSTREAM), u32_at(e, 76).unwrap_or(NOSTREAM)]);
        }
    }

    fn read_minifat(&mut self) {
        let start = u32_at(&self.data, 60).unwrap_or(ENDOFCHAIN);
        let mut table = Vec::new();
        for s in self.chain(&self.fat, start) {
            let Some(sector) = self.sector(s) else { break };
            for k in 0..self.sector_size / 4 {
                table.push(u32_at(sector, k * 4).unwrap_or(FREESECT));
            }
        }
        self.minifat = table;
        // 미니 스트림 컨테이너는 루트 항목의 사슬이다. 읽는 바이트는 루트가 선언한 크기까지다.
        let Some(root) = self.entries.first().filter(|e| e.kind == EntryKind::Root).cloned() else { return };
        let want = root.size.min(self.limits.max_stream) as usize;
        let mut buf = Vec::new();
        for s in self.chain(&self.fat, root.start) {
            let Some(sector) = self.sector(s) else { break };
            buf.extend_from_slice(sector);
            if buf.len() >= want {
                break;
            }
        }
        buf.truncate(want);
        self.mini_stream = buf;
    }

    /// 형제·자식 트리를 따라 각 항목의 부모를 정한다. 순환은 방문 표시로 끊는다.
    fn link_parents(&mut self) {
        let n = self.entries.len();
        let mut visited = vec![false; n];
        if n > 0 {
            visited[0] = true;
        }
        let mut stack: Vec<(u32, u32)> = Vec::new(); // (저장소 ID, 이 저장소의 자식 트리 노드 ID)
        if let Some(sib) = self.siblings.first() {
            stack.push((0, sib[2]));
        }
        while let Some((parent, node)) = stack.pop() {
            let Some(idx) = (node != NOSTREAM).then_some(node as usize).filter(|&i| i < n) else { continue };
            if visited[idx] {
                continue;
            }
            visited[idx] = true;
            self.entries[idx].parent = Some(parent);
            let [left, right, child] = self.siblings[idx];
            stack.push((parent, left));
            stack.push((parent, right));
            if matches!(self.entries[idx].kind, EntryKind::Storage | EntryKind::Root) {
                stack.push((idx as u32, child));
            }
        }
    }

    /// 읽은 디렉터리 항목 전체(루트가 앞).
    pub fn entries(&self) -> &[Entry] {
        &self.entries
    }

    /// 주버전(3 또는 4. 헤더가 다른 값을 적었으면 그 값).
    pub fn major_version(&self) -> u16 {
        self.major
    }

    /// 읽으면서 발견한 비정상 목록.
    pub fn anomalies(&self) -> &[Anomaly] {
        &self.anomalies
    }

    /// `/`로 이은 경로(`"Macros/VBA/dir"`)로 항목을 찾는다. 루트 아래에서 시작하고 이름은 대소문자를 가리지 않는다.
    pub fn find(&self, path: &str) -> Option<&Entry> {
        let mut parent = 0u32;
        let mut found: Option<&Entry> = None;
        for part in path.split('/') {
            let lower = part.to_lowercase();
            let hit = self.entries.iter().find(|e| e.id != 0 && e.parent == Some(parent) && e.name.to_lowercase() == lower)?;
            parent = hit.id;
            found = Some(hit);
        }
        found
    }

    /// 스트림의 바이트를 읽는다. 선언 크기가 `max_stream`을 넘으면 할당하지 않고 `Limit` 오류다.
    /// 사슬이 선언 크기보다 짧으면 읽은 만큼만 돌려준다.
    pub fn read_stream(&mut self, entry: &Entry) -> Result<Vec<u8>, OleError> {
        if entry.size > self.limits.max_stream {
            return Err(OleError::Limit(format!("stream {:?} declares {} bytes (max {})", entry.name, entry.size, self.limits.max_stream)));
        }
        if self.spent.saturating_add(entry.size) > self.limits.max_total {
            return Err(OleError::Limit("total read limit reached".into()));
        }
        let size = entry.size as usize;
        let mut out = Vec::with_capacity(size.min(self.data.len()));
        if entry.size < MINI_CUTOFF && entry.kind != EntryKind::Root {
            for s in self.chain(&self.minifat, entry.start) {
                let at = (s as usize).saturating_mul(MINI_SECTOR);
                let Some(piece) = self.mini_stream.get(at..at.saturating_add(MINI_SECTOR).min(self.mini_stream.len())) else { break };
                out.extend_from_slice(piece);
                if out.len() >= size {
                    break;
                }
            }
        } else {
            for s in self.chain(&self.fat, entry.start) {
                let Some(sector) = self.sector(s) else { break };
                out.extend_from_slice(sector);
                if out.len() >= size {
                    break;
                }
            }
        }
        out.truncate(size);
        self.spent += out.len() as u64;
        Ok(out)
    }
}
