//! Text of legacy Word 97–2003 documents (.doc): the piece table in the
//! table stream maps character positions to 8-bit or UTF-16 text runs in
//! the WordDocument stream ([MS-DOC] 2.4.1).

use crate::error::{AppError, AppResult, ErrorCode};
use std::path::Path;

fn u16le(d: &[u8], at: usize) -> Option<u16> {
    Some(u16::from_le_bytes(d.get(at..at + 2)?.try_into().ok()?))
}

fn u32le(d: &[u8], at: usize) -> Option<u32> {
    Some(u32::from_le_bytes(d.get(at..at + 4)?.try_into().ok()?))
}

fn bad() -> AppError {
    AppError::new(ErrorCode::Unsupported, "Unsupported Word document")
}

/// Removes field codes (keeping results) and maps Word control characters.
fn clean(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len());
    // Per open field: true while still in the code part.
    let mut fields: Vec<bool> = Vec::new();
    for c in raw.chars() {
        match c {
            '\u{13}' => fields.push(true),
            '\u{14}' => {
                if let Some(f) = fields.last_mut() {
                    *f = false;
                }
            }
            '\u{15}' => {
                fields.pop();
            }
            _ if fields.last() == Some(&true) => {}
            '\r' | '\u{0B}' | '\u{0C}' => out.push('\n'),
            '\u{07}' => out.push('\t'),
            '\u{1E}' => out.push('-'),
            '\u{1F}' | '\u{01}' | '\u{08}' => {}
            c if (c as u32) < 0x20 && c != '\t' && c != '\n' => {}
            c => out.push(c),
        }
    }
    out
}

pub fn doc_text(path: &Path) -> AppResult<String> {
    let mut comp = super::compound::Compound::open(path).map_err(|e| AppError::io(e, path))?;
    let word = comp.stream("WordDocument").map_err(|e| AppError::io(e, path))?;
    if u16le(&word, 0) != Some(0xA5EC) {
        return Err(bad());
    }
    let flags = u16le(&word, 0x0A).ok_or_else(bad)?;
    if flags & 0x0100 != 0 {
        return Err(AppError::new(ErrorCode::Unsupported, "The document is encrypted"));
    }
    let table_name = if flags & 0x0200 != 0 { "1Table" } else { "0Table" };
    let ccp_text = u32le(&word, 0x4C).ok_or_else(bad)? as usize;
    let fc_clx = u32le(&word, 0x1A2).ok_or_else(bad)? as usize;
    let lcb_clx = u32le(&word, 0x1A6).ok_or_else(bad)? as usize;
    let table = comp.stream(table_name).map_err(|e| AppError::io(e, path))?;
    let clx = table.get(fc_clx..fc_clx + lcb_clx).ok_or_else(bad)?;

    // Skip Prc entries, then the Pcdt holds the piece table.
    let mut i = 0;
    while clx.get(i) == Some(&0x01) {
        let cb = u16le(clx, i + 1).ok_or_else(bad)? as usize;
        i += 3 + cb;
    }
    if clx.get(i) != Some(&0x02) {
        return Err(bad());
    }
    let lcb = u32le(clx, i + 1).ok_or_else(bad)? as usize;
    let plc = clx.get(i + 5..i + 5 + lcb).ok_or_else(bad)?;
    let n = lcb.saturating_sub(4) / 12;
    let pcds = &plc[(n + 1) * 4..];

    let mut text = String::new();
    for k in 0..n {
        let start = u32le(plc, k * 4).ok_or_else(bad)? as usize;
        if start >= ccp_text {
            break;
        }
        let end = (u32le(plc, (k + 1) * 4).ok_or_else(bad)? as usize).min(ccp_text);
        let count = end.saturating_sub(start);
        let fc_raw = u32le(pcds, k * 8 + 2).ok_or_else(bad)?;
        let fc = (fc_raw & 0x3FFF_FFFF) as usize;
        if fc_raw & 0x4000_0000 != 0 {
            let off = fc / 2;
            let bytes = word.get(off..off + count).ok_or_else(bad)?;
            text.push_str(&encoding_rs::WINDOWS_1252.decode_without_bom_handling(bytes).0);
        } else {
            let bytes = word.get(fc..fc + count * 2).ok_or_else(bad)?;
            let units: Vec<u16> = bytes.chunks_exact(2).map(|c| u16::from_le_bytes([c[0], c[1]])).collect();
            text.push_str(&String::from_utf16_lossy(&units));
        }
    }
    Ok(clean(&text))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    /// Builds a minimal Word 97 file: one UTF-16 piece and one 8-bit piece.
    fn make_doc(path: &Path) {
        let utf16 = "Příliš žluťoučký\r";
        let ansi = b"kun\x13 HYPERLINK x \x14odkaz\x15\r";
        let units: Vec<u16> = utf16.encode_utf16().collect();
        let mut word = vec![0u8; 0x800];
        word[0..2].copy_from_slice(&0xA5ECu16.to_le_bytes());
        let ccp = units.len() + ansi.len();
        word[0x4C..0x50].copy_from_slice(&(ccp as u32).to_le_bytes());
        let text_at = word.len();
        for u in &units {
            word.extend(u.to_le_bytes());
        }
        let ansi_at = word.len();
        word.extend_from_slice(ansi);
        // Piece table: CPs [0, n16, ccp], two PCDs.
        let mut plc = Vec::new();
        for cp in [0u32, units.len() as u32, ccp as u32] {
            plc.extend(cp.to_le_bytes());
        }
        for (fc, compressed) in [(text_at as u32, false), ((ansi_at * 2) as u32, true)] {
            plc.extend([0u8, 0]);
            plc.extend((fc | if compressed { 0x4000_0000 } else { 0 }).to_le_bytes());
            plc.extend([0u8, 0]);
        }
        let mut clx = vec![0x02];
        clx.extend((plc.len() as u32).to_le_bytes());
        clx.extend(plc);
        let fc_clx = 16u32;
        let mut table = vec![0u8; fc_clx as usize];
        table.extend(&clx);
        word[0x1A2..0x1A6].copy_from_slice(&fc_clx.to_le_bytes());
        word[0x1A6..0x1AA].copy_from_slice(&(clx.len() as u32).to_le_bytes());

        let mut comp = cfb::create(path).unwrap();
        comp.create_stream("/WordDocument").unwrap().write_all(&word).unwrap();
        comp.create_stream("/0Table").unwrap().write_all(&table).unwrap();
        comp.flush().unwrap();
    }

    #[test]
    fn reads_pieces_and_strips_fields() {
        let dir = std::env::temp_dir().join(format!("dualion-doc-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let p = dir.join("a.doc");
        make_doc(&p);
        assert_eq!(doc_text(&p).unwrap(), "Příliš žluťoučký\nkunodkaz\n");
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
