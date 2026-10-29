//! PDF 정적 동작·활성 콘텐츠 검사 (pdfscan에서 이식).
//!
//! PDF를 렌더링하거나 JavaScript를 실행하거나 첨부를 열거나 프로그램을 띄우지 않는다. PDF 파싱 자체가 공격면이므로
//! 신뢰할 수 없는 파일은 권한 없는 샌드박스에서 검사한다.

use std::collections::BTreeSet;
use std::sync::LazyLock;

use regex::bytes::Regex;

use lopdf::xref::XrefEntry;
use lopdf::{Dictionary, Document, LoadOptions, Object, ObjectId};
use scan_common::sniff::{Sniffed, sniff};
use scan_common::text::sanitize as sanitize_text;
use scan_common::{Category, FileReport, Severity};

use crate::epub::security::guarded;

const MAX_DEPTH: usize = 100;
const PREVIEW_CHARS: usize = 100;
const JS_STREAM_LIMIT: usize = 64 * 1024;
const MAX_DECOMPRESSED_STREAM_BYTES: usize = 4 * 1024 * 1024;
/// 로딩 중 스트림 내용을 이만큼만 남긴다. lopdf는 모든 스트림 내용을 복사해서 들고 있어서, 이미지가 많은 PDF가 입력의 2배
/// 넘게 메모리를 쓴다(223MB 파일이 458MB). 한도를 256KB→64KB→4KB로 낮추면 454→302→240MB로 줄었다(입력은 226MB).
/// 64KB는 JavaScript 해제분 미리보기 한도와 같고, JS는 압축하면 보통 수 KB다. 4KB는 일반 JS까지 잘라서 쓸 수 없다.
/// 이보다 큰 JavaScript 스트림은 잘려서 끝까지 못 본 것이므로 `not fully inspected`로 알린다.
const STREAM_KEEP_BYTES: usize = 64 * 1024;
/// 숫자·이름·참조만 든 배열은 이 길이를 넘으면 로딩 중에 비운다. lopdf는 원소 하나를 `Object`(약 120B)로 만들어서
/// 파일에서 2바이트인 숫자가 50배로 부풀고, 숫자 배열이 많은 PDF는 입력의 25배까지 메모리를 쓴다(22MB가 549MB).
const ARRAY_KEEP_LEN: usize = 32;
/// 참조만 든 배열은 이 길이까지 남긴다. 쪽 트리의 /Kids는 쪽 수만큼 길다(표본 최대 2,710쪽). 그 약 7배인 2만 개면 노드 약 2.4MB다.
/// 훅은 lopdf가 rayon 작업 스레드에서 부르므로(객체 16개 이상이거나 입력 1MiB 이상) 상태를 둘 수 없다. 그래서 배열 하나만 보는
/// 무상태 규칙이고, 파일 전체의 합계는 읽은 뒤 호출 스레드에서 `LoadInfo`로 센다.
const REF_ARRAY_KEEP_LEN: usize = 20_000;
/// 메모리 상한(최악의 경우): 참조 하나는 파일에서 최소 6바이트(`1 0 R `)이고 lopdf 노드(`Object`)는 약 120B라 증폭은 최대 약 20배다.
/// 배열 하나는 2만 개 x 120B = 약 2.4MB다. 배열 수는 파일 크기가 막지만 크기 한도(`--max-total-mb`)만으로는 참조 폭탄이
/// 한도 x 20배를 쓸 수 있어 안전하지 않다. 그래서 읽기 전에 원본 참조 수를 세어 이 예산을 넘으면 참조 배열도 비우는 훅을 쓴다.
/// 예산 안이면 참조 노드는 최대 50만 x 120B = 약 60MB다. 읽은 뒤 센 "남긴 참조 노드" 합계가 이를 넘으면 쪽 트리 판정을 믿지 않고 `incomplete`로 알린다. 표본 최대 쪽 수의 약 180배(노드 약 60MB)다.
const REF_FILE_BUDGET: usize = 500_000;
/// 훅이 비운 참조 배열이 들어 있던 자리에 남기는 표지 이름. 진짜 빈 배열과 구별해 읽은 뒤에 찾는다.
const EMPTIED_REF_ARRAY_MARK: &[u8] = b"PdfscanEmptiedRefArray";
/// Bound the amount of input PDF data held by concurrent scans. `lopdf` may
/// allocate more than the input size while parsing, so this is a conservative
/// concurrency guard rather than a hard process-memory limit.
pub const MAX_PARALLEL_INPUT_MB: u64 = 256;

/// File extensions that an embedded file or launch target should not carry.
const RISKY_EXT: &[&str] = &[
    "exe", "com", "scr", "pif", "bat", "cmd", "ps1", "psm1", "vbs", "vbe", "js", "jse", "wsf", "wsh", "hta", "msi", "msp", "dll", "jar", "lnk", "reg", "cpl", "sh", "bash", "run", "bin", "appimage",
    "desktop", "py", "pl", "rb", "docm", "xlsm", "pptm", "swf",
];

struct Finding {
    severity: Severity,
    kind: &'static str,
    object: ObjectId,
    path: String,
    detail: String,
}

// ---------------------------------------------------------------- 로딩 중 줄이기

/// 스캐너가 들여다보지 않는 값인가. `walk`는 사전·스트림·배열만 따라가고, 참조는 평평한 객체 순회가 따로 처리한다.
fn is_plain(o: &Object) -> bool {
    matches!(o, Object::Null | Object::Boolean(_) | Object::Integer(_) | Object::Real(_) | Object::Name(_) | Object::Reference(_))
}

/// 줄이기가 내려가는 최대 깊이. lopdf 파서가 중첩을 100으로 제한하므로 정상 경로에서는 닿지 않는다.
/// 그 보장이 의존성 버전에 있어서, 넘으면 줄이지 않고 그대로 둔다(스택을 지키려는 방어).
const SHRINK_MAX_DEPTH: usize = 128;

/// 배열이 어느 깊이에서도 평범한 값과 배열로만 이루어졌다면 노드 수를, 사전·스트림·문자열이 하나라도 있으면 None을 돌려준다.
/// 좌표 쌍 목록(`[[x y] [x y] …]`)처럼 중첩된 숫자 배열도 한 덩어리로 센다. 너무 깊으면 None(비우지 않는다).
fn plain_nodes(items: &[Object], depth: usize) -> Option<usize> {
    if depth > SHRINK_MAX_DEPTH {
        return None;
    }
    items.iter().try_fold(1usize, |n, o| match o {
        Object::Array(inner) => plain_nodes(inner, depth + 1).map(|c| n + c),
        o if is_plain(o) => Some(n + 1),
        _ => None,
    })
}

/// 읽은 문서에서 센 참조 배열 정보. 훅이 아니라 호출 스레드에서 `doc.objects`를 훑어 만든다.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct LoadInfo {
    /// 훅이 참조 배열(쪽 트리의 /Kids일 수 있다)을 비운 흔적이 있다.
    pub emptied_ref_array: bool,
    /// 남아 있는 긴(32개 초과) 참조 전용 배열의 원소 합계.
    pub kept_ref_nodes: usize,
}

impl LoadInfo {
    pub fn from_document(doc: &Document) -> Self {
        let mut info = Self::default();
        for obj in doc.objects.values() {
            info.visit(obj, 0);
        }
        info
    }

    fn visit(&mut self, obj: &Object, depth: usize) {
        if depth > SHRINK_MAX_DEPTH {
            return;
        }
        match obj {
            Object::Array(items) => {
                if matches!(items.as_slice(), [Object::Name(n)] if n.as_slice() == EMPTIED_REF_ARRAY_MARK) {
                    self.emptied_ref_array = true;
                } else if items.len() > ARRAY_KEEP_LEN && items.iter().all(|o| matches!(o, Object::Reference(_))) {
                    self.kept_ref_nodes += items.len();
                } else {
                    items.iter().for_each(|o| self.visit(o, depth + 1));
                }
            }
            Object::Dictionary(d) => d.iter().for_each(|(_, v)| self.visit(v, depth + 1)),
            Object::Stream(st) => st.dict.iter().for_each(|(_, v)| self.visit(v, depth + 1)),
            _ => {}
        }
    }

    /// 쪽 트리가 잘렸을 수 있어 0쪽 판정을 믿을 수 없는가.
    pub fn page_tree_unreliable(&self) -> bool {
        self.emptied_ref_array || self.kept_ref_nodes > REF_FILE_BUDGET
    }
}

