//! Spreadsheet previews: Excel (xls, xlsx, xlsm, xlsb), OpenDocument (ods)
//! via calamine, and delimited text (csv, tsv) with delimiter and encoding
//! detection.

use crate::error::{AppError, AppResult, ErrorCode};
use calamine::{open_workbook_auto, Data, Reader};
use serde::Serialize;
use std::path::Path;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SheetPreview {
    pub sheets: Vec<String>,
    pub sheet: usize,
    pub rows: Vec<Vec<String>>,
    /// Row / column of the first cell (for headers "A", "1"…).
    pub start_row: u32,
    pub start_col: u32,
    pub total_rows: usize,
    pub total_cols: usize,
    pub truncated: bool,
}

const MAX_COLS: usize = 200;

fn excel_date(serial: f64) -> String {
    // Excel day 0 = 1899-12-30 (1900 date system).
    let days = serial.floor() as i64;
    let secs = ((serial - serial.floor()) * 86_400.0).round() as i64;
    let ms = (days - 25_569) * 86_400_000 + secs * 1000;
    let (y, m, d, h, mi, s) = crate::util::utc_parts(ms);
    if secs == 0 {
        format!("{y:04}-{m:02}-{d:02}")
    } else if days == 0 {
        format!("{h:02}:{mi:02}:{s:02}")
    } else {
        format!("{y:04}-{m:02}-{d:02} {h:02}:{mi:02}")
    }
}

fn cell(d: &Data) -> String {
    match d {
        Data::Empty => String::new(),
        Data::String(s) => s.clone(),
        Data::Int(i) => i.to_string(),
        Data::Float(f) => {
            if f.fract() == 0.0 && f.abs() < 1e15 {
                format!("{}", *f as i64)
            } else {
                let s = format!("{:.10}", f);
                s.trim_end_matches('0').trim_end_matches('.').to_string()
            }
        }
        Data::Bool(b) => if *b { "TRUE" } else { "FALSE" }.into(),
        Data::DateTime(dt) => {
            if dt.is_duration() {
                format!("{:.4}", dt.as_f64())
            } else {
                excel_date(dt.as_f64())
            }
        }
        Data::DateTimeIso(s) | Data::DurationIso(s) => s.clone(),
        Data::Error(e) => format!("#{e:?}"),
    }
}

fn trim(rows: &mut Vec<Vec<String>>) {
    while rows.last().is_some_and(|r| r.iter().all(String::is_empty)) {
        rows.pop();
    }
    let width = rows.iter().map(|r| r.iter().rposition(|c| !c.is_empty()).map(|i| i + 1).unwrap_or(0)).max().unwrap_or(0);
    for r in rows.iter_mut() {
        r.truncate(width);
    }
}

pub fn workbook(path: &Path, sheet: usize, max_rows: usize) -> AppResult<SheetPreview> {
    let ext = path.extension().map(|e| e.to_string_lossy().to_lowercase()).unwrap_or_default();
    if ext == "csv" || ext == "tsv" || ext == "tab" {
        return delimited(path, max_rows);
    }
    let mut wb = open_workbook_auto(path).map_err(|e| AppError::new(ErrorCode::Unsupported, e.to_string()))?;
    let sheets = wb.sheet_names();
    let idx = sheet.min(sheets.len().saturating_sub(1));
    let name = sheets.get(idx).cloned().ok_or_else(|| AppError::new(ErrorCode::NotFound, "The workbook has no sheets"))?;
    let range = wb.worksheet_range(&name).map_err(|e| AppError::new(ErrorCode::Unsupported, e.to_string()))?;
    let (total_rows, total_cols) = range.get_size();
    let (start_row, start_col) = range.start().unwrap_or((0, 0));
    let mut rows: Vec<Vec<String>> = range.rows().take(max_rows).map(|r| r.iter().take(MAX_COLS).map(cell).collect()).collect();
    trim(&mut rows);
    Ok(SheetPreview {
        sheets,
        sheet: idx,
        truncated: total_rows > rows.len() || total_cols > MAX_COLS,
        rows,
        start_row,
        start_col,
        total_rows,
        total_cols,
    })
}

/// Splits one CSV record honouring quotes ("a ""b"""; doubled quotes).
fn parse_delimited(text: &str, delim: char, max_rows: usize) -> (Vec<Vec<String>>, bool) {
    let mut rows = Vec::new();
    let mut row = Vec::new();
    let mut field = String::new();
    let mut quoted = false;
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        if quoted {
            if c == '"' {
                if chars.peek() == Some(&'"') {
                    field.push('"');
                    chars.next();
                } else {
                    quoted = false;
                }
            } else {
                field.push(c);
            }
            continue;
        }
        match c {
            '"' if field.is_empty() => quoted = true,
            c if c == delim => row.push(std::mem::take(&mut field)),
            '\r' => {}
            '\n' => {
                row.push(std::mem::take(&mut field));
                if row.len() > MAX_COLS {
                    row.truncate(MAX_COLS);
                }
                rows.push(std::mem::take(&mut row));
                if rows.len() >= max_rows {
                    return (rows, chars.peek().is_some());
                }
            }
            c => field.push(c),
        }
    }
    if !field.is_empty() || !row.is_empty() {
        row.push(field);
        rows.push(row);
    }
    (rows, false)
}

fn detect_delimiter(first_line: &str) -> char {
    let mut best = (',', 0usize);
    for d in [';', '\t', ',', '|'] {
        let mut n = 0;
        let mut quoted = false;
        for c in first_line.chars() {
            if c == '"' {
                quoted = !quoted;
            } else if c == d && !quoted {
                n += 1;
            }
        }
        if n > best.1 {
            best = (d, n);
        }
    }
    best.0
}

fn delimited(path: &Path, max_rows: usize) -> AppResult<SheetPreview> {
    let bytes = crate::viewer::read_bytes(path, 0, 32 * 1024 * 1024)?;
    let enc = crate::viewer::detect_encoding(&bytes[..bytes.len().min(64 * 1024)]);
    let enc = encoding_rs::Encoding::for_label(enc.as_bytes()).unwrap_or(encoding_rs::UTF_8);
    let text = enc.decode_with_bom_removal(&bytes).0;
    let delim = detect_delimiter(text.lines().next().unwrap_or(""));
    let (mut rows, more) = parse_delimited(&text, delim, max_rows);
    trim(&mut rows);
    let cols = rows.iter().map(Vec::len).max().unwrap_or(0);
    Ok(SheetPreview {
        sheets: Vec::new(),
        sheet: 0,
        total_rows: rows.len(),
        total_cols: cols,
        truncated: more,
        rows,
        start_row: 0,
        start_col: 0,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn csv_with_quotes_and_semicolons() {
        let (rows, more) = parse_delimited("jméno;cena\n\"Kůň; hnědý\";1 200\n\"a \"\"b\"\"\";3\n", ';', 100);
        assert!(!more);
        assert_eq!(rows[1], vec!["Kůň; hnědý", "1 200"]);
        assert_eq!(rows[2], vec!["a \"b\"", "3"]);
        assert_eq!(detect_delimiter("a;b;c"), ';');
        assert_eq!(detect_delimiter("a,b,\"c;d\""), ',');
    }

    #[test]
    fn excel_dates() {
        assert_eq!(excel_date(45292.0), "2024-01-01");
        assert_eq!(excel_date(45292.5), "2024-01-01 12:00");
        assert_eq!(cell(&Data::Float(3.0)), "3");
        assert_eq!(cell(&Data::Float(0.1)), "0.1");
    }
}
