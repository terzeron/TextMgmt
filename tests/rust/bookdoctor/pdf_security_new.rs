//! bookdoctor PDF 보안 신규 검사(계획 Task 11 + 11-A/B/C 보충): 코드마다 양성·음성 쌍을 둔다.

use std::path::Path;

use bookdoctor::pdf::security;
use scan_common::{Category, FileReport, Severity};

/// 객체 본문 목록으로 PDF를 만든다. 1번 Catalog, 2번 Pages, 3번 Page, 4번부터 `more`다. 본문은 바이트라 스트림을 담을 수 있다.
fn pdf(catalog_extra: &str, more: &[Vec<u8>]) -> Vec<u8> {
    let mut bodies: Vec<Vec<u8>> = vec![
        format!("<< /Type /Catalog /Pages 2 0 R {catalog_extra} >>").into_bytes(),
        b"<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_vec(),
        b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 10 10] >>".to_vec(),
    ];
    bodies.extend(more.iter().cloned());
    build(&bodies)
}

fn build(bodies: &[Vec<u8>]) -> Vec<u8> {
    let mut out = b"%PDF-1.7\n".to_vec();
    let mut offsets = Vec::new();
    for (i, body) in bodies.iter().enumerate() {
        offsets.push(out.len());
        out.extend(format!("{} 0 obj\n", i + 1).bytes());
        out.extend(body);
        out.extend(b"\nendobj\n");
    }
    let xref = out.len();
    out.extend(format!("xref\n0 {}\n0000000000 65535 f \n", bodies.len() + 1).bytes());
    for o in offsets {
        out.extend(format!("{o:010} 00000 n \n").bytes());
    }
    out.extend(format!("trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n", bodies.len() + 1).bytes());
    out
}

fn stream(dict_extra: &str, content: &[u8]) -> Vec<u8> {
    let mut v = format!("<< /Length {} {dict_extra} >>\nstream\n", content.len()).into_bytes();
    v.extend(content);
    v.extend(b"\nendstream");
    v
}

fn run(bytes: &[u8]) -> FileReport {
    let mut r = FileReport::new(Path::new("t.pdf"), None);
    security::scan(bytes, &mut r);
    r
}

fn hits<'a>(r: &'a FileReport, code: &str) -> Vec<&'a scan_common::Finding> {
    r.findings.iter().filter(|f| f.code == code && f.category == Category::Security).collect()
}

fn s(x: &str) -> Vec<u8> {
    x.as_bytes().to_vec()
}

/// 카탈로그 OpenAction으로 동작 사전을 4번 객체에 달아 검사한다.
fn action(body: &str, more: &[Vec<u8>]) -> FileReport {
    let mut objs = vec![s(body)];
    objs.extend_from_slice(more);
    run(&pdf("/OpenAction 4 0 R", &objs))
}

// ---------------------------------------------------------------- 동작 코드

#[test]
fn flags_gotor_action_with_target() {
    let r = action("<< /S /GoToR /F (other.pdf) /D [0 /Fit] >>", &[]);
    let h = hits(&r, "GoToR action");
    assert_eq!(h.len(), 1);
    assert_eq!(h[0].severity, Severity::Medium);
    assert!(h[0].detail.contains("other.pdf"), "{}", h[0].detail);
    assert!(h[0].location.starts_with("obj 4 0:"), "{}", h[0].location);
}

#[test]
fn flags_gotoe_action() {
    let r = action("<< /S /GoToE /T << /R /C /N (a) >> >>", &[]);
    let h = hits(&r, "GoToE action");
    assert_eq!(h.len(), 1);
    assert_eq!(h[0].severity, Severity::Medium);
}

#[test]
fn flags_submitform_action_with_target() {
    let r = action("<< /S /SubmitForm /F (http://x.example/post) >>", &[]);
    let h = hits(&r, "SubmitForm action");
    assert_eq!(h.len(), 1);
    assert_eq!(h[0].severity, Severity::Medium);
    assert!(h[0].detail.contains("x.example"), "{}", h[0].detail);
}

