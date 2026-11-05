//! PDF를 메모리 상한 안에서 읽는다. lopdf 로딩 훅이 객체를 파싱하자마자 스캔에 필요 없는 부분을 줄이고,
//! 읽은 뒤에는 줄이느라 잘렸을 수 있는 쪽 트리를 `LoadInfo`로 알린다.

use super::*;

/// 로딩 중 스트림 내용을 이만큼만 남긴다. lopdf는 모든 스트림 내용을 복사해서 들고 있어서, 이미지가 많은 PDF가 입력의 2배
/// 넘게 메모리를 쓴다(223MB 파일이 458MB). 한도를 256KB→64KB→4KB로 낮추면 454→302→240MB로 줄었다(입력은 226MB).
/// 64KB는 JavaScript 해제분 미리보기 한도와 같고, JS는 압축하면 보통 수 KB다. 4KB는 일반 JS까지 잘라서 쓸 수 없다.
/// 이보다 큰 JavaScript 스트림은 잘려서 끝까지 못 본 것이므로 `not fully inspected`로 알린다.
pub(super) const STREAM_KEEP_BYTES: usize = 64 * 1024;
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
pub(crate) const REF_FILE_BUDGET: usize = 500_000;
/// 훅이 비운 참조 배열이 들어 있던 자리에 남기는 표지 이름. 진짜 빈 배열과 구별해 읽은 뒤에 찾는다.
const EMPTIED_REF_ARRAY_MARK: &[u8] = b"PdfscanEmptiedRefArray";

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
pub(super) const FILTER_KEEP_LEN: usize = 16;
// 필터 키의 배열은 길이를 제한한 뒤에도 일반 줄이기를 거친다. 그 배열이 비워지면(해독이 사라지면) 안 되므로, 제한한 길이가
// `ARRAY_KEEP_LEN`보다 작아야 한다(작으면 노드 수가 비우는 기준에 닿지 않는다). 둘을 바꿀 때 어기지 않게 컴파일 시점에 묶는다.
const _: () = assert!(FILTER_KEEP_LEN + 1 < ARRAY_KEEP_LEN);
/// 필터 체인을 잘랐다는 표지. 잘리면 해독이 틀어지므로 그 스트림은 "끝까지 검사하지 못한" 것으로 알려야 한다.
/// 파일이 같은 키를 직접 넣어도 알림이 하나 더 붙을 뿐이라 안전한 쪽이다.
pub(super) const FILTER_TRUNCATED_KEY: &[u8] = b"PdfscanFilterTruncated";
/// `/Filter` 배열 자체를 잘랐다는 표지. 위 표지는 `DecodeParms` 등 다른 해독 키만 잘려도 붙으므로 어느 키인지 말하지 못한다.
/// `long filter chain`은 필터 체인이 길다는 뜻이라 `/Filter`가 잘린 경우에만 이 표지로 판단한다(위 표지는 그대로 둔다).
pub(super) const FILTER_ARRAY_TRUNCATED_KEY: &[u8] = b"PdfscanFilterArrayTruncated";

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

