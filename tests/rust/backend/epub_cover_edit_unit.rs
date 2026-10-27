use super::*;
use std::io::{Cursor, Write};

fn page_archive(page: &[u8], image_name: &str) -> ZipArchive<Cursor<Vec<u8>>> {
    let mut writer = zip::ZipWriter::new(Cursor::new(Vec::new()));
    let options = zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored);
    writer.start_file("OPS/page.xhtml", options).unwrap();
    writer.write_all(page).unwrap();
    if image_name == "img.jpg" {
        writer.start_file("OPS/img.jpg", options).unwrap();
        writer.write_all(b"image").unwrap();
    }
    ZipArchive::new(writer.finish().unwrap()).unwrap()
}

#[test]
fn page_decoding_and_encoding_preserve_supported_charsets() {
    let (text, encoding) = decode_page(b"\xef\xbb\xbfhello").unwrap();
    assert_eq!(text, "hello");
    assert!(encoding == Encoding::Utf8Sig);
    assert_eq!(encode_page(&text, encoding).unwrap(), b"\xef\xbb\xbfhello");

    let encoded = encode_page("hello", Encoding::Utf16).unwrap();
    let (decoded, encoding) = decode_page(&encoded).unwrap();
    assert_eq!(decoded, "hello");
    assert!(encoding == Encoding::Utf16);

    let latin = b"<?xml version='1.0' encoding='ISO-8859-1'?><p>\xe9</p>";
    let (text, encoding) = decode_page(latin).unwrap();
    assert!(text.ends_with("é</p>"));
    assert_eq!(encode_page(&text, encoding).unwrap(), latin);
    assert!(encode_page("한글", Encoding::Latin1).is_err());
    assert!(decode_page(b"<?xml encoding='windows-1252'?><p>x</p>").is_err());
    assert!(decode_page(b"<p>\xff</p>").is_err());
}

#[test]
fn image_tag_helpers_ignore_masked_markup_and_preserve_other_attributes() {
    let text = "<img src='a.jpg' data-src='lazy.jpg'><!-- <img src='ignored.jpg'> --><![CDATA[<image href='ignored.svg'>]]><image xlink:href=\"b.png\"/>";
    let tags = image_tags(text);
    assert_eq!(tags.len(), 2);
    let first = &text[tags[0].0..tags[0].1];
    assert_eq!(split_tag_copy(first, "new image.jpg"), "<img src='new%20image.jpg' data-src='lazy.jpg'>");
    assert_eq!(split_tag_copy("<image href=\"old.svg\"/>", "new.svg"), "<image href=\"new.svg\"/>");
    assert_eq!(is_word('_'), true);
    assert_eq!(is_word('한'), true);
    assert_eq!(is_word('-'), false);
}

#[test]
fn crop_rect_and_format_helpers_cover_valid_and_invalid_shapes() {
    assert_eq!(parse_rect(Some(&json!([0, 1, 2, 3]))).unwrap(), (0.0, 1.0, 2.0, 3.0, 0.0));
    assert_eq!(parse_rect(Some(&json!([0, 1, 2, 3, 45]))).unwrap(), (0.0, 1.0, 2.0, 3.0, 45.0));
    for value in [None, Some(&json!(null)), Some(&json!([0, 1, 2])), Some(&json!([0, 1, 2, 3, 4, 5])), Some(&json!([0, 1, 2, "bad"]))] {
        assert!(parse_rect(value).is_err());
    }

    assert_eq!(format_name(None), "알 수 없음");
    assert_eq!(format_name(Some(ImageFormat::Gif)), "GIF");
    assert_eq!(format_name(Some(ImageFormat::WebP)), "WEBP");
    assert_eq!(format_name(Some(ImageFormat::Png)), "PNG");
    assert_eq!(format_parts(ImageFormat::Png), (".png", "image/png"));
    assert_eq!(format_parts(ImageFormat::Jpeg), (".jpg", "image/jpeg"));
    assert!(image_size(b"not an image").is_err());
}