#[test]
fn flags_importdata_action() {
    let r = action("<< /S /ImportData /F (data.fdf) >>", &[]);
    let h = hits(&r, "ImportData action");
    assert_eq!(h.len(), 1);
    assert_eq!(h[0].severity, Severity::Medium);
}

#[test]
fn new_action_codes_do_not_fire_for_goto_and_legacy_code_is_kept() {
    let r = action("<< /S /GoTo /D [0 /Fit] >>", &[]);
    for code in ["GoToR action", "GoToE action", "SubmitForm action", "ImportData action", "unresolvable action type"] {
        assert!(hits(&r, code).is_empty(), "{code}");
    }
    // 옛 묶음 지표는 새 코드와 함께 그대로 나온다.
    let r = action("<< /S /GoToR /F (o.pdf) >>", &[]);
    assert_eq!(hits(&r, "external/media action").len(), 1);
}

// ---------------------------------------------------------------- 은닉 이름

#[test]
fn flags_hex_obfuscated_action_name() {
    // /J#53은 /JS, /Java#53cript는 /JavaScript, /L#61unch는 /Launch다.
    for (token, plain) in [("/J#53", "JS"), ("/Java#53cript", "JavaScript"), ("/L#61unch", "Launch")] {
        let r = run(&pdf(&format!("{token} (x)"), &[]));
        let h = hits(&r, "hex-obfuscated action name");
        assert_eq!(h.len(), 1, "{token}");
        assert_eq!(h[0].severity, Severity::Medium);
        assert!(h[0].detail.contains(plain), "{}", h[0].detail);
    }
}

#[test]
fn hex_name_check_ignores_plain_names_and_unrelated_escapes() {
    for extra in ["/JS (x)", "/Launch (x)", "/Foo#20Bar 1", "/J#5 1", "/Other#53 1"] {
        let r = run(&pdf(extra, &[]));
        assert!(hits(&r, "hex-obfuscated action name").is_empty(), "{extra}");
    }
}

// ---------------------------------------------------------------- 필터 체인

#[test]
fn flags_long_filter_chain() {
    let names = "/ASCIIHexDecode ".repeat(5);
    let r = run(&pdf("", &[stream(&format!("/Filter [{names}]"), b"x")]));
    let h = hits(&r, "long filter chain");
    assert_eq!(h.len(), 1);
    assert_eq!(h[0].severity, Severity::Medium);
    assert!(h[0].location.starts_with("obj 4 0:"));
}

#[test]
fn short_filter_chain_is_not_flagged() {
    let names = "/ASCIIHexDecode ".repeat(4);
    let r = run(&pdf("", &[stream(&format!("/Filter [{names}]"), b"x")]));
    assert!(hits(&r, "long filter chain").is_empty());
    let r = run(&pdf("", &[stream("/Filter /FlateDecode", b"x")]));
    assert!(hits(&r, "long filter chain").is_empty());
}

#[test]
fn filter_chain_truncated_at_load_is_still_flagged() {
    // 로딩 훅이 16개로 자르고 표지를 단다. 자른 체인은 "16개 이상"으로 본다.
    let names = "/ASCIIHexDecode ".repeat(300);
    let r = run(&pdf("", &[stream(&format!("/Filter [{names}]"), b"x")]));
    let h = hits(&r, "long filter chain");
    assert_eq!(h.len(), 1);
    assert!(h[0].detail.contains("16 or more"), "{}", h[0].detail);
}

// ---------------------------------------------------------------- 증분 저장

#[test]
fn flags_many_incremental_updates() {
    let mut bytes = pdf("", &[]);
    for i in 0..9 {
        bytes.extend(format!("startxref\n{i}\n%%EOF\n").bytes());
    }
    // 원본의 startxref 1개 + 9개 = 10개
    let r = run(&bytes);
    let h = hits(&r, "many incremental updates");
    assert_eq!(h.len(), 1);
    assert_eq!(h[0].severity, Severity::Low);
}

