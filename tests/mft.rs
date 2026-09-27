//! Integration tests for the Windows NTFS `$MFT` record parsing.
//!
//! Windows-only: the parsing helpers live behind `#[cfg(windows)]`.

#![cfg(windows)]

use sizetree::mft::{apply_fixup, ns_rank, parse_bootsector, parse_record, parse_runs, u16_at};

fn le16(v: u16) -> [u8; 2] {
    v.to_le_bytes()
}
fn le32(v: u32) -> [u8; 4] {
    v.to_le_bytes()
}
fn le64(v: u64) -> [u8; 8] {
    v.to_le_bytes()
}

/// Build a minimal $FILE_NAME attribute (type 0x30).
fn fake_filename_attr(name: &str, parent: u64, ns: u8) -> Vec<u8> {
    let name_u16: Vec<u16> = name.encode_utf16().collect();
    // content: parent(8) alloc(8) real(8) times(32) + flags(4) ea/reparse(8)
    //   + name_len(1) + ns(1) + name(2*len); name starts at content+0x42
    let content_len = 0x42 + name_u16.len() * 2;
    let mut attr = Vec::new();
    attr.extend_from_slice(&le32(0x30)); // type
    attr.extend_from_slice(&le32(8 + 8 + content_len as u32)); // total len
    attr.push(0); // non-resident flag
    attr.push(0); // name len
    attr.extend_from_slice(&le16(0)); // name offset
    attr.extend_from_slice(&le16(0)); // flags
    attr.extend_from_slice(&le16(0)); // attr id
    attr.extend_from_slice(&le32(content_len as u32)); // content size
    attr.extend_from_slice(&le16(24)); // content offset
    attr.push(1); // indexed flag
    attr.push(0); // padding
    let mut content = vec![0u8; content_len];
    content[0..8].copy_from_slice(&le64(parent));
    content[0x40] = name_u16.len() as u8;
    content[0x41] = ns;
    for (i, u) in name_u16.iter().enumerate() {
        content[0x42 + i * 2..0x42 + i * 2 + 2].copy_from_slice(&le16(*u));
    }
    attr.extend_from_slice(&content);
    attr
}

/// Build a minimal 1024-byte FILE record (sector size 512) wrapping `attrs`.
fn fake_record(attrs: &[Vec<u8>]) -> Vec<u8> {
    let mut rec = vec![0u8; 1024];
    rec[0..4].copy_from_slice(b"FILE");
    // update sequence: USN at 0x30, one replacement per sector
    rec[0x1E..0x20].copy_from_slice(&le16(0x30)); // usa offset
    rec[0x20..0x22].copy_from_slice(&le16(3)); // usa count (usn + 2 sectors)
    rec[0x30..0x32].copy_from_slice(&le16(0x1234)); // USN
    rec[0x32..0x34].copy_from_slice(&le16(0x5678)); // replacement #1
    rec[0x34..0x36].copy_from_slice(&le16(0x9ABC)); // replacement #2
    rec[510..512].copy_from_slice(&le16(0x1234)); // sector-end USN #1
    rec[1022..1024].copy_from_slice(&le16(0x1234)); // sector-end USN #2
    rec[0x16..0x18].copy_from_slice(&le16(0x01)); // in-use flag
    let mut off = 0x38usize;
    for a in attrs {
        rec[off..off + a.len()].copy_from_slice(a);
        off += a.len();
    }
    // end marker
    rec[off..off + 4].copy_from_slice(&le32(0xFFFFFFFF));
    // attrs start offset
    rec[0x14..0x16].copy_from_slice(&le16(0x38));
    rec
}

/// Build a minimal NTFS boot sector: 512 B/sector, 8 sectors/cluster,
/// MFT at LCN 786432, 1024-byte MFT records.
fn fake_bootsector() -> Vec<u8> {
    let mut bs = vec![0u8; 512];
    bs[3..7].copy_from_slice(b"NTFS");
    bs[0x0B..0x0D].copy_from_slice(&512u16.to_le_bytes());
    bs[0x0D] = 8;
    bs[0x30..0x38].copy_from_slice(&786432i64.to_le_bytes());
    bs[0x40] = 0xF6; // clusters per MFT record = -10 -> 1024 bytes
    bs
}