fn shrink_array(items: &mut Vec<Object>, depth: usize, keep_refs: bool) {
    // 참조만 든 배열은 쪽 트리(/Kids)와 /Contents 같은 구조 정보다. `/Kids 21 0 R`처럼 배열이 독립 객체일 때는 훅이 키를 모르므로
    // 내용으로 가려야 한다. 비우면 lopdf가 쪽을 하나도 찾지 못해 멀쩡한 PDF가 0쪽이 된다. 그래서 배열당 상한까지는 남기고,
    // 넘으면 비우되 표지를 남겨 읽은 뒤에 알아볼 수 있게 한다.
    if items.len() > ARRAY_KEEP_LEN && items.iter().all(|o| matches!(o, Object::Reference(_))) {
        if !keep_refs || items.len() > REF_ARRAY_KEEP_LEN {
            *items = vec![Object::Name(EMPTIED_REF_ARRAY_MARK.to_vec())];
        }
        return;
    }
    if plain_nodes(items, depth).is_some_and(|n| n > ARRAY_KEEP_LEN) {
        *items = Vec::new();
        return;
    }
    items.iter_mut().for_each(|o| shrink_with(o, depth + 1, keep_refs));
}

fn shrink_dict(d: &mut Dictionary, depth: usize, keep_refs: bool) {
    d.iter_mut().for_each(|(_, v)| shrink_with(v, depth + 1, keep_refs));
}

/// 스트림을 어떻게 해석할지 정하는 키. 배열이 길다고 비우면 필터가 사라져서, 인코딩된 JS가 해독되지 않은 채 검사를 빠져나간다.
const STREAM_DECODING_KEYS: [&[u8]; 4] = [b"Filter", b"DecodeParms", b"FFilter", b"FDecodeParms"];
/// 현실적인 필터 체인은 몇 단계다. 이보다 길면 노드로 메모리를 부풀리려는 것이므로 잘라 낸다.
const FILTER_KEEP_LEN: usize = 16;
/// 한 스트림의 `/Filter` 배열이 이 길이 이상이면 `long filter chain`(Medium)이다. 계획 11행의 기준값이다.
/// 정상 PDF의 체인은 보통 1~2단계(예: ASCII85+Flate)라서 5단계 이상은 해독 검사를 지치게 하려는 수법으로 본다. 경험적 값이다.
/// 로딩 중 `FILTER_KEEP_LEN`(16)으로 잘린 배열은 표지(`FILTER_TRUNCATED_KEY`)가 있으므로 "16단계 이상"으로 보고 지표를 낸다.
const LONG_FILTER_CHAIN: usize = 5;
// 잘리지 않은 배열은 길이를 그대로 읽으므로, 자르는 길이가 기준보다 커야 기준 판정이 잘림에 가려지지 않는다.
const _: () = assert!(FILTER_KEEP_LEN > LONG_FILTER_CHAIN);
/// `startxref`가 이 개수 이상이면 `many incremental updates`(Low)다. 계획 11행의 기준값으로, 외부 근거 없는 경험적 값이다.
/// 증분 저장은 이전 내용을 지우지 않고 덧붙이므로, 많이 쌓일수록 눈에 보이지 않는 옛 객체가 남을 수 있다.
const MANY_INCREMENTAL_UPDATES: usize = 10;
/// 내장 파일을 내용으로 판별할 때 보는 앞부분 길이. 계획은 1024바이트였으나(11-C), 해제한 스트림 전체가 메모리에 있으므로
/// 64KiB까지 본다. PE 헤더가 1KiB 밖에 있어도 찾기 위해서다. 헤더가 64KiB 밖에 있는 PE는 놓치는 한계가 있다(정상 PE는 앞 수백 바이트).
const EMBEDDED_SNIFF_BYTES: usize = 64 * 1024;
// 필터 키의 배열은 길이를 제한한 뒤에도 일반 줄이기를 거친다. 그 배열이 비워지면(해독이 사라지면) 안 되므로, 제한한 길이가
// `ARRAY_KEEP_LEN`보다 작아야 한다(작으면 노드 수가 비우는 기준에 닿지 않는다). 둘을 바꿀 때 어기지 않게 컴파일 시점에 묶는다.
const _: () = assert!(FILTER_KEEP_LEN + 1 < ARRAY_KEEP_LEN);
/// 필터 체인을 잘랐다는 표지. 잘리면 해독이 틀어지므로 그 스트림은 "끝까지 검사하지 못한" 것으로 알려야 한다.
/// 파일이 같은 키를 직접 넣어도 알림이 하나 더 붙을 뿐이라 안전한 쪽이다.
const FILTER_TRUNCATED_KEY: &[u8] = b"PdfscanFilterTruncated";
/// `/Filter` 배열 자체를 잘랐다는 표지. 위 표지는 `DecodeParms` 등 다른 해독 키만 잘려도 붙으므로 어느 키인지 말하지 못한다.
/// `long filter chain`은 필터 체인이 길다는 뜻이라 `/Filter`가 잘린 경우에만 이 표지로 판단한다(위 표지는 그대로 둔다).
const FILTER_ARRAY_TRUNCATED_KEY: &[u8] = b"PdfscanFilterArrayTruncated";

/// 객체 하나를 스캔에 필요한 만큼만 남긴다. 제자리에서 줄인다.
#[cfg(test)]
fn shrink(obj: &mut Object, depth: usize) {
    shrink_with(obj, depth, true);
}

/// `keep_refs`가 참이면 참조 전용 배열을 배열당 상한까지 남기고, 거짓이면 32개 초과를 모두 비운다(표지는 남긴다).
fn shrink_with(obj: &mut Object, depth: usize, keep_refs: bool) {
    if depth > SHRINK_MAX_DEPTH {
        return;
    }
    match obj {
        Object::Array(items) => shrink_array(items, depth, keep_refs),
        Object::Dictionary(d) => shrink_dict(d, depth, keep_refs),
        Object::Stream(st) => {
            // 객체 스트림과 xref 스트림은 구조 해석에 쓰인다. 훅이 디코딩보다 먼저 돌아서, 자르면 안의 객체를 잃는다.
            let structural = st.dict.has_type(b"ObjStm") || st.dict.has_type(b"XRef");
            // 비우면 lopdf가 "길이 0 스트림"으로 보고 파일에서 다시 읽는다. 잘라서 비어 있지 않게 둔다.
            if !structural && st.content.len() > STREAM_KEEP_BYTES {
                st.content.truncate(STREAM_KEEP_BYTES);
                st.content.shrink_to_fit();
            }
            // 필터 키는 배열이 길다는 이유로 비우면 해독이 사라지므로 비우지 않되, 노드로 메모리를 부풀리지 못하게 길이는 제한한다.
            let mut truncated = false;
            let mut filter_truncated = false;
            for (k, v) in st.dict.iter_mut() {
                if STREAM_DECODING_KEYS.contains(&k.as_slice())
                    && let Object::Array(items) = v
                    && items.len() > FILTER_KEEP_LEN
                {
                    items.truncate(FILTER_KEEP_LEN);
                    items.shrink_to_fit();
                    truncated = true;
                    filter_truncated |= k.as_slice() == b"Filter";
                }
                shrink_with(v, depth + 1, keep_refs);
            }
            if truncated {
                st.dict.set(FILTER_TRUNCATED_KEY.to_vec(), Object::Boolean(true));
            }
            if filter_truncated {
                st.dict.set(FILTER_ARRAY_TRUNCATED_KEY.to_vec(), Object::Boolean(true));
            }
        }
        _ => {}
    }
}

/// lopdf 로딩 훅. 객체를 파싱하자마자 줄여서, 문서 전체가 아니라 객체 하나분만 일시적으로 쓴다.
/// 일반 경로는 제자리 수정한 객체를 그대로 쓰고, 객체 스트림 경로는 반환값을 쓰므로 복제본을 돌려준다(줄인 뒤라 작다).
fn shrink_on_load(id: ObjectId, obj: &mut Object) -> Option<(ObjectId, Object)> {
    shrink_with(obj, 0, true);
    Some((id, obj.clone()))
}

