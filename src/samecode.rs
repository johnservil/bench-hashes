//! Whether two builds of the bench hold the same code (bench-hashes #125):
//! `check`'s first verdict, before it measures anything. Two builds from one
//! path, against two commits of BLAKE3-Servil, compared byte for byte:
//! every allocated section but the notes (the build ID), with
//! - the two sides' commit names masked, which the bench writes on purpose
//!   (its provenance);
//! - every panic location's line and column masked: a change that moves
//!   lines (a test added above the code, a comment) moves those numbers,
//!   and they are data a panic prints, never code that runs.
//!
//! Nothing else is allowed to differ: no threshold, no allowance. Built at
//! one path, a test-only change of the library comes out identical, and a
//! change to code the bench runs differs in the functions it reaches (the
//! EPYC 9255 and the AmpereOne VM, data/2026-11-04/b125-ident: #228 against
//! its base identical but for 17 bytes of line numbers; MIN_SPLIT_LEN
//! halved, 7 functions; an AArch64-only constant identical on x86-64 and
//! 754 functions on AArch64). Built at two paths, everything moves: the
//! paths are in the panic messages, and the crate's metadata hash with
//! them (956 functions' bytes differed), so `check` builds both sides at
//! one path for this.
//!
//! ELF (Linux) and Mach-O (macOS), each read here: on another format
//! `compare` answers None, and `check` measures as before. On Mach-O a
//! section is named `SEGMENT,section`, a function's size runs to the next
//! symbol (its symbol table records none), and a pointer in data is a
//! chained fixup (dyld's rebase: the target's offset in its low 36 bits),
//! which the location mask decodes.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::path::Path;
use std::process::Command;

const SHF_ALLOC: u64 = 2;
const SHT_RELA: u32 = 4;
const SHT_NOBITS: u32 = 8;
const SHT_NOTE: u32 = 7;
/// R_X86_64_RELATIVE and R_AARCH64_RELATIVE: a PIE's pointer stored as the
/// relocation's addend (on disk the pointer's own bytes may be zero).
const RELATIVE: [u32; 2] = [8, 1027];

/// An executable's allocated sections (name: address, size, contents, the
/// contents empty for NOBITS or zero-fill) and each one's file offset; its
/// relative relocations (where: target), ELF's; the names of its code and
/// string sections; and, for Mach-O, its base address and its functions'
/// symbols (address, name, demangled), in the code section.
struct Image {
    sections: BTreeMap<String, (u64, u64, Vec<u8>)>,
    offsets: BTreeMap<String, usize>,
    relocs: HashMap<u64, u64>,
    text: &'static str,
    strings: &'static str,
    macho: Option<(u64, Vec<(u64, String)>)>,
}

impl Image {
    /// The address a pointer stored at `at` as `raw` holds, or addresses it
    /// may hold: ELF's relocation target, or its own bytes; on Mach-O, a
    /// rebase's target (dyld's chained fixups: bit 63 clear, the target in
    /// bits 0-35 and its top byte in bits 36-43, as an address or as an
    /// offset from the base, by the pointer format), none for a bind.
    fn pointers(&self, at: u64, raw: u64) -> Vec<u64> {
        match &self.macho {
            None => vec![self.relocs.get(&at).copied().unwrap_or(raw)],
            Some(_) if raw >> 63 == 1 => Vec::new(),
            Some((base, _)) => {
                let target = (raw & 0xF_FFFF_FFFF) | ((raw >> 36) & 0xFF) << 56;
                vec![target, target.wrapping_add(*base)]
            }
        }
    }
}

fn u16_at(d: &[u8], o: usize) -> u16 {
    u16::from_le_bytes(d[o..o + 2].try_into().unwrap())
}
fn u32_at(d: &[u8], o: usize) -> u32 {
    u32::from_le_bytes(d[o..o + 4].try_into().unwrap())
}
fn u64_at(d: &[u8], o: usize) -> u64 {
    u64::from_le_bytes(d[o..o + 8].try_into().unwrap())
}