#[test]
fn nine_startxref_markers_are_not_flagged() {
    let mut bytes = pdf("", &[]);
    for i in 0..8 {
        bytes.extend(format!("startxref\n{i}\n%%EOF\n").bytes());
    }
    assert!(hits(&run(&bytes), "many incremental updates").is_empty());
}

// ---------------------------------------------------------------- 내장 실행 파일

fn elf() -> Vec<u8> {
    let mut v = b"\x7fELF\x02\x01\x01".to_vec();
    v.resize(64, 0);
    v
}

fn pe_at(offset: usize) -> Vec<u8> {
    let mut v = vec![0u8; offset + 64];
    v[0] = b'M';
    v[1] = b'Z';
    v[0x3C..0x40].copy_from_slice(&u32::try_from(offset).unwrap().to_le_bytes());
    v[offset..offset + 4].copy_from_slice(b"PE\0\0");
    v
}

fn embedded(content: &[u8]) -> FileReport {
    run(&pdf("", &[stream("/Type /EmbeddedFile", content)]))
}

#[test]
fn flags_embedded_elf() {
    let r = embedded(&elf());
    let h = hits(&r, "embedded executable content");
    assert_eq!(h.len(), 1);
    assert_eq!(h[0].severity, Severity::High);
}

#[test]
fn flags_embedded_macho_and_pe() {
    let macho = [0xCF, 0xFA, 0xED, 0xFE, 0, 0, 0, 0];
    assert_eq!(hits(&embedded(&macho), "embedded executable content").len(), 1);
    assert_eq!(hits(&embedded(&pe_at(0x80)), "embedded executable content").len(), 1);
}

#[test]
fn flags_pe_whose_header_is_past_1kib() {
    // 11-C: 계획의 1024바이트 창으로는 놓치는 PE. 헤더는 오프셋 4000에 있다.
    let r = embedded(&pe_at(4000));
    assert_eq!(hits(&r, "embedded executable content").len(), 1);
}

#[test]
fn embedded_text_starting_with_mz_is_not_flagged() {
    assert!(hits(&embedded(b"MZ Software notes, plain text"), "embedded executable content").is_empty());
    assert!(hits(&embedded(b"hello"), "embedded executable content").is_empty());
}

#[test]
fn stream_referenced_through_ef_is_checked_and_non_embedded_stream_is_not() {
    let objs = [s("<< /Type /Filespec /F (a.bin) /EF << /F 5 0 R >> >>"), stream("", &elf())];
    let r = run(&pdf("", &objs));
    let h = hits(&r, "embedded executable content");
    assert_eq!(h.len(), 1);
    assert!(h[0].location.starts_with("obj 5 0:"), "{}", h[0].location);
    // 내장 파일이 아닌 스트림(이미지 등)은 ELF 바이트여도 이 코드를 내지 않는다.
    let r = run(&pdf("", &[stream("", &elf())]));
    assert!(hits(&r, "embedded executable content").is_empty());
}

// ---------------------------------------------------------------- 11-A 간접 /S

#[test]
fn indirect_action_type_is_judged_like_inline() {
    let r = action("<< /S 5 0 R /JS (app.alert(1)) >>", &[s("/JavaScript")]);
    let h = hits(&r, "JavaScript action");
    assert_eq!(h.len(), 1);
    assert_eq!(h[0].severity, Severity::High);
    let inline = action("<< /S /JavaScript /JS (app.alert(1)) >>", &[]);
    assert_eq!(hits(&inline, "JavaScript action")[0].severity, h[0].severity);

    let r = action("<< /S 5 0 R /F (o.pdf) >>", &[s("/GoToR")]);
    assert_eq!(hits(&r, "GoToR action").len(), 1);
    let r = action("<< /S 5 0 R /F (a.txt) >>", &[s("/Launch")]);
    assert_eq!(hits(&r, "Launch action").len(), 1);
}

#[test]
fn indirect_goto_action_type_is_not_flagged() {
    let r = action("<< /S 5 0 R /D [0 /Fit] >>", &[s("/GoTo")]);
    let codes: Vec<_> = r.findings.iter().map(|f| f.code.as_str()).collect();
    assert!(r.findings.iter().all(|f| f.code == "OpenAction (runs on open)"), "{codes:?}");
}