/// 참조가 너무 많은 파일용 훅. 참조 전용 배열도 32개 초과면 모두 비운다. 훅은 함수 포인터이고 lopdf가 rayon 작업 스레드에서 부르므로
/// 상태(공유 예산 등)를 둘 수 없다. 그래서 읽기 전에 `load`가 두 훅 중 하나를 고른다. 이 훅이 만든 표지는 `LoadInfo`가 찾는다.
fn shrink_on_load_strict(id: ObjectId, obj: &mut Object) -> Option<(ObjectId, Object)> {
    shrink_with(obj, 0, false);
    Some((id, obj.clone()))
}

/// 파일 원본에서 `N 0 R` 꼴 참조가 몇 개인지 센다. 압축 객체 스트림 안의 참조는 세지 못한다(해제 한도 `MAX_DECOMPRESSED_STREAM_BYTES`가
/// 스트림당 따로 막는다). 세는 값은 일반 객체 속 참조 노드 수의 상한에 가깝다.
fn count_raw_references(data: &[u8]) -> usize {
    static RE: LazyLock<Option<Regex>> = LazyLock::new(|| Regex::new(r" 0 R\b").ok());
    RE.as_ref().map_or(0, |re| re.find_iter(data).count())
}

// ---------------------------------------------------------------- helpers

/// Make attacker-controlled text safe to print: no control chars, bounded length.
fn sanitize(bytes: &[u8]) -> String {
    sanitize_text(&String::from_utf8_lossy(bytes), PREVIEW_CHARS, Some('…'))
}

fn name_of(obj: &Object) -> Option<String> {
    match obj {
        Object::Name(n) => Some(String::from_utf8_lossy(n).into_owned()),
        _ => None,
    }
}

fn has_risky_ext(name: &[u8]) -> bool {
    name_readings(name).iter().any(|reading| ext_is_risky(&reading.to_ascii_lowercase()))
}

/// 소문자로 만든 이름 하나의 확장자가 실행 파일인가. 주소 형태는 호스트가 아니라 경로를 본다.
fn ext_is_risky(lower: &str) -> bool {
    // 맨 앞이 http·https·ftp 주소이고 호스트가 비어 있지 않을 때만 호스트가 아니라 경로의 확장자를 본다.
    // `https://example.com`의 `.com`은 DOS 실행 파일이 아니고, `https://x.example/evil.exe`는 경로가 .exe라서 잡힌다.
    // 문자열 중간의 `://`나 모르는 형태는 이름 그대로 검사한다(뷰어가 어떻게 읽을지 모르므로 열어 두지 않는다).
    let url_rest = ["http://", "https://", "ftp://"].iter().find_map(|scheme| lower.strip_prefix(scheme)).filter(|rest| !rest.is_empty() && !rest.starts_with('/'));
    let target = if let Some(rest) = url_rest {
        let rest = rest.split(['?', '#']).next().unwrap_or("");
        rest.split_once('/').map_or("", |(_, path)| path)
    } else if let Some(address) = lower.strip_prefix("mailto:").filter(|a| a.contains('@')) {
        // 주소의 도메인(`.com`)이 아니라 `@` 앞 사서함 이름만 본다.
        address.split('@').next().unwrap_or("")
    } else {
        lower
    };
    // Windows ignores trailing dots/spaces, so strip them before the check.
    let trimmed = target.trim_end_matches(['.', ' ']);
    match trimmed.rsplit_once('.') {
        Some((_, ext)) => RISKY_EXT.contains(&ext),
        None => false,
    }
}

/// Text of a string / stream value, following one reference. Inert data only.
fn text_preview(doc: &Document, obj: &Object) -> String {
    match obj {
        Object::String(s, _) => sanitize(s),
        Object::Reference(id) => match doc.get_object(*id) {
            Ok(Object::String(s, _)) => format!("{} {} R: {}", id.0, id.1, sanitize(s)),
            Ok(Object::Stream(st)) => {
                let data = st.decompressed_content_with_limit(JS_STREAM_LIMIT).unwrap_or_else(|_| st.content.clone());
                format!("{} {} R (stream): {}", id.0, id.1, sanitize(&data))
            }
            _ => format!("{} {} R", id.0, id.1),
        },
        Object::Stream(st) => sanitize(&st.content),
        _ => String::new(),
    }
}

/// 문자열이거나 문자열·스트림을 가리키는 참조의 원본 바이트. 상한을 넘는 스트림은 앞부분만 본다.
fn raw_bytes(doc: &Document, obj: &Object) -> Vec<u8> {
    match obj {
        Object::String(s, _) => s.clone(),
        Object::Reference(id) => match doc.get_object(*id) {
            Ok(Object::String(s, _)) => s.clone(),
            // 탐지용이라 미리보기(64KB)보다 넉넉히 푼다. 64KB를 넘는 JS를 압축 원본으로 되돌리면 패딩만으로 탐지를 피한다.
            Ok(Object::Stream(st)) => st.decompressed_content_with_limit(MAX_DECOMPRESSED_STREAM_BYTES).unwrap_or_else(|_| st.content.iter().take(JS_STREAM_LIMIT).copied().collect()),
            _ => Vec::new(),
        },
        Object::Stream(st) => st.content.iter().take(JS_STREAM_LIMIT).copied().collect(),
        _ => Vec::new(),
    }
}

/// 파일 지정 사전에서 이름을 담는 키. 뷰어는 /UF를 먼저 쓰기도 해서, 하나만 보면 다른 키에 실행 파일을 숨길 수 있다.
const FILESPEC_KEYS: [&[u8]; 5] = [b"F", b"UF", b"DOS", b"Mac", b"Unix"];

/// 파일 지정(문자열이거나 위 키를 가진 사전)이 문서에서 열 이유가 없는 대상을 가리키는가. 키 하나라도 위험하면 위험이다.
fn filespec_is_dangerous(doc: &Document, obj: &Object) -> bool {
    // 11-B(R16 보안 검토): 참조가 가리키는 사전도 인라인 사전과 똑같이 본다. 인라인 값의 동작은 그대로다.
    match resolve(doc, obj) {
        Some(Object::Dictionary(d)) => FILESPEC_KEYS.iter().filter_map(|k| d.get(k).ok()).any(|v| launch_target_is_dangerous(&raw_bytes(doc, v))),
        _ => launch_target_is_dangerous(&raw_bytes(doc, obj)),
    }
}

/// 참조하는 JavaScript 스트림을 끝까지 검사하지 못했는가: 로딩 중에 잘렸거나(`STREAM_KEEP_BYTES` 이상), 해제 한도를 넘거나 풀 수 없다.
/// 이때 탐지는 앞부분(또는 압축 원본)만 본 것이므로, 조용히 지나가지 않고 알려야 한다. 패딩이나 거대한 해제분으로 탐지를 피하지 못하게 한다.
fn js_stream_not_fully_inspected(doc: &Document, obj: &Object) -> bool {
    let Object::Reference(id) = obj else { return false };
    matches!(doc.get_object(*id), Ok(Object::Stream(st)) if st.content.len() >= STREAM_KEEP_BYTES || st.dict.has(FILTER_TRUNCATED_KEY) || st.decompressed_content_with_limit(MAX_DECOMPRESSED_STREAM_BYTES).is_err())
}

/// 문서에서 열 이유가 없는 대상인가: 실행 파일 확장자이거나 셸·스크립트 인터프리터를 부른다.
fn launch_target_is_dangerous(target: &[u8]) -> bool {
    const SHELLS: &[&str] = &["cmd.exe", "cmd/c", "powershell", "pwsh", "mshta", "wscript", "cscript", "rundll32", "regsvr32", "/bin/sh", "/bin/bash"];
    has_risky_ext(target)
        || name_readings(target).iter().any(|reading| {
            let flat: String = reading.to_ascii_lowercase().chars().filter(|c| !c.is_whitespace()).collect();
            SHELLS.iter().any(|sh| flat.contains(sh))
        })
}

/// JavaScript 원문을 글로 바꾼다. PDF 문자열은 UTF-16(BOM)일 수 있어서, 그대로 찾으면 인코딩만 바꿔 탐지를 피할 수 있다.
fn js_text(bytes: &[u8]) -> String {
    let units = |b: &[u8], be: bool| -> String {
        let u16s: Vec<u16> = b.chunks_exact(2).map(|c| if be { u16::from_be_bytes([c[0], c[1]]) } else { u16::from_le_bytes([c[0], c[1]]) }).collect();
        String::from_utf16_lossy(&u16s)
    };
    match bytes {
        [0xFE, 0xFF, rest @ ..] => units(rest, true),
        [0xFF, 0xFE, rest @ ..] => units(rest, false),
        _ => String::from_utf8_lossy(bytes).into_owned(),
    }
}

