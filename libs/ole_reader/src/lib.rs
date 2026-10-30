//! OLE 복합 파일(CFB, MS-CFB)을 관대하게 읽는 파서.
//!
//! 정상 파서는 손상된 구조를 오류로 거부한다. 이 crate는 읽을 수 있는 만큼 읽고, 파일이 어떻게 비정상인지는 [`Anomaly`] 목록으로 돌려준다.
//! 악의적인 입력을 가정한다: 모든 인덱스는 `get`으로 읽고, 섹터 체인은 방문 표시로 순환을 끊고, 할당량은 [`Limits`]와 파일 크기로 막는다.
//! panic하지 않는다. 디렉터리는 형제·자식 트리를 믿지 않고 항목을 선형으로 읽는다(트리는 부모 경로를 알아내는 데만 쓰고, 순환은 끊는다).
//!
//! 특수 섹터 값은 MS-CFB 2.3(FAT 섹터)을 따른다: 일반 섹터 번호는 `0`부터 `0xFFFFFFF9`까지이고,
//! `MAXREGSECT = 0xFFFFFFFA`, `DIFSECT = 0xFFFFFFFC`, `FATSECT = 0xFFFFFFFD`, `ENDOFCHAIN = 0xFFFFFFFE`, `FREESECT = 0xFFFFFFFF`다.
//! 헤더·디렉터리 항목의 필드 위치는 MS-CFB 2.2와 2.6.1을 따른다(`docs/superpowers/plans/2026-10-30-bookdoctor-p2.md`의 표).

pub mod vba;

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
pub enum Anomaly {
    /// 바이트 순서 표지가 `0xFFFE`가 아니다.
    BadByteOrder,
    /// 섹터 시프트가 주버전과 맞지 않는다(3이면 9, 4이면 12). 주버전에 맞춰 계속 읽는다.
    BadSectorShift(u16),
    /// 미니 섹터 시프트가 6이 아니다.
    BadMiniSectorShift(u16),
    /// 미니 스트림 한도가 4096이 아니다.
    BadMiniCutoff(u32),
    /// 섹터 사슬이 순환한다. 값은 사슬이 시작하는 섹터다.
    FatChainCycle { start: u32 },
    /// 사슬이 FAT 밖이나 파일 밖의 섹터, 또는 특수 값으로 이어진다.
    SectorOutOfRange { sector: u32 },
    /// 헤더가 적은 FAT 섹터 수와 실제로 읽은 수가 다르다.
    FatSectorCountMismatch { header: u32, found: u32 },
    /// 헤더가 적은 DIFAT 섹터 수만큼 사슬이 이어지지 않는다.
    DifatTruncated,
    /// 디렉터리 사슬이 파일 끝을 넘는다.
    DirectoryTruncated,
    /// 디렉터리 사슬이 순환한다.
    DirectoryCycle,
    /// 디렉터리 항목이 상한보다 많다. 값은 읽은 항목 수다.
    TooManyEntries(usize),
    /// 스트림이 파일 전체보다 크다고 선언한다.
    StreamSizeExceedsFile { id: u32, size: u64 },
    /// 작은 스트림이 있는데 미니 스트림이 없다.
    MiniStreamMissing,
    /// 서로 다른 스트림이 같은 섹터를 쓴다.
    CrossLinkedSectors { sector: u32 },
    /// 마지막 섹터 뒤에 남는 바이트 수.
    TrailingGarbage(u64),
    /// 이름 길이 필드가 실제 이름과 맞지 않는다.
    NameLengthMismatch { id: u32 },
    /// 명세가 정하지 않은 객체 종류.
    InvalidObjectType { id: u32, value: u8 },
}