#[test]
fn dangling_action_type_is_reported_low() {
    let r = action("<< /S 99 0 R >>", &[]);
    let h = hits(&r, "unresolvable action type");
    assert_eq!(h.len(), 1);
    assert_eq!(h[0].severity, Severity::Low);
    assert!(h[0].detail.contains("99 0 R"), "{}", h[0].detail);
    assert!(h[0].location.starts_with("obj 4 0:"));
}

#[test]
fn action_type_of_wrong_type_is_reported_low() {
    // 인라인 비-Name은 사전이 `/Type /Action`이라고 밝힐 때만 알린다(I1).
    let r = action("<< /Type /Action /S 7 >>", &[]);
    assert_eq!(hits(&r, "unresolvable action type").len(), 1);
    let r = action("<< /S 5 0 R >>", &[s("42")]);
    assert_eq!(hits(&r, "unresolvable action type").len(), 1);
}

#[test]
fn cyclic_action_type_reference_terminates() {
    let r = action("<< /S 5 0 R >>", &[s("6 0 R"), s("5 0 R")]);
    let h = hits(&r, "unresolvable action type");
    assert_eq!(h.len(), 1, "{:?}", r.findings.iter().map(|f| (&f.code, &f.location)).collect::<Vec<_>>());
    assert!(h[0].detail.contains("does not resolve"));
}

// ---------------------------------------------------------------- 11-B 간접 filespec

#[test]
fn indirect_filespec_dictionary_under_launch_is_critical() {
    let r = action("<< /S /Launch /F 5 0 R >>", &[s("<< /Type /Filespec /F (cmd.exe) >>")]);
    let h = hits(&r, "Launch action");
    assert_eq!(h.len(), 1);
    assert_eq!(h[0].severity, Severity::Critical);
    let inline = action("<< /S /Launch /F << /Type /Filespec /F (cmd.exe) >> >>", &[]);
    assert_eq!(hits(&inline, "Launch action")[0].severity, Severity::Critical);
}

#[test]
fn indirect_filespec_string_under_launch_is_critical() {
    let r = action("<< /S /Launch /F 5 0 R >>", &[s("(calc.exe)")]);
    assert_eq!(hits(&r, "Launch action")[0].severity, Severity::Critical);
}

#[test]
fn benign_indirect_filespec_under_launch_stays_high() {
    let r = action("<< /S /Launch /F 5 0 R >>", &[s("<< /Type /Filespec /F (notes.txt) >>")]);
    assert_eq!(hits(&r, "Launch action")[0].severity, Severity::High);
}

#[test]
fn indirect_win_dictionary_is_judged_like_inline() {
    let r = action("<< /S /Launch /F (a.txt) /Win 5 0 R >>", &[s("<< /F (cmd.exe) /P (/c calc) >>")]);
    let h = hits(&r, "platform launch parameters");
    assert_eq!(h.len(), 1);
    assert_eq!(h[0].severity, Severity::Critical);
    assert!(h[0].detail.contains("cmd.exe"), "{}", h[0].detail);
}

#[test]
fn indirect_file_name_string_is_flagged_as_executable_looking() {
    let r = run(&pdf("", &[s("<< /Type /Filespec /F 5 0 R /EF << >> >>"), s("(evil.exe)")]));
    assert_eq!(hits(&r, "executable-looking file name").len(), 1);
    let r = run(&pdf("", &[s("<< /Type /Filespec /F 5 0 R /EF << >> >>"), s("(notes.txt)")]));
    assert!(hits(&r, "executable-looking file name").is_empty());
}

// ---------------------------------------------------------------- 리뷰 보완 (I1, M2~M5)

#[test]
fn linearized_hint_stream_integer_s_is_not_flagged() {
    // I1: 선형화 PDF의 힌트 스트림 사전은 `/S <정수>`를 가진다. 동작이 아니므로 지표가 없어야 한다.
    let r = run(&pdf("", &[stream("/Filter /FlateDecode /S 36", b"x")]));
    assert!(hits(&r, "unresolvable action type").is_empty());
    // 동작이 아닌 사전의 인라인 비-Name도 마찬가지다.
    let r = action("<< /S 7 /D [0 /Fit] >>", &[]);
    assert!(hits(&r, "unresolvable action type").is_empty());
}