/// 보이는 이름. 지표의 설명에만 쓴다(판정에는 `name_readings`).
fn name_text(bytes: &[u8]) -> String {
    js_text(bytes).replace('\0', "")
}

/// 이름을 읽는 방법은 뷰어마다 다르다. 한 가지만 믿으면 읽는 쪽이 다른 뷰어를 속일 수 있어서, 가능한 해석을 모두 돌려준다.
/// - 해석: BOM을 따른 글(`FE FF`, 그리고 이 스캐너가 알아보는 `FF FE`)과 BOM을 무시하고 그대로 읽은 글.
///   `FF FE` 뒤에 ASCII 이름이 오면 LE로 풀 때는 한자 쓰레기가 되지만, 명세대로 읽는 뷰어에는 `ÿþcalc.exe`다.
/// - NUL: 지운 글(BOM 없는 UTF-16)과 첫 NUL에서 자른 글(C 문자열). `calc.exe\0.txt`는 지우면 안전해 보이지만 자르면 `calc.exe`다.
///
/// 호출하는 쪽은 이 중 하나라도 위험하면 위험으로 본다.
fn name_readings(bytes: &[u8]) -> Vec<String> {
    let decoded = js_text(bytes);
    let raw = String::from_utf8_lossy(bytes).into_owned();
    let mut texts = vec![decoded];
    if texts[0] != raw {
        texts.push(raw);
    }
    let mut out = Vec::new();
    for text in texts {
        out.push(text.replace('\0', ""));
        if let Some(head) = text.split('\0').next().filter(|h| h.len() != text.len()) {
            out.push(head.to_owned());
        }
    }
    out
}

/// 알려진 공격 수법이 든 JavaScript인가: 첨부 자동 실행, 로컬·UNC 경로 실행, 복호화한 페이로드의 eval, 셸코드 NOP sled.
fn js_is_exploit(source: &[u8]) -> bool {
    // 파일 이름과 같은 이유로 BOM을 따른 글과 그대로 읽은 글을 모두 본다. BOM이 없으면 둘이 같다.
    let decoded = js_text(source);
    let raw = String::from_utf8_lossy(source).into_owned();
    exploit_in(&decoded) || (decoded != raw && exploit_in(&raw))
}

fn exploit_in(text: &str) -> bool {
    let flat: String = text.to_lowercase().chars().filter(|c| !c.is_whitespace() && *c != '\0').collect();
    (flat.contains("exportdataobject") && flat.contains("nlaunch"))
        || (flat.contains("launchurl") && (flat.contains("file:") || flat.contains("\\\\")))
        || flat.contains("eval(unescape(")
        || flat.contains("eval(string.fromcharcode(")
        || flat.matches("%u9090").count() >= 4
}

/// 참조를 따라가는 최대 횟수. 이 안에 값에 닿지 못하면(끊긴 참조·순환) None이다.
/// lopdf의 `dereference`는 한도가 더 크고 오류 종류만 다르므로, 한도를 이 스캐너가 정하도록 직접 따라간다.
const MAX_REF_HOPS: usize = 8;

/// 참조 사슬을 끝까지 따라가 값과(참조를 거쳤다면) 마지막 객체 번호를 돌려준다. 한도 안에 닿지 못하면 None.
fn resolve_with_id<'a>(doc: &'a Document, obj: &'a Object) -> Option<(Option<ObjectId>, &'a Object)> {
    let mut cur = obj;
    let mut id = None;
    for _ in 0..MAX_REF_HOPS {
        let Object::Reference(r) = cur else { return Some((id, cur)) };
        id = Some(*r);
        cur = doc.objects.get(r)?;
    }
    if matches!(cur, Object::Reference(_)) { None } else { Some((id, cur)) }
}

/// 참조를 한도 안에서 따라간 값. 순환이나 끊긴 참조는 None.
fn resolve<'a>(doc: &'a Document, obj: &'a Object) -> Option<&'a Object> {
    resolve_with_id(doc, obj).map(|(_, o)| o)
}

/// `unresolvable action type`의 설명. 값이 무엇이었는지만 밝힌다(내용은 싣지 않는다).
fn describe_unresolved(raw: &Object, resolved: Option<&Object>) -> String {
    let kind = |o: &Object| match o {
        Object::Null => "null",
        Object::Boolean(_) => "boolean",
        Object::Integer(_) => "integer",
        Object::Real(_) => "real",
        Object::Name(_) => "name",
        Object::String(..) => "string",
        Object::Array(_) => "array",
        Object::Dictionary(_) => "dictionary",
        Object::Stream(_) => "stream",
        Object::Reference(_) => "reference",
    };
    let article = |k: &str| if k.starts_with(['a', 'e', 'i', 'o', 'u']) { "an" } else { "a" };
    match (raw, resolved) {
        (Object::Reference(id), None) => format!("reference {} {} R does not resolve", id.0, id.1),
        (Object::Reference(id), Some(o)) => format!("reference {} {} R resolves to {} {}, not a name", id.0, id.1, article(kind(o)), kind(o)),
        (o, _) => format!("value is {} {}, not a name", article(kind(o)), kind(o)),
    }
}

/// Join the listed keys of an inline dictionary value, e.g. /Win << /F /P >>.
fn value_dict_text(doc: &Document, value: &Object, keys: &[&[u8]]) -> String {
    // 11-B: 참조가 가리키는 사전도 인라인과 같게 읽는다.
    let Some(Object::Dictionary(d)) = resolve(doc, value) else {
        return String::new();
    };
    keys.iter().filter_map(|k| d.get(k).ok().map(|v| text_preview(doc, v))).collect::<Vec<_>>().join(" ")
}

// ---------------------------------------------------------------- scanning

struct Scanner<'a> {
    doc: &'a Document,
    findings: Vec<Finding>,
}

