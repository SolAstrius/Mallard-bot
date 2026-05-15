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
}

#[derive(Debug, Clone)]
pub struct BinwalkFinding {
    pub offset: u64,
    pub name: String,
    pub description: String,
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

    FileInfo {
        size: data.len() as u64,
        sha256,
        infer_mime,
        format_short,
        format_full,
        binwalk_findings,
        binwalk_truncated,
    }
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