/// 파일 원본에서 `번호 세대 R` 꼴 참조가 몇 개인지 센다. 패턴은 lopdf 0.46 `reference` 문법을 따른다: `(정수 space)(정수 space) R`,
/// space는 공백 바이트(공백·탭·CR·LF·FF·NUL)나 `%` 주석(줄끝까지)이 0개 이상이다. 두 정수 사이는 `digit1`이 탐욕적이라 구분이 하나
/// 이상 있어야 하고, 세대 번호와 R 사이는 없어도 되며(`1 0R`), R 뒤 구분자도 요구하지 않는다(`1 0 R2 0 R`). 그래서 `\b`를 두지 않는다.
/// 이 때문에 `0 0 1 RG` 같은 비참조도 셀 수 있으나, 이 값은 예산 판정용 상한이라 많이 세는 쪽이 안전하다. 한 번의 선형 훑기다.
///
/// 한계(압축 객체 스트림): 스트림 안의 참조는 압축돼 있어 세지 못하고, 안쪽 객체도 훅을 거치지만 31개 이하 참조 배열은 두 훅 모두
/// 남긴다. 해제 한도(`MAX_DECOMPRESSED_STREAM_BYTES`, 4MiB)는 스트림 하나에만 걸린다. 4MiB 스트림은 압축하면 수 KB라 그런 스트림
/// 하나가 참조 약 70만 개를 남기고(노드만 세면 약 80MB), 실측으로는 ObjStm 하나당 RSS가 약 200MB였다(합성 PDF: ObjStm 10개,
/// 31개짜리 배열 50만 개, 파일 3.7MB가 RSS 2.0GB이고 보고는 `normal`, 종료 코드 0). 노드 외 할당자 오버헤드와 파싱 중간 사본이 더해진
/// 값이다. 일반 PDF도 31개짜리 숫자 배열 20만 개(21MB)가 RSS 849MB를 썼고 `normal`이었다. 남은 노드는 `doc.objects`에 문서
/// 수명 내내 유지되고 `/ObjStm` 개수 상한이 없으므로 최악은 약 200MB × ObjStm 개수이며, 프로세스 안에는 이를 막는 한도가 없다. 이는 예전 PDF 검사 도구와 같은 동작이다(Ruling R20).
/// 작업자 수 상한은 문서 하나의 메모리를 막지 못하고, 외부 메모리 한도(cgroup/ulimit)만 막는다. `/ObjStm` 개수로 훅을 고르면
/// ObjStm를 쓰는 현대 PDF 대부분이 엄격 훅이 되어 긴 /Kids가 모두 비워지므로(오탐성 incomplete) 하지 않는다.
pub(crate) fn count_raw_references(data: &[u8]) -> usize {
    static RE: LazyLock<Option<Regex>> = LazyLock::new(|| Regex::new(r"(?-u)[0-9]+(?:[ \t\r\n\x0c\x00]|%[^\r\n]*[\r\n])+[0-9]+(?:[ \t\r\n\x0c\x00]|%[^\r\n]*[\r\n])*R").ok());
    RE.as_ref().map_or(0, |re| re.find_iter(data).count())
}

