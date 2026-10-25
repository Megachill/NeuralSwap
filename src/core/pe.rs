//! Minimal PE reader: import table, delay-import table, bitness, version
//! resource and a byte-marker search. Never loads or executes a file.
//!
//! Used to tell a game executable apart from a launcher, to pick the right
//! ReShade hook, and to show which DLSS version a file actually is.

use std::collections::HashSet;
use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::Path;

struct Section {
    virtual_size: u32,
    virtual_address: u32,
    raw_size: u32,
    raw_offset: u32,
}

struct Headers {
    is64: bool,
    dirs: Vec<(u32, u32)>,
    sections: Vec<Section>,
}

fn read_at(f: &mut File, size: usize, pos: u64) -> Vec<u8> {
    let mut buf = vec![0u8; size];
    if f.seek(SeekFrom::Start(pos)).is_err() {
        return Vec::new();
    }
    let mut filled = 0;
    while filled < size {
        match f.read(&mut buf[filled..]) {
            Ok(0) => break,
            Ok(n) => filled += n,
            Err(_) => break,
        }
    }
    buf.truncate(filled);
    buf
}

fn u16le(b: &[u8], o: usize) -> u16 {
    u16::from_le_bytes([b[o], b[o + 1]])
}
fn u32le(b: &[u8], o: usize) -> u32 {
    u32::from_le_bytes([b[o], b[o + 1], b[o + 2], b[o + 3]])
}

fn read_headers(f: &mut File) -> Option<Headers> {
    let dos = read_at(f, 0x40, 0);
    if dos.len() < 0x40 || u16le(&dos, 0) != 0x5a4d {
        return None;
    }
    let pe_off = u32le(&dos, 0x3c) as u64;
    let coff = read_at(f, 24, pe_off);
    if coff.len() < 24 || u32le(&coff, 0) != 0x0000_4550 {
        return None;
    }
    let num_sections = u16le(&coff, 6) as usize;
    let opt_size = u16le(&coff, 20) as usize;
    let opt_off = pe_off + 24;
    let opt = read_at(f, opt_size, opt_off);
    if opt.len() < 2 {
        return None;
    }
    let is64 = u16le(&opt, 0) == 0x20b;
    let dd_off = if is64 { 112 } else { 96 };
    let mut dirs = Vec::new();
    for i in 0..16 {
        let o = dd_off + i * 8;
        if o + 8 > opt.len() {
            break;
        }
        dirs.push((u32le(&opt, o), u32le(&opt, o + 4)));
    }
    let table = read_at(f, num_sections.min(96) * 40, opt_off + opt_size as u64);
    let mut sections = Vec::new();
    for i in 0..num_sections {
        let o = i * 40;
        if o + 40 > table.len() {
            break;
        }
        sections.push(Section {
            virtual_size: u32le(&table, o + 8),
            virtual_address: u32le(&table, o + 12),
            raw_size: u32le(&table, o + 16),
            raw_offset: u32le(&table, o + 20),
        });
    }
    Some(Headers { is64, dirs, sections })
}

fn rva_to_offset(h: &Headers, rva: u32) -> Option<u64> {
    for s in &h.sections {
        let span = s.virtual_size.max(s.raw_size);
        if rva >= s.virtual_address && rva < s.virtual_address.saturating_add(span) {
            return Some(s.raw_offset as u64 + (rva - s.virtual_address) as u64);
        }
    }
    None
}

fn read_cstring(f: &mut File, pos: u64) -> String {
    let buf = read_at(f, 256, pos);
    let end = buf.iter().position(|&b| b == 0).unwrap_or(buf.len());
    buf[..end].iter().map(|&b| b as char).collect()
}

