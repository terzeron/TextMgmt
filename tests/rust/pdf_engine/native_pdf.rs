//! MuPDF worker(`native_pdf`)와 어댑터(`pdf_engine::pdf_worker_client`) 테스트. worker는 프로세스를 띄우지 않고 `run_worker`로 직접 부른다.
use lopdf::{Document, Object, Stream, dictionary};
use pdf_engine::pdf_worker_client::run_worker;
use std::io;
use std::io::Write;
use std::path::{Path, PathBuf};

/// 16×8 빨간 JPEG.
const JPEG: &[u8] = include_bytes!("fixtures/red_16x8.jpg");

fn worker(args: &[&str]) -> Result<Vec<u8>, String> {
    let argv: Vec<String> = args.iter().map(|s| s.to_string()).collect();
    let mut out = Vec::new();
    run_worker(&argv, &mut out).map(|()| out)
}

fn p(path: &Path) -> &str {
    path.to_str().unwrap()
}

/// `n`쪽짜리 글자 PDF. MediaBox와 Resources는 Pages에 두어 쪽이 물려받게 한다. `extra`는 카탈로그에 더할 항목이다.
fn text_doc(n: usize) -> Document {
    let mut doc = Document::with_version("1.5");
    let pages_id = doc.new_object_id();
    let font = doc.add_object(dictionary! {"Type" => "Font", "Subtype" => "Type1", "BaseFont" => "Helvetica"});
    let kids: Vec<Object> = (1..=n)
        .map(|i| {
            let content = doc.add_object(Stream::new(dictionary! {}, format!("BT /F1 12 Tf 10 10 Td (Page {i}) Tj ET").into_bytes()));
            Object::Reference(doc.add_object(dictionary! {"Type" => "Page", "Parent" => pages_id, "Contents" => content}))
        })
        .collect();
    doc.objects.insert(
        pages_id,
        Object::Dictionary(dictionary! {
            "Type" => "Pages", "Kids" => kids, "Count" => n as i64,
            "MediaBox" => vec![0.into(), 0.into(), 200.into(), 100.into()],
            "Resources" => dictionary! {"Font" => dictionary! {"F1" => font}},
        }),
    );
    let catalog = doc.add_object(dictionary! {"Type" => "Catalog", "Pages" => pages_id});
    doc.trailer.set("Root", catalog);
    doc
}

fn save(doc: &mut Document, dir: &Path, name: &str) -> PathBuf {
    let path = dir.join(name);
    doc.save(&path).unwrap();
    path
}

// ---- 명령과 인자 ----

#[test]
fn usage_unknown_operation_and_missing_file_are_errors() {
    assert!(worker(&[]).unwrap_err().starts_with("usage:"));
    let d = tempfile::tempdir().unwrap();
    let f = save(&mut text_doc(1), d.path(), "a.pdf");
    assert_eq!(worker(&["bogus", p(&f)]).unwrap_err(), "unknown PDF operation");
    assert!(worker(&["info", p(&d.path().join("none.pdf"))]).is_err());
}

#[test]
fn info_reports_the_page_count() {
    let d = tempfile::tempdir().unwrap();
    let f = save(&mut text_doc(3), d.path(), "a.pdf");
    assert_eq!(String::from_utf8(worker(&["info", p(&f)]).unwrap()).unwrap(), "Pages: 3\n");
}

#[test]
fn text_reads_each_page_and_rejects_bad_ranges() {
    let d = tempfile::tempdir().unwrap();
    let f = save(&mut text_doc(2), d.path(), "a.pdf");
    let text = String::from_utf8(worker(&["text", p(&f), "1", "2"]).unwrap()).unwrap();
    assert!(text.contains("Page 1") && text.contains("Page 2") && text.matches('\x0c').count() == 2, "{text:?}");
    for (a, b) in [("0", "1"), ("2", "1"), ("1", "3")] {
        assert_eq!(worker(&["text", p(&f), a, b]).unwrap_err(), "invalid PDF page range", "{a}-{b}");
    }
    assert_eq!(worker(&["text", p(&f), "1"]).unwrap_err(), "missing PDF range");
    assert_eq!(worker(&["text", p(&f), "x", "1"]).unwrap_err(), "invalid PDF range");
}