/// A 64-bit little-endian ELF or Mach-O file's image; None for any other file.
fn read(path: &Path) -> Option<Image> {
    let d = std::fs::read(path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    if d.len() >= 32 && u32_at(&d, 0) == MH_MAGIC_64 {
        return Some(read_macho(&d));
    }
    if d.len() < 64 || &d[..4] != b"\x7fELF" || d[4] != 2 || d[5] != 1 {
        return None;
    }
    let shoff = u64_at(&d, 0x28) as usize;
    let (entsize, count, names) = (
        u16_at(&d, 0x3A) as usize,
        u16_at(&d, 0x3C) as usize,
        u16_at(&d, 0x3E) as usize,
    );
    let header = |i: usize| {
        let o = shoff + i * entsize;
        // name, type, flags, addr, offset, size
        (
            u32_at(&d, o),
            u32_at(&d, o + 4),
            u64_at(&d, o + 8),
            u64_at(&d, o + 16),
            u64_at(&d, o + 24) as usize,
            u64_at(&d, o + 32),
        )
    };
    let names_at = header(names).4;
    let name_of = |off: u32| {
        let start = names_at + off as usize;
        let end = start
            + d[start..]
                .iter()
                .position(|&b| b == 0)
                .expect("a section name ends");
        String::from_utf8_lossy(&d[start..end]).into_owned()
    };
    let mut sections = BTreeMap::new();
    let mut offsets = BTreeMap::new();
    let mut relocs = HashMap::new();
    for i in 0..count {
        let (name, kind, flags, addr, offset, size) = header(i);
        if kind == SHT_RELA {
            for r in d[offset..offset + size as usize].chunks_exact(24) {
                if RELATIVE.contains(&(u64_at(r, 8) as u32)) {
                    relocs.insert(u64_at(r, 0), u64_at(r, 16));
                }
            }
        }
        if flags & SHF_ALLOC == 0 || kind == SHT_NOTE {
            continue;
        }
        let body = if kind == SHT_NOBITS {
            Vec::new()
        } else {
            d[offset..offset + size as usize].to_vec()
        };
        offsets.insert(name_of(name), offset);
        sections.insert(name_of(name), (addr, size, body));
    }
    Some(Image {
        sections,
        offsets,
        relocs,
        text: ".text",
        strings: ".rodata",
        macho: None,
    })
}

const MH_MAGIC_64: u32 = 0xFEED_FACF;
const LC_SYMTAB: u32 = 0x2;
const LC_SEGMENT_64: u32 = 0x19;
/// A Mach-O section's types whose contents are not in the file.
const ZERO_FILL: [u32; 3] = [0x1, 0xC, 0x12];

/// A 64-bit Mach-O file's image (its magic read already): every section of
/// every segment, and the defined symbols of `__TEXT,__text`.
fn read_macho(d: &[u8]) -> Image {
    let fixed = |b: &[u8]| {
        String::from_utf8_lossy(&b[..b.iter().position(|&c| c == 0).unwrap_or(b.len())])
            .into_owned()
    };
    let (mut sections, mut offsets) = (BTreeMap::new(), BTreeMap::new());
    let (mut order, mut base, mut symtab) = (Vec::new(), None, None);
    let mut o = 32;
    for _ in 0..u32_at(d, 16) {
        let (cmd, size) = (u32_at(d, o), u32_at(d, o + 4) as usize);
        if cmd == LC_SEGMENT_64 {
            let segment = fixed(&d[o + 8..o + 24]);
            if segment == "__TEXT" {
                base = Some(u64_at(d, o + 24));
            }
            for i in 0..u32_at(d, o + 64) as usize {
                let s = o + 72 + 80 * i;
                let name = format!("{},{}", fixed(&d[s + 16..s + 32]), fixed(&d[s..s + 16]));
                let (addr, len, offset) = (
                    u64_at(d, s + 32),
                    u64_at(d, s + 40),
                    u32_at(d, s + 48) as usize,
                );
                let body = if ZERO_FILL.contains(&(u32_at(d, s + 64) & 0xFF)) {
                    Vec::new()
                } else {
                    d[offset..offset + len as usize].to_vec()
                };
                order.push(name.clone());
                offsets.insert(name.clone(), offset);
                sections.insert(name, (addr, len, body));
            }
        } else if cmd == LC_SYMTAB {
            symtab = Some((
                u32_at(d, o + 8) as usize,
                u32_at(d, o + 12) as usize,
                u32_at(d, o + 16) as usize,
            ));
        }
        o += size;
    }
    // The defined symbols of __TEXT,__text: nlist_64 entries of type N_SECT,
    // no debugging entry, whose section (counted from 1) is it.
    let mut symbols = Vec::new();
    if let (Some((symoff, nsyms, stroff)), Some(text)) =
        (symtab, order.iter().position(|n| n == "__TEXT,__text"))
    {
        for i in 0..nsyms {
            let e = symoff + 16 * i;
            let (strx, kind, sect) = (u32_at(d, e) as usize, d[e + 4], d[e + 5] as usize);
            if kind & 0xE0 == 0 && kind & 0x0E == 0x0E && sect == text + 1 {
                symbols.push((u64_at(d, e + 8), demangle(&fixed(&d[stroff + strx..]))));
            }
        }
    }
    symbols.sort();
    Image {
        sections,
        offsets,
        relocs: HashMap::new(),
        text: "__TEXT,__text",
        strings: "__TEXT,__const",
        macho: Some((base.unwrap_or(0), symbols)),
    }
}

/// A Rust symbol's name, demangled as `nm -C` does (legacy mangling: the
/// path's parts by length, their escapes, and the hash dropped later); a C
/// symbol without its leading underscore, as it is.
fn demangle(symbol: &str) -> String {
    let s = symbol.strip_prefix('_').unwrap_or(symbol);
    let Some(mut rest) = s.strip_prefix("_ZN") else {
        return s.to_string();
    };
    let mut parts = Vec::new();
    while let Some(len_end) = rest.find(|c: char| !c.is_ascii_digit()).filter(|&i| i > 0) {
        let len: usize = rest[..len_end].parse().unwrap();
        let Some(part) = rest.get(len_end..len_end + len) else {
            return s.to_string();
        };
        parts.push(
            part.strip_prefix("_$")
                .map_or(part.to_string(), |p| format!("${p}")),
        );
        rest = &rest[len_end + len..];
    }
    let mut name = parts.join("::");
    for (escape, ch) in [
        ("$LT$", "<"),
        ("$GT$", ">"),
        ("$RF$", "&"),
        ("$BP$", "*"),
        ("$C$", ","),
        ("$SP$", "@"),
        ("$u20$", " "),
        ("$u27$", "'"),
        ("$u5b$", "["),
        ("$u5d$", "]"),
        ("$u7b$", "{"),
        ("$u7d$", "}"),
        ("$u7e$", "~"),
        ("$u3b$", ";"),
        ("$u2b$", "+"),
        ("$u22$", "\""),
        ("$LP$", "("),
        ("$RP$", ")"),
        ("..", "::"),
    ] {
        name = name.replace(escape, ch);
    }
    name
}

/// Every panic location's line and column in `image`, zeroed: a
/// core::panic::Location is a file's &str (pointer, length), then line and
/// column (u32 each); one is a record, 8-aligned in a data section, whose
/// pointer (its relocation's target, or its own bytes) reaches a string in
/// .rodata of that length ending in ".rs". Where it zeroed them (section,
/// address of the line).
fn mask_locations(image: &mut Image) -> Vec<(String, u64)> {
    let Some((ro_addr, ro_size, rodata)) = image.sections.get(image.strings).cloned() else {
        return Vec::new();
    };
    let mut masked = Vec::new();
    let mut sections = std::mem::take(&mut image.sections);
    for (name, (addr, size, body)) in sections.iter_mut() {
        if body.is_empty() || name == image.text || name == image.strings {
            continue;
        }
        let mut o = 0;
        while o + 24 <= (*size as usize).min(body.len()) {
            let at = *addr + o as u64;
            let len = u64_at(body, o + 8);
            let points = image.pointers(at, u64_at(body, o)).into_iter().any(|ptr| {
                (ro_addr..ro_addr + ro_size).contains(&ptr)
                    && (4..4096).contains(&len)
                    && ptr + len <= ro_addr + ro_size
                    && {
                        let text =
                            &rodata[(ptr - ro_addr) as usize..(ptr - ro_addr + len) as usize];
                        text.ends_with(b".rs") && !text.contains(&0)
                    }
            });
            if points {
                body[o + 16..o + 24].fill(0);
                masked.push((name.clone(), at + 16));
            }
            o += 8;
        }
    }
    image.sections = sections;
    masked
}

/// `body` with every occurrence of each of `masks` zeroed.
fn mask_strings(body: &mut [u8], masks: &[&str]) {
    for m in masks.iter().map(|m| m.as_bytes()).filter(|m| !m.is_empty()) {
        let mut i = 0;
        while i + m.len() <= body.len() {
            if &body[i..i + m.len()] == m {
                body[i..i + m.len()].fill(b'0');
                i += m.len();
            } else {
                i += 1;
            }
        }
    }
}

/// What two builds hold.
#[derive(Debug, PartialEq)]
pub enum Verdict {
    /// The same code and data, as masked.
    Same,
    /// The sections that differ, and the functions (by name, demangled, the
    /// legacy hash suffix dropped): those of another size or on one side
    /// alone, where a change lands; and those of the same size and other
    /// bytes, among them every function whose calls' targets moved.
    Differs {
        sections: Vec<String>,
        resized: Vec<String>,
        changed: Vec<String>,
    },
}

/// Each function's sizes and bytes, by name: on Mach-O from the image's
/// symbols, each running to the next; on ELF by nm -S -C over the symbol table.
fn functions_of(path: &Path, image: &Image) -> BTreeMap<String, Vec<(u64, Vec<u8>)>> {
    let text = image.sections.get(image.text);
    let Some((_, symbols)) = &image.macho else {
        return functions(path, text);
    };
    let mut found: BTreeMap<String, Vec<(u64, Vec<u8>)>> = BTreeMap::new();
    let (start, len, body) = text.cloned().unwrap_or_default();
    for (i, (addr, symbol)) in symbols.iter().enumerate() {
        let end = symbols.get(i + 1).map_or(start + len, |s| s.0);
        let name = strip_hash(symbol);
        let (o, e) = ((addr - start) as usize, (end - start) as usize);
        found.entry(name.to_string()).or_default().push((
            end - addr,
            body.get(o..e.min(body.len())).unwrap_or_default().to_vec(),
        ));
    }
    for v in found.values_mut() {
        v.sort();
    }
    found
}

/// A demangled name without the legacy mangling's hash (`::h` and 16 hex digits).
fn strip_hash(name: &str) -> &str {
    match name.rfind("::h") {
        Some(i) if name.len() - i == 19 && name[i + 3..].bytes().all(|b| b.is_ascii_hexdigit()) => {
            &name[..i]
        }
        _ => name,
    }
}

/// Each function's sizes and bytes, by name (nm -S -C over the symbol table).
fn functions(
    path: &Path,
    text: Option<&(u64, u64, Vec<u8>)>,
) -> BTreeMap<String, Vec<(u64, Vec<u8>)>> {
    let out = Command::new("nm")
        .args(["-S", "-C", "--defined-only"])
        .arg(path)
        .output()
        .expect("nm runs");
    assert!(
        out.status.success(),
        "nm {}: {}",
        path.display(),
        String::from_utf8_lossy(&out.stderr)
    );
    let mut found: BTreeMap<String, Vec<(u64, Vec<u8>)>> = BTreeMap::new();
    for line in String::from_utf8_lossy(&out.stdout).lines() {
        let f: Vec<&str> = line.splitn(4, ' ').collect();
        if f.len() != 4 || !matches!(f[2], "t" | "T") {
            continue;
        }
        let (Ok(addr), Ok(size)) = (u64::from_str_radix(f[0], 16), u64::from_str_radix(f[1], 16))
        else {
            continue;
        };
        let name = strip_hash(f[3]);
        let bytes = match text {
            Some((t, tsize, body)) if (*t..t + tsize).contains(&addr) => {
                let o = (addr - t) as usize;
                body[o..(o + size as usize).min(body.len())].to_vec()
            }
            _ => Vec::new(),
        };
        found
            .entry(name.to_string())
            .or_default()
            .push((size, bytes));
    }
    for v in found.values_mut() {
        v.sort();
    }
    found
}

/// Whether the builds `a` and `b` hold the same code, `masks` (the sides'
/// commit names) zeroed in both, and with `locations` every panic
/// location's line and column; None where either is neither ELF nor Mach-O.
pub fn compare(a: &Path, b: &Path, masks: &[&str], locations: bool) -> Option<Verdict> {
    let (mut ia, mut ib) = (read(a)?, read(b)?);
    if locations {
        mask_locations(&mut ia);
        mask_locations(&mut ib);
    }
    for image in [&mut ia, &mut ib] {
        for (_, _, body) in image.sections.values_mut() {
            mask_strings(body, masks);
        }
    }
    let names: BTreeSet<&String> = ia.sections.keys().chain(ib.sections.keys()).collect();
    let sections: Vec<String> = names
        .into_iter()
        .filter(|n| match (ia.sections.get(*n), ib.sections.get(*n)) {
            (Some((_, sa, ba)), Some((_, sb, bb))) => sa != sb || ba != bb,
            _ => true,
        })
        .cloned()
        .collect();
    if sections.is_empty() {
        return Some(Verdict::Same);
    }
    let (fa, fb) = (functions_of(a, &ia), functions_of(b, &ib));
    let sizes = |f: &BTreeMap<String, Vec<(u64, Vec<u8>)>>, n: &String| {
        f.get(n)
            .map(|v| v.iter().map(|(s, _)| *s).collect::<Vec<_>>())
    };
    let (mut resized, mut changed) = (Vec::new(), Vec::new());
    for n in fa.keys().chain(fb.keys()).collect::<BTreeSet<_>>() {
        if sizes(&fa, n) != sizes(&fb, n) {
            resized.push(n.clone());
        } else if fa.get(n) != fb.get(n) {
            changed.push(n.clone());
        }
    }
    Some(Verdict::Differs {
        sections,
        resized,
        changed,
    })
}

#[cfg(all(test, target_os = "linux"))]
mod tests {
    use super::*;

    /// A string the test binary holds, as the bench holds its commits.
    const MARK: &str = "0123456789abcdef0123456789abcdef01234567";

    #[inline(never)]
    fn marker_function(x: u64) -> u64 {
        std::hint::black_box(x).wrapping_mul(0x9E37_79B9_7F4A_7C15) ^ 0x1234
    }

    /// A copy of this test binary in a directory of the test's own, changed by `edit`.
    fn copy_with(tag: &str, edit: impl FnOnce(&mut Vec<u8>)) -> std::path::PathBuf {
        let exe = std::env::current_exe().unwrap();
        let dir = std::env::temp_dir().join(format!("samecode-{}-{tag}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let copy = dir.join("exe");
        let mut data = std::fs::read(&exe).unwrap();
        edit(&mut data);
        std::fs::write(&copy, data).unwrap();
        copy
    }

    /// The file offset of virtual address `addr` in `section` of `exe`.
    fn offset_of(exe: &[u8], section: &str, addr: u64) -> usize {
        let shoff = u64_at(exe, 0x28) as usize;
        let (entsize, count, names) = (
            u16_at(exe, 0x3A) as usize,
            u16_at(exe, 0x3C) as usize,
            u16_at(exe, 0x3E) as usize,
        );
        let names_at = u64_at(exe, shoff + names * entsize + 24) as usize;
        (0..count)
            .map(|i| shoff + i * entsize)
            .find(|&o| {
                let start = names_at + u32_at(exe, o) as usize;
                exe[start..].starts_with(section.as_bytes()) && exe[start + section.len()] == 0
            })
            .map(|o| u64_at(exe, o + 24) as usize + (addr - u64_at(exe, o + 16)) as usize)
            .expect("the section")
    }

    #[test]
    fn a_build_is_the_same_as_itself() {
        let exe = std::env::current_exe().unwrap();
        assert_eq!(compare(&exe, &exe, &[], true), Some(Verdict::Same));
        std::hint::black_box(MARK);
    }

    #[test]
    fn a_changed_function_differs_by_name() {
        let exe = std::env::current_exe().unwrap();
        assert_eq!(
            marker_function(1),
            std::hint::black_box(1u64).wrapping_mul(0x9E37_79B9_7F4A_7C15) ^ 0x1234
        );
        let image = read(&exe).unwrap();
        let fs = functions(&exe, image.sections.get(".text"));
        let (name, _) = fs
            .iter()
            .find(|(n, _)| n.ends_with("samecode::tests::marker_function"))
            .expect("the marker in the symbol table");
        let nm = Command::new("nm")
            .args(["-S", "-C", "--defined-only"])
            .arg(&exe)
            .output()
            .unwrap();
        let line = String::from_utf8_lossy(&nm.stdout)
            .lines()
            .find(|l| l.contains("samecode::tests::marker_function"))
            .unwrap()
            .to_string();
        let addr = u64::from_str_radix(line.split(' ').next().unwrap(), 16).unwrap();
        let copy = copy_with("function", |data| {
            let o = offset_of(data, ".text", addr);
            data[o] ^= 0xFF;
        });
        match compare(&exe, &copy, &[], true) {
            Some(Verdict::Differs {
                sections,
                resized,
                changed,
            }) => {
                assert_eq!(sections, [".text"]);
                assert_eq!(resized, Vec::<String>::new());
                assert_eq!(changed, [name.clone()]);
            }
            other => panic!("one changed byte reads {other:?}"),
        }
        std::fs::remove_dir_all(copy.parent().unwrap()).unwrap();
    }

    #[test]
    fn the_commits_are_masked() {
        let exe = std::env::current_exe().unwrap();
        std::hint::black_box(MARK);
        let other = "fedcba9876543210fedcba9876543210fedcba98";
        let copy = copy_with("commit", |data| {
            let at = data
                .windows(MARK.len())
                .position(|w| w == MARK.as_bytes())
                .expect("the mark in the binary");
            data[at..at + MARK.len()].copy_from_slice(other.as_bytes());
        });
        assert!(
            matches!(
                compare(&exe, &copy, &[], true),
                Some(Verdict::Differs { .. })
            ),
            "unmasked, a commit differs"
        );
        assert_eq!(
            compare(&exe, &copy, &[MARK, other], true),
            Some(Verdict::Same)
        );
        std::fs::remove_dir_all(copy.parent().unwrap()).unwrap();
    }

    #[test]
    fn a_moved_line_is_masked_and_only_with_the_mask() {
        let exe = std::env::current_exe().unwrap();
        // A panic location of this file, so the binary holds one.
        let here = std::panic::Location::caller();
        assert!(here.file().ends_with(".rs"));
        let mut image = read(&exe).unwrap();
        let masked = mask_locations(&mut image);
        assert!(!masked.is_empty(), "the test binary has panic locations");
        // The first location's line, moved by one.
        let (section, line_at) = masked[0].clone();
        let copy = copy_with("line", |data| {
            let o = offset_of(data, &section, line_at);
            let line = u32_at(data, o).wrapping_add(1);
            data[o..o + 4].copy_from_slice(&line.to_le_bytes());
        });
        assert!(
            matches!(
                compare(&exe, &copy, &[], false),
                Some(Verdict::Differs { .. })
            ),
            "a moved line differs unmasked"
        );
        assert_eq!(
            compare(&exe, &copy, &[], true),
            Some(Verdict::Same),
            "and is the same masked"
        );
        std::fs::remove_dir_all(copy.parent().unwrap()).unwrap();
    }
}

#[cfg(all(test, target_os = "macos"))]
mod macho_tests {
    use super::*;

    const MARK: &str = "0123456789abcdef0123456789abcdef01234567";

    #[inline(never)]
    fn marker_function(x: u64) -> u64 {
        std::hint::black_box(x).wrapping_mul(0x9E37_79B9_7F4A_7C15) ^ 0x1234
    }

    /// A copy of this test binary, changed by `edit`. (The copy's code
    /// signature no longer matches; it is read, never run.)
    fn copy_with(tag: &str, edit: impl FnOnce(&mut Vec<u8>)) -> std::path::PathBuf {
        let exe = std::env::current_exe().unwrap();
        let dir = std::env::temp_dir().join(format!("samecode-{}-{tag}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let copy = dir.join("exe");
        let mut data = std::fs::read(&exe).unwrap();
        edit(&mut data);
        std::fs::write(&copy, data).unwrap();
        copy
    }

    /// The file offset of `addr` in `section` of the image.
    fn offset_of(image: &Image, section: &str, addr: u64) -> usize {
        image.offsets[section] + (addr - image.sections[section].0) as usize
    }

    #[test]
    fn this_test_binary_reads_as_mach_o() {
        let image = read(&std::env::current_exe().unwrap()).expect("a Mach-O image");
        assert!(
            image.sections.contains_key("__TEXT,__text")
                && image.sections.contains_key("__TEXT,__const")
        );
        let (_, symbols) = image.macho.as_ref().unwrap();
        assert!(
            symbols
                .iter()
                .any(|(_, n)| n.contains("samecode::macho_tests::marker_function")),
            "demangled symbols"
        );
    }

    #[test]
    fn a_build_is_the_same_as_itself() {
        let exe = std::env::current_exe().unwrap();
        assert_eq!(compare(&exe, &exe, &[], true), Some(Verdict::Same));
        std::hint::black_box(MARK);
    }

    #[test]
    fn a_changed_function_differs_by_name() {
        let exe = std::env::current_exe().unwrap();
        assert_eq!(
            marker_function(1),
            std::hint::black_box(1u64).wrapping_mul(0x9E37_79B9_7F4A_7C15) ^ 0x1234
        );
        let image = read(&exe).unwrap();
        let (_, symbols) = image.macho.as_ref().unwrap();
        let (addr, name) = symbols
            .iter()
            .find(|(_, n)| n.contains("samecode::macho_tests::marker_function"))
            .unwrap()
            .clone();
        let copy = copy_with("function", |data| {
            let o = offset_of(&image, "__TEXT,__text", addr);
            data[o] ^= 0xFF;
        });
        match compare(&exe, &copy, &[], true) {
            Some(Verdict::Differs {
                sections,
                resized,
                changed,
            }) => {
                assert_eq!(sections, ["__TEXT,__text"]);
                assert_eq!(resized, Vec::<String>::new());
                assert_eq!(changed, [strip_hash(&name).to_string()]);
            }
            other => panic!("one changed byte reads {other:?}"),
        }
        std::fs::remove_dir_all(copy.parent().unwrap()).unwrap();
    }

    #[test]
    fn the_commits_are_masked() {
        let exe = std::env::current_exe().unwrap();
        std::hint::black_box(MARK);
        let other = "fedcba9876543210fedcba9876543210fedcba98";
        let copy = copy_with("commit", |data| {
            let at = data
                .windows(MARK.len())
                .position(|w| w == MARK.as_bytes())
                .expect("the mark in the binary");
            data[at..at + MARK.len()].copy_from_slice(other.as_bytes());
        });
        assert!(
            matches!(
                compare(&exe, &copy, &[], true),
                Some(Verdict::Differs { .. })
            ),
            "unmasked, a commit differs"
        );
        assert_eq!(
            compare(&exe, &copy, &[MARK, other], true),
            Some(Verdict::Same)
        );
        std::fs::remove_dir_all(copy.parent().unwrap()).unwrap();
    }

    #[test]
    fn a_moved_line_is_masked_and_only_with_the_mask() {
        let exe = std::env::current_exe().unwrap();
        let here = std::panic::Location::caller();
        assert!(here.file().ends_with(".rs"));
        let mut image = read(&exe).unwrap();
        let masked = mask_locations(&mut image);
        assert!(
            !masked.is_empty(),
            "the test binary's panic locations, through their chained fixups"
        );
        let (section, line_at) = masked[0].clone();
        let o = offset_of(&image, &section, line_at);
        let copy = copy_with("line", |data| {
            let line = u32_at(data, o).wrapping_add(1);
            data[o..o + 4].copy_from_slice(&line.to_le_bytes());
        });
        assert!(
            matches!(
                compare(&exe, &copy, &[], false),
                Some(Verdict::Differs { .. })
            ),
            "a moved line differs unmasked"
        );
        assert_eq!(
            compare(&exe, &copy, &[], true),
            Some(Verdict::Same),
            "and is the same masked"
        );
        std::fs::remove_dir_all(copy.parent().unwrap()).unwrap();
    }
}

#[cfg(test)]
mod demangle_tests {
    #[test]
    fn rust_symbols_demangle_as_nm_does() {
        assert_eq!(
            super::demangle("__ZN13blake3_servil5lanes7sources6Source7publish17h0123456789abcdefE"),
            "blake3_servil::lanes::sources::Source::publish::h0123456789abcdef"
        );
        assert_eq!(
            super::strip_hash(&super::demangle("__ZN3foo3bar17h0123456789abcdefE")),
            "foo::bar"
        );
        assert_eq!(super::demangle("__ZN4core3ptr85drop_in_place$LT$std..rt..lang_start$LT$$LP$$RP$$GT$..$u7b$$u7b$closure$u7d$$u7d$$GT$17h0123456789abcdefE"),
            "core::ptr::drop_in_place<std::rt::lang_start<()>::{{closure}}>::h0123456789abcdef");
        assert_eq!(super::demangle("_main"), "main");
    }
}
