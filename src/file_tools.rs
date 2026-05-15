//! `/file`, `/binwalk`, `/exif` — attachment introspection.
//!
//! Everything operates on a downloaded byte buffer; nothing escapes to the
//! filesystem. `binwalk::Binwalk::scan` is the scan-only API (extraction
//! is a separate call we never make), so even crafted firmware blobs only
//! produce signature results, not files written to disk.

use sha2::{Digest, Sha256};
use std::io::Cursor;

/// Summary returned by [`analyze`]. `binwalk_findings` is bounded — large
/// blobs can produce hundreds of matches; we keep the first N so the chat
/// message stays under Telegram's size cap.
#[derive(Debug, Clone)]
pub struct FileInfo {
    pub size: u64,
    pub sha256: String,
    pub infer_mime: Option<String>,
    pub format_short: Option<String>,
    pub format_full: Option<String>,
    pub binwalk_findings: Vec<BinwalkFinding>,
    pub binwalk_truncated: bool,
    /// `Some` when ELF/PE/Mach-O/Wasm/COFF: arch comes from the header.
    pub header_arch: Option<HeaderArch>,
    /// Always populated; for header-bearing binaries it's redundant info,
    /// for raw stripped binaries it's the only signal.
    pub arch_guesses: Vec<ArchGuess>,
}

#[derive(Debug, Clone)]
pub struct BinwalkFinding {
    pub offset: u64,
    pub name: String,
    pub description: String,
}

/// Architecture identification from a binary's header. Returned by
/// [`parse_header_arch`] when the input is an ELF, PE, or Mach-O.
#[derive(Debug, Clone)]
pub struct HeaderArch {
    pub format: &'static str,
    pub arch: String,
    pub endian: &'static str,
    pub bits: u8,
    pub entry: u64,
    pub sections: usize,
}

/// Heuristic match score for a single ISA candidate. The candidate with
/// the highest `bytes_covered` over the disassembly window is the most
/// likely architecture for a raw stripped binary.
#[derive(Debug, Clone)]
pub struct ArchGuess {
    pub label: &'static str,
    pub instructions: usize,
    pub bytes_covered: usize,
    pub window_bytes: usize,
}

const BINWALK_FINDING_CAP: usize = 24;

pub fn analyze(data: &[u8]) -> FileInfo {
    let sha256 = {
        let mut h = Sha256::new();
        h.update(data);
        hex::encode(h.finalize())
    };
    let infer_mime = infer::get(data).map(|t| t.mime_type().to_string());
    let fmt = file_format::FileFormat::from_bytes(data);
    // `name()` always returns a string; `short_name()` is None for some
    // formats — keep both, the renderer collapses if they're the same.
    let format_short = fmt.short_name().map(|s| s.to_string());
    let format_full = Some(fmt.name().to_string());

    let binwalker = binwalk::Binwalk::new();
    let raw = binwalker.scan(data);
    let total = raw.len();
    let binwalk_findings: Vec<BinwalkFinding> = raw
        .into_iter()
        .take(BINWALK_FINDING_CAP)
        .map(|r| BinwalkFinding {
            offset: r.offset as u64,
            name: r.name,
            description: r.description,
        })
        .collect();
    let binwalk_truncated = total > BINWALK_FINDING_CAP;

    let header_arch = parse_header_arch(data);
    // For raw stripped binaries the heuristic is the whole story; for
    // header-bearing ones it's a "second opinion" + lets the user see the
    // confidence margin between top candidates.
    let arch_guesses = guess_arch_heuristic(data, 3);

    FileInfo {
        size: data.len() as u64,
        sha256,
        infer_mime,
        format_short,
        format_full,
        binwalk_findings,
        binwalk_truncated,
        header_arch,
        arch_guesses,
    }
}