#[test]
fn render_writes_a_bounded_pnm_and_rejects_bad_sizes() {
    let d = tempfile::tempdir().unwrap();
    let f = save(&mut text_doc(1), d.path(), "a.pdf");
    let pnm = worker(&["render", p(&f), "1", "100"]).unwrap();
    assert!(pnm.starts_with(b"P6\n100 50\n"), "{:?}", &pnm[..16.min(pnm.len())]);
    for (page, max) in [("0", "100"), ("1", "0"), ("1", "1201")] {
        assert_eq!(worker(&["render", p(&f), page, max]).unwrap_err(), "invalid PDF render dimensions");
    }
}

// ---- extract ----

#[test]
fn extract_keeps_inherited_page_attributes_and_rejects_bad_ranges() {
    let d = tempfile::tempdir().unwrap();
    let f = save(&mut text_doc(3), d.path(), "a.pdf");
    let out = Document::load_mem(&worker(&["extract", p(&f), "2", "3"]).unwrap()).unwrap();
    let pages = out.get_pages();
    assert_eq!(pages.len(), 2);
    let first = out.get_dictionary(*pages.values().next().unwrap()).unwrap();
    assert!(first.has(b"MediaBox") && first.has(b"Resources"));
    assert_eq!(worker(&["extract", p(&f), "2", "4"]).unwrap_err(), "invalid PDF page range");
}

#[test]
fn a_cyclic_page_parent_is_rejected() {
    let d = tempfile::tempdir().unwrap();
    let mut doc = text_doc(1);
    let page = *doc.get_pages().values().next().unwrap();
    doc.get_dictionary_mut(page).unwrap().set("Parent", page);
    let f = save(&mut doc, d.path(), "cycle.pdf");
    assert_eq!(worker(&["extract", p(&f), "1", "1"]).unwrap_err(), "cyclic PDF page tree");
}

/// 64MiB를 넘는 PDF는 lopdf로 읽지 않는다. 쓰지 않는 큰 스트림을 붙여 크기만 키운다.
#[test]
fn a_large_pdf_is_extracted_with_mupdf_and_is_not_a_jpeg_candidate() {
    let d = tempfile::tempdir().unwrap();
    let mut doc = text_doc(2);
    let filler = doc.add_object(Stream::new(dictionary! {}, vec![b' '; 65 * 1024 * 1024]).with_compression(false));
    let catalog = doc.catalog().unwrap().clone();
    let mut catalog = catalog;
    catalog.set("Filler", filler);
    let root = doc.trailer.get(b"Root").unwrap().as_reference().unwrap();
    doc.objects.insert(root, Object::Dictionary(catalog));
    let f = save(&mut doc, d.path(), "big.pdf");
    assert!(std::fs::metadata(&f).unwrap().len() > 64 * 1024 * 1024);
    let out = Document::load_mem(&worker(&["extract", p(&f), "2", "2"]).unwrap()).unwrap();
    assert_eq!(out.get_pages().len(), 1);
    assert_eq!(worker(&["extract", p(&f), "1", "3"]).unwrap_err(), "invalid PDF page range");
    assert!(worker(&["jpeg", p(&f)]).unwrap().is_empty());
}

// ---- extract: 무손실 이미지를 JPEG로 줄인다 ----

/// 결정적인 의사 난수(LCG). 테스트 이미지의 잡음에 쓴다.
fn noise(seed: &mut u32) -> u8 {
    *seed = seed.wrapping_mul(1_103_515_245).wrapping_add(12_345);
    (*seed >> 16) as u8
}

/// 대각선 그러데이션에 작은 잡음을 섞은 화소. Flate로는 덜 줄고 JPEG로는 크게 준다.
fn smooth_pixels(width: usize, height: usize, components: usize) -> Vec<u8> {
    let mut seed = 1;
    (0..width * height * components).map(|i| (((i / components % width + i / components / width) / 4) as u8).wrapping_add(noise(&mut seed) % 8)).collect()
}

/// 0과 255만 있는 흑백 잡음. Flate는 잘 줄이고 JPEG는 오히려 커진다.
fn binary_noise_pixels(width: usize, height: usize) -> Vec<u8> {
    let mut seed = 7;
    (0..width * height).map(|_| if noise(&mut seed) & 1 == 1 { 255 } else { 0 }).collect()
}

