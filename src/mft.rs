//! Disk scanner.
//!
//! On Windows: reads the NTFS Master File Table ($MFT) directly, WizTree-style,
//! enumerating the whole volume in seconds without walking directories.
//! On other platforms: falls back to a plain recursive directory walk.

#[derive(Clone, Debug)]
pub struct FileEntry {
    pub path: String,
    pub size: u64, // allocated bytes
    pub is_dir: bool,
}

// ---------------------------------------------------------------------------
// Windows: direct $MFT reader
// ---------------------------------------------------------------------------

#[cfg(windows)]
mod imp {
    use super::FileEntry;
    use std::collections::{HashMap, HashSet};
    use std::ffi::OsStr;
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::Foundation::{CloseHandle, HANDLE, INVALID_HANDLE_VALUE};
    use windows_sys::Win32::Storage::FileSystem::{
        CreateFileW, GetDiskFreeSpaceExW, GetLogicalDriveStringsW, ReadFile, SetFilePointerEx,
        FILE_BEGIN, FILE_SHARE_READ, FILE_SHARE_WRITE, OPEN_EXISTING,
    };

    const GENERIC_READ: u32 = 0x8000_0000;

    pub fn list_drives() -> Vec<String> {
        let mut buf = [0u16; 256];
        let n = unsafe { GetLogicalDriveStringsW(buf.len() as u32, buf.as_mut_ptr()) } as usize;
        if n == 0 || n > buf.len() {
            return vec!["C:".to_string()];
        }
        let mut out = Vec::new();
        let mut start = 0;
        for i in 0..n {
            if buf[i] == 0 {
                if i > start {
                    let s = String::from_utf16_lossy(&buf[start..i]);
                    out.push(s.trim_end_matches('\\').to_string());
                }
                start = i + 1;
            }
        }
        if out.is_empty() {
            out.push("C:".to_string());
        }
        out
    }