impl Anomaly {
    /// 보고서에 쓰는 안정적인 이름.
    pub fn code(&self) -> &'static str {
        match self {
            Anomaly::BadByteOrder => "OLE_BAD_BYTE_ORDER",
            Anomaly::BadSectorShift(_) => "OLE_BAD_SECTOR_SHIFT",
            Anomaly::BadMiniSectorShift(_) => "OLE_BAD_MINI_SECTOR_SHIFT",
            Anomaly::BadMiniCutoff(_) => "OLE_BAD_MINI_CUTOFF",
            Anomaly::FatChainCycle { .. } => "OLE_FAT_CYCLE",
            Anomaly::SectorOutOfRange { .. } => "OLE_SECTOR_OUT_OF_RANGE",
            Anomaly::FatSectorCountMismatch { .. } => "OLE_FAT_COUNT_MISMATCH",
            Anomaly::DifatTruncated => "OLE_DIFAT_TRUNCATED",
            Anomaly::DirectoryTruncated => "OLE_DIRECTORY_TRUNCATED",
            Anomaly::DirectoryCycle => "OLE_DIRECTORY_CYCLE",
            Anomaly::TooManyEntries(_) => "OLE_TOO_MANY_ENTRIES",
            Anomaly::StreamSizeExceedsFile { .. } => "OLE_STREAM_SIZE_EXCEEDS_FILE",
            Anomaly::MiniStreamMissing => "OLE_MINI_STREAM_MISSING",
            Anomaly::CrossLinkedSectors { .. } => "OLE_CROSS_LINKED",
            Anomaly::TrailingGarbage(_) => "OLE_TRAILING_GARBAGE",
            Anomaly::NameLengthMismatch { .. } => "OLE_NAME_LENGTH_MISMATCH",
            Anomaly::InvalidObjectType { .. } => "OLE_INVALID_OBJECT_TYPE",
        }
    }

    /// 사람이 읽는 설명.
    pub fn detail(&self) -> String {
        match self {
            Anomaly::BadByteOrder => "byte order mark is not 0xFFFE".into(),
            Anomaly::BadSectorShift(v) => format!("sector shift {v} does not match the major version"),
            Anomaly::BadMiniSectorShift(v) => format!("mini sector shift {v} (expected 6)"),
            Anomaly::BadMiniCutoff(v) => format!("mini stream cutoff {v} (expected 4096)"),
            Anomaly::FatChainCycle { start } => format!("sector chain starting at {start} loops"),
            Anomaly::SectorOutOfRange { sector } => format!("sector chain reaches {sector:#x}, outside the FAT or the file"),
            Anomaly::FatSectorCountMismatch { header, found } => format!("header lists {header} FAT sectors but {found} could be read"),
            Anomaly::DifatTruncated => "DIFAT chain ends before the header's sector count".into(),
            Anomaly::DirectoryTruncated => "directory chain continues past the end of the file".into(),
            Anomaly::DirectoryCycle => "directory sector chain loops".into(),
            Anomaly::TooManyEntries(n) => format!("{n} directory entries read (limit exceeded)"),
            Anomaly::StreamSizeExceedsFile { id, size } => format!("stream {id} declares {size} bytes, more than the whole file"),
            Anomaly::MiniStreamMissing => "small streams exist but the mini stream does not".into(),
            Anomaly::CrossLinkedSectors { sector } => format!("sector {sector} is used by more than one stream"),
            Anomaly::TrailingGarbage(n) => format!("{n} bytes after the last full sector"),
            Anomaly::NameLengthMismatch { id } => format!("entry {id}: name length field does not match the name"),
            Anomaly::InvalidObjectType { id, value } => format!("entry {id}: object type {value:#x} is not defined"),
        }
    }
}