/// Flate로 압축한 이미지 스트림. `tweak`이 사전을 바꾼다.
fn flate_image(width: i64, height: i64, color: &str, pixels: Vec<u8>, tweak: impl FnOnce(&mut lopdf::Dictionary)) -> Stream {
    let mut image = Stream::new(dictionary! {"Type" => "XObject", "Subtype" => "Image", "Width" => width, "Height" => height, "ColorSpace" => color, "BitsPerComponent" => 8}, pixels);
    image.compress().unwrap();
    tweak(&mut image.dict);
    image
}

/// 쪽마다 이미지 하나를 그리는 PDF.
fn image_doc(images: Vec<Stream>) -> Document {
    let mut doc = Document::with_version("1.5");
    let pages_id = doc.new_object_id();
    let kids: Vec<Object> = images
        .into_iter()
        .map(|image| {
            let image = doc.add_object(image);
            let content = doc.add_object(Stream::new(dictionary! {}, b"q 10 0 0 10 0 0 cm /Im0 Do Q".to_vec()));
            Object::Reference(doc.add_object(dictionary! {
                "Type" => "Page", "Parent" => pages_id, "Contents" => content,
                "MediaBox" => vec![0.into(), 0.into(), 10.into(), 10.into()],
                "Resources" => dictionary! {"XObject" => dictionary! {"Im0" => image}},
            }))
        })
        .collect();
    let count = kids.len() as i64;
    doc.objects.insert(pages_id, Object::Dictionary(dictionary! {"Type" => "Pages", "Kids" => kids, "Count" => count}));
    let catalog = doc.add_object(dictionary! {"Type" => "Catalog", "Pages" => pages_id});
    doc.trailer.set("Root", catalog);
    doc
}

/// 추출 결과에서 쪽마다 그리는 이미지 스트림.
fn page_images(out: &Document) -> Vec<Stream> {
    out.get_pages()
        .values()
        .map(|page| {
            let resources = out.get_dictionary(*page).unwrap().get(b"Resources").unwrap().as_dict().unwrap();
            let image = resources.get(b"XObject").unwrap().as_dict().unwrap().get(b"Im0").unwrap().as_reference().unwrap();
            out.get_object(image).unwrap().as_stream().unwrap().clone()
        })
        .collect()
}

#[test]
fn extract_turns_large_lossless_rgb_and_gray_images_into_jpeg() {
    let d = tempfile::tempdir().unwrap();
    let rgb = flate_image(400, 400, "DeviceRGB", smooth_pixels(400, 400, 3), |_| {});
    let gray = flate_image(700, 700, "DeviceGray", smooth_pixels(700, 700, 1), |_| {});
    // 필터를 하나짜리 배열로 쓰고 빈 DecodeParms를 둔 이미지도 바꾼다. DecodeParms는 함께 지운다.
    let listed = flate_image(400, 400, "DeviceRGB", smooth_pixels(400, 400, 3), |dict| {
        dict.set("Filter", vec![Object::Name(b"FlateDecode".to_vec())]);
        dict.set("DecodeParms", dictionary! {});
    });
    let before: Vec<usize> = [&rgb, &gray, &listed].iter().map(|image| image.content.len()).collect();
    assert!(before.iter().all(|len| *len >= 256 * 1024), "{before:?}");
    let f = save(&mut image_doc(vec![rgb, gray, listed]), d.path(), "images.pdf");

    let out = Document::load_mem(&worker(&["extract", p(&f), "1", "3"]).unwrap()).unwrap();
    for (image, before) in page_images(&out).iter().zip(before) {
        assert_eq!(image.dict.get(b"Filter").unwrap().as_name().unwrap(), b"DCTDecode");
        assert!(!image.dict.has(b"DecodeParms"));
        assert!(image.content.starts_with(&[0xFF, 0xD8]), "JPEG SOI");
        assert_eq!(image.dict.get(b"Length").unwrap().as_i64().unwrap(), image.content.len() as i64);
        assert!(image.content.len() * 2 <= before, "{} vs {before}", image.content.len());
    }
    // 바꾼 JPEG는 원래 크기 그대로 풀린다.
    let decoded = mupdf::Image::from_bytes(&page_images(&out)[1].content).unwrap();
    assert_eq!((decoded.width(), decoded.height()), (700, 700));
}

