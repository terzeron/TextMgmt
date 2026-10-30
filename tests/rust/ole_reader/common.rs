//! 테스트용 CFB 파일 만들기. 주버전 3(512바이트 섹터)과 4(4096바이트 섹터), 4096바이트 미만 스트림은 미니 스트림에 둔다.
#![allow(dead_code)]

pub const FREESECT: u32 = 0xFFFF_FFFF;
pub const ENDOFCHAIN: u32 = 0xFFFF_FFFE;
pub const FATSECT: u32 = 0xFFFF_FFFD;
pub const MINI_CUTOFF: usize = 4096;

/// 만든 파일과 구조 위치. 손상 테스트가 바이트를 직접 고치는 데 쓴다.
pub struct Cfb {
    pub bytes: Vec<u8>,
    pub sector_size: usize,
    pub dir_sector: u32,
    pub dir_sectors: u32,
    pub fat_sectors: Vec<u32>,
    pub minifat_sector: u32,
    pub mini_stream_sector: u32,
    /// 항목 ID(0은 루트) → 이름.
    pub names: Vec<String>,
}

impl Cfb {
    /// 섹터 번호의 파일 오프셋(헤더가 섹터 하나 크기를 차지한다).
    pub fn sector_offset(&self, sector: u32) -> usize {
        self.sector_size * (sector as usize + 1)
    }
    pub fn entry_offset(&self, id: usize) -> usize {
        self.sector_offset(self.dir_sector) + id * 128
    }
    pub fn fat_entry_offset(&self, sector: u32) -> usize {
        let per = self.sector_size / 4;
        self.sector_offset(self.fat_sectors[sector as usize / per]) + (sector as usize % per) * 4
    }
    pub fn id_of(&self, name: &str) -> usize {
        self.names.iter().position(|n| n == name).unwrap_or_else(|| panic!("no entry {name}"))
    }
}

fn put32(b: &mut [u8], at: usize, v: u32) {
    b[at..at + 4].copy_from_slice(&v.to_le_bytes());
}

fn entry_bytes(name: &str, kind: u8, left: u32, right: u32, child: u32, start: u32, size: u64) -> [u8; 128] {
    let mut e = [0u8; 128];
    let units: Vec<u16> = name.encode_utf16().collect();
    for (i, u) in units.iter().enumerate() {
        e[i * 2..i * 2 + 2].copy_from_slice(&u.to_le_bytes());
    }
    let len = ((units.len() + 1) * 2) as u16;
    e[64..66].copy_from_slice(&len.to_le_bytes());
    e[66] = kind;
    e[67] = 1;
    e[68..72].copy_from_slice(&left.to_le_bytes());
    e[72..76].copy_from_slice(&right.to_le_bytes());
    e[76..80].copy_from_slice(&child.to_le_bytes());
    e[116..120].copy_from_slice(&start.to_le_bytes());
    e[120..128].copy_from_slice(&size.to_le_bytes());
    e
}

struct Node {
    name: String,
    kind: u8,
    data: Vec<u8>,
    parent: usize,
}