#[test]
fn filename_parses_name_and_parent() {
    let attr = fake_filename_attr("hello.txt", 5, 1);
    let rec = fake_record(&[attr]);
    let e = parse_record(&rec, 512).expect("must parse");
    assert_eq!(e.name, "hello.txt");
    assert_eq!(e.parent, 5);
}

#[test]
fn filename_parses_long_name() {
    let long: String = "这是一个很长的文件名_".repeat(10) + ".txt";
    let attr = fake_filename_attr(&long, 5, 1);
    let rec = fake_record(&[attr]);
    let e = parse_record(&rec, 512).expect("must parse");
    assert_eq!(e.name, long);
}

#[test]
fn filename_prefers_win32_long_name_over_dos_short() {
    // DOS 8.3 short name first, Win32 long name second: must pick the long one.
    let dos = fake_filename_attr("ORchar~1", 5, 2);
    let win32 = fake_filename_attr("OrchardCore", 5, 1);
    let rec = fake_record(&[dos, win32]);
    let e = parse_record(&rec, 512).expect("must parse");
    assert_eq!(e.name, "OrchardCore");
    assert_eq!(e.parent, 5);
}

#[test]
fn filename_falls_back_to_dos_short_name() {
    // A DOS-only record keeps its short name rather than failing.
    let dos = fake_filename_attr("ORchar~1", 5, 2);
    let rec = fake_record(&[dos]);
    let e = parse_record(&rec, 512).expect("must parse");
    assert_eq!(e.name, "ORchar~1");
}

#[test]
fn ns_rank_orders_win32_above_dos() {
    assert!(ns_rank(1) < ns_rank(2));
    assert!(ns_rank(3) < ns_rank(2));
    assert_eq!(ns_rank(1), ns_rank(3));
    assert!(ns_rank(0) < ns_rank(2));
}

#[test]
fn record_rejects_bad_magic() {
    let mut rec = fake_record(&[fake_filename_attr("a", 5, 1)]);
    rec[0..4].copy_from_slice(b"BAAD");
    assert!(parse_record(&rec, 512).is_none());
}

#[test]
fn record_rejects_not_in_use() {
    let mut rec = fake_record(&[fake_filename_attr("a", 5, 1)]);
    rec[0x16] = 0x00; // clear the in-use flag
    rec[0x17] = 0x00;
    assert!(parse_record(&rec, 512).is_none());
}

#[test]
fn fixup_applies_replacement() {
    let rec = fake_record(&[fake_filename_attr("a", 5, 1)]);
    let fixed = apply_fixup(&rec, 512).expect("must apply");
    assert_eq!(u16_at(&fixed, 510), 0x5678);
}

#[test]
fn fixup_rejects_torn_record() {
    let mut rec = fake_record(&[fake_filename_attr("a", 5, 1)]);
    rec[510] = 0x00; // corrupt the sector-end update sequence
    rec[511] = 0x00;
    assert!(apply_fixup(&rec, 512).is_none());
}

#[test]
fn bootsector_parses_fields() {
    let (bps, spc, lcn, rec) = parse_bootsector(&fake_bootsector()).expect("must parse");
    assert_eq!((bps, spc, lcn, rec), (512, 8, 786432, 1024));
}

#[test]
fn bootsector_rejects_non_ntfs() {
    assert!(parse_bootsector(&vec![0u8; 512]).is_err());
    assert!(parse_bootsector(&vec![0u8; 100]).is_err());
}

#[test]
fn runs_decode_single() {
    // 0x11: 1 length byte, 1 offset byte; length 8, delta 5
    assert_eq!(parse_runs(&[0x11, 0x08, 0x05]), vec![(5, 8)]);
}

#[test]
fn runs_decode_negative_delta() {
    // delta 0xFF sign-extends to -1
    assert_eq!(parse_runs(&[0x11, 0x04, 0xFF]), vec![(-1, 4)]);
}

#[test]
fn runs_decode_multiple_and_stop_at_terminator() {
    let runs = parse_runs(&[0x11, 0x08, 0x05, 0x11, 0x04, 0xFF, 0x00]);
    assert_eq!(runs, vec![(5, 8), (-1, 4)]);
}