/// PDF를 읽는다. 구문 오류와 파서 panic을 모두 `Err(사유)`로 돌려준다(사유는 정화한다).
/// `data`에 `%PDF-` 머리가 있는지는 호출자가 이미 확인했다.
pub fn load(data: &[u8]) -> Result<(Document, LoadInfo), String> {
    // 참조 수가 예산을 넘는 파일은 참조 배열을 남기지 않는 훅으로 읽는다. 선택은 읽기 전에 끝나므로 스레드·동시 읽기와 무관하다.
    let hook: lopdf::FilterFunc = if count_raw_references(data) > REF_FILE_BUDGET { shrink_on_load_strict } else { shrink_on_load };
    // A malformed file must not take the whole batch down.
    let parsed =
        std::panic::catch_unwind(|| Document::load_mem_with_options(data, LoadOptions { filter: Some(hook), max_decompressed_size: Some(MAX_DECOMPRESSED_STREAM_BYTES), ..LoadOptions::default() }));
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
    fn reference_counting_covers_other_generations_and_whitespace_separators() {
        assert_eq!(count_raw_references(b"[1 0 R 2 5 R 3\n0\nR 4 0\nR 5\t1\tR 6\r\n0\r\nR]"), 6);
        // 참조가 아닌 것: 숫자가 하나뿐이거나 R이 없다. 두 정수 사이는 구분이 꼭 있어야 한다(`10R`은 정수 하나다).
        assert_eq!(count_raw_references(b"7 R /Name 3 0 obj 10R"), 0);
        // `1 0 Rx`는 센다. lopdf는 R 뒤 구분자를 요구하지 않고, 상한은 많이 세는 쪽이 안전하다.
        assert_eq!(count_raw_references(b"1 0 Rx"), 1);
    }

    #[test]
    fn reference_counting_follows_the_lopdf_grammar_without_a_trailing_delimiter() {
        // lopdf `reference`: (정수 space)(정수 space) 뒤 `R`. space는 0개 이상이고 R 뒤 구분자는 요구하지 않는다.
        assert_eq!(count_raw_references(b"[1 0R2 0R3 0R]"), 3);
        assert_eq!(count_raw_references(b"[1 0 R2 0 R3 0 R]"), 3);
        // 구분으로 주석(`%...` + 줄끝)도 쓸 수 있다.
        assert_eq!(count_raw_references(b"[1 %c\n0 %c\nR 2%x\r\n0%y\rR]"), 2);
        // 위 꼴들을 lopdf가 실제로 참조로 읽는지 확인한다(세는 규칙이 문법과 어긋나지 않게).
        let pdf = b"%PDF-1.4\n10 0 obj\n[1 0R2 0 R3 %c\n0%c\nR4 0R]\nendobj\n1 0 obj\n<< /Type /Catalog >>\nendobj\ntrailer\n<< /Root 1 0 R >>\n%%EOF\n";
        assert_eq!(count_raw_references(pdf), 5);
        let (doc, _) = load(pdf).unwrap();
        let arr = doc.get_object((10, 0)).unwrap().as_array().unwrap();
        assert_eq!(arr.len(), 4);
        assert!(arr.iter().all(|o| o.as_reference().is_ok()));
    }

    #[test]
    fn a_bomb_written_without_space_before_r_goes_to_the_strict_hook() {
        // `N 0R` 꼴 1만 개 배열 80개 = 80만 참조. 예전 패턴은 이 꼴을 0으로 세어 관대한 훅으로 보냈다.
        let mut pdf = b"%PDF-1.4\n".to_vec();
        for n in 0..80u32 {
            pdf.extend_from_slice(format!("{} 0 obj\n[", 10 + n).as_bytes());
            for r in 0..10_000u32 {
                pdf.extend_from_slice(format!("{} 0R", 1000 + r).as_bytes());
            }
            pdf.extend_from_slice(b"]\nendobj\n");
        }
        pdf.extend_from_slice(b"1 0 obj\n<< /Type /Catalog >>\nendobj\ntrailer\n<< /Root 1 0 R >>\n%%EOF\n");
        assert!(count_raw_references(&pdf) > REF_FILE_BUDGET);
        let (loaded, info) = load(&pdf).unwrap();
        assert!(info.emptied_ref_array && info.page_tree_unreliable());
        assert_eq!(info.kept_ref_nodes, 0);
        let kept: usize = loaded.objects.values().filter_map(|o| o.as_array().ok()).map(Vec::len).sum();
        assert!(kept <= 80, "남은 원소 {kept}");
    }

    #[test]
    fn a_hand_written_bomb_with_generation_one_and_newlines_goes_to_the_strict_hook() {
        // ` 0 R`만 세던 때는 이 형태가 예산을 피했다. 한 배열 1만 개(상한 밑)로 80개를 쓴 PDF를 직접 쓴다.
        let mut pdf = b"%PDF-1.4\n".to_vec();
        for n in 0..80u32 {
            pdf.extend_from_slice(format!("{} 0 obj\n[", 10 + n).as_bytes());
            for r in 0..10_000u32 {
                pdf.extend_from_slice(format!("{}\n1\nR ", 1000 + r).as_bytes());
            }
            pdf.extend_from_slice(b"]\nendobj\n");
        }
        pdf.extend_from_slice(b"1 0 obj\n<< /Type /Catalog >>\nendobj\ntrailer\n<< /Root 1 0 R >>\n%%EOF\n");
        assert!(count_raw_references(&pdf) > REF_FILE_BUDGET);
        // 엄격 훅은 32개 초과 참조 배열을 모두 비운다(표지 하나로 바꾼다).
        let mut arr = Object::Array((0..10_000).map(|n| Object::Reference((n, 0))).collect());
        shrink_with(&mut arr, 0, false);
        assert_eq!(arr.as_array().unwrap().len(), 1);
        // 31개 이하 배열은 엄격 훅도 남긴다(`plain_nodes`가 배열 자신을 1로 세므로 실제 경계는 31/32다). 압축 객체 스트림 위험이 훅 선택으로 줄지 않는 이유다.
        let mut small = Object::Array((0..31).map(|n| Object::Reference((n, 0))).collect());
        shrink_with(&mut small, 0, false);
        assert_eq!(small.as_array().unwrap().len(), 31);
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
}
