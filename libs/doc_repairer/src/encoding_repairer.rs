//! TXT 파일을 UTF-8로 재인코딩해 같은 inode에 덮어쓰는 교정 기법.
//!
//! 바이트 왕복, 타당성(인코딩 손상 구간 수 포함), 저장 후 재읽기를 모두 통과해야 파일을 바꾼다.

use std::path::Path;
use std::{fs, io};

use text_decoder::encoding_guesser::{DETECT_PREFIX, HIGH_BYTE_PAIRED_MIN, TEXT_ENCODINGS, TextDetector, detect_hint, high_byte_paired_ratio};
use text_decoder::text_transcoder::{Candidate, Enc, decode, encode};

use text_decoder::head_chars;
use text_decoder::mojibake_counter::{PDF_MOJIBAKE_RUN_LIMIT, RUN_SCAN_CHARS, count_mojibake_runs};

pub type BackupWriter<'a> = dyn Fn(&Path, &[u8]) -> io::Result<()> + 'a;
pub type TextReader<'a> = dyn Fn(&Path) -> io::Result<Vec<u8>> + 'a;

/// 재인코딩의 판별과 I/O 주입 경계.
#[derive(Clone, Default)]
pub struct ReencodeHooks<'a> {
    pub detect: Option<&'a TextDetector<'a>>,
    pub write_backup: Option<&'a BackupWriter<'a>>,
    pub read_bytes: Option<&'a TextReader<'a>>,
    pub encodings: Option<Vec<Candidate<'a>>>,
}

/// `reencode_text_file_to_utf8`이 쓰는 인코딩 순서. 근거 없는 UTF-16과, 고립 상위바이트일 때의 한글 코덱을 뺀다.
pub fn reencode_order(raw: &[u8]) -> Vec<Enc> {
    let mut order: Vec<Enc> = TEXT_ENCODINGS.to_vec();
    let has_bom = raw.starts_with(&[0xff, 0xfe]) || raw.starts_with(&[0xfe, 0xff]);
    let nul_ratio = raw.iter().filter(|&&b| b == 0).count() as f64 / raw.len().max(1) as f64;
    if !has_bom && nul_ratio < 0.02 {
        order.retain(|e| !matches!(e, Enc::Utf16 | Enc::Utf16Le | Enc::Utf16Be));
    }
    if let Some(ratio) = high_byte_paired_ratio(raw)
        && ratio < HIGH_BYTE_PAIRED_MIN
    {
        order.retain(|e| !matches!(e, Enc::Cp949 | Enc::EucKr));
        order.push(Enc::Cp1252);
    }
    order
}

/// 한 인코딩으로 읽은 결과가 원본을 무손실로 되살리고 타당한지 검사한다. 통과하면 텍스트를 돌려준다.
fn validated_decode(enc: Enc, raw: &[u8]) -> Option<String> {
    let text = decode(enc, raw, false)?;
    if text.contains('\u{fffd}') {
        return None;
    }
    // 1. 바이트 왕복: 무손실·가역을 확정한다.
    let round = encode(enc, &text)?;
    if round != raw {
        return None;
    }
    // 2. 타당성: 실제 글자가 있어야 하고 mojibake로 읽힌 결과여서는 안 된다.
    if !text.chars().any(|c| ('\u{ac00}'..='\u{d7a3}').contains(&c) || c.is_ascii_alphabetic() || c.is_ascii_digit() || ('\u{3400}'..='\u{9fff}').contains(&c)) {
        return None;
    }
    if count_mojibake_runs(&text) >= PDF_MOJIBAKE_RUN_LIMIT {
        return None;
    }
    // 텍스트에는 줄바꿈이 있다. 엔디언을 잘못 고른 UTF-16은 줄바꿈이 사라진다.
    if text.chars().count() >= 200 && !text.contains('\n') && !text.contains('\r') {
        return None;
    }
    let sample = head_chars(&text, RUN_SCAN_CHARS);
    let total = sample.chars().count();
    if total > 0 && sample.chars().filter(|c| ('\u{80}'..='\u{ff}').contains(c)).count() as f64 / total as f64 > 0.1 {
        return None;
    }
    Some(text)
}

fn validated_custom_decode(decode: fn(&[u8]) -> String, encode: fn(&str) -> Option<Vec<u8>>, raw: &[u8]) -> Option<String> {
    let text = decode(raw);
    if text.contains('\u{fffd}') || encode(&text)?.as_slice() != raw {
        return None;
    }
    if !text.chars().any(|c| ('\u{ac00}'..='\u{d7a3}').contains(&c) || c.is_ascii_alphanumeric() || ('\u{3400}'..='\u{9fff}').contains(&c)) {
        return None;
    }
    (count_mojibake_runs(&text) < PDF_MOJIBAKE_RUN_LIMIT).then_some(text)
}

/// 재인코딩 결과.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Reencode {
    pub changed: bool,
    pub reason: String,
}