impl Scanner<'_> {
    fn add(&mut self, sev: Severity, kind: &'static str, id: ObjectId, path: &str, detail: String) {
        self.findings.push(Finding { severity: sev, kind, object: id, path: path.to_owned(), detail });
    }

    fn walk(&mut self, obj: &Object, id: ObjectId, path: &str, depth: usize) {
        if depth > MAX_DEPTH {
            self.add(Severity::Low, "nesting limit exceeded", id, path, "structure deeper than the scanner follows".into());
            return;
        }
        match obj {
            Object::Dictionary(d) => self.dict(d, id, path, depth),
            Object::Stream(s) => self.dict(&s.dict, id, path, depth),
            Object::Array(items) => {
                for (i, v) in items.iter().enumerate() {
                    self.walk(v, id, &format!("{path}[{i}]"), depth + 1);
                }
            }
            _ => {}
        }
    }

    fn dict(&mut self, d: &Dictionary, id: ObjectId, path: &str, depth: usize) {
        let doc = self.doc;
        // 11-A(R16 보안 검토): `/S 12 0 R`처럼 간접으로 쓴 동작 이름도 인라인과 같게 판정한다. 이름으로 풀리지 않으면
        // 모든 동작 규칙을 피해 가므로 Low로 알린다. 인라인 이름은 예전과 같고, 이름이 아닌 값은 이제 Low 지표가 하나 붙는다.
        let action = match d.get(b"S").ok() {
            None => None,
            Some(v) => match resolve(doc, v) {
                Some(resolved @ Object::Name(_)) => name_of(resolved),
                other => {
                    // 선형화 PDF의 힌트 스트림 사전은 `/S <정수>`를 가진다(공유 객체 힌트 표의 오프셋). 인라인 비-Name `/S`는
                    // 사전이 스스로 동작(`/Type /Action`)이라 밝힐 때만, 참조는 끊기거나 순환하거나 Name이 아닐 때 알린다.
                    let declared_action = matches!(d.get(b"Type").ok().and_then(|t| resolve(doc, t)), Some(Object::Name(n)) if n.as_slice() == b"Action");
                    if matches!(v, Object::Reference(_)) || declared_action {
                        self.add(Severity::Low, "unresolvable action type", id, path, describe_unresolved(v, other));
                    }
                    None
                }
            },
        };

        if let Some(s) = action.as_deref() {
            match s {
                "Launch" => {
                    let target = d.get(b"F").map(|v| text_preview(doc, v)).unwrap_or_default();
                    let dangerous = d.get(b"F").is_ok_and(|v| filespec_is_dangerous(doc, v));
                    self.add(if dangerous { Severity::Critical } else { Severity::High }, "Launch action", id, path, target);
                }
                "JavaScript" => {
                    if d.get(b"JS").is_ok_and(|v| js_stream_not_fully_inspected(doc, v)) {
                        self.add(Severity::High, "JavaScript stream not fully inspected", id, path, String::new());
                    }
                    let js = d.get(b"JS").map(|v| text_preview(doc, v)).unwrap_or_default();
                    if d.get(b"JS").is_ok_and(|v| js_is_exploit(&raw_bytes(doc, v))) {
                        self.add(Severity::Critical, "exploit JavaScript", id, path, js);
                    } else {
                        self.add(Severity::High, "JavaScript action", id, path, js);
                    }
                }
                "RichMediaExecute" => self.add(Severity::High, "RichMediaExecute action", id, path, String::new()),
                "ImportData" | "SubmitForm" | "GoToR" | "GoToE" | "Rendition" | "Movie" | "Sound" => {
                    let target = d.get(b"F").or_else(|_| d.get(b"URI")).map(|v| text_preview(doc, v)).unwrap_or_default();
                    self.add(Severity::Medium, "external/media action", id, path, format!("/S /{s} {target}"));
                    // 계획 11행: 옛 묶음 지표는 그대로 두고, 동작별 코드를 더한다. 대상(/F)은 GoToR·SubmitForm에 싣는다.
                    let code = match s {
                        "GoToR" => Some("GoToR action"),
                        "GoToE" => Some("GoToE action"),
                        "SubmitForm" => Some("SubmitForm action"),
                        "ImportData" => Some("ImportData action"),
                        _ => None,
                    };
                    if let Some(code) = code {
                        let f = d.get(b"F").ok();
                        let detail = match (s, f) {
                            ("GoToR" | "SubmitForm", Some(v)) => match resolve(doc, v) {
                                Some(Object::Dictionary(_)) => value_dict_text(doc, v, &[&b"F"[..], &b"UF"[..]]),
                                _ => text_preview(doc, v),
                            },
                            _ => String::new(),
                        };
                        self.add(Severity::Medium, code, id, path, detail);
                    }
                }
                "URI" => {
                    let uri = d.get(b"URI").map(|v| text_preview(doc, v)).unwrap_or_default();
                    self.add(Severity::Low, "URI action", id, path, uri);
                }
                _ => {}
            }
        }

        // File specification: an embedded or referenced executable-looking name.
        for key in FILESPEC_KEYS {
            // 11-B: `/F 13 0 R`처럼 문자열을 가리키는 참조도 인라인 문자열과 같게 본다.
            if let Some(Object::String(name, _)) = d.get(key).ok().and_then(|v| resolve(doc, v)) {
                let is_filespec = d.has(b"EF") || matches!(d.get(b"Type").ok().and_then(name_of).as_deref(), Some("Filespec")) || action.as_deref() == Some("Launch");
                if is_filespec && has_risky_ext(name) {
                    self.add(Severity::High, "executable-looking file name", id, path, sanitize(name_text(name).as_bytes()));
                }
            }
        }

        for (key, value) in d.iter() {
            let key_name = String::from_utf8_lossy(key).into_owned();
            // 경로는 출력에 그대로 나간다. PDF가 정한 키 이름의 제어문자는 걷어낸다. 판정은 원래 이름으로 한다.
            let child = format!("{path}/{}", sanitize(key));
            match key_name.as_str() {
                // The action dict case is already reported through /S above.
                "JS" if action.as_deref() != Some("JavaScript") => {
                    if js_stream_not_fully_inspected(doc, value) {
                        self.add(Severity::High, "JavaScript stream not fully inspected", id, &child, String::new());
                    }
                    if js_is_exploit(&raw_bytes(doc, value)) {
                        self.add(Severity::Critical, "exploit JavaScript", id, &child, text_preview(doc, value));
                    } else {
                        self.add(Severity::High, "JavaScript code", id, &child, text_preview(doc, value));
                    }
                }
                "JavaScript" => {
                    self.add(Severity::Medium, "JavaScript name tree", id, &child, String::new());
                }
                "Win" | "Mac" | "Unix" => {
                    let cmd = value_dict_text(doc, value, &[&b"F"[..], &b"P"[..]]);
                    // 11-B: 참조가 가리키는 사전도 인라인과 같게 본다.
                    let dangerous = match resolve(doc, value) {
                        Some(Object::Dictionary(v)) => [&b"F"[..], &b"P"[..]].iter().any(|k| v.get(k).is_ok_and(|x| filespec_is_dangerous(doc, x))),
                        _ => false,
                    };
                    self.add(if dangerous { Severity::Critical } else { Severity::High }, "platform launch parameters", id, &child, cmd);
                }
                "OpenAction" => {
                    self.add(Severity::Medium, "OpenAction (runs on open)", id, &child, String::new());
                }
                "AA" => {
                    self.add(Severity::Medium, "additional-actions trigger", id, &child, String::new());
                }
                "EmbeddedFiles" | "EF" => {
                    self.add(Severity::Medium, "embedded file", id, &child, String::new());
                }
                "RichMediaContent" => {
                    self.add(Severity::Medium, "RichMedia content", id, &child, String::new());
                }
                "XFA" => {
                    self.add(Severity::Medium, "XFA form", id, &child, String::new());
                }
                "3DD" => {
                    self.add(Severity::Low, "3D content", id, &child, String::new());
                }
                "Subtype" => match name_of(value).as_deref() {
                    Some("FileAttachment") => self.add(Severity::Medium, "file attachment annotation", id, path, String::new()),
                    Some("Screen") => self.add(Severity::Low, "screen annotation", id, path, String::new()),
                    _ => {}
                },
                _ => {}
            }
            // References are resolved by the flat object loop, so only inline
            // children are followed here.
            self.walk(value, id, &child, depth + 1);
        }
    }
}

/// 계획 11행 `long filter chain`·`embedded executable content`: 스트림 객체마다 보는 검사. `walk`와 독립이라 옛 지표의 순서·내용에 닿지 않는다.
fn stream_checks(doc: &Document, findings: &mut Vec<Finding>) {
    // `/EF` 사전이 가리키는 스트림과 `/Type /EmbeddedFile`인 스트림이 내장 파일이다.
    let mut embedded: BTreeSet<ObjectId> = BTreeSet::new();
    for obj in doc.objects.values() {
        collect_embedded(doc, obj, &mut embedded, 0);
    }
    for (id, obj) in &doc.objects {
        let Object::Stream(st) = obj else { continue };
        let chain = resolve(doc, st.dict.get(b"Filter").unwrap_or(&Object::Null));
        let long_chain = matches!(chain, Some(Object::Array(items)) if items.len() >= LONG_FILTER_CHAIN);
        let filter_cut = st.dict.has(FILTER_ARRAY_TRUNCATED_KEY);
        if long_chain || filter_cut {
            let detail = match chain {
                Some(Object::Array(items)) if !filter_cut => format!("{} filters", items.len()),
                _ => format!("{FILTER_KEEP_LEN} or more filters"),
            };
            findings.push(Finding { severity: Severity::Medium, kind: "long filter chain", object: *id, path: String::new(), detail });
        }
        // 한계: 로딩 훅이 스트림 내용을 64KiB로 자른다. Flate는 잘린 앞부분도 해제되어(앞 64KiB 해제분으로 판별) 잡히지만,
        // ASCII85·LZW 등 앞부분만으로는 해제되지 않는 필터 체인은 64KiB를 넘으면 놓친다.
        if embedded.contains(id) || st.dict.has_type(b"EmbeddedFile") {
            let data = st.decompressed_content_with_limit(MAX_DECOMPRESSED_STREAM_BYTES).unwrap_or_else(|_| st.content.clone());
            let head = data.get(..data.len().min(EMBEDDED_SNIFF_BYTES)).unwrap_or(&[]);
            let kind = match sniff(head) {
                Sniffed::Elf => Some("ELF"),
                Sniffed::MachO => Some("Mach-O"),
                Sniffed::Pe => Some("PE"),
                _ => None,
            };
            if let Some(kind) = kind {
                findings.push(Finding { severity: Severity::High, kind: "embedded executable content", object: *id, path: String::new(), detail: format!("{kind} executable") });
            }
        }
    }
}