/// Try parsing `data` as ELF / PE / Mach-O / Wasm / COFF and extract
/// architectural facts directly from the header. Returns `None` when the
/// input has no recognized binary container header (raw stripped firmware,
/// archives, images, etc.).
pub fn parse_header_arch(data: &[u8]) -> Option<HeaderArch> {
    use object::read::Object;
    let file = object::File::parse(data).ok()?;
    let arch = match file.architecture() {
        object::Architecture::Aarch64 => "aarch64",
        object::Architecture::Aarch64_Ilp32 => "aarch64-ilp32",
        object::Architecture::Arm => "arm",
        object::Architecture::Avr => "avr",
        object::Architecture::Bpf => "bpf",
        object::Architecture::I386 => "x86 (32)",
        object::Architecture::X86_64 => "x86_64",
        object::Architecture::X86_64_X32 => "x86_64-x32",
        object::Architecture::Hexagon => "hexagon",
        object::Architecture::LoongArch64 => "loongarch64",
        object::Architecture::Mips => "mips32",
        object::Architecture::Mips64 => "mips64",
        object::Architecture::Msp430 => "msp430",
        object::Architecture::PowerPc => "powerpc",
        object::Architecture::PowerPc64 => "powerpc64",
        object::Architecture::Riscv32 => "riscv32",
        object::Architecture::Riscv64 => "riscv64",
        object::Architecture::S390x => "s390x",
        object::Architecture::Sbf => "solana-bpf",
        object::Architecture::Sparc => "sparc",
        object::Architecture::Sparc32Plus => "sparc32+",
        object::Architecture::Sparc64 => "sparc64",
        object::Architecture::Wasm32 => "wasm32",
        object::Architecture::Wasm64 => "wasm64",
        object::Architecture::Xtensa => "xtensa",
        other => return Some(HeaderArch {
            format: format_label(&file),
            arch: format!("{other:?}"),
            endian: if file.is_little_endian() { "little" } else { "big" },
            bits: if file.is_64() { 64 } else { 32 },
            entry: file.entry(),
            sections: file.sections().count(),
        }),
    };
    Some(HeaderArch {
        format: format_label(&file),
        arch: arch.to_string(),
        endian: if file.is_little_endian() { "little" } else { "big" },
        bits: if file.is_64() { 64 } else { 32 },
        entry: file.entry(),
        sections: file.sections().count(),
    })
}

fn format_label(file: &object::File) -> &'static str {
    use object::BinaryFormat;
    match file.format() {
        BinaryFormat::Elf => "ELF",
        BinaryFormat::Coff => "COFF",
        BinaryFormat::Pe => "PE",
        BinaryFormat::MachO => "Mach-O",
        BinaryFormat::Wasm => "Wasm",
        BinaryFormat::Xcoff => "XCOFF",
        _ => "binary",
    }
}

/// For header-less binaries (raw firmware), try disassembling a window of
/// bytes against every supported ISA and rank by the fraction the
/// disassembler could chew through before stalling. Returns at most
/// `keep_top` candidates, ordered by descending coverage.
pub fn guess_arch_heuristic(data: &[u8], keep_top: usize) -> Vec<ArchGuess> {
    use capstone::prelude::*;

    let window_len = data.len().min(64 * 1024);
    let window = &data[..window_len];
    if window.is_empty() {
        return Vec::new();
    }

    let candidates: Vec<(&'static str, fn() -> Result<Capstone, capstone::Error>)> = vec![
        ("x86_64",     || Capstone::new().x86().mode(arch::x86::ArchMode::Mode64).build()),
        ("x86 (32)",   || Capstone::new().x86().mode(arch::x86::ArchMode::Mode32).build()),
        ("x86 (16)",   || Capstone::new().x86().mode(arch::x86::ArchMode::Mode16).build()),
        ("aarch64",    || Capstone::new().arm64().mode(arch::arm64::ArchMode::Arm).build()),
        ("arm (LE)",   || Capstone::new().arm().mode(arch::arm::ArchMode::Arm).endian(capstone::Endian::Little).build()),
        ("arm (BE)",   || Capstone::new().arm().mode(arch::arm::ArchMode::Arm).endian(capstone::Endian::Big).build()),
        ("thumb",      || Capstone::new().arm().mode(arch::arm::ArchMode::Thumb).build()),
        ("riscv64",    || Capstone::new().riscv().mode(arch::riscv::ArchMode::RiscV64).build()),
        ("riscv32",    || Capstone::new().riscv().mode(arch::riscv::ArchMode::RiscV32).build()),
        ("mips32 (BE)",|| Capstone::new().mips().mode(arch::mips::ArchMode::Mips32).endian(capstone::Endian::Big).build()),
        ("mips32 (LE)",|| Capstone::new().mips().mode(arch::mips::ArchMode::Mips32).endian(capstone::Endian::Little).build()),
        ("mips64 (BE)",|| Capstone::new().mips().mode(arch::mips::ArchMode::Mips64).endian(capstone::Endian::Big).build()),
        ("mips64 (LE)",|| Capstone::new().mips().mode(arch::mips::ArchMode::Mips64).endian(capstone::Endian::Little).build()),
        ("powerpc",    || Capstone::new().ppc().mode(arch::ppc::ArchMode::Mode32).endian(capstone::Endian::Big).build()),
        ("powerpc64",  || Capstone::new().ppc().mode(arch::ppc::ArchMode::Mode64).endian(capstone::Endian::Big).build()),
        ("sparc",      || Capstone::new().sparc().mode(arch::sparc::ArchMode::Default).build()),
    ];

    let mut out: Vec<ArchGuess> = Vec::new();
    for (label, builder) in candidates {
        let Ok(cs) = builder() else { continue };
        let Ok(insns) = cs.disasm_all(window, 0) else { continue };
        let bytes_covered: usize = insns.iter().map(|i| i.bytes().len()).sum();
        if bytes_covered == 0 {
            continue;
        }
        out.push(ArchGuess {
            label,
            instructions: insns.len(),
            bytes_covered,
            window_bytes: window_len,
        });
    }
    out.sort_by(|a, b| b.bytes_covered.cmp(&a.bytes_covered));
    out.truncate(keep_top);
    out
}