#[test]
fn extract_keeps_images_it_cannot_or_need_not_shrink() {
    let d = tempfile::tempdir().unwrap();
    let smooth = || smooth_pixels(400, 400, 3);
    let rgb = |tweak: fn(&mut lopdf::Dictionary)| flate_image(400, 400, "DeviceRGB", smooth(), tweak);
    let images = vec![
        // JPEG가 Flate의 절반보다 크면 그대로 둔다
        flate_image(1500, 1500, "DeviceGray", binary_noise_pixels(1500, 1500), |_| {}),
        // 작은 이미지
        flate_image(40, 40, "DeviceRGB", smooth_pixels(40, 40, 3), |_| {}),
        // 투명도·마스크·디코드 배열이 있는 이미지
        rgb(|dict| dict.set("SMask", 1)),
        rgb(|dict| dict.set("Mask", 1)),
        rgb(|dict| dict.set("Decode", vec![0.into(), 1.into()])),
        // JPEG로 옮길 수 없는 색공간·비트 깊이
        rgb(|dict| dict.set("ColorSpace", "DeviceCMYK")),
        rgb(|dict| dict.set("ColorSpace", vec![Object::Name(b"ICCBased".to_vec()), 1.into()])),
        rgb(|dict| dict.set("BitsPerComponent", 16)),
        // 이미지가 아니거나 Flate 하나가 아닌 필터
        rgb(|dict| dict.set("Subtype", "Form")),
        rgb(|dict| dict.set("Filter", "DCTDecode")),
        rgb(|dict| dict.set("Filter", vec![Object::Name(b"FlateDecode".to_vec()), Object::Name(b"FlateDecode".to_vec())])),
        rgb(|dict| dict.set("Filter", 1)),
        rgb(|dict| {
            dict.remove(b"Filter");
        }),
        // 크기가 화소 수와 맞지 않거나 JPEG 한도(65535)·화소 상한을 넘는 이미지
        rgb(|dict| dict.set("Width", 300)),
        rgb(|dict| dict.set("Width", 0)),
        rgb(|dict| dict.set("Height", 70_000)),
        rgb(|dict| {
            dict.set("Width", 65_535);
            dict.set("Height", 700);
        }),
    ];
    let before: Vec<Vec<u8>> = images.iter().map(|image| image.content.clone()).collect();
    let f = save(&mut image_doc(images), d.path(), "keep.pdf");

    let count = before.len().to_string();
    let out = Document::load_mem(&worker(&["extract", p(&f), "1", &count]).unwrap()).unwrap();
    let after: Vec<Vec<u8>> = page_images(&out).into_iter().map(|image| image.content).collect();
    assert_eq!(after.len(), before.len());
    for (index, (after, before)) in after.iter().zip(&before).enumerate() {
        assert!(after == before, "{}번째 이미지가 바뀌었다", index + 1);
    }
}

// ---- 암호화 ----

fn encrypted(dir: &Path, name: &str, user_password: &str) -> PathBuf {
    let mut doc = text_doc(1);
    let id = Object::string_literal("0123456789abcdef");
    doc.trailer.set("ID", vec![id.clone(), id]);
    let state =
        lopdf::EncryptionState::try_from(lopdf::EncryptionVersion::V2 { document: &doc, owner_password: "owner", user_password, key_length: 128, permissions: lopdf::Permissions::all() }).unwrap();
    doc.encrypt(&state).unwrap();
    save(&mut doc, dir, name)
}

#[test]
fn an_owner_only_password_opens_and_a_user_password_does_not() {
    let d = tempfile::tempdir().unwrap();
    let open = encrypted(d.path(), "open.pdf", "");
    assert_eq!(String::from_utf8(worker(&["info", p(&open)]).unwrap()).unwrap(), "Pages: 1\n");
    let out = Document::load_mem(&worker(&["extract", p(&open), "1", "1"]).unwrap()).unwrap();
    assert!(!out.trailer.has(b"Encrypt"));
    let locked = encrypted(d.path(), "locked.pdf", "secret");
    assert_eq!(worker(&["info", p(&locked)]).unwrap_err(), "PDF password required");
    assert!(worker(&["extract", p(&locked), "1", "1"]).unwrap_err().starts_with("PDF password required"));
}

// ---- jpeg: 쪽 전체를 덮는 JPEG 하나만 그대로 꺼낸다 ----