fn read_name_table(f: &mut File, h: &Headers, dir_index: usize, stride: usize, name_field: usize) -> Vec<String> {
    let Some(&(rva, size)) = h.dirs.get(dir_index) else { return Vec::new() };
    if rva == 0 {
        return Vec::new();
    }
    let Some(off) = rva_to_offset(h, rva) else { return Vec::new() };
    let want = if size == 0 { 4096 } else { size as usize };
    let table = read_at(f, want.min(64 * 1024), off);
    let mut names = Vec::new();
    let mut o = 0;
    while o + stride <= table.len() {
        let name_rva = u32le(&table, o + name_field);
        if name_rva == 0 {
            break;
        }
        if let Some(name_off) = rva_to_offset(h, name_rva) {
            names.push(read_cstring(f, name_off).to_lowercase());
        }
        o += stride;
    }
    names
}

/// Names of the DLLs the executable links against, lower-cased. Delay-loaded
/// imports count too — plenty of games bind d3d12 that way.
pub fn imports(file: &Path) -> Vec<String> {
    let Ok(mut f) = File::open(file) else { return Vec::new() };
    let Some(h) = read_headers(&mut f) else { return Vec::new() };
    let mut out: Vec<String> = Vec::new();
    for name in read_name_table(&mut f, &h, 1, 20, 12)
        .into_iter()
        .chain(read_name_table(&mut f, &h, 13, 32, 4))
    {
        if !out.contains(&name) {
            out.push(name);
        }
    }
    out
}

/// 32 or 64, from the optional-header magic. None when the file is not PE.
pub fn bitness(file: &Path) -> Option<u32> {
    let mut f = File::open(file).ok()?;
    read_headers(&mut f).map(|h| if h.is64 { 64 } else { 32 })
}

/// Which of the given ASCII markers appear anywhere in the file. A game that
/// loads Direct3D with LoadLibrary has no import entry for it, but the DLL name
/// and the entry point it asks for are still sitting there as plain strings.
pub fn find_markers(file: &Path, markers: &[&str]) -> HashSet<String> {
    let mut found = HashSet::new();
    let Ok(mut f) = File::open(file) else { return found };
    let finders: Vec<_> = markers.iter().map(|m| memchr::memmem::Finder::new(m.as_bytes())).collect();
    let longest = markers.iter().map(|m| m.len()).max().unwrap_or(0);
    let chunk = 4 * 1024 * 1024;
    let mut buf = vec![0u8; chunk + longest];
    let mut carry = 0usize;
    loop {
        let n = match f.read(&mut buf[carry..carry + chunk]) {
            Ok(0) | Err(_) => break,
            Ok(n) => n,
        };
        let view = &buf[..carry + n];
        for (i, finder) in finders.iter().enumerate() {
            if !found.contains(markers[i]) && finder.find(view).is_some() {
                found.insert(markers[i].to_string());
            }
        }
        if found.len() == markers.len() {
            break;
        }
        let keep = longest.min(view.len());
        let start = view.len() - keep;
        buf.copy_within(start..start + keep, 0);
        carry = keep;
    }
    found
}

/// True when the whole file contains the given byte string.
pub fn file_contains(file: &Path, needle: &str) -> bool {
    find_markers(file, &[needle]).contains(needle)
}

