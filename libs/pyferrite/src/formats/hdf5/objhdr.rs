//! HDF5 object headers, versions 1 and 2 (including continuation blocks).

use super::superblock::Superblock;
use crate::error::{Error, Result};
use crate::util::Cursor;

/// One header message: its type tag and its raw payload.
#[derive(Clone, Debug)]
pub struct Message {
    pub kind: u16,
    pub data: Vec<u8>,
}

/// Parse the object header at `addr`, following continuation blocks.
pub fn parse(file: &[u8], sb: &Superblock, addr: u64) -> Result<Vec<Message>> {
    let mut out = Vec::new();
    let mut queue = vec![(sb.at(addr), None::<usize>)];
    let mut guard = 0;
    while let Some((pos, limit)) = queue.pop() {
        guard += 1;
        if guard > 256 {
            return Err(Error::format("object header continuation loop"));
        }
        if file.get(pos..pos + 4).map(|s| s == b"OHDR").unwrap_or(false) {
            parse_v2(file, sb, pos, &mut out, &mut queue)?;
        } else if file.get(pos..pos + 4).map(|s| s == b"OCHK").unwrap_or(false) {
            parse_v2_chunk(file, sb, pos + 4, limit, &mut out, &mut queue)?;
        } else if let Some(limit) = limit {
            parse_v1_messages(file, sb, pos, limit, &mut out, &mut queue)?;
        } else {
            parse_v1(file, sb, pos, &mut out, &mut queue)?;
        }
    }
    Ok(out)
}

fn parse_v1(
    file: &[u8],
    sb: &Superblock,
    pos: usize,
    out: &mut Vec<Message>,
    queue: &mut Vec<(usize, Option<usize>)>,
) -> Result<()> {
    let mut c = Cursor::at(file, pos);
    let version = c.u8()?;
    if version != 1 {
        return Err(Error::unsupported(format!("object header version {version}")));
    }
    c.skip(1)?;
    let _nmsgs = c.u16()?;
    let _refcount = c.u32()?;
    let size = c.u32()? as usize;
    // The prefix is twelve bytes plus four of padding, so messages always begin
    // sixteen bytes in. Aligning the *absolute* cursor would be wrong: the
    // library is free to place a header at an unaligned address, and does.
    let start = pos + 16;
    parse_v1_messages(file, sb, start, start + size, out, queue)
}

fn parse_v1_messages(
    file: &[u8],
    sb: &Superblock,
    start: usize,
    end: usize,
    out: &mut Vec<Message>,
    queue: &mut Vec<(usize, Option<usize>)>,
) -> Result<()> {
    let mut c = Cursor::at(file, start);
    while c.pos() + 8 <= end.min(file.len()) {
        let kind = c.u16()?;
        let size = c.u16()? as usize;
        let _flags = c.u8()?;
        c.skip(3)?;
        if c.pos() + size > file.len() {
            break;
        }
        let data = c.take(size)?.to_vec();
        if kind == super::msg::MSG_CONTINUATION {
            let mut cc = Cursor::new(&data);
            let a = cc.uint(sb.offset_size)?;
            let l = cc.uint(sb.length_size)? as usize;
            if sb.in_bounds(a) {
                let p = sb.at(a);
                queue.push((p, Some(p + l)));
            }
        } else if kind != 0 {
            out.push(Message { kind, data });
        }
    }
    Ok(())
}

fn parse_v2(
    file: &[u8],
    sb: &Superblock,
    pos: usize,
    out: &mut Vec<Message>,
    queue: &mut Vec<(usize, Option<usize>)>,
) -> Result<()> {
    let mut c = Cursor::at(file, pos + 4);
    let version = c.u8()?;
    if version != 2 {
        return Err(Error::unsupported(format!("OHDR version {version}")));
    }
    let flags = c.u8()?;
    if flags & 0x20 != 0 {
        c.skip(16)?; // access/mod/change/birth times
    }
    if flags & 0x10 != 0 {
        c.skip(4)?; // max compact / min dense
    }
    let size = match flags & 0x03 {
        0 => c.u8()? as usize,
        1 => c.u16()? as usize,
        2 => c.u32()? as usize,
        _ => c.u64()? as usize,
    };
    let start = c.pos();
    parse_v2_body(file, sb, start, start + size, flags, out, queue)
}

fn parse_v2_chunk(
    file: &[u8],
    sb: &Superblock,
    start: usize,
    limit: Option<usize>,
    out: &mut Vec<Message>,
    queue: &mut Vec<(usize, Option<usize>)>,
) -> Result<()> {
    let end = limit.unwrap_or(file.len()).min(file.len());
    parse_v2_body(file, sb, start, end.saturating_sub(4), 0, out, queue)
}