/// 16×8 JPEG 하나를 쪽 전체에 그리는 PDF. `tweak`이 쪽·이미지·내용을 바꾼다.
fn jpeg_pdf(dir: &Path, name: &str, content: &str, tweak: impl FnOnce(&mut Document, lopdf::ObjectId, lopdf::ObjectId)) -> PathBuf {
    let mut doc = Document::with_version("1.5");
    let pages_id = doc.new_object_id();
    let image = doc.add_object(Stream::new(
        dictionary! {"Type" => "XObject", "Subtype" => "Image", "Width" => 16, "Height" => 8, "ColorSpace" => "DeviceRGB", "BitsPerComponent" => 8, "Filter" => "DCTDecode"},
        JPEG.to_vec(),
    ));
    let content = doc.add_object(Stream::new(dictionary! {}, content.as_bytes().to_vec()));
    let page = doc.add_object(dictionary! {
        "Type" => "Page", "Parent" => pages_id, "Contents" => content,
        "MediaBox" => vec![0.into(), 0.into(), 16.into(), 8.into()],
        "Resources" => dictionary! {"XObject" => dictionary! {"Im0" => image}},
    });
    doc.objects.insert(pages_id, Object::Dictionary(dictionary! {"Type" => "Pages", "Kids" => vec![Object::Reference(page)], "Count" => 1}));
    let catalog = doc.add_object(dictionary! {"Type" => "Catalog", "Pages" => pages_id});
    doc.trailer.set("Root", catalog);
    tweak(&mut doc, page, image);
    save(&mut doc, dir, name)
}

const FULL_PAGE: &str = "q 16 0 0 8 0 0 cm /Im0 Do Q";

#[test]
fn a_single_full_page_jpeg_is_returned_as_is() {
    let d = tempfile::tempdir().unwrap();
    let f = jpeg_pdf(d.path(), "ok.pdf", FULL_PAGE, |_, _, _| {});
    assert_eq!(worker(&["jpeg", p(&f)]).unwrap(), JPEG);
}

#[test]
fn anything_but_a_plain_full_page_jpeg_is_not_extracted() {
    type Tweak = Box<dyn FnOnce(&mut Document, lopdf::ObjectId, lopdf::ObjectId)>;
    let image_key = |key: &'static str, value: Object| -> Tweak { Box::new(move |doc: &mut Document, _, image| doc.get_object_mut(image).unwrap().as_stream_mut().unwrap().dict.set(key, value)) };
    let page_key = |key: &'static str, value: Object| -> Tweak { Box::new(move |doc: &mut Document, page, _| doc.get_dictionary_mut(page).unwrap().set(key, value)) };
    let page_remove = |key: &'static str| -> Tweak { Box::new(move |doc: &mut Document, page, _| drop(doc.get_dictionary_mut(page).unwrap().remove(key.as_bytes()))) };
    let none: Tweak = Box::new(|_, _, _| {});
    let cases: Vec<(&str, &str, Tweak)> = vec![
        ("other operator", "q 16 0 0 8 0 0 cm /Im0 Do Q BT ET", Box::new(|_, _, _| {})),
        ("two images", "q 16 0 0 8 0 0 cm /Im0 Do /Im0 Do Q", Box::new(|_, _, _| {})),
        ("no cm", "q /Im0 Do Q", Box::new(|_, _, _| {})),
        ("partial cm", "q 8 0 0 8 0 0 cm /Im0 Do Q", Box::new(|_, _, _| {})),
        ("no resources", FULL_PAGE, page_remove("Resources")),
        ("no media box", FULL_PAGE, page_remove("MediaBox")),
        ("short media box", FULL_PAGE, page_key("MediaBox", vec![0.into(), 0.into(), 16.into()].into())),
        ("rotated", FULL_PAGE, page_key("Rotate", 90.into())),
        ("crop box", FULL_PAGE, page_key("CropBox", vec![0.into(), 0.into(), 16.into(), 8.into()].into())),
        ("flate", FULL_PAGE, image_key("Filter", Object::Name(b"FlateDecode".to_vec()))),
        ("soft mask", FULL_PAGE, image_key("SMask", Object::Null)),
        ("mask", FULL_PAGE, image_key("Mask", Object::Null)),
        ("zero width", FULL_PAGE, image_key("Width", 0.into())),
        ("huge", FULL_PAGE, image_key("Height", 1_000_000_000.into())),
        ("size mismatch", FULL_PAGE, image_key("Width", 17.into())),
        ("not an image", FULL_PAGE, image_key("Subtype", Object::Name(b"Form".to_vec()))),
        ("baseline", FULL_PAGE, none),
    ];
    let d = tempfile::tempdir().unwrap();
    for (i, (label, content, tweak)) in cases.into_iter().enumerate() {
        let f = jpeg_pdf(d.path(), &format!("{i}.pdf"), content, tweak);
        let out = worker(&["jpeg", p(&f)]).unwrap_or_else(|e| panic!("{label}: {e}"));
        assert_eq!(out.is_empty(), label != "baseline", "{label}");
    }
}