    /// Returns (total_bytes, free_bytes) for the volume.
    pub fn disk_space(drive: &str) -> Result<(u64, u64), String> {
        let root = format!(r"{}:\", drive.trim_end_matches(':'));
        let wide: Vec<u16> = OsStr::new(&root).encode_wide().chain(Some(0)).collect();
        let mut free_to_caller = 0u64;
        let mut total = 0u64;
        let mut total_free = 0u64;
        let ok = unsafe {
            GetDiskFreeSpaceExW(
                wide.as_ptr(),
                &mut free_to_caller,
                &mut total,
                &mut total_free,
            )
        };
        if ok == 0 {
            return Err("GetDiskFreeSpaceExW failed".to_string());
        }
        Ok((total, total_free))
    }

    struct Volume {
        h: HANDLE,
    }

    impl Volume {
        fn open(drive: &str) -> Result<Self, String> {
            let name = format!(r"\\.\{}:", drive.trim_end_matches(':'));
            let wide: Vec<u16> = OsStr::new(&name).encode_wide().chain(Some(0)).collect();
            let h = unsafe {
                CreateFileW(
                    wide.as_ptr(),
                    GENERIC_READ,
                    FILE_SHARE_READ | FILE_SHARE_WRITE,
                    std::ptr::null(),
                    OPEN_EXISTING,
                    0,
                    std::ptr::null_mut(),
                )
            };
            if h == INVALID_HANDLE_VALUE {
                return Err(format!("cannot open {name} — run as Administrator"));
            }
            Ok(Volume { h })
        }

        fn read_at(&self, offset: u64, size: usize) -> Result<Vec<u8>, String> {
            let mut out = vec![0u8; size];
            let mut done = 0usize;
            while done < size {
                let chunk = (size - done).min(64 * 1024 * 1024) as u32;
                let ok = unsafe {
                    SetFilePointerEx(
                        self.h,
                        (offset + done as u64) as i64,
                        std::ptr::null_mut(),
                        FILE_BEGIN,
                    )
                };
                if ok == 0 {
                    return Err("seek failed".into());
                }
                let mut nread: u32 = 0;
                let ok = unsafe {
                    ReadFile(
                        self.h,
                        out[done..].as_mut_ptr() as *mut _,
                        chunk,
                        &mut nread,
                        std::ptr::null_mut(),
                    )
                };
                if ok == 0 || nread != chunk {
                    return Err("read failed".into());
                }
                done += chunk as usize;
            }
            Ok(out)
        }
    }

    impl Drop for Volume {
        fn drop(&mut self) {
            unsafe {
                CloseHandle(self.h);
            }
        }
    }

    fn u16_at(b: &[u8], off: usize) -> u16 {
        u16::from_le_bytes([b[off], b[off + 1]])
    }
    fn u32_at(b: &[u8], off: usize) -> u32 {
        u32::from_le_bytes([b[off], b[off + 1], b[off + 2], b[off + 3]])
    }
    fn u64_at(b: &[u8], off: usize) -> u64 {
        u64::from_le_bytes([
            b[off],
            b[off + 1],
            b[off + 2],
            b[off + 3],
            b[off + 4],
            b[off + 5],
            b[off + 6],
            b[off + 7],
        ])
    }

    fn parse_bootsector(bs: &[u8]) -> Result<(u32, u8, i64, usize), String> {
        if bs.len() < 512 || &bs[3..7] != b"NTFS" {
            return Err("not an NTFS volume".into());
        }
        let bps = u16_at(bs, 0x0B) as u32;
        let spc = bs[0x0D];
        let mft_lcn = i64::from_le_bytes(bs[0x30..0x38].try_into().unwrap());
        let cpmft = bs[0x40] as i8;
        let rec_size = if cpmft < 0 {
            1usize << (-(cpmft as i32) as u32)
        } else {
            cpmft as usize * spc as usize * bps as usize
        };
        Ok((bps, spc, mft_lcn, rec_size))
    }

    /// Apply the Update Sequence Number fixup. Returns None on torn records.
    fn apply_fixup(rec: &[u8], bps: u32) -> Option<Vec<u8>> {
        let usn_off = u16_at(rec, 4) as usize;
        let usn_cnt = u16_at(rec, 6) as usize;
        let nsectors = usn_cnt.checked_sub(1)?;
        if nsectors == 0 || rec.len() < nsectors * bps as usize {
            return None;
        }
        let usn = u16_at(rec, usn_off);
        let mut out = rec.to_vec();
        for i in 1..usn_cnt {
            let pos = i * bps as usize - 2;
            if u16_at(&out, pos) != usn {
                return None;
            }
            let repl = u16_at(rec, usn_off + 2 * i);
            out[pos] = (repl & 0xFF) as u8;
            out[pos + 1] = (repl >> 8) as u8;
        }
        Some(out)
    }

    fn for_each_attr(rec: &[u8], mut f: impl FnMut(u32, &[u8])) {
        let mut off = u16_at(rec, 0x14) as usize;
        while off + 8 <= rec.len() {
            let atype = u32_at(rec, off);
            let alen = u32_at(rec, off + 4) as usize;
            if atype == 0xFFFF_FFFF || alen < 8 || off + alen > rec.len() {
                break;
            }
            f(atype, &rec[off..off + alen]);
            off += alen;
        }
    }

    /// Parse an NTFS run list into (delta_lcn, length_in_clusters).
    fn parse_runs(mut data: &[u8]) -> Vec<(i64, u64)> {
        let mut runs = Vec::new();
        while !data.is_empty() && data[0] != 0 {
            let b = data[0];
            data = &data[1..];
            let (llen, olen) = ((b & 0x0F) as usize, (b >> 4) as usize);
            if data.len() < llen + olen {
                break;
            }
            let mut len_b = [0u8; 8];
            len_b[..llen].copy_from_slice(&data[..llen]);
            let length = u64::from_le_bytes(len_b);
            let mut off_b = [0u8; 8];
            off_b[..olen].copy_from_slice(&data[llen..llen + olen]);
            // sign-extend
            if olen > 0 && off_b[olen - 1] & 0x80 != 0 {
                for x in off_b.iter_mut().skip(olen) {
                    *x = 0xFF;
                }
            }
            let delta = i64::from_le_bytes(off_b);
            runs.push((delta, length));
            data = &data[llen + olen..];
        }
        runs
    }

    fn mft_runs(rec0: &[u8]) -> Result<Vec<(u64, u64)>, String> {
        let mut result = None;
        for_each_attr(rec0, |atype, attr| {
            if atype == 0x80 && attr[8] != 0 {
                // non-resident $DATA
                let run_off = u16_at(attr, 0x20) as usize;
                if run_off < attr.len() {
                    let mut runs = Vec::new();
                    let mut prev: i64 = 0;
                    for (delta, len) in parse_runs(&attr[run_off..]) {
                        prev += delta;
                        runs.push((prev as u64, len));
                    }
                    result = Some(runs);
                }
            }
        });
        result.ok_or_else(|| "no $DATA run list in MFT record 0".into())
    }

    struct RawEntry {
        name: String,
        parent: u64,
        size: u64,
        is_dir: bool,
    }

    /// Returns (name, parent_record, filename namespace).
    fn parse_filename(attr: &[u8]) -> Option<(String, u64, u8)> {
        // resident attribute: content offset is a u16 at header offset 0x14
        let coff = u16_at(attr, 0x14) as usize;
        let c = attr.get(coff..)?;
        let parent = u64_at(c, 0) & 0xFFFF_FFFF_FFFF;
        let nlen = *c.get(0x40)? as usize;
        // namespace: 1 = Win32, 2 = DOS 8.3 short name, 3 = Win32 & DOS
        let ns = *c.get(0x41)?;
        let nb = c.get(0x42..0x42 + nlen * 2)?;
        let name = String::from_utf16_lossy(
            &nb.chunks_exact(2)
                .map(|w| u16::from_le_bytes([w[0], w[1]]))
                .collect::<Vec<_>>(),
        );
        Some((name, parent, ns))
    }

    /// Lower rank = more preferred filename namespace.
    fn ns_rank(ns: u8) -> u8 {
        match ns {
            3 | 1 => 0, // Win32 & DOS / Win32: long names
            0 => 1,     // POSIX
            _ => 2,     // DOS 8.3 short names and anything else
        }
    }

    fn parse_record(rec: &[u8], bps: u32) -> Option<RawEntry> {
        if rec.len() < 48 || &rec[0..4] != b"FILE" {
            return None;
        }
        let rec = apply_fixup(rec, bps)?;
        let flags = u16_at(&rec, 0x16);
        if flags & 0x01 == 0 {
            return None; // not in use
        }
        let is_dir = flags & 0x02 != 0;
        let mut name: Option<(String, u64)> = None;
        let mut name_ns_rank = u8::MAX;
        let mut size: u64 = 0;
        for_each_attr(&rec, |atype, attr| {
            match atype {
                0x30 if attr.len() > 9 && attr[8] == 0 => {
                    // A record may carry several $FILE_NAME attributes
                    // (Win32 long name + DOS 8.3 short name); prefer the long one.
                    if let Some((n, p, ns)) = parse_filename(attr) {
                        let r = ns_rank(ns);
                        if r < name_ns_rank {
                            name = Some((n, p));
                            name_ns_rank = r;
                        }
                    }
                }
                0x80 if attr.len() > 9 && attr[9] == 0 => {
                    // unnamed $DATA only (skip alternate data streams)
                    if attr[8] != 0 {
                        size = size.max(u64_at(attr, 0x28)); // allocated size
                    } else if attr.len() > 0x14 {
                        size = size.max(u32_at(attr, 0x10) as u64); // resident
                    }
                }
                _ => {}
            }
        });
        let (name, parent) = name?;
        Some(RawEntry {
            name,
            parent,
            size,
            is_dir,
        })
    }

    pub fn scan(drive: &str, progress: &dyn Fn(u64) -> bool) -> Result<Vec<FileEntry>, String> {
        let vol = Volume::open(drive)?;
        let bs = vol.read_at(0, 512)?;
        let (bps, spc, mft_lcn, rec_size) = parse_bootsector(&bs)?;
        let cluster = bps as u64 * spc as u64;
        let raw0 = vol.read_at((mft_lcn as u64) * cluster, rec_size)?;
        let rec0 = apply_fixup(&raw0, bps).ok_or("MFT record 0 fixup failed")?;
        let runs = mft_runs(&rec0)?;

        let total_clusters: u64 = runs.iter().map(|(_, n)| n).sum();
        let mut mft =
            Vec::with_capacity((total_clusters * cluster).min(512 * 1024 * 1024) as usize);
        for (lcn, ncl) in &runs {
            mft.extend_from_slice(&vol.read_at(*lcn * cluster, (*ncl * cluster) as usize)?);
        }
        drop(vol);

        let nrec = mft.len() / rec_size;
        let mut raws: HashMap<u64, RawEntry> = HashMap::with_capacity(nrec / 2);
        for (i, chunk) in mft.chunks_exact(rec_size).enumerate() {
            if i % 4096 == 0 && !progress(i as u64) {
                return Err("cancelled".to_string());
            }
            if let Some(e) = parse_record(chunk, bps) {
                raws.insert(i as u64, e);
            }
        }
        progress(nrec as u64);

        // children index for directory aggregation
        let mut children: HashMap<u64, Vec<u64>> = HashMap::new();
        for (&num, e) in &raws {
            children.entry(e.parent).or_default().push(num);
        }

        // full paths via parent chain (5 = root)
        fn path_of(
            num: u64,
            raws: &HashMap<u64, RawEntry>,
            drive: &str,
            memo: &mut HashMap<u64, String>,
        ) -> String {
            if let Some(p) = memo.get(&num) {
                return p.clone();
            }
            let mut parts: Vec<String> = Vec::new();
            let mut cur = num;
            let mut seen = HashSet::new();
            while cur != 5 && seen.insert(cur) {
                match raws.get(&cur) {
                    Some(e) => {
                        parts.push(e.name.clone());
                        cur = e.parent;
                    }
                    None => break,
                }
                if parts.len() > 512 {
                    break;
                }
            }
            parts.reverse();
            let mut p = format!(r"{}:\", drive.trim_end_matches(':'));
            for part in parts {
                p.push_str(&part);
                p.push('\\');
            }
            let p = p.trim_end_matches('\\').to_string();
            memo.insert(num, p.clone());
            p
        }

        // aggregate directory sizes (post-order, iterative)
        let mut total: HashMap<u64, u64> = HashMap::new();
        let mut order: Vec<u64> = raws.keys().cloned().collect();
        // process leaves first: sort by depth descending
        fn depth(num: u64, raws: &HashMap<u64, RawEntry>, memo: &mut HashMap<u64, usize>) -> usize {
            if let Some(&d) = memo.get(&num) {
                return d;
            }
            let mut d = 0;
            let mut cur = num;
            let mut seen = HashSet::new();
            while cur != 5 && seen.insert(cur) {
                match raws.get(&cur) {
                    Some(e) => {
                        cur = e.parent;
                        d += 1;
                    }
                    None => break,
                }
                if d > 512 {
                    break;
                }
            }
            memo.insert(num, d);
            d
        }
        let mut dmemo = HashMap::new();
        order.sort_by_key(|n| std::cmp::Reverse(depth(*n, &raws, &mut dmemo)));
        for &num in &order {
            let mut s = raws[&num].size;
            if let Some(ch) = children.get(&num) {
                for c in ch {
                    s += total.get(c).copied().unwrap_or(0);
                }
            }
            total.insert(num, s);
        }

        let mut memo = HashMap::new();
        let mut out = Vec::with_capacity(raws.len());
        for (&num, e) in &raws {
            let size = if e.is_dir {
                total.get(&num).copied().unwrap_or(e.size)
            } else {
                e.size
            };
            out.push(FileEntry {
                path: path_of(num, &raws, drive, &mut memo),
                size,
                is_dir: e.is_dir,
            });
        }
        Ok(out)
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        fn le16(v: u16) -> [u8; 2] {
            v.to_le_bytes()
        }
        fn le32(v: u32) -> [u8; 4] {
            v.to_le_bytes()
        }

        /// Build a minimal resident $FILE_NAME attribute for `name` with the
        /// given parent MFT record number and filename namespace.
        fn fake_filename_attr(name: &str, parent: u64, ns: u8) -> Vec<u8> {
            let name_utf16: Vec<u16> = name.encode_utf16().collect();
            let content_off = 0x18usize;
            let content_len = 0x42 + name_utf16.len() * 2;
            let total_len = (content_off + content_len) as u32;
            let mut attr = vec![0u8; content_off + content_len];
            attr[0..4].copy_from_slice(&le32(0x30)); // type = $FILE_NAME
            attr[4..8].copy_from_slice(&le32(total_len));
            attr[8] = 0; // resident
            attr[0x10..0x14].copy_from_slice(&le32(content_len as u32));
            attr[0x14..0x16].copy_from_slice(&le16(content_off as u16));
            let c = content_off;
            attr[c..c + 6].copy_from_slice(&parent.to_le_bytes()[..6]);
            attr[c + 0x40] = name_utf16.len() as u8;
            attr[c + 0x41] = ns; // filename namespace
            for (i, w) in name_utf16.iter().enumerate() {
                attr[c + 0x42 + i * 2..c + 0x44 + i * 2].copy_from_slice(&le16(*w));
            }
            attr
        }

        /// Build a minimal MFT record containing the given attributes.
        fn fake_record(attrs: &[Vec<u8>]) -> Vec<u8> {
            let bps = 512usize;
            let mut rec = vec![0u8; bps];
            rec[0..4].copy_from_slice(b"FILE");
            rec[0x16..0x18].copy_from_slice(&le16(0x01)); // in use
            rec[4..6].copy_from_slice(&le16(0x2A)); // fixup offset
            rec[6..8].copy_from_slice(&le16(2)); // fixup count: 1 sector
            rec[0x2A..0x2C].copy_from_slice(&le16(0x1234)); // USN
            rec[0x2C..0x2E].copy_from_slice(&le16(0x5678)); // replacement
            rec[bps - 2..bps].copy_from_slice(&le16(0x1234)); // sector end matches USN
            rec[0x14..0x16].copy_from_slice(&le16(0x30)); // first attribute offset
            let mut off = 0x30usize;
            for a in attrs {
                rec[off..off + a.len()].copy_from_slice(a);
                off += a.len();
            }
            rec[off..off + 4].copy_from_slice(&le32(0xFFFF_FFFF)); // terminator
            rec
        }

        #[test]
        fn filename_parses_name_and_parent() {
            let attr = fake_filename_attr("hello.txt", 5, 1);
            let (name, parent, _ns) = parse_filename(&attr).expect("must parse");
            assert_eq!(name, "hello.txt");
            assert_eq!(parent, 5);
        }

        #[test]
        fn filename_parses_long_name() {
            let long = "this_is_a_much_longer_filename_12345.docx";
            let attr = fake_filename_attr(long, 123456, 1);
            let (name, parent, _ns) = parse_filename(&attr).expect("must parse");
            assert_eq!(name, long);
            assert_eq!(parent, 123456);
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
    }
}

// ---------------------------------------------------------------------------
// Non-Windows fallback: recursive directory walk
// ---------------------------------------------------------------------------

#[cfg(not(windows))]
mod imp {
    use super::FileEntry;
    use std::collections::HashMap;
    use std::path::PathBuf;

    pub fn list_drives() -> Vec<String> {
        vec!["/".to_string()]
    }

    /// Returns (total_bytes, free_bytes) for the mount point.
    pub fn disk_space(drive: &str) -> Result<(u64, u64), String> {
        use std::ffi::CString;
        let c = CString::new(drive).map_err(|e| e.to_string())?;
        let mut st: libc::statvfs = unsafe { std::mem::zeroed() };
        let r = unsafe { libc::statvfs(c.as_ptr(), &mut st) };
        if r != 0 {
            return Err(std::io::Error::last_os_error().to_string());
        }
        Ok((
            st.f_blocks as u64 * st.f_frsize as u64,
            st.f_bavail as u64 * st.f_frsize as u64,
        ))
    }

    pub fn scan(drive: &str, progress: &dyn Fn(u64) -> bool) -> Result<Vec<FileEntry>, String> {
        let mut files: Vec<(PathBuf, u64)> = Vec::new();
        let mut stack = vec![PathBuf::from(drive)];
        let mut count = 0u64;
        while let Some(dir) = stack.pop() {
            let rd = match std::fs::read_dir(&dir) {
                Ok(rd) => rd,
                Err(_) => continue,
            };
            for ent in rd.flatten() {
                let p = ent.path();
                let md = match ent.metadata() {
                    Ok(m) => m,
                    Err(_) => continue,
                };
                if md.is_dir() {
                    stack.push(p);
                } else {
                    count += 1;
                    if count.is_multiple_of(4096) && !progress(count) {
                        return Err("cancelled".to_string());
                    }
                    files.push((p, md.len()));
                }
            }
        }
        let mut dir_size: HashMap<PathBuf, u64> = HashMap::new();
        for (p, s) in &files {
            let mut anc = p.parent();
            while let Some(a) = anc {
                *dir_size.entry(a.to_path_buf()).or_insert(0) += *s;
                anc = a.parent();
            }
        }
        let mut out: Vec<FileEntry> = files
            .into_iter()
            .map(|(p, s)| FileEntry {
                path: p.to_string_lossy().into_owned(),
                size: s,
                is_dir: false,
            })
            .collect();
        for (p, s) in dir_size {
            out.push(FileEntry {
                path: p.to_string_lossy().into_owned(),
                size: s,
                is_dir: true,
            });
        }
        progress(count);
        Ok(out)
    }
}

pub use imp::{disk_space, list_drives, scan};
