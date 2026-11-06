//! `doc_checker::exec_extension_table` 단위 테스트.

use doc_checker::exec_extension_table::{ARCHIVE_MEMBER, EPUB_MEMBER, ExecExtensions, NATIVE_BINARY, NATIVE_CORE, ZIP_MEMBER, is_exec_extension};
use std::collections::BTreeSet;

fn set(table: &ExecExtensions) -> BTreeSet<&'static str> {
    table.all().into_iter().collect()
}

fn of(items: &[&'static str]) -> BTreeSet<&'static str> {
    items.iter().copied().collect()
}

#[test]
fn child_tables_include_every_ancestor_extension() {
    for (child, parent) in [(&NATIVE_BINARY, &NATIVE_CORE), (&ARCHIVE_MEMBER, &NATIVE_CORE), (&EPUB_MEMBER, &ARCHIVE_MEMBER), (&ZIP_MEMBER, &ARCHIVE_MEMBER)] {
        assert!(set(parent).is_subset(&set(child)));
        assert!(parent.all().iter().all(|e| child.contains(e)));
    }
}

/// 계층으로 바꾸기 전 세 목록과 같은 집합이다.
#[test]
fn tables_match_the_former_flat_lists() {
    assert_eq!(set(&NATIVE_BINARY), of(&["exe", "dll", "sys", "scr", "ocx", "cpl", "drv", "efi", "so", "elf", "ko", "dylib", "bundle", "bin", "out"]));
    assert_eq!(
        set(&EPUB_MEMBER),
        of(&[
            "exe", "dll", "scr", "com", "bat", "cmd", "ps1", "psm1", "vbs", "vbe", "jse", "wsf", "wsh", "hta", "msi", "msp", "jar", "lnk", "reg", "cpl", "sh", "bash", "zsh", "py", "pl", "rb", "php",
            "apk", "app", "dmg", "so", "dylib", "bin", "run", "swf", "class",
        ])
    );
    assert_eq!(
        set(&ZIP_MEMBER),
        of(&[
            "exe", "dll", "scr", "com", "bat", "cmd", "ps1", "psm1", "vbs", "vbe", "jse", "wsf", "wsh", "hta", "msi", "msp", "jar", "lnk", "reg", "cpl", "sh", "bash", "zsh", "py", "pl", "rb", "php",
            "apk", "app", "dmg", "so", "dylib", "run", "swf", "class", "js", "pif", "msc", "scf", "url", "iso", "img", "vhd", "vhdx", "chm", "xll", "wsc", "one", "appx", "msix",
        ])
    );
}

#[test]
fn contains_is_exact_and_contains_ignore_case_is_not() {
    assert!(ZIP_MEMBER.contains("js") && !EPUB_MEMBER.contains("js"));
    assert!(EPUB_MEMBER.contains("bin") && !ZIP_MEMBER.contains("bin"));
    assert!(!EPUB_MEMBER.contains("EXE") && EPUB_MEMBER.contains_ignore_case("EXE"));
}

#[test]
fn extension_match_is_case_insensitive() {
    assert!(is_exec_extension("ExE") && is_exec_extension("SO"));
}

#[test]
fn only_executable_extensions_are_executable_extensions() {
    for ext in ["exe", "dll", "sys", "scr", "ocx", "cpl", "drv", "efi", "so", "elf", "ko", "dylib", "bundle", "bin", "out"] {
        assert!(is_exec_extension(ext) && NATIVE_BINARY.contains(ext), "{ext}");
    }
    for ext in ["txt", "epub", "pdf", "zip", "doc", "hwp", ""] {
        assert!(!is_exec_extension(ext), "{ext}");
    }
}