/// 사슬을 따라가다 멈춘 이유.
enum ChainEnd {
    /// `ENDOFCHAIN`에 닿았다.
    End,
    Cycle,
    /// 다음 값이 일반 섹터 번호도 `ENDOFCHAIN`도 아니거나 표(FAT) 밖이다.
    Bad(u32),
    /// 호출자가 정한 길이에서 멈췄다(필요한 만큼만 걷는다).
    Cap,
}

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
    /// 실제로 읽은 서로 다른 FAT 섹터 수.
    fat_found: u32,
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
        let mut file = OleFile { data, sector_size, major, fat: Vec::new(), minifat: Vec::new(), mini_stream: Vec::new(), entries: Vec::new(), siblings: Vec::new(), anomalies: Vec::new(), fat_found: 0, limits: *limits, spent: 0 };
        file.read_header_anomalies();
        file.read_fat();
        file.read_directory();
        file.read_minifat();
        file.check_streams();
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

    fn read_header_anomalies(&mut self) {
        if u16_at(&self.data, 28) != Some(0xFFFE) {
            self.anomalies.push(Anomaly::BadByteOrder);
        }
        let shift = u16_at(&self.data, 30).unwrap_or(0);
        if !matches!((self.major, shift), (3, 9) | (4, 12)) {
            self.anomalies.push(Anomaly::BadSectorShift(shift));
        }
        let mini = u16_at(&self.data, 32).unwrap_or(0);
        if mini != 6 {
            self.anomalies.push(Anomaly::BadMiniSectorShift(mini));
        }
        let cutoff = u32_at(&self.data, 56).unwrap_or(0);
        if u64::from(cutoff) != MINI_CUTOFF {
            self.anomalies.push(Anomaly::BadMiniCutoff(cutoff));
        }
        let extra = (self.data.len() % self.sector_size) as u64;
        if extra != 0 {
            self.anomalies.push(Anomaly::TrailingGarbage(extra));
        }
    }

    fn read_fat(&mut self) {
        let per = self.sector_size / 4;
        let max_sectors = self.sector_count();
        // FAT 섹터 목록은 파일에 실제로 있는 섹터 수를 넘지 못한다(파일 밖 번호는 버린다). DIFAT 섹터마다 항목이 수백 개라
        // 상한 없이 모으면 파일 크기의 수십 배를 할당할 수 있다.
        let mut fat_sectors: Vec<u32> = Vec::new();
        let keep = |s: u32, v: &Vec<u32>| s <= MAXREGSECT && (s as usize) < max_sectors && v.len() < max_sectors;
        for i in 0..109 {
            if let Some(s) = u32_at(&self.data, 76 + i * 4)
                && keep(s, &fat_sectors)
            {
                fat_sectors.push(s);
            }
        }
        // DIFAT 사슬: 각 섹터의 마지막 항목이 다음 DIFAT 섹터다. 헤더가 적은 섹터 수만큼 이어지지 않으면 알린다.
        let mut remaining = u32_at(&self.data, 72).unwrap_or(0);
        let mut next = u32_at(&self.data, 68).unwrap_or(ENDOFCHAIN);
        let mut seen = HashSet::new();
        while remaining > 0 {
            if next > MAXREGSECT || seen.len() >= max_sectors || !seen.insert(next) {
                self.anomalies.push(Anomaly::DifatTruncated);
                break;
            }
            let Some(sector) = self.sector(next) else {
                self.anomalies.push(Anomaly::DifatTruncated);
                break;
            };
            for k in 0..per.saturating_sub(1) {
                if let Some(s) = u32_at(sector, k * 4)
                    && keep(s, &fat_sectors)
                {
                    fat_sectors.push(s);
                }
            }
            next = u32_at(sector, (per - 1) * 4).unwrap_or(ENDOFCHAIN);
            remaining -= 1;
        }
        let mut seen_fat = HashSet::new();
        let mut table = Vec::new();
        for s in fat_sectors {
            if seen_fat.len() >= max_sectors {
                break;
            }
            if !seen_fat.insert(s) {
                continue;
            }
            if let Some(sector) = self.sector(s) {
                for k in 0..per {
                    table.push(u32_at(sector, k * 4).unwrap_or(FREESECT));
                }
            }
        }
        self.fat_found = (table.len() / per.max(1)) as u32;
        let header = u32_at(&self.data, 44).unwrap_or(0);
        if header != self.fat_found {
            self.anomalies.push(Anomaly::FatSectorCountMismatch { header, found: self.fat_found });
        }
        self.fat = table;
    }

    /// 표(FAT 또는 미니 FAT)를 따라 섹터 번호 사슬을 모은다. 순환은 방문 표시로 끊고, `max_len`개를 넘게 걷지 않는다.
    /// 읽을 만큼(선언 크기가 덮는 섹터 수 + 1)만 걷게 해서, 긴 사슬을 가리키는 작은 스트림이 일을 키우지 못하게 한다.
    fn chain_ex(table: &[u32], start: u32, max_len: usize) -> (Vec<u32>, ChainEnd) {
        let mut out = Vec::new();
        let mut seen = HashSet::new();
        let mut cur = start;
        loop {
            if cur == ENDOFCHAIN {
                return (out, ChainEnd::End);
            }
            if cur > MAXREGSECT || cur as usize >= table.len() {
                return (out, ChainEnd::Bad(cur));
            }
            if out.len() >= max_len {
                return (out, ChainEnd::Cap);
            }
            if !seen.insert(cur) {
                return (out, ChainEnd::Cycle);
            }
            out.push(cur);
            cur = table[cur as usize];
        }
    }

    fn chain(table: &[u32], start: u32, max_len: usize) -> Vec<u32> {
        Self::chain_ex(table, start, max_len).0
    }

    fn read_directory(&mut self) {
        let start = u32_at(&self.data, 48).unwrap_or(ENDOFCHAIN);
        // 상한이 허용하는 항목 수를 담는 섹터까지만 걷는다.
        let dir_max = (self.limits.max_entries.saturating_mul(ENTRY_LEN)).div_ceil(self.sector_size) + 1;
        let (chain, end) = Self::chain_ex(&self.fat, start, dir_max);
        match end {
            ChainEnd::Cycle => self.anomalies.push(Anomaly::DirectoryCycle),
            ChainEnd::Bad(s) => self.anomalies.push(Anomaly::SectorOutOfRange { sector: s }),
            ChainEnd::End | ChainEnd::Cap => {}
        }
        let mut raw = Vec::new();
        for s in chain {
            let Some(sector) = self.sector(s) else {
                self.anomalies.push(Anomaly::DirectoryTruncated);
                break;
            };
            raw.extend_from_slice(sector);
            if raw.len() / ENTRY_LEN > self.limits.max_entries {
                break;
            }
        }
        let total = raw.len() / ENTRY_LEN;
        if total > self.limits.max_entries {
            self.anomalies.push(Anomaly::TooManyEntries(total));
        }
        let count = total.min(self.limits.max_entries);
        for id in 0..count {
            let e = &raw[id * ENTRY_LEN..(id + 1) * ENTRY_LEN];
            let name_len = u16_at(e, 64).unwrap_or(0) as usize;
            let usable = name_len.min(64) & !1;
            let units: Vec<u16> = (0..usable / 2).map(|i| u16::from_le_bytes([e[i * 2], e[i * 2 + 1]])).collect();
            let units = units.split(|&u| u == 0).next().unwrap_or(&[]);
            // 실제 파일 중에는 안 쓰는 슬롯을 0xFF로 채우는 작성기가 있다. 유형, 시작 섹터, 크기가 모두 0xFF인 슬롯은 빈 슬롯으로 본다.
            let free_slot = e[66] == 0xFF && u32_at(e, 116) == Some(FREESECT) && u64_at(e, 120) == Some(u64::MAX);
            let kind = match e[66] {
                _ if free_slot => EntryKind::Unallocated,
                0 => EntryKind::Unallocated,
                1 => EntryKind::Storage,
                2 => EntryKind::Stream,
                5 => EntryKind::Root,
                other => EntryKind::Invalid(other),
            };
            if let EntryKind::Invalid(value) = kind {
                self.anomalies.push(Anomaly::InvalidObjectType { id: id as u32, value });
            }
            if kind != EntryKind::Unallocated && name_len != (units.len() + 1) * 2 {
                self.anomalies.push(Anomaly::NameLengthMismatch { id: id as u32 });
            }
            let raw_size = u64_at(e, 120).unwrap_or(0);
            let size = if self.major == 3 { raw_size & 0xFFFF_FFFF } else { raw_size };
            let mut clsid = [0u8; 16];
            clsid.copy_from_slice(&e[80..96]);
            self.entries.push(Entry { id: id as u32, name: String::from_utf16_lossy(units), kind, start: u32_at(e, 116).unwrap_or(ENDOFCHAIN), size, clsid, parent: None });
            self.siblings.push([u32_at(e, 68).unwrap_or(NOSTREAM), u32_at(e, 72).unwrap_or(NOSTREAM), u32_at(e, 76).unwrap_or(NOSTREAM)]);
        }
    }

    /// 스트림마다 사슬을 한 번 걸어 순환·범위 밖·교차 연결·크기 모순을 찾는다. 다른 스트림이 쓰는 섹터를 만나면 그 사슬은 거기서 멈춘다
    /// (여러 항목이 같은 긴 사슬을 가리키는 입력에서 일이 항목 수 × 사슬 길이로 늘지 않게 한다).
    fn check_streams(&mut self) {
        let file_len = self.data.len() as u64;
        let mut owner: std::collections::HashMap<u32, u32> = std::collections::HashMap::new();
        let mut mini_owner: std::collections::HashMap<u32, u32> = std::collections::HashMap::new();
        let mut reported: HashSet<u32> = HashSet::new();
        let mut found = Vec::new();
        let mut small_streams = false;
        for i in 0..self.entries.len() {
            let e = self.entries[i].clone();
            if e.kind != EntryKind::Stream || e.size == 0 {
                continue;
            }
            if e.size > file_len {
                found.push(Anomaly::StreamSizeExceedsFile { id: e.id, size: e.size });
            }
            let mini = e.size < MINI_CUTOFF;
            small_streams |= mini;
            let (table, owners) = if mini { (&self.minifat, &mut mini_owner) } else { (&self.fat, &mut owner) };
            let mut seen = HashSet::new();
            let mut cur = e.start;
            loop {
                if cur == ENDOFCHAIN {
                    break;
                }
                if cur > MAXREGSECT || cur as usize >= table.len() {
                    found.push(Anomaly::SectorOutOfRange { sector: cur });
                    break;
                }
                if !seen.insert(cur) {
                    found.push(Anomaly::FatChainCycle { start: e.start });
                    break;
                }
                if let Some(&other) = owners.get(&cur)
                    && other != e.id
                {
                    if reported.insert(cur) {
                        found.push(Anomaly::CrossLinkedSectors { sector: cur });
                    }
                    break;
                }
                owners.insert(cur, e.id);
                if !mini && self.sector(cur).is_none() {
                    found.push(Anomaly::SectorOutOfRange { sector: cur });
                    break;
                }
                cur = table[cur as usize];
            }
        }
        if small_streams && self.mini_stream.is_empty() {
            found.push(Anomaly::MiniStreamMissing);
        }
        self.anomalies.extend(found);
    }

    fn read_minifat(&mut self) {
        let start = u32_at(&self.data, 60).unwrap_or(ENDOFCHAIN);
        let mut table = Vec::new();
        for s in Self::chain(&self.fat, start, self.fat.len()) {
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
        for s in Self::chain(&self.fat, root.start, want.div_ceil(self.sector_size) + 1) {
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

    /// 항목의 경로(루트 아래 `/`로 이은 이름). 루트는 빈 문자열이다. 부모를 따라가지 못한 항목은 이름만 돌려준다.
    pub fn path_of(&self, entry: &Entry) -> String {
        if entry.id == 0 {
            return String::new();
        }
        let mut parts = vec![entry.name.clone()];
        let mut cur = entry.parent;
        // 부모 사슬은 `link_parents`가 방문 표시로 만든 트리라 순환이 없지만, 방어로 길이를 제한한다.
        while let Some(p) = cur.filter(|&p| p != 0).filter(|_| parts.len() < 64) {
            let Some(parent) = self.entries.get(p as usize) else { break };
            parts.push(parent.name.clone());
            cur = parent.parent;
        }
        parts.reverse();
        parts.join("/")
    }

    /// 스트림의 앞 `max`바이트만 읽는다(선언 크기가 더 작으면 그만큼). 큰 스트림을 통째로 읽지 않고 머리만 볼 때 쓴다.
    pub fn read_stream_prefix(&mut self, entry: &Entry, max: usize) -> Result<Vec<u8>, OleError> {
        let size = entry.size.min(max as u64);
        let mut partial = entry.clone();
        partial.size = size;
        // 미니 스트림 여부는 원래 크기로 정해진다. 크기만 줄이면 일반 스트림이 미니 경로를 타므로 원래 종류를 유지한다.
        if (entry.size < MINI_CUTOFF) != (partial.size < MINI_CUTOFF) {
            let mut all = self.read_stream(entry)?;
            all.truncate(size as usize);
            return Ok(all);
        }
        self.read_stream(&partial)
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
            for s in Self::chain(&self.minifat, entry.start, size.div_ceil(MINI_SECTOR) + 1) {
                let at = (s as usize).saturating_mul(MINI_SECTOR);
                let Some(piece) = self.mini_stream.get(at..at.saturating_add(MINI_SECTOR).min(self.mini_stream.len())) else { break };
                out.extend_from_slice(piece);
                if out.len() >= size {
                    break;
                }
            }
        } else {
            for s in Self::chain(&self.fat, entry.start, size.div_ceil(self.sector_size) + 1) {
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

#[cfg(test)]
mod tests {
    use super::*;

    /// 사슬 걷기는 호출자가 정한 길이에서 멈춘다. 긴 사슬을 가리키는 작은 스트림이 표 전체를 훑지 않게 한다.
    #[test]
    fn chain_walk_stops_at_the_requested_length() {
        let table: Vec<u32> = (1..=1_000_000u32).chain(std::iter::once(ENDOFCHAIN)).collect();
        let (out, end) = OleFile::chain_ex(&table, 0, 10);
        assert_eq!(out.len(), 10);
        assert!(matches!(end, ChainEnd::Cap));
        let (all, end) = OleFile::chain_ex(&table, 999_990, usize::MAX);
        assert_eq!(all.len(), 11);
        assert!(matches!(end, ChainEnd::End));
    }

    /// DIFAT 섹터가 파일 밖 섹터 번호를 잔뜩 담아도 FAT 섹터 목록은 파일의 섹터 수를 넘지 않는다.
    #[test]
    fn fat_sector_list_is_bounded_by_the_file_even_with_a_hostile_difat() {
        // 헤더 + DIFAT 섹터 3개(자기들끼리 사슬). 각 DIFAT 섹터의 항목은 전부 파일 밖 번호다.
        let sectors = 3usize;
        let mut data = vec![0u8; 512 * (sectors + 1)];
        data[..8].copy_from_slice(&SIGNATURE);
        data[26..28].copy_from_slice(&3u16.to_le_bytes());
        data[28..30].copy_from_slice(&0xFFFEu16.to_le_bytes());
        data[30..32].copy_from_slice(&9u16.to_le_bytes());
        data[32..34].copy_from_slice(&6u16.to_le_bytes());
        data[56..60].copy_from_slice(&4096u32.to_le_bytes());
        data[48..52].copy_from_slice(&ENDOFCHAIN.to_le_bytes());
        data[68..72].copy_from_slice(&0u32.to_le_bytes());
        data[72..76].copy_from_slice(&3u32.to_le_bytes());
        for i in 0..109 {
            data[76 + i * 4..80 + i * 4].copy_from_slice(&0x00FF_FFFFu32.to_le_bytes());
        }
        for s in 0..sectors {
            let base = 512 * (s + 1);
            for k in 0..127 {
                data[base + k * 4..base + k * 4 + 4].copy_from_slice(&0x00FF_FFFFu32.to_le_bytes());
            }
            let next = if s + 1 < sectors { (s + 1) as u32 } else { ENDOFCHAIN };
            data[base + 127 * 4..base + 128 * 4].copy_from_slice(&next.to_le_bytes());
        }
        let mut f = OleFile { data, sector_size: 512, major: 3, fat: Vec::new(), minifat: Vec::new(), mini_stream: Vec::new(), entries: Vec::new(), siblings: Vec::new(), anomalies: Vec::new(), fat_found: 0, limits: Limits::default(), spent: 0 };
        f.read_fat();
        assert_eq!(f.fat.len(), 0, "파일 밖 번호는 FAT 섹터로 모으지 않는다");
        assert_eq!(f.fat_found, 0);
    }
}
