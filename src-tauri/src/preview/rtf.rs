//! Plain text from RTF: control words, groups, hex escapes in the document
//! code page (\ansicpgN) and \uN Unicode escapes.

const SKIP_DESTINATIONS: &[&str] = &[
    "fonttbl", "colortbl", "stylesheet", "info", "pict", "object", "header", "footer", "headerl", "headerr",
    "footerl", "footerr", "fldinst", "themedata", "colorschememapping", "latentstyles", "datastore", "xmlnstbl",
    "listtable", "listoverridetable", "rsidtbl", "generator", "filetbl", "revtbl", "pgdsctbl", "mmathPr",
];

#[derive(Clone, Copy)]
struct State {
    skip: bool,
    uc: usize,
}

fn decode_byte(b: u8, cp: u32) -> char {
    if b < 0x80 {
        return b as char;
    }
    let label = match cp {
        10000 => "macintosh".to_string(),
        cp => format!("windows-{cp}"),
    };
    let enc = encoding_rs::Encoding::for_label(label.as_bytes()).unwrap_or(encoding_rs::WINDOWS_1252);
    enc.decode_without_bom_handling(&[b]).0.chars().next().unwrap_or('?')
}

pub fn rtf_text(data: &[u8]) -> String {
    let mut out = String::new();
    let mut stack: Vec<State> = Vec::new();
    let mut st = State { skip: false, uc: 1 };
    let mut cp = 1252u32;
    let mut pending_skip = 0usize;
    let mut i = 0;
    let emit = |out: &mut String, st: &State, pending: &mut usize, c: char| {
        if *pending > 0 {
            *pending -= 1;
        } else if !st.skip {
            out.push(c);
        }
    };
    while i < data.len() {
        let b = data[i];
        match b {
            b'{' => {
                stack.push(st);
                i += 1;
            }
            b'}' => {
                st = stack.pop().unwrap_or(st);
                i += 1;
            }
            b'\r' | b'\n' => i += 1,
            b'\\' => {
                i += 1;
                let Some(&n) = data.get(i) else { break };
                if n.is_ascii_alphabetic() {
                    let start = i;
                    while i < data.len() && data[i].is_ascii_alphabetic() {
                        i += 1;
                    }
                    let word = std::str::from_utf8(&data[start..i]).unwrap_or("");
                    let pstart = i;
                    if data.get(i) == Some(&b'-') {
                        i += 1;
                    }
                    while i < data.len() && data[i].is_ascii_digit() {
                        i += 1;
                    }
                    let param: Option<i32> = std::str::from_utf8(&data[pstart..i]).ok().and_then(|s| s.parse().ok());
                    if data.get(i) == Some(&b' ') {
                        i += 1;
                    }
                    match word {
                        "par" | "line" | "sect" | "page" | "row" => emit(&mut out, &st, &mut 0, '\n'),
                        "tab" | "cell" => emit(&mut out, &st, &mut 0, '\t'),
                        "bullet" => emit(&mut out, &st, &mut pending_skip, '•'),
                        "emdash" => emit(&mut out, &st, &mut pending_skip, '—'),
                        "endash" => emit(&mut out, &st, &mut pending_skip, '–'),
                        "lquote" => emit(&mut out, &st, &mut pending_skip, '‘'),
                        "rquote" => emit(&mut out, &st, &mut pending_skip, '’'),
                        "ldblquote" => emit(&mut out, &st, &mut pending_skip, '“'),
                        "rdblquote" => emit(&mut out, &st, &mut pending_skip, '”'),
                        "ansicpg" => cp = param.unwrap_or(1252).max(0) as u32,
                        "uc" => st.uc = param.unwrap_or(1).max(0) as usize,
                        "u" => {
                            let v = param.unwrap_or(0);
                            let v = if v < 0 { v + 65536 } else { v } as u32;
                            if let Some(c) = char::from_u32(v) {
                                if !st.skip {
                                    out.push(c);
                                }
                            }
                            pending_skip = st.uc;
                        }
                        w if SKIP_DESTINATIONS.contains(&w) => st.skip = true,
                        _ => {}
                    }
                } else {
                    i += 1;
                    match n {
                        b'\'' => {
                            let hex = data.get(i..i + 2).and_then(|h| std::str::from_utf8(h).ok()).and_then(|h| u8::from_str_radix(h, 16).ok());
                            i += 2;
                            if let Some(byte) = hex {
                                emit(&mut out, &st, &mut pending_skip, decode_byte(byte, cp));
                            }
                        }
                        b'*' => st.skip = true,
                        // "\<newline>" is a paragraph break (macOS writes RTF this way).
                        b'\n' | b'\r' => emit(&mut out, &st, &mut 0, '\n'),
                        b'~' => emit(&mut out, &st, &mut pending_skip, '\u{a0}'),
                        b'_' => emit(&mut out, &st, &mut pending_skip, '-'),
                        b'{' | b'}' | b'\\' => emit(&mut out, &st, &mut pending_skip, n as char),
                        _ => {}
                    }
                }
            }
            _ => {
                emit(&mut out, &st, &mut pending_skip, decode_byte(b, cp));
                i += 1;
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decodes_rtf() {
        let rtf = br"{\rtf1\ansi\ansicpg1250{\fonttbl{\f0 Helvetica;}}{\*\generator Test;}\f0 P\'f8\'edli\'9a \u382?lu\u357?ou\u269?k\'fd\par
druh\'fd \{\}\tab x}";
        assert_eq!(rtf_text(rtf), "Příliš žluťoučký\ndruhý {}\tx");
        assert_eq!(rtf_text(b"{\\rtf1 a\\\nb}"), "a\nb");
    }
}