pub fn format_file_report(name_hint: Option<&str>, info: &FileInfo) -> String {
    let mut out = String::new();
    if let Some(n) = name_hint {
        out.push_str(&format!("; {n}\n"));
    }
    out.push_str(&format!(
        "size:   {} ({} байт)\n",
        human_size(info.size),
        info.size
    ));
    out.push_str(&format!("sha256: {}\n", info.sha256));
    if let Some(m) = &info.infer_mime {
        out.push_str(&format!("mime:   {m}\n"));
    }
    match (&info.format_short, &info.format_full) {
        (Some(s), Some(f)) if s != f => {
            out.push_str(&format!("format: {s} — {f}\n"));
        }
        (_, Some(f)) => out.push_str(&format!("format: {f}\n")),
        _ => {}
    }
    if let Some(h) = &info.header_arch {
        out.push_str(&format!(
            "\n{} {}-bit {}-endian, arch={}, entry=0x{:x}, sections={}\n",
            h.format, h.bits, h.endian, h.arch, h.entry, h.sections
        ));
    } else if !info.arch_guesses.is_empty() {
        out.push_str("\narch (heuristic, top по покрытию):\n");
        for g in &info.arch_guesses {
            let pct = (g.bytes_covered as f64 / g.window_bytes as f64) * 100.0;
            out.push_str(&format!(
                "  {:<14} {:>6.2}% ({} инстр в {} байт окне)\n",
                g.label, pct, g.instructions, g.window_bytes
            ));
        }
    }
    if info.binwalk_findings.is_empty() {
        out.push_str("\nbinwalk: ничего вложенного не нашла\n");
    } else {
        out.push_str(&format!(
            "\nbinwalk: {}{} находок\n",
            info.binwalk_findings.len(),
            if info.binwalk_truncated { "+" } else { "" }
        ));
        for f in &info.binwalk_findings {
            out.push_str(&format!(
                "  +0x{:08x} [{}] {}\n",
                f.offset, f.name, f.description
            ));
        }
        if info.binwalk_truncated {
            out.push_str(&format!(
                "  … (всего больше {}; /binwalk покажет полный список)\n",
                BINWALK_FINDING_CAP
            ));
        }
    }
    out
}

pub fn format_binwalk_full(name_hint: Option<&str>, data: &[u8]) -> String {
    let binwalker = binwalk::Binwalk::new();
    let results = binwalker.scan(data);
    let mut out = String::new();
    if let Some(n) = name_hint {
        out.push_str(&format!("; binwalk {n}\n"));
    }
    if results.is_empty() {
        out.push_str("ничего вложенного не нашла\n");
        return out;
    }
    out.push_str(&format!("{} сигнатур:\n", results.len()));
    for r in results.iter().take(120) {
        out.push_str(&format!(
            "  +0x{:08x} [{}] {}\n",
            r.offset as u64, r.name, r.description
        ));
    }
    if results.len() > 120 {
        out.push_str(&format!("  … (ещё {})\n", results.len() - 120));
    }
    out
}

/// EXIF / metadata extraction. Flat dump in IFD order — readers decide
/// for themselves what matters; the bot doesn't moralize.
pub fn format_exif(name_hint: Option<&str>, data: &[u8]) -> String {
    use exif::Reader;
    let mut out = String::new();
    if let Some(n) = name_hint {
        out.push_str(&format!("; exif {n}\n"));
    }
    let mut cursor = Cursor::new(data);
    let exif_data = match Reader::new().read_from_container(&mut cursor) {
        Ok(e) => e,
        Err(e) => {
            out.push_str(&format!("EXIF не найден: {e}\n"));
            return out;
        }
    };
    let mut count = 0usize;
    for field in exif_data.fields() {
        if count >= 80 {
            out.push_str("  … (обрезано)\n");
            break;
        }
        out.push_str(&format!(
            "  {} [{:?}]: {}\n",
            field.tag,
            field.ifd_num,
            field.display_value().with_unit(&exif_data)
        ));
        count += 1;
    }
    if count == 0 {
        out.push_str("  (пусто)\n");
    }
    out
}

fn human_size(bytes: u64) -> String {
    const UNITS: &[&str] = &["B", "KiB", "MiB", "GiB", "TiB"];
    let mut v = bytes as f64;
    let mut i = 0usize;
    while v >= 1024.0 && i + 1 < UNITS.len() {
        v /= 1024.0;
        i += 1;
    }
    if i == 0 {
        format!("{} {}", bytes, UNITS[0])
    } else {
        format!("{v:.2} {}", UNITS[i])
    }
}