#[test]
fn flatten_and_svg_fit_handle_alpha_and_unmatched_documents() {
    let rgba = DynamicImage::ImageRgba8(image::RgbaImage::from_pixel(1, 1, image::Rgba([10, 20, 30, 0])));
    let flat = flatten(rgba).to_rgb8();
    assert_eq!(flat.get_pixel(0, 0).0, [255, 255, 255]);
    let gray = DynamicImage::ImageLuma8(image::GrayImage::from_pixel(1, 1, image::Luma([42])));
    assert_eq!(flatten(gray).to_luma8().get_pixel(0, 0).0, [42]);
    let gray16 = DynamicImage::ImageLuma16(image::ImageBuffer::from_pixel(1, 1, image::Luma([42_u16])));
    assert_eq!(flatten(gray16).to_rgb8().get_pixel(0, 0).0, [0, 0, 0]);

    assert_eq!(fit_svg_page(b"<invalid", (100, 200), (50, 100)), b"<invalid");
    let unchanged = b"<svg xmlns='http://www.w3.org/2000/svg'><rect/></svg>";
    assert_eq!(fit_svg_page(unchanged, (100, 200), (50, 100)), unchanged);
    let source = br#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 100 200"><image width="100" height="200"/></svg>"#;
    let fitted = String::from_utf8(fit_svg_page(source, (100, 200), (50, 100))).unwrap();
    assert!(fitted.contains("viewBox=\"0 0 50 100\""));
    assert!(fitted.contains("width=\"50\""));
    assert!(fitted.contains("height=\"100\""));
}

#[test]
fn reference_variants_and_edit_request_validation_cover_fallbacks() {
    assert_eq!(reference_variants("OPS/images/cover.jpg"), [b"cover.jpg".to_vec()]);
    assert_eq!(reference_variants("OPS/images/cover art.jpg"), [b"cover art.jpg".to_vec(), b"cover%20art.jpg".to_vec()]);

    let result = edit_images_with(Path::new("not-opened.epub"), "unused", Path::new("unused-backups"), EditImages { crops: vec![json!("invalid")], ..Default::default() }, &EpubCoverHooks::default());
    assert!(result.unwrap_err().to_string().contains("편집 항목이 올바르지 않습니다"));
}

#[test]
fn page_info_checks_page_limits_image_references_and_processing_instructions() {
    let page = b"<html><body><?reader keep?><img src='img.jpg'/></body></html>";
    let items = BTreeMap::from([("p".to_string(), "page.xhtml".to_string())]);
    let mut archive = page_archive(page, "img.jpg");
    let info = page_info(&mut archive, "OPS", &items, "p", 0, &EpubCoverHooks::default()).unwrap();
    assert!(info.image_only);
    assert_eq!(info.image_member, "OPS/img.jpg");
    assert_eq!(info.image_refs, ["OPS/img.jpg"]);

    let tiny_hooks = EpubCoverHooks::default().with_limits(crate::epub_cover::Limits { max_xml_bytes: 1, ..Default::default() });
    let mut archive = page_archive(page, "img.jpg");
    assert!(page_info(&mut archive, "OPS", &items, "p", 0, &tiny_hooks).is_err());

    let missing_image = b"<html><body><img src='missing.jpg'/></body></html>";
    let mut archive = page_archive(missing_image, "");
    assert!(page_info(&mut archive, "OPS", &items, "p", 0, &EpubCoverHooks::default()).is_err());
}

#[test]
fn crop_image_bytes_enforces_limits_formats_and_round_trips_png() {
    let rect = (0.0, 0.0, 1.0, 1.0, 0.0);
    assert!(crop_image_bytes(b"invalid", rect, &EpubCoverHooks::default()).is_err());

    let mut gif = Cursor::new(Vec::new());
    DynamicImage::ImageRgba8(image::RgbaImage::new(2, 2)).write_to(&mut gif, ImageFormat::Gif).unwrap();
    assert!(crop_image_bytes(&gif.into_inner(), rect, &EpubCoverHooks::default()).unwrap_err().to_string().contains("GIF"));

    let mut png = Cursor::new(Vec::new());
    DynamicImage::ImageRgba8(image::RgbaImage::from_pixel(200, 100, image::Rgba([20, 30, 40, 128]))).write_to(&mut png, ImageFormat::Png).unwrap();
    let png = png.into_inner();
    let byte_limited = EpubCoverHooks::default().with_limits(crate::epub_cover::Limits { max_image_bytes: 1, ..Default::default() });
    assert!(crop_image_bytes(&png, rect, &byte_limited).unwrap_err().to_string().contains("너무 큼"));
    let pixel_limited = EpubCoverHooks::default().with_limits(crate::epub_cover::Limits { max_image_pixels: 1, ..Default::default() });
    assert!(crop_image_bytes(&png, rect, &pixel_limited).unwrap_err().to_string().contains("해상도"));

    let (cropped, format) = crop_image_bytes(&png, rect, &EpubCoverHooks::default()).unwrap();
    assert_eq!(format, ImageFormat::Png);
    assert_eq!(image_size(&cropped).unwrap(), (200, 100));
}