/// `/EF` 사전(인라인이든 참조든)이 가리키는 스트림의 객체 번호를 모은다. 깊이는 `MAX_DEPTH`로 막는다.
fn collect_embedded(doc: &Document, obj: &Object, out: &mut BTreeSet<ObjectId>, depth: usize) {
    if depth > MAX_DEPTH {
        return;
    }
    let dict = match obj {
        Object::Dictionary(d) => d,
        Object::Stream(s) => &s.dict,
        Object::Array(items) => return items.iter().for_each(|v| collect_embedded(doc, v, out, depth + 1)),
        _ => return,
    };
    for (key, value) in dict.iter() {
        if key.as_slice() == b"EF"
            && let Some(Object::Dictionary(ef)) = resolve(doc, value)
        {
            out.extend(ef.iter().filter_map(|(_, v)| resolve_with_id(doc, v)).filter(|(_, o)| matches!(o, Object::Stream(_))).filter_map(|(id, _)| id));
        }
        collect_embedded(doc, value, out, depth + 1);
    }
}

/// 이름 토큰 안에 `#xx` 이스케이프가 든 것. 토큰 길이는 앞뒤 각 64바이트로 제한해, 긴 입력에서도 선형 시간이다.
static HEX_NAME: LazyLock<Option<Regex>> = LazyLock::new(|| Regex::new(r"/[^\x00\t\n\x0c\r ()<>\[\]{}/%]{0,64}#[0-9A-Fa-f]{2}[^\x00\t\n\x0c\r ()<>\[\]{}/%]{0,64}").ok());
static STARTXREF: LazyLock<Option<Regex>> = LazyLock::new(|| Regex::new("startxref").ok());

/// `#xx`를 푼 이름. 이스케이프가 아닌 `#`는 그대로 둔다.
fn unescape_name(token: &[u8]) -> Vec<u8> {
    let hex = |b: u8| char::from(b).to_digit(16).and_then(|d| u8::try_from(d).ok());
    let mut out = Vec::with_capacity(token.len());
    let mut i = 0;
    while let Some(&b) = token.get(i) {
        if b == b'#'
            && let (Some(h), Some(l)) = (token.get(i + 1).and_then(|&c| hex(c)), token.get(i + 2).and_then(|&c| hex(c)))
        {
            out.push(h * 16 + l);
            i += 3;
        } else {
            out.push(b);
            i += 1;
        }
    }
    out
}

/// 계획 11행 `hex-obfuscated action name`·`many incremental updates`: 파일 원본 바이트로 하는 검사. lopdf는 이름을 풀어서
/// 읽으므로 `/J#53`가 `/JS`와 구별되지 않는다. 그래서 원본에서 `#xx`로 쓴 `JS`·`JavaScript`·`Launch` 키를 직접 찾는다.
fn raw_checks(data: &[u8], report: &mut FileReport) {
    let mut seen: BTreeSet<Vec<u8>> = BTreeSet::new();
    for m in HEX_NAME.as_ref().into_iter().flat_map(|re| re.find_iter(data)) {
        let name = unescape_name(m.as_bytes().get(1..).unwrap_or(&[]));
        if [&b"JS"[..], b"JavaScript", b"Launch"].contains(&name.as_slice()) && seen.insert(name.clone()) {
            // 원본 바이트라 객체를 모른다. 위치는 바이트 오프셋으로 남긴다.
            let location = format!("obj 0 0:raw@{}", m.start());
            report.push(Category::Security, Severity::Medium, "hex-obfuscated action name", &location, &format!("/{} written with #xx escapes", sanitize(&name)));
        }
    }
    let updates = STARTXREF.as_ref().map_or(0, |re| re.find_iter(data).count());
    if updates >= MANY_INCREMENTAL_UPDATES {
        report.push(Category::Security, Severity::Low, "many incremental updates", "obj 0 0:startxref", &format!("{updates} startxref markers"));
    }
}

/// 한 건의 위치. 옛 `object`(번호, 세대)는 `obj N G:` 접두로 붙인다.
fn location_of(f: &Finding) -> String {
    format!("obj {} {}:{}", f.object.0, f.object.1, f.path)
}

fn scan_document(doc: &Document, report: &mut FileReport) {
    if doc.is_encrypted() {
        report.mark_incomplete("encrypted: string values may be unreadable");
    }
    let incomplete_object_streams: BTreeSet<u32> = doc
        .reference_table
        .entries
        .iter()
        .filter_map(|(object, entry)| match entry {
            XrefEntry::Compressed { container, .. } if !doc.objects.contains_key(&(*object, 0)) => Some(*container),
            _ => None,
        })
        .collect();
    for container in incomplete_object_streams {
        report.mark_incomplete(&format!("objects in object stream {container} could not be decoded within the {} byte limit", MAX_DECOMPRESSED_STREAM_BYTES));
    }
    let mut sc = Scanner { doc, findings: Vec::new() };
    sc.walk(&Object::Dictionary(doc.trailer.clone()), (0, 0), "trailer", 0);
    for (id, obj) in &doc.objects {
        sc.walk(obj, *id, "", 0);
    }
    stream_checks(doc, &mut sc.findings);
    // Stable, most severe first.
    sc.findings.sort_by(|a, b| b.severity.cmp(&a.severity).then(a.object.cmp(&b.object)));
    for f in &sc.findings {
        report.push(Category::Security, f.severity, f.kind, &location_of(f), &f.detail);
    }
}

/// PDF를 읽는다. 구문 오류와 파서 panic을 모두 `Err(사유)`로 돌려준다(사유는 정화한다).
/// `data`에 `%PDF-` 머리가 있는지는 호출자가 이미 확인했다.
pub fn load(data: &[u8]) -> Result<(Document, LoadInfo), String> {
    // 참조 수가 예산을 넘는 파일은 참조 배열을 남기지 않는 훅으로 읽는다. 선택은 읽기 전에 끝나므로 스레드·동시 읽기와 무관하다.
    let hook: lopdf::FilterFunc = if count_raw_references(data) > REF_FILE_BUDGET { shrink_on_load_strict } else { shrink_on_load };
    // A malformed file must not take the whole batch down.
    let parsed = std::panic::catch_unwind(|| {
        Document::load_mem_with_options(data, LoadOptions { filter: Some(hook), max_decompressed_size: Some(MAX_DECOMPRESSED_STREAM_BYTES), ..LoadOptions::default() })
    });
    match parsed {
        Ok(Ok(doc)) => {
            let info = LoadInfo::from_document(&doc);
            Ok((doc, info))
        }
        Ok(Err(e)) => Err(sanitize_text(&e.to_string(), PREVIEW_CHARS, Some('…'))),
        Err(payload) => Err(panic_reason(payload.as_ref())),
    }
}

/// 파서 panic 페이로드를 `incomplete` 사유로 만든다. 페이로드는 파일 내용에서 올 수 있으므로 `epub::security::guarded`처럼 정화한다.
fn panic_reason(payload: &(dyn std::any::Any + Send)) -> String {
    let msg = payload.downcast_ref::<&str>().map(|m| (*m).to_owned()).or_else(|| payload.downcast_ref::<String>().cloned()).unwrap_or_else(|| "unknown".into());
    sanitize_text(&format!("panic: parser panicked on malformed input: {msg}"), PREVIEW_CHARS, Some('…'))
}

/// 이미 읽은 문서(없을 수 있다)로 보안 검사를 한다. 구문 오류 보고는 하지 않는다(`pdf::structure`가 한다).
pub fn scan_loaded(data: &[u8], doc: Option<&Document>, report: &mut FileReport) {
    // 구문 분석 성패와 무관하게 원본 바이트 검사를 한다. 깨진 파일일수록 은닉을 쓸 수 있다.
    guarded(report, |r| raw_checks(data, r));
    // 문서를 훑다 panic하면 그때까지 모은 지표는 버린다(옛 구조와 같다: 지표는 정렬 뒤에 한꺼번에 낸다).
    if let Some(doc) = doc {
        guarded(report, |r| scan_document(doc, r));
    }
}

/// `data`의 앞 1024바이트에 `%PDF-`가 있는가.
pub fn has_header(data: &[u8]) -> bool {
    data.windows(5).take(1024).any(|w| w == b"%PDF-")
}