#[test]
fn a_page_without_pages_or_with_a_nameless_draw_is_handled() {
    let d = tempfile::tempdir().unwrap();
    let f = jpeg_pdf(d.path(), "nameless.pdf", "q 16 0 0 8 0 0 cm 1 Do Q", |_, _, _| {});
    assert_eq!(worker(&["jpeg", p(&f)]).unwrap_err(), "missing PDF image name");
    let empty = jpeg_pdf(d.path(), "empty.pdf", FULL_PAGE, |doc, _, _| {
        let pages = doc.catalog().unwrap().get(b"Pages").unwrap().as_reference().unwrap();
        doc.objects.insert(pages, Object::Dictionary(dictionary! {"Type" => "Pages", "Kids" => Vec::<Object>::new(), "Count" => 0}));
    });
    assert!(worker(&["jpeg", p(&empty)]).unwrap().is_empty());
}

// ---- CJK 글꼴: 임베드하지 않은 CID 글꼴은 Noto Serif CJK로 대신한다 ----

#[test]
fn unembedded_cjk_fonts_fall_back_to_noto_for_every_ordering() {
    let d = tempfile::tempdir().unwrap();
    let mut doc = text_doc(1);
    let mut fonts = lopdf::Dictionary::new();
    let mut ops = String::from("BT ");
    for (i, (base, ordering)) in
        [("HYSMyeongJo-Medium", "Korea1"), ("HYGoThic-Bold", "Korea1"), ("STSong-Light", "GB1"), ("MSung-Light", "CNS1"), ("KozMinPro-Regular", "Japan1")].into_iter().enumerate()
    {
        let cid = doc.add_object(dictionary! {
            "Type" => "Font", "Subtype" => "CIDFontType0", "BaseFont" => base,
            "CIDSystemInfo" => dictionary! {"Registry" => Object::string_literal("Adobe"), "Ordering" => Object::string_literal(ordering), "Supplement" => 2},
            "FontDescriptor" => dictionary! {"Type" => "FontDescriptor", "FontName" => base, "Flags" => 4, "FontBBox" => vec![0.into(), 0.into(), 1000.into(), 1000.into()], "ItalicAngle" => 0, "Ascent" => 880, "Descent" => -120, "CapHeight" => 700, "StemV" => 80},
        });
        let font = doc.add_object(dictionary! {"Type" => "Font", "Subtype" => "Type0", "BaseFont" => base, "Encoding" => "Identity-H", "DescendantFonts" => vec![Object::Reference(cid)]});
        fonts.set(format!("K{i}"), font);
        ops.push_str(&format!("/K{i} 12 Tf 10 {} Td <0011> Tj ", 10 + i * 12));
    }
    ops.push_str("ET");
    let page = *doc.get_pages().values().next().unwrap();
    let content = doc.add_object(Stream::new(dictionary! {}, ops.into_bytes()));
    let page_dict = doc.get_dictionary_mut(page).unwrap();
    page_dict.set("Contents", content);
    page_dict.set("Resources", dictionary! {"Font" => fonts});
    let f = save(&mut doc, d.path(), "cjk.pdf");
    assert!(worker(&["render", p(&f), "1", "200"]).unwrap().starts_with(b"P6"));
}

// ---- 쓰기 실패 ----

struct Broken;

impl Write for Broken {
    fn write(&mut self, _: &[u8]) -> io::Result<usize> {
        Err(io::Error::other("disk full"))
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

#[test]
fn output_write_failures_are_errors() {
    let d = tempfile::tempdir().unwrap();
    let f = save(&mut text_doc(1), d.path(), "a.pdf");
    let jpeg = jpeg_pdf(d.path(), "j.pdf", FULL_PAGE, |_, _, _| {});
    for args in [vec!["info", p(&f)], vec!["text", p(&f), "1", "1"], vec!["render", p(&f), "1", "10"], vec!["extract", p(&f), "1", "1"], vec!["jpeg", p(&jpeg)]] {
        let argv: Vec<String> = args.iter().map(|s| s.to_string()).collect();
        assert!(run_worker(&argv, &mut Broken).unwrap_err().contains("disk full"), "{args:?}");
    }
}