fn no(reason: impl Into<String>) -> Reencode {
    Reencode { changed: false, reason: reason.into() }
}

/// txt 파일을 UTF-8로 재인코딩해 같은 inode에 덮어쓴다.
///
/// 검증을 모두 통과하지 못하면 파일에 손대지 않는다. 바이트 왕복, 타당성, 저장 후 재읽기를 확인한다.
/// ES 문서 `_id`가 inode이므로 임시 파일 교체를 쓰지 않는다. `backup_dir`이 있으면 덮어쓰기 전에
/// 원본을 그 아래 `relative`로 복사·검증하고, 실패하면 변환하지 않는다.
pub fn reencode_text_file_to_utf8(path: &Path, dry_run: bool, backup_dir: Option<&Path>, relative: &Path) -> Reencode {
    reencode_text_file_to_utf8_with(path, dry_run, backup_dir, relative, &ReencodeHooks::default())
}

pub fn reencode_text_file_to_utf8_with(path: &Path, dry_run: bool, backup_dir: Option<&Path>, relative: &Path, hooks: &ReencodeHooks<'_>) -> Reencode {
    let read_bytes = |path: &Path| hooks.read_bytes.map_or_else(|| fs::read(path), |f| f(path));
    let raw = match read_bytes(path) {
        Ok(raw) => raw,
        Err(e) => return no(format!("읽기 실패: {e}")),
    };
    if raw.is_empty() {
        return no("빈 파일");
    }
    if std::str::from_utf8(&raw).is_ok() {
        return no("이미 UTF-8");
    }
    // 판별 힌트가 가리키는 인코딩을 앞으로 보낸다(허용 목록 안에서만).
    let mut order = reencode_order(&raw);
    if let Some(hint) = hooks.detect.map_or_else(|| detect_hint(&raw[..raw.len().min(DETECT_PREFIX)]), |f| f(&raw[..raw.len().min(DETECT_PREFIX)]))
        && let Some(pos) = order.iter().position(|e| *e == hint)
    {
        let e = order.remove(pos);
        order.insert(0, e);
    }
    let standard = || order.iter().find_map(|e| validated_decode(*e, &raw).map(|t| (*e, t)));
    let selected = hooks
        .encodings
        .as_ref()
        .and_then(|candidates| {
            candidates.iter().find_map(|candidate| match candidate {
                Candidate::Builtin(enc) => validated_decode(*enc, &raw).map(|text| (*enc, text)),
                Candidate::Custom { decode, encode, name: _ } => validated_custom_decode(*decode, *encode, &raw).map(|text| (Enc::Utf8, text)),
            })
        })
        .or_else(|| if hooks.encodings.is_none() { standard() } else { None });
    let Some((enc, text)) = selected else {
        return no("무손실 디코딩 가능한 인코딩을 찾지 못함");
    };
    let utf8 = text.as_bytes();
    let summary = format!("{} → utf-8 ({}자, {}B → {}B)", enc.name(), text.chars().count(), raw.len(), utf8.len());
    if dry_run {
        return Reencode { changed: true, reason: format!("{summary} [dry-run]") };
    }
    if let Some(dir) = backup_dir {
        let backup = dir.join(relative);
        let result = (|| -> std::io::Result<bool> {
            if let Some(parent) = backup.parent() {
                fs::create_dir_all(parent)?;
            }
            if let Some(write_backup) = hooks.write_backup {
                write_backup(&backup, &raw)?;
            } else {
                fs::write(&backup, &raw)?;
            }
            Ok(read_bytes(&backup)? == raw)
        })();
        match result {
            Ok(true) => {}
            Ok(false) => return no(format!("백업 검증 실패: {}", backup.display())),
            Err(e) => return no(format!("백업 실패: {e}")),
        }
    }
    // 검증된 내용을 먼저 임시 파일에 보존한 뒤 같은 inode에 덮어쓴다.
    let tmp = path.with_file_name(format!("{}.utf8.tmp", path.file_name().map(|n| n.to_string_lossy()).unwrap_or_default()));
    let written = (|| -> std::io::Result<()> {
        use std::io::Write;
        fs::write(&tmp, utf8)?;
        let mut file = fs::OpenOptions::new().write(true).open(path)?;
        file.write_all(utf8)?;
        file.set_len(utf8.len() as u64)?;
        file.flush()?;
        file.sync_all()
    })();
    let _ = fs::remove_file(&tmp);
    if let Err(e) = written {
        return no(format!("쓰기 실패: {e}"));
    }
    match read_bytes(path).map_err(|e| e.to_string()).and_then(|b| String::from_utf8(b).map_err(|e| e.to_string())) {
        Err(e) => no(format!("저장 후 재읽기 실패: {e}")),
        Ok(saved) if saved != text => no(format!("저장 후 불일치: {}자 → {}자", text.chars().count(), saved.chars().count())),
        Ok(_) => Reencode { changed: true, reason: summary },
    }
}
