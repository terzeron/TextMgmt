//! Just enough gzip (RFC 1952) framing to unwrap `joblib.dump(compress='gzip')`.

use crate::error::{Error, Result};
use crate::util::decompress_raw;

/// Strip the gzip header/footer and inflate the enclosed deflate stream.
pub fn decode(buf: &[u8]) -> Result<Vec<u8>> {
    if buf.len() < 18 || buf[0] != 0x1f || buf[1] != 0x8b {
        return Err(Error::format("not a gzip stream"));
    }
    if buf[2] != 8 {
        return Err(Error::unsupported("gzip compression method != deflate"));
    }
    let flg = buf[3];
    let payload_end = buf.len() - 8;
    let mut p = 10usize;
    if flg & 0x04 != 0 {
        if p + 2 > payload_end {
            return Err(Error::format("truncated gzip extra header length"));
        }
        let xlen = u16::from_le_bytes([buf[p], buf[p + 1]]) as usize;
        p += 2 + xlen;
        if p > payload_end {
            return Err(Error::format("truncated gzip extra header"));
        }
    }
    if flg & 0x08 != 0 {
        while p < payload_end && buf[p] != 0 {
            p += 1;
        }
        if p == payload_end {
            return Err(Error::format("unterminated gzip filename"));
        }
        p += 1;
    }
    if flg & 0x10 != 0 {
        while p < payload_end && buf[p] != 0 {
            p += 1;
        }
        if p == payload_end {
            return Err(Error::format("unterminated gzip comment"));
        }
        p += 1;
    }
    if flg & 0x02 != 0 {
        if p + 2 > payload_end {
            return Err(Error::format("truncated gzip header checksum"));
        }
        p += 2;
    }
    let isize_ = u32::from_le_bytes([
        buf[buf.len() - 4],
        buf[buf.len() - 3],
        buf[buf.len() - 2],
        buf[buf.len() - 1],
    ]) as usize;
    decompress_raw(&buf[p..payload_end], isize_.max(1 << 16))
}

/// `true` when the buffer looks like one of the containers joblib may produce.
pub fn kind(buf: &[u8]) -> Option<&'static str> {
    match buf {
        [0x1f, 0x8b, ..] => Some("gzip"),
        [0x78, _, ..] => Some("zlib"),
        [b'B', b'Z', b'h', ..] => Some("bz2"),
        [0xfd, b'7', b'z', b'X', b'Z', ..] => Some("xz"),
        [0x04, 0x22, 0x4d, 0x18, ..] => Some("lz4"),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn gzip_with_flags(flags: u8, optional: &[u8], payload: &[u8]) -> Vec<u8> {
        let compressed = crate::util::compress_raw(payload, 6).unwrap();
        let mut out = vec![0x1f, 0x8b, 8, flags, 0, 0, 0, 0, 0, 3];
        out.extend_from_slice(optional);
        out.extend_from_slice(&compressed);
        out.extend_from_slice(&0u32.to_le_bytes());
        out.extend_from_slice(&(payload.len() as u32).to_le_bytes());
        out
    }

    #[test]
    fn identifies_known_compression_signatures() {
        assert_eq!(kind(&[0x1f, 0x8b]), Some("gzip"));
        assert_eq!(kind(&[0x78, 0x9c]), Some("zlib"));
        assert_eq!(kind(b"BZh9"), Some("bz2"));
        assert_eq!(kind(&[0xfd, b'7', b'z', b'X', b'Z']), Some("xz"));
        assert_eq!(kind(&[0x04, 0x22, 0x4d, 0x18]), Some("lz4"));
        assert_eq!(kind(&[]), None);
        assert_eq!(kind(b"plain"), None);
    }

    #[test]
    fn decodes_plain_and_optional_gzip_headers() {
        let payload = b"joblib payload";
        assert_eq!(decode(&gzip_with_flags(0, &[], payload)).unwrap(), payload);
        let optional =
            [2, 0, 0xaa, 0xbb, b'n', b'a', b'm', b'e', 0, b'c', b'm', b't', 0, 0x12, 0x34];
        assert_eq!(decode(&gzip_with_flags(0x1e, &optional, payload)).unwrap(), payload);
    }

    #[test]
    fn rejects_bad_signatures_methods_and_truncated_optional_headers() {
        assert!(decode(b"not gzip").is_err());
        let mut wrong_method = gzip_with_flags(0, &[], b"x");
        wrong_method[2] = 0;
        assert!(decode(&wrong_method).is_err());

        let mut extra_too_long = vec![0x1f, 0x8b, 8, 0x04, 0, 0, 0, 0, 0, 0, 0xff, 0xff];
        extra_too_long.extend_from_slice(&[0; 8]);
        assert!(decode(&extra_too_long).is_err());

        let mut missing_extra_length = vec![0x1f, 0x8b, 8, 0x04, 0, 0, 0, 0, 0, 0];
        missing_extra_length.extend_from_slice(&[0; 8]);
        assert!(decode(&missing_extra_length).is_err());

        let mut name_without_terminator = vec![0x1f, 0x8b, 8, 0x08, 0, 0, 0, 0, 0, 0];
        name_without_terminator.extend_from_slice(&[b'a'; 8]);
        assert!(decode(&name_without_terminator).is_err());

        let mut comment_without_terminator = vec![0x1f, 0x8b, 8, 0x10, 0, 0, 0, 0, 0, 0];
        comment_without_terminator.extend_from_slice(&[b'a'; 8]);
        assert!(decode(&comment_without_terminator).is_err());

        let mut short_header_crc = vec![0x1f, 0x8b, 8, 0x02, 0, 0, 0, 0, 0, 0];
        short_header_crc.extend_from_slice(&[0; 8]);
        assert!(decode(&short_header_crc).is_err());
    }
}