fn parse_v2_body(
    file: &[u8],
    sb: &Superblock,
    start: usize,
    end: usize,
    flags: u8,
    out: &mut Vec<Message>,
    queue: &mut Vec<(usize, Option<usize>)>,
) -> Result<()> {
    let mut c = Cursor::at(file, start);
    let track_order = flags & 0x04 != 0;
    while c.pos() + 4 <= end.min(file.len()) {
        let kind = c.u8()? as u16;
        let size = c.u16()? as usize;
        let _mflags = c.u8()?;
        if track_order {
            c.skip(2)?;
        }
        if c.pos() + size > file.len() {
            break;
        }
        let data = c.take(size)?.to_vec();
        if kind == super::msg::MSG_CONTINUATION {
            let mut cc = Cursor::new(&data);
            let a = cc.uint(sb.offset_size)?;
            let l = cc.uint(sb.length_size)? as usize;
            if sb.in_bounds(a) {
                let p = sb.at(a);
                queue.push((p, Some(p + l)));
            }
        } else if kind != 0 {
            out.push(Message { kind, data });
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn superblock() -> Superblock {
        Superblock {
            version: 3,
            offset_size: 8,
            length_size: 8,
            base_address: 0,
            root_address: 0,
            eof_address: 4096,
        }
    }

    fn v1_message(kind: u16, data: &[u8]) -> Vec<u8> {
        let mut out = Vec::new();
        out.extend(kind.to_le_bytes());
        out.extend((data.len() as u16).to_le_bytes());
        out.push(0);
        out.extend([0; 3]);
        out.extend(data);
        out
    }

    fn v1_header(messages: &[u8]) -> Vec<u8> {
        let mut out = vec![1, 0];
        out.extend(1u16.to_le_bytes());
        out.extend(1u32.to_le_bytes());
        out.extend((messages.len() as u32).to_le_bytes());
        out.extend([0; 4]);
        out.extend(messages);
        out
    }

    fn v2_message(kind: u8, data: &[u8], track_order: bool) -> Vec<u8> {
        let mut out = vec![kind];
        out.extend((data.len() as u16).to_le_bytes());
        out.push(0);
        if track_order {
            out.extend(9u16.to_le_bytes());
        }
        out.extend(data);
        out
    }

    fn v2_header(flags: u8, body: &[u8]) -> Vec<u8> {
        let mut out = b"OHDR".to_vec();
        out.extend([2, flags]);
        if flags & 0x20 != 0 {
            out.extend([0; 16]);
        }
        if flags & 0x10 != 0 {
            out.extend([0; 4]);
        }
        match flags & 3 {
            0 => out.push(body.len() as u8),
            1 => out.extend((body.len() as u16).to_le_bytes()),
            2 => out.extend((body.len() as u32).to_le_bytes()),
            _ => out.extend((body.len() as u64).to_le_bytes()),
        }
        out.extend(body);
        out
    }

    #[test]
    fn version_one_headers_parse_at_unaligned_offsets_and_follow_continuations() {
        let message = v1_message(3, b"abc");
        let mut file = vec![0; 3];
        file.extend(v1_header(&message));
        assert_eq!(parse(&file, &superblock(), 3).unwrap()[0].kind, 3);

        let mut continuation = Vec::new();
        continuation.extend(100u64.to_le_bytes());
        continuation.extend(9u64.to_le_bytes());
        let mut root = v1_header(&v1_message(0x10, &continuation));
        root.resize(100, 0);
        root.extend(v1_message(4, b"x"));
        let messages = parse(&root, &superblock(), 0).unwrap();
        assert_eq!(messages.len(), 1);
        assert_eq!((messages[0].kind, messages[0].data.as_slice()), (4, b"x".as_slice()));
    }

    #[test]
    fn version_two_headers_parse_all_size_widths_and_optional_fields() {
        for (size_flag, extra_flags) in [(0, 0), (1, 0), (2, 0x30), (3, 0x04)] {
            let flags = size_flag | extra_flags;
            let body = v2_message(7, b"v", flags & 0x04 != 0);
            let messages = parse(&v2_header(flags, &body), &superblock(), 0).unwrap();
            assert_eq!(messages.len(), 1, "flags={flags:#x}");
            assert_eq!(messages[0].kind, 7);
            assert_eq!(messages[0].data, b"v");
        }
    }

    #[test]
    fn version_two_continuations_and_unknown_messages_are_handled() {
        let mut continuation = Vec::new();
        continuation.extend(100u64.to_le_bytes());
        continuation.extend(13u64.to_le_bytes());
        let body = v2_message(0x10, &continuation, false);
        let mut file = v2_header(0, &body);
        file.resize(100, 0);
        file.extend(b"OCHK");
        file.extend(v2_message(8, b"z", false));
        file.extend([0; 4]); // chunk checksum
        let messages = parse(&file, &superblock(), 0).unwrap();
        assert_eq!(messages.len(), 1);
        assert_eq!((messages[0].kind, messages[0].data.as_slice()), (8, b"z".as_slice()));

        let zero_kind = v2_header(0, &v2_message(0, b"ignored", false));
        assert!(parse(&zero_kind, &superblock(), 0).unwrap().is_empty());
    }

    #[test]
    fn malformed_headers_and_continuation_cycles_return_or_skip_safely() {
        assert!(parse(b"BAD!", &superblock(), 0).is_err());
        assert!(parse(b"OHDR\x03", &superblock(), 0).is_err());

        let mut continuation = Vec::new();
        continuation.extend(u64::MAX.to_le_bytes());
        continuation.extend(8u64.to_le_bytes());
        let out_of_bounds = v1_header(&v1_message(0x10, &continuation));
        assert!(parse(&out_of_bounds, &superblock(), 0).unwrap().is_empty());

        let mut self_ref = Vec::new();
        self_ref.extend(0u64.to_le_bytes());
        self_ref.extend(24u64.to_le_bytes());
        let mut file = v1_header(&v1_message(0x10, &self_ref));
        // A V1 continuation to its own message data keeps the queue alive until the guard trips.
        file[16 + 8..16 + 16].copy_from_slice(&16u64.to_le_bytes());
        let err = parse(&file, &superblock(), 0).unwrap_err();
        assert!(err.to_string().contains("continuation loop"));
    }
}