/// `streams`의 경로(`"Macros/VBA/dir"`)로 저장소를 만들고 스트림을 넣는다. 형제는 오른쪽 형제 사슬로 잇는다(적흑 트리는 아니다).
pub fn make_cfb_sized(streams: &[(&str, &[u8])], sector_size: usize) -> Cfb {
    assert!(sector_size == 512 || sector_size == 4096);
    let mut nodes: Vec<Node> = vec![Node { name: "Root Entry".into(), kind: 5, data: Vec::new(), parent: 0 }];
    for (path, data) in streams {
        let parts: Vec<&str> = path.split('/').collect();
        let mut parent = 0usize;
        for (i, part) in parts.iter().enumerate() {
            let last = i + 1 == parts.len();
            let found = nodes.iter().position(|n| n.parent == parent && n.name == *part && !std::ptr::eq(n, &nodes[0]));
            match found {
                Some(id) if !last => parent = id,
                _ => {
                    nodes.push(Node { name: (*part).into(), kind: if last { 2 } else { 1 }, data: if last { data.to_vec() } else { Vec::new() }, parent });
                    parent = nodes.len() - 1;
                }
            }
        }
    }
    let n = nodes.len();
    let per_fat = sector_size / 4;
    // 일반 섹터 수 계산: 미니 스트림 컨테이너, 일반 스트림, 디렉터리, 미니 FAT, FAT.
    let sectors_for = |len: usize| len.div_ceil(sector_size).max(usize::from(len > 0));
    let mut mini_stream = Vec::new();
    let mut mini_starts: Vec<Option<u32>> = vec![None; n];
    let mut minifat: Vec<u32> = Vec::new();
    for (i, node) in nodes.iter().enumerate() {
        if node.kind == 2 && !node.data.is_empty() && node.data.len() < MINI_CUTOFF {
            let first = (mini_stream.len() / 64) as u32;
            let count = node.data.len().div_ceil(64);
            for k in 0..count {
                minifat.push(if k + 1 == count { ENDOFCHAIN } else { first + k as u32 + 1 });
            }
            mini_stream.extend_from_slice(&node.data);
            mini_stream.resize(mini_stream.len().div_ceil(64) * 64, 0);
            mini_starts[i] = Some(first);
        }
    }
    let dir_bytes = n * 128;
    let dir_sectors = sectors_for(dir_bytes);
    let mini_container = sectors_for(mini_stream.len());
    let minifat_sectors = sectors_for(minifat.len() * 4);
    let regular: Vec<usize> = (0..n).filter(|&i| nodes[i].kind == 2 && nodes[i].data.len() >= MINI_CUTOFF).collect();
    let regular_sectors: usize = regular.iter().map(|&i| sectors_for(nodes[i].data.len())).sum();
    let base = dir_sectors + minifat_sectors + mini_container + regular_sectors;
    let mut fat_count = 1;
    while (base + fat_count).div_ceil(per_fat) > fat_count {
        fat_count += 1;
    }
    assert!(fat_count <= 109, "test fixtures stay below the DIFAT threshold");
    let total = base + fat_count;
    let mut fat = vec![FREESECT; fat_count * per_fat];
    let mut next = 0u32;
    let chain = |len: usize, fat: &mut Vec<u32>, next: &mut u32| -> u32 {
        let start = *next;
        for k in 0..len {
            fat[(start as usize) + k] = if k + 1 == len { ENDOFCHAIN } else { start + k as u32 + 1 };
        }
        *next += len as u32;
        start
    };
    let dir_start = chain(dir_sectors, &mut fat, &mut next);
    let minifat_start = if minifat_sectors > 0 { chain(minifat_sectors, &mut fat, &mut next) } else { ENDOFCHAIN };
    let mini_start = if mini_container > 0 { chain(mini_container, &mut fat, &mut next) } else { ENDOFCHAIN };
    let mut starts: Vec<u32> = vec![0; n];
    for &i in &regular {
        starts[i] = chain(sectors_for(nodes[i].data.len()), &mut fat, &mut next);
    }
    let fat_start = next;
    let fat_sectors: Vec<u32> = (0..fat_count as u32).map(|k| fat_start + k).collect();
    for s in &fat_sectors {
        fat[*s as usize] = FATSECT;
    }
    assert_eq!(next as usize + fat_count, total);

    let mut bytes = vec![0u8; sector_size * (total + 1)];
    let h = &mut bytes[..512];
    h[..8].copy_from_slice(&[0xD0, 0xCF, 0x11, 0xE0, 0xA1, 0xB1, 0x1A, 0xE1]);
    let (major, shift) = if sector_size == 512 { (3u16, 9u16) } else { (4u16, 12u16) };
    h[24..26].copy_from_slice(&0x3Eu16.to_le_bytes());
    h[26..28].copy_from_slice(&major.to_le_bytes());
    h[28..30].copy_from_slice(&0xFFFEu16.to_le_bytes());
    h[30..32].copy_from_slice(&shift.to_le_bytes());
    h[32..34].copy_from_slice(&6u16.to_le_bytes());
    put32(h, 40, if major == 4 { dir_sectors as u32 } else { 0 });
    put32(h, 44, fat_count as u32);
    put32(h, 48, dir_start);
    put32(h, 56, MINI_CUTOFF as u32);
    put32(h, 60, minifat_start);
    put32(h, 64, minifat_sectors as u32);
    put32(h, 68, ENDOFCHAIN);
    put32(h, 72, 0);
    for k in 0..109 {
        put32(h, 76 + k * 4, fat_sectors.get(k).copied().unwrap_or(FREESECT));
    }
    let off = |s: u32| sector_size * (s as usize + 1);
    // 디렉터리: 부모별로 자식을 오른쪽 형제 사슬로 잇는다.
    let mut dir = Vec::new();
    for (i, node) in nodes.iter().enumerate() {
        let siblings: Vec<usize> = (1..n).filter(|&j| nodes[j].parent == i && j != i).collect();
        let child = siblings.first().map_or(FREESECT, |&c| c as u32);
        let right = if i == 0 {
            FREESECT
        } else {
            let peers: Vec<usize> = (1..n).filter(|&j| nodes[j].parent == node.parent).collect();
            let pos = peers.iter().position(|&j| j == i).unwrap();
            peers.get(pos + 1).map_or(FREESECT, |&j| j as u32)
        };
        let (start, size) = match node.kind {
            5 => (if mini_container > 0 { mini_start } else { ENDOFCHAIN }, mini_stream.len() as u64),
            2 => (mini_starts[i].unwrap_or(starts[i]), node.data.len() as u64),
            _ => (0, 0),
        };
        dir.extend_from_slice(&entry_bytes(&node.name, node.kind, FREESECT, right, child, start, size));
    }
    dir.resize(dir_sectors * sector_size, 0);
    bytes[off(dir_start)..off(dir_start) + dir.len()].copy_from_slice(&dir);
    let mut mf: Vec<u8> = minifat.iter().flat_map(|v| v.to_le_bytes()).collect();
    mf.resize(minifat_sectors * sector_size, 0xFF);
    if minifat_sectors > 0 {
        bytes[off(minifat_start)..off(minifat_start) + mf.len()].copy_from_slice(&mf);
    }
    if mini_container > 0 {
        bytes[off(mini_start)..off(mini_start) + mini_stream.len()].copy_from_slice(&mini_stream);
    }
    for &i in &regular {
        let d = &nodes[i].data;
        bytes[off(starts[i])..off(starts[i]) + d.len()].copy_from_slice(d);
    }
    for (k, s) in fat_sectors.iter().enumerate() {
        let chunk: Vec<u8> = fat[k * per_fat..(k + 1) * per_fat].iter().flat_map(|v| v.to_le_bytes()).collect();
        bytes[off(*s)..off(*s) + sector_size].copy_from_slice(&chunk);
    }
    Cfb { bytes, sector_size, dir_sector: dir_start, dir_sectors: dir_sectors as u32, fat_sectors, minifat_sector: minifat_start, mini_stream_sector: mini_start, names: nodes.iter().map(|n| n.name.clone()).collect() }
}

pub fn make_cfb(streams: &[(&str, &[u8])]) -> Cfb {
    make_cfb_sized(streams, 512)
}

pub fn make_cfb_v4(streams: &[(&str, &[u8])]) -> Cfb {
    make_cfb_sized(streams, 4096)
}