/// PDF 바이트를 검사해 `report`에 더한다. 크기 제한은 호출자(`common::inspect`)가 이미 걸었다.
/// 구문 오류는 `incomplete` 사유로만 남긴다. 구조 지표(`XREF_BROKEN` 등)는 `pdf::structure::inspect`가 낸다.
pub fn scan(data: &[u8], report: &mut FileReport) {
    if !has_header(data) {
        report.mark_incomplete("no %PDF- header");
        return;
    }
    match load(data) {
        Ok((doc, _)) => scan_loaded(data, Some(&doc), report),
        Err(e) => {
            scan_loaded(data, None, report);
            report.mark_incomplete(&format!("parse error: {e}"));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn panic_reason_replaces_control_characters_and_truncates() {
        let payload: Box<dyn std::any::Any + Send> = Box::new(format!("bad\x1b[31mname\n{}", "x".repeat(300)));
        let r = panic_reason(payload.as_ref());
        assert!(!r.chars().any(|c| c.is_control()), "{r:?}");
        assert!(r.starts_with("panic: parser panicked") && r.chars().count() <= PREVIEW_CHARS + 1, "{r:?}");
        let unknown: Box<dyn std::any::Any + Send> = Box::new(5u8);
        assert!(panic_reason(unknown.as_ref()).ends_with("unknown"));
    }

    fn long_name_array() -> Object {
        Object::Array((0..40).map(|_| Object::Name(b"FlateDecode".to_vec())).collect())
    }

    fn filter_stream(chain_len: usize) -> Object {
        let mut dict = Dictionary::new();
        dict.set("Filter", Object::Array((0..chain_len).map(|_| Object::Name(b"FlateDecode".to_vec())).collect()));
        dict.set("DecodeParms", Object::Array((0..chain_len).map(|_| Object::Null).collect()));
        dict.set("Widths", Object::Array((0..100).map(Object::Integer).collect()));
        let mut obj = Object::Stream(lopdf::Stream::new(dict, b"x".to_vec()));
        shrink(&mut obj, 0);
        obj
    }

    #[test]
    fn shrink_keeps_a_realistic_filter_chain_and_drops_other_long_numeric_arrays() {
        let Object::Stream(st) = filter_stream(FILTER_KEEP_LEN) else { unreachable!() };
        // 필터 배열이 비면 인코딩된 JS가 해독되지 않은 채 검사를 빠져나간다.
        assert_eq!(st.dict.get(b"Filter").unwrap().as_array().unwrap().len(), FILTER_KEEP_LEN);
        assert_eq!(st.dict.get(b"DecodeParms").unwrap().as_array().unwrap().len(), FILTER_KEEP_LEN);
        assert!(!st.dict.has(FILTER_TRUNCATED_KEY), "자르지 않았으면 표지가 없어야 한다");
        assert!(st.dict.get(b"Widths").unwrap().as_array().unwrap().is_empty());
    }

    #[test]
    fn shrink_never_empties_a_decoding_key_but_still_shrinks_what_is_inside_its_dictionaries() {
        // DecodeParms 안의 사전에 든 거대한 숫자 배열은 비워서 메모리를 지킨다. 하지만 필터 배열 자체는 절대 비우면 안 된다.
        let mut dict = Dictionary::new();
        dict.set("Filter", Object::Array((0..FILTER_KEEP_LEN).map(|_| Object::Name(b"FlateDecode".to_vec())).collect()));
        let parms: Vec<Object> = (0..FILTER_KEEP_LEN)
            .map(|_| {
                let mut p = Dictionary::new();
                p.set("Predictor", 12);
                p.set("Big", Object::Array((0..500).map(Object::Integer).collect()));
                Object::Dictionary(p)
            })
            .collect();
        dict.set("DecodeParms", Object::Array(parms));
        let mut obj = Object::Stream(lopdf::Stream::new(dict, b"x".to_vec()));
        shrink(&mut obj, 0);
        let Object::Stream(st) = obj else { unreachable!() };
        assert_eq!(st.dict.get(b"Filter").unwrap().as_array().unwrap().len(), FILTER_KEEP_LEN);
        let parms = st.dict.get(b"DecodeParms").unwrap().as_array().unwrap();
        assert_eq!(parms.len(), FILTER_KEEP_LEN);
        let Object::Dictionary(first) = &parms[0] else { unreachable!() };
        assert!(first.get(b"Big").unwrap().as_array().unwrap().is_empty());
        assert_eq!(first.get(b"Predictor").unwrap().as_i64().unwrap(), 12);
        assert!(!st.dict.has(FILTER_TRUNCATED_KEY));
    }

    #[test]
    fn shrink_bounds_an_absurdly_long_filter_chain_and_marks_the_stream() {
        // 이름 수백만 개짜리 /Filter가 그대로 남으면 노드(약 120B)로 메모리를 부풀린다. 길이를 제한하고 표지를 붙인다.
        let Object::Stream(st) = filter_stream(100_000) else { unreachable!() };
        assert_eq!(st.dict.get(b"Filter").unwrap().as_array().unwrap().len(), FILTER_KEEP_LEN);
        assert_eq!(st.dict.get(b"DecodeParms").unwrap().as_array().unwrap().len(), FILTER_KEEP_LEN);
        assert!(st.dict.has(FILTER_TRUNCATED_KEY));
    }

    #[test]
    fn a_stream_with_a_truncated_filter_chain_is_reported_as_not_fully_inspected() {
        let mut doc = Document::new();
        let Object::Stream(marked) = filter_stream(100_000) else { unreachable!() };
        // 필터가 없는 작은 스트림: 표지도 없고 해제도 문제없다.
        let plain = lopdf::Stream::new(Dictionary::new(), b"app.alert(1)".to_vec());
        doc.objects.insert((5, 0), Object::Stream(marked));
        doc.objects.insert((6, 0), Object::Stream(plain));
        assert!(js_stream_not_fully_inspected(&doc, &Object::Reference((5, 0))));
        // 표지가 없고 작은 스트림이면 알리지 않는다. 참조가 아니거나 없는 객체도 마찬가지다.
        assert!(!js_stream_not_fully_inspected(&doc, &Object::Reference((6, 0))));
        assert!(!js_stream_not_fully_inspected(&doc, &Object::Integer(1)));
        assert!(!js_stream_not_fully_inspected(&doc, &Object::Reference((99, 0))));
    }

    #[test]
    fn shrink_drops_long_numeric_arrays_in_plain_dictionaries_but_not_mixed_ones() {
        let mut d = Dictionary::new();
        d.set("Widths", Object::Array((0..100).map(Object::Integer).collect()));
        d.set("Names", long_name_array());
        let mut mixed: Vec<Object> = (0..40).map(Object::Integer).collect();
        mixed.push(Object::String(b"keep".to_vec(), lopdf::StringFormat::Literal));
        d.set("Mixed", Object::Array(mixed));
        let mut obj = Object::Dictionary(d);
        shrink(&mut obj, 0);
        let Object::Dictionary(d) = obj else { unreachable!() };
        assert!(d.get(b"Widths").unwrap().as_array().unwrap().is_empty());
        assert!(d.get(b"Names").unwrap().as_array().unwrap().is_empty());
        assert_eq!(d.get(b"Mixed").unwrap().as_array().unwrap().len(), 41);
    }

    #[test]
    fn load_keeps_a_page_tree_with_many_kids() {
        // /Kids는 참조만 든 긴 배열이다. 숫자 배열처럼 비우면 쪽 트리가 사라져 멀쩡한 PDF가 0쪽이 된다(선형화 이미지 PDF에서 재현).
        let mut doc = Document::with_version("1.5");
        let mut kids = Vec::new();
        for n in 0..40u32 {
            let mut page = Dictionary::new();
            page.set("Type", "Page");
            page.set("Parent", Object::Reference((2, 0)));
            page.set("MediaBox", Object::Array(vec![0.into(), 0.into(), 100.into(), 100.into()]));
            doc.objects.insert((10 + n, 0), Object::Dictionary(page));
            kids.push(Object::Reference((10 + n, 0)));
        }
        let mut pages = Dictionary::new();
        pages.set("Type", "Pages");
        pages.set("Count", 40);
        pages.set("Kids", Object::Array(kids));
        doc.objects.insert((2, 0), Object::Dictionary(pages));
        let mut cat = Dictionary::new();
        cat.set("Type", "Catalog");
        cat.set("Pages", Object::Reference((2, 0)));
        doc.objects.insert((1, 0), Object::Dictionary(cat));
        doc.trailer.set("Root", Object::Reference((1, 0)));
        doc.max_id = 50;
        let mut bytes = Vec::new();
        doc.save_to(&mut bytes).unwrap();
        assert_eq!(load(&bytes).unwrap().0.get_pages().len(), 40);
    }

    #[test]
    fn load_keeps_a_page_tree_whose_kids_is_an_indirect_array() {
        // 쪽 많은 PDF는 `/Kids 21 0 R`처럼 배열을 독립 객체로 둔다. 훅은 키를 모르므로 참조 배열은 내용으로 보존해야 한다.
        let mut doc = Document::with_version("1.5");
        let mut kids = Vec::new();
        for n in 0..40u32 {
            let mut page = Dictionary::new();
            page.set("Type", "Page");
            page.set("Parent", Object::Reference((2, 0)));
            doc.objects.insert((10 + n, 0), Object::Dictionary(page));
            kids.push(Object::Reference((10 + n, 0)));
        }
        doc.objects.insert((21, 0), Object::Array(kids));
        let mut pages = Dictionary::new();
        pages.set("Type", "Pages");
        pages.set("Count", 40);
        pages.set("Kids", Object::Reference((21, 0)));
        doc.objects.insert((2, 0), Object::Dictionary(pages));
        let mut cat = Dictionary::new();
        cat.set("Type", "Catalog");
        cat.set("Pages", Object::Reference((2, 0)));
        doc.objects.insert((1, 0), Object::Dictionary(cat));
        doc.trailer.set("Root", Object::Reference((1, 0)));
        doc.max_id = 60;
        let mut bytes = Vec::new();
        doc.save_to(&mut bytes).unwrap();
        assert_eq!(load(&bytes).unwrap().0.get_pages().len(), 40);
    }

    #[test]
    fn shrink_keeps_reference_arrays_up_to_the_cap_and_marks_longer_ones() {
        let mut short = Object::Array((0..1000).map(|n| Object::Reference((n, 0))).collect());
        shrink(&mut short, 0);
        assert_eq!(short.as_array().unwrap().len(), 1000);
        let mut long = Object::Array((0..=REF_ARRAY_KEEP_LEN as u32).map(|n| Object::Reference((n, 0))).collect());
        shrink(&mut long, 0);
        assert!(matches!(long.as_array().unwrap().as_slice(), [Object::Name(n)] if n.as_slice() == EMPTIED_REF_ARRAY_MARK));
    }

    /// `kids`개 쪽을 가진 PDF 바이트. 쪽 외에 `filler`개의 빈 사전 객체를 더해 객체 수를 늘린다(16개 이상이면 lopdf가 rayon으로 읽는다).
    fn pdf_with_kids(kids: u32, filler: u32) -> Vec<u8> {
        let mut doc = Document::with_version("1.5");
        let mut list = Vec::new();
        for n in 0..kids {
            let mut page = Dictionary::new();
            page.set("Type", "Page");
            page.set("Parent", Object::Reference((2, 0)));
            doc.objects.insert((10 + n, 0), Object::Dictionary(page));
            list.push(Object::Reference((10 + n, 0)));
        }
        for n in 0..filler {
            doc.objects.insert((kids + 20 + n, 0), Object::Dictionary(Dictionary::new()));
        }
        let mut pages = Dictionary::new();
        pages.set("Type", "Pages");
        pages.set("Count", i64::from(kids));
        pages.set("Kids", Object::Array(list));
        doc.objects.insert((2, 0), Object::Dictionary(pages));
        let mut cat = Dictionary::new();
        cat.set("Type", "Catalog");
        cat.set("Pages", Object::Reference((2, 0)));
        doc.objects.insert((1, 0), Object::Dictionary(cat));
        doc.trailer.set("Root", Object::Reference((1, 0)));
        doc.max_id = kids + filler + 30;
        let mut bytes = Vec::new();
        doc.save_to(&mut bytes).unwrap();
        bytes
    }

    #[test]
    fn real_load_of_a_parallel_sized_pdf_reports_an_emptied_oversized_kids_array() {
        // 객체가 16개를 넘으면 lopdf가 훅을 rayon 작업 스레드에서 부른다. 정보는 스레드와 무관하게 읽은 뒤 계산돼야 한다.
        let (doc, info) = load(&pdf_with_kids(REF_ARRAY_KEEP_LEN as u32 + 5, 40)).unwrap();
        assert!(info.emptied_ref_array && info.page_tree_unreliable());
        assert!(doc.get_pages().is_empty());
    }

    #[test]
    fn consecutive_loads_do_not_affect_each_other() {
        let big = pdf_with_kids(REF_ARRAY_KEEP_LEN as u32 + 5, 40);
        let small = pdf_with_kids(40, 40);
        for _ in 0..3 {
            assert!(load(&big).unwrap().1.emptied_ref_array);
            let (doc, info) = load(&small).unwrap();
            assert_eq!(info, LoadInfo { emptied_ref_array: false, kept_ref_nodes: 40 });
            assert_eq!(doc.get_pages().len(), 40);
        }
    }

    #[test]
    fn a_reference_bomb_is_read_with_the_strict_hook_so_peak_memory_stays_bounded() {
        // 1만 개짜리 배열(배열당 상한 2만 밑) 80개 = 80만 참조는 예산(50만)을 넘는다. 엄격 훅이 모두 비우고 표지를 남긴다.
        let mut doc = Document::with_version("1.5");
        for n in 0..80u32 {
            doc.objects.insert((10 + n, 0), Object::Array((0..10_000).map(|r| Object::Reference((1000 + r, 0))).collect()));
        }
        let mut cat = Dictionary::new();
        cat.set("Type", "Catalog");
        doc.objects.insert((1, 0), Object::Dictionary(cat));
        doc.trailer.set("Root", Object::Reference((1, 0)));
        doc.max_id = 100;
        let mut bytes = Vec::new();
        doc.save_to(&mut bytes).unwrap();
        assert!(count_raw_references(&bytes) > REF_FILE_BUDGET);
        let (loaded, info) = load(&bytes).unwrap();
        assert!(info.emptied_ref_array && info.page_tree_unreliable());
        assert_eq!(info.kept_ref_nodes, 0);
        let kept: usize = loaded.objects.values().filter_map(|o| o.as_array().ok()).map(Vec::len).sum();
        assert!(kept <= 80, "남은 원소 {kept}");
    }

    #[test]
    fn kept_reference_arrays_over_the_file_budget_make_the_page_tree_unreliable() {
        let info = LoadInfo { emptied_ref_array: false, kept_ref_nodes: REF_FILE_BUDGET + 1 };
        assert!(info.page_tree_unreliable());
        assert!(!LoadInfo { emptied_ref_array: false, kept_ref_nodes: REF_FILE_BUDGET }.page_tree_unreliable());
    }

    #[test]
    fn shrink_stops_at_the_depth_guard_instead_of_recursing_without_bound() {
        // lopdf 파서는 중첩을 100으로 제한하지만 그 보장은 의존성 버전에 있다. 넘는 깊이는 건드리지 않고 스택도 쓰지 않는다.
        let mut obj = Object::Integer(1);
        for _ in 0..(SHRINK_MAX_DEPTH * 3) {
            obj = Object::Array(vec![obj]);
        }
        shrink(&mut obj, 0);
        let Object::Array(outer) = &obj else { unreachable!() };
        assert_eq!(outer.len(), 1, "깊이 가드를 넘는 배열은 비우지 않는다");
    }

    #[test]
    fn finding_paths_never_carry_control_characters_from_key_names() {
        let doc = Document::new();
        let mut action = Dictionary::new();
        action.set("S", Object::Name(b"Launch".to_vec()));
        let mut outer = Dictionary::new();
        outer.set(b"\x1b[2J\x07evil".to_vec(), Object::Dictionary(action));
        let mut sc = Scanner { doc: &doc, findings: Vec::new() };
        sc.walk(&Object::Dictionary(outer), (1, 0), "", 0);
        assert!(!sc.findings.is_empty());
        for f in &sc.findings {
            assert!(!f.path.chars().any(char::is_control), "{:?}", f.path);
        }
    }
}