/// The raw RT_VERSION resource blob, or None when the file has none.
fn version_blob(file: &Path) -> Option<Vec<u8>> {
    let mut f = File::open(file).ok()?;
    let h = read_headers(&mut f)?;
    let &(rsrc_rva, _) = h.dirs.get(2)?;
    if rsrc_rva == 0 {
        return None;
    }
    let base = rva_to_offset(&h, rsrc_rva)?;

    let entries_of = |f: &mut File, dir_off: u32| -> Vec<(u32, u32)> {
        let hdr = read_at(f, 16, base + dir_off as u64);
        if hdr.len() < 16 {
            return Vec::new();
        }
        let count = (u16le(&hdr, 12) as usize + u16le(&hdr, 14) as usize).min(4096);
        let raw = read_at(f, count * 8, base + dir_off as u64 + 16);
        let mut out = Vec::new();
        for i in 0..count {
            let o = i * 8;
            if o + 8 > raw.len() {
                break;
            }
            out.push((u32le(&raw, o), u32le(&raw, o + 4)));
        }
        out
    };

    // type (16 = RT_VERSION) -> name -> language -> data entry
    let ty = entries_of(&mut f, 0)
        .into_iter()
        .find(|&(id, off)| (id & 0x7fff_ffff) == 16 && (off & 0x8000_0000) != 0)?;
    let name = *entries_of(&mut f, ty.1 & 0x7fff_ffff).first()?;
    if name.1 & 0x8000_0000 == 0 {
        return None;
    }
    let lang = *entries_of(&mut f, name.1 & 0x7fff_ffff).first()?;
    let data = read_at(&mut f, 16, base + lang.1 as u64);
    if data.len() < 16 {
        return None;
    }
    let data_off = rva_to_offset(&h, u32le(&data, 0))?;
    let data_size = u32le(&data, 4) as usize;
    if data_size == 0 {
        return None;
    }
    Some(read_at(&mut f, data_size.min(64 * 1024), data_off))
}

fn utf16(text: &str) -> Vec<u8> {
    text.encode_utf16().flat_map(|u| u.to_le_bytes()).collect()
}

/// "310.8.0.0" style version, read from VS_FIXEDFILEINFO, falling back to the
/// FileVersion/ProductVersion strings a few vendor DLLs use instead.
pub fn file_version(file: &Path) -> Option<String> {
    let blob = version_blob(file)?;
    if let Some(sig) = memchr::memmem::find(&blob, &[0xbd, 0x04, 0xef, 0xfe]) {
        if sig + 24 <= blob.len() {
            let ms = u32le(&blob, sig + 8);
            let ls = u32le(&blob, sig + 12);
            let fixed = format!("{}.{}.{}.{}", ms >> 16, ms & 0xffff, ls >> 16, ls & 0xffff);
            if fixed != "0.0.0.0" {
                return Some(fixed);
            }
        }
    }
    let re = regex::Regex::new(r"\d+(?:\s*[.,]\s*\d+){1,3}").unwrap();
    for key in ["FileVersion", "ProductVersion"] {
        let mut needle = utf16(key);
        needle.extend_from_slice(&[0, 0]);
        let Some(at) = memchr::memmem::find(&blob, &needle) else { continue };
        let mut value_at = (at + needle.len() + 3) & !3;
        let mut units = Vec::new();
        while value_at + 1 < blob.len() {
            let u = u16le(&blob, value_at);
            if u == 0 {
                break;
            }
            units.push(u);
            value_at += 2;
        }
        let raw = String::from_utf16_lossy(&units);
        if let Some(m) = re.find(raw.trim()) {
            let cleaned: String = m.as_str().chars().filter(|c| !c.is_whitespace()).collect();
            return Some(cleaned.replace(',', "."));
        }
    }
    None
}

/// True when the version resource mentions the given text (UTF-16 strings).
pub fn version_mentions(file: &Path, text: &str) -> bool {
    version_blob(file)
        .map(|blob| memchr::memmem::find(&blob, &utf16(text)).is_some())
        .unwrap_or(false)
}

/// Lexical compare of the first three numeric parts. Positive when a > b.
pub fn compare_versions(a: Option<&str>, b: Option<&str>) -> i64 {
    fn parse(v: &str) -> Option<[i64; 3]> {
        let re = regex::Regex::new(r"(\d+)\.(\d+)\.(\d+)").unwrap();
        let c = re.captures(v)?;
        Some([c[1].parse().ok()?, c[2].parse().ok()?, c[3].parse().ok()?])
    }
    let (Some(x), Some(y)) = (a.and_then(parse), b.and_then(parse)) else { return 0 };
    for i in 0..3 {
        if x[i] != y[i] {
            return x[i] - y[i];
        }
    }
    0
}
