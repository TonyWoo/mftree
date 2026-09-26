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
        CreateFileW, GetLogicalDriveStringsW, ReadFile, SetFilePointerEx, FILE_BEGIN,
        FILE_SHARE_READ, FILE_SHARE_WRITE, OPEN_EXISTING,
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
                    0,
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
                let ok =
                    unsafe { SetFilePointerEx(self.h, (offset + done as u64) as i64, std::ptr::null_mut(), FILE_BEGIN) };
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
            b[off], b[off + 1], b[off + 2], b[off + 3], b[off + 4], b[off + 5], b[off + 6],
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

    fn parse_filename(attr: &[u8]) -> Option<(String, u64)> {
        let coff = u16_at(attr, 0x12) as usize;
        let c = attr.get(coff..)?;
        let parent = u64_at(c, 0) & 0xFFFF_FFFF_FFFF;
        let nlen = *c.get(0x40)? as usize;
        let nb = c.get(0x42..0x42 + nlen * 2)?;
        let name = String::from_utf16_lossy(
            &nb.chunks_exact(2)
                .map(|w| u16::from_le_bytes([w[0], w[1]]))
                .collect::<Vec<_>>(),
        );
        Some((name, parent))
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
        let mut size: u64 = 0;
        for_each_attr(&rec, |atype, attr| {
            match atype {
                0x30 if name.is_none() && attr[8] == 0 => {
                    name = parse_filename(attr);
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
        Some(RawEntry { name, parent, size, is_dir })
    }

    pub fn scan(drive: &str, progress: &dyn Fn(u64)) -> Result<Vec<FileEntry>, String> {
        let vol = Volume::open(drive)?;
        let bs = vol.read_at(0, 512)?;
        let (bps, spc, mft_lcn, rec_size) = parse_bootsector(&bs)?;
        let cluster = bps as u64 * spc as u64;
        let raw0 = vol.read_at((mft_lcn as u64) * cluster, rec_size)?;
        let rec0 = apply_fixup(&raw0, bps).ok_or("MFT record 0 fixup failed")?;
        let runs = mft_runs(&rec0)?;

        let total_clusters: u64 = runs.iter().map(|(_, n)| n).sum();
        let mut mft = Vec::with_capacity((total_clusters * cluster).min(512 * 1024 * 1024) as usize);
        for (lcn, ncl) in &runs {
            mft.extend_from_slice(&vol.read_at(lcn * cluster, ncl * cluster as usize)?);
        }
        drop(vol);

        let nrec = mft.len() / rec_size;
        let mut raws: HashMap<u64, RawEntry> = HashMap::with_capacity(nrec / 2);
        for (i, chunk) in mft.chunks_exact(rec_size).enumerate() {
            if i % 4096 == 0 {
                progress(i as u64);
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
            let size = if e.is_dir { total.get(&num).copied().unwrap_or(e.size) } else { e.size };
            out.push(FileEntry { path: path_of(num, &raws, drive, &mut memo), size, is_dir: e.is_dir });
        }
        Ok(out)
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

    pub fn scan(drive: &str, progress: &dyn Fn(u64)) -> Result<Vec<FileEntry>, String> {
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
                    if count % 4096 == 0 {
                        progress(count);
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
            .map(|(p, s)| FileEntry { path: p.to_string_lossy().into_owned(), size: s, is_dir: false })
            .collect();
        for (p, s) in dir_size {
            out.push(FileEntry { path: p.to_string_lossy().into_owned(), size: s, is_dir: true });
        }
        progress(count);
        Ok(out)
    }
}

pub use imp::{list_drives, scan};