/// 객체 5부터 `refs`개의 참조가 이어지고 마지막이 `/Launch`인 사슬. `/S 5 0 R`가 따라가는 참조 횟수는 `refs + 1`이다.
fn chain(refs: usize) -> Vec<Vec<u8>> {
    let mut objs: Vec<Vec<u8>> = (0..refs).map(|i| s(&format!("{} 0 R", 6 + i))).collect();
    objs.push(s("/Launch"));
    objs
}

#[test]
fn reference_chain_of_eight_hops_resolves_and_nine_does_not() {
    // M4: 8번 따라가면 Name에 닿고(참조 7개 + Name), 9번이면 한도에 걸려 풀리지 않는다.
    let r = action("<< /S 5 0 R /F (a.txt) >>", &chain(7));
    assert_eq!(hits(&r, "Launch action").len(), 1);
    assert!(hits(&r, "unresolvable action type").is_empty());
    let r = action("<< /S 5 0 R /F (a.txt) >>", &chain(8));
    assert!(hits(&r, "Launch action").is_empty());
    assert_eq!(hits(&r, "unresolvable action type").len(), 1);
}

#[test]
fn raw_byte_findings_carry_offset_and_startxref_locations() {
    let mut bytes = pdf("/J#53 (x)", &[]);
    let offset = bytes.windows(5).position(|w| w == b"/J#53").unwrap();
    for i in 0..9 {
        bytes.extend(format!("startxref\n{i}\n%%EOF\n").bytes());
    }
    let r = run(&bytes);
    assert_eq!(hits(&r, "hex-obfuscated action name")[0].location, format!("obj 0 0:raw@{offset}"));
    assert_eq!(hits(&r, "many incremental updates")[0].location, "obj 0 0:startxref");
}

/// 저장(stored) 블록만 쓰는 zlib 스트림. 압축되지 않아 입력과 같은 크기다.
fn zlib_stored(data: &[u8]) -> Vec<u8> {
    let mut out = vec![0x78, 0x01];
    let mut chunks = data.chunks(65535).peekable();
    while let Some(c) = chunks.next() {
        out.push(u8::from(chunks.peek().is_none()));
        let len = u16::try_from(c.len()).unwrap();
        out.extend(len.to_le_bytes());
        out.extend((!len).to_le_bytes());
        out.extend(c);
    }
    let (mut a, mut b) = (1u32, 0u32);
    for &x in data {
        a = (a + u32::from(x)) % 65521;
        b = (b + a) % 65521;
    }
    out.extend(((b << 16) | a).to_be_bytes());
    out
}

#[test]
fn flate_embedded_pe_larger_than_64kib_is_still_found() {
    // M2: 로딩 훅이 스트림을 64KiB로 자르지만, Flate는 잘린 앞부분이 해제되어 PE를 판별할 수 있다.
    let mut payload = pe_at(0x80);
    let mut x = 12345u32;
    payload.extend((0..100_000).map(|_| {
        x = x.wrapping_mul(1_103_515_245).wrapping_add(12345);
        (x >> 16) as u8
    }));
    let z = zlib_stored(&payload);
    assert!(z.len() > 64 * 1024);
    let r = run(&pdf("", &[stream("/Type /EmbeddedFile /Filter /FlateDecode", &z)]));
    assert_eq!(hits(&r, "embedded executable content").len(), 1);
}

#[test]
fn long_filter_finding_does_not_claim_truncation_when_only_decode_parms_were_cut() {
    // M3: DecodeParms만 16개를 넘고 /Filter는 짧다. 필터 체인이 길다는 지표는 없어야 한다.
    let parms = "null ".repeat(30);
    let r = run(&pdf("", &[stream(&format!("/Filter /FlateDecode /DecodeParms [{parms}]"), b"x")]));
    assert!(hits(&r, "long filter chain").is_empty());
}
