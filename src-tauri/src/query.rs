//! Query language for searching and filtering files by name and metadata.
//!
//! ```text
//! report type:pdf modified:2025            words match the name, key:value filters
//! type:video codec:hevc res>=4k duration>10m
//! (ext:jpg | ext:heic) camera:canon -taken:2020
//! size:10mb..1gb  "exact phrase"  *.tar.gz  artist:"Daft Punk"
//! ```
//! Terms are AND-ed; `|`/`OR` gives alternatives, `-`/`!`/`NOT` negates,
//! parentheses group. Keys have Czech aliases (`typ:`, `velikost:`, …).

use crate::error::{AppError, AppResult};
use crate::meta::{kind_of, FileMeta};
use crate::util::{local_naive_to_ms, local_parts, now_ms};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Field {
    Name,
    Ext,
    Type,
    Size,
    Modified,
    Width,
    Height,
    Res,
    Orient,
    Duration,
    Codec,
    VCodec,
    ACodec,
    Fps,
    Bitrate,
    Camera,
    Lens,
    Iso,
    Focal,
    Taken,
    Gps,
    Artist,
    Album,
    Title,
    Genre,
    Year,
    Pages,
    Author,
    Lines,
    Path,
    Tag,
    Content,
}

impl Field {
    fn parse(key: &str) -> Option<Field> {
        use Field::*;
        Some(match fold(key).as_str() {
            "name" | "nazev" | "jmeno" => Name,
            "ext" | "pripona" => Ext,
            "type" | "kind" | "typ" | "druh" => Type,
            "size" | "velikost" => Size,
            "modified" | "date" | "dm" | "mtime" | "zmeneno" | "datum" => Modified,
            "width" | "w" | "sirka" => Width,
            "height" | "h" | "vyska" => Height,
            "res" | "resolution" | "rozliseni" => Res,
            "orient" | "orientation" | "orientace" => Orient,
            "duration" | "length" | "len" | "delka" => Duration,
            "codec" | "kodek" => Codec,
            "vcodec" => VCodec,
            "acodec" => ACodec,
            "fps" => Fps,
            "bitrate" => Bitrate,
            "camera" | "fotoaparat" | "aparat" => Camera,
            "lens" | "objektiv" => Lens,
            "iso" => Iso,
            "focal" | "ohnisko" => Focal,
            "taken" | "porizeno" | "foceno" => Taken,
            "gps" => Gps,
            "artist" | "interpret" | "umelec" => Artist,
            "album" => Album,
            "title" | "titul" => Title,
            "genre" | "zanr" => Genre,
            "year" | "rok" => Year,
            "pages" | "stranky" | "stran" => Pages,
            "author" | "autor" => Author,
            "lines" | "radky" | "radku" => Lines,
            "path" | "in" | "cesta" | "v" => Path,
            "tag" | "stitek" => Tag,
            "content" | "text" | "obsah" => Content,
            _ => return None,
        })
    }

    fn needs_meta(self) -> bool {
        use Field::*;
        !matches!(self, Name | Ext | Type | Size | Modified | Path | Tag | Content)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Op {
    Is,
    Lt,
    Le,
    Gt,
    Ge,
}

#[derive(Debug, Clone)]
pub enum Value {
    /// Folded text; `glob` when it contains wildcards.
    Text { text: String, glob: Option<regex::Regex> },
    /// Inclusive numeric range (a single number is `a..=a`).
    Num(f64, f64),
    /// Time range [start, end) in ms.
    Time(i64, i64),
    List(Vec<String>),
    Bool(bool),
}

#[derive(Debug, Clone)]
pub enum Node {
    All(Vec<Node>),
    Any(Vec<Node>),
    Not(Box<Node>),
    Term { field: Field, op: Op, value: Value },
}

/// What a query is evaluated against.
pub trait Subject {
    fn name(&self) -> &str;
    fn path(&self) -> &str;
    fn is_dir(&self) -> bool;
    fn size(&self) -> u64;
    fn mtime(&self) -> i64;
    fn meta(&self) -> Option<&FileMeta>;
    fn tags(&self) -> Vec<String> {
        Vec::new()
    }
    fn contains_text(&self, _folded: &str) -> bool {
        false
    }
}

#[derive(Debug, Clone)]
pub struct Query {
    pub root: Node,
    pub needs_meta: bool,
    pub needs_content: bool,
    pub needs_tags: bool,
}

impl Query {
    pub fn parse(input: &str) -> AppResult<Query> {
        let tokens = tokenize(input)?;
        let mut p = Parser { t: &tokens, i: 0 };
        let root = p.or()?;
        if p.i < tokens.len() {
            return Err(AppError::invalid("Unbalanced parentheses in query"));
        }
        let mut q = Query { root, needs_meta: false, needs_content: false, needs_tags: false };
        q.scan(&q.root.clone());
        Ok(q)
    }

    fn scan(&mut self, n: &Node) {
        match n {
            Node::All(v) | Node::Any(v) => v.iter().for_each(|c| self.scan(c)),
            Node::Not(c) => self.scan(c),
            Node::Term { field, .. } => {
                self.needs_meta |= field.needs_meta();
                self.needs_content |= *field == Field::Content;
                self.needs_tags |= *field == Field::Tag;
            }
        }
    }

    pub fn matches(&self, s: &dyn Subject) -> bool {
        eval(&self.root, s)
    }
}

// ---------------------------------------------------------------- tokens

#[derive(Debug, Clone, PartialEq)]
enum Tok {
    Word { text: String, quoted: bool },
    Open,
    Close,
    Or,
    Not,
}

fn tokenize(s: &str) -> AppResult<Vec<Tok>> {
    let mut out = Vec::new();
    let mut chars = s.chars().peekable();
    while let Some(&c) = chars.peek() {
        if c.is_whitespace() {
            chars.next();
            continue;
        }
        match c {
            '(' => {
                chars.next();
                out.push(Tok::Open);
                continue;
            }
            ')' => {
                chars.next();
                out.push(Tok::Close);
                continue;
            }
            '|' => {
                chars.next();
                out.push(Tok::Or);
                continue;
            }
            '-' | '!' if out.last() != Some(&Tok::Not) => {
                chars.next();
                if chars.peek().is_some_and(|n| !n.is_whitespace()) {
                    out.push(Tok::Not);
                    continue;
                }
                out.push(Tok::Word { text: c.to_string(), quoted: false });
                continue;
            }
            _ => {}
        }
        let mut text = String::new();
        let mut quoted = false;
        while let Some(&c) = chars.peek() {
            if c == '"' {
                chars.next();
                quoted = true;
                let mut closed = false;
                for q in chars.by_ref() {
                    if q == '"' {
                        closed = true;
                        break;
                    }
                    text.push(q);
                }
                if !closed {
                    return Err(AppError::invalid("Missing closing quote in query"));
                }
                continue;
            }
            if c.is_whitespace() || c == '(' || c == ')' || c == '|' {
                break;
            }
            text.push(c);
            chars.next();
        }
        if !quoted && (text == "OR" || text == "NEBO") {
            out.push(Tok::Or);
        } else if !quoted && (text == "NOT" || text == "NE") {
            out.push(Tok::Not);
        } else if !quoted && (text == "AND" || text == "A") {
            // implicit
        } else {
            out.push(Tok::Word { text, quoted });
        }
    }
    Ok(out)
}

struct Parser<'a> {
    t: &'a [Tok],
    i: usize,
}

impl Parser<'_> {
    fn or(&mut self) -> AppResult<Node> {
        let mut alts = vec![self.and()?];
        while self.t.get(self.i) == Some(&Tok::Or) {
            self.i += 1;
            alts.push(self.and()?);
        }
        Ok(if alts.len() == 1 { alts.pop().unwrap() } else { Node::Any(alts) })
    }

    fn and(&mut self) -> AppResult<Node> {
        let mut items = Vec::new();
        while let Some(t) = self.t.get(self.i) {
            if matches!(t, Tok::Or | Tok::Close) {
                break;
            }
            items.push(self.unary()?);
        }
        Ok(if items.len() == 1 { items.pop().unwrap() } else { Node::All(items) })
    }

    fn unary(&mut self) -> AppResult<Node> {
        match self.t.get(self.i) {
            Some(Tok::Not) => {
                self.i += 1;
                Ok(Node::Not(Box::new(self.unary()?)))
            }
            Some(Tok::Open) => {
                self.i += 1;
                let n = self.or()?;
                if self.t.get(self.i) != Some(&Tok::Close) {
                    return Err(AppError::invalid("Missing ')' in query"));
                }
                self.i += 1;
                Ok(n)
            }
            Some(Tok::Word { text, quoted }) => {
                self.i += 1;
                term(text, *quoted)
            }
            _ => Err(AppError::invalid("Unexpected token in query")),
        }
    }
}

fn term(text: &str, quoted: bool) -> AppResult<Node> {
    if !quoted || text.contains(':') {
        if let Some((key, op, val)) = split_key(text) {
            if let Some(field) = Field::parse(key) {
                let value = parse_value(field, op, val)?;
                return Ok(Node::Term { field, op, value });
            }
        }
    }
    Ok(Node::Term { field: Field::Name, op: Op::Is, value: text_value(text, !quoted) })
}

fn split_key(s: &str) -> Option<(&str, Op, &str)> {
    let end = s.find(|c: char| !(c.is_alphanumeric() || c == '_'))?;
    if end == 0 {
        return None;
    }
    let rest = &s[end..];
    let (op, len) = if rest.starts_with(">=") {
        (Op::Ge, 2)
    } else if rest.starts_with("<=") {
        (Op::Le, 2)
    } else if rest.starts_with('>') {
        (Op::Gt, 1)
    } else if rest.starts_with('<') {
        (Op::Lt, 1)
    } else if rest.starts_with(':') || rest.starts_with('=') {
        // "size:>10mb" is accepted too.
        let r = &rest[1..];
        let (op, l) = if r.starts_with(">=") {
            (Op::Ge, 3)
        } else if r.starts_with("<=") {
            (Op::Le, 3)
        } else if r.starts_with('>') {
            (Op::Gt, 2)
        } else if r.starts_with('<') {
            (Op::Lt, 2)
        } else {
            (Op::Is, 1)
        };
        (op, l)
    } else {
        return None;
    };
    Some((&s[..end], op, &rest[len..]))
}

fn text_value(s: &str, allow_glob: bool) -> Value {
    let text = fold(s);
    let glob = (allow_glob && (text.contains('*') || text.contains('?'))).then(|| {
        let mut re = String::from("^");
        for c in text.chars() {
            match c {
                '*' => re.push_str(".*"),
                '?' => re.push('.'),
                c => re.push_str(&regex::escape(&c.to_string())),
            }
        }
        re.push('$');
        regex::Regex::new(&re).unwrap()
    });
    Value::Text { text, glob }
}

fn bad(field: Field, v: &str) -> AppError {
    AppError::invalid(format!("Invalid value for {field:?}: '{v}'").to_lowercase())
}

fn parse_value(field: Field, op: Op, v: &str) -> AppResult<Value> {
    use Field::*;
    let range = |parse: &dyn Fn(&str) -> Option<f64>| -> AppResult<Value> {
        if let Some((a, b)) = v.split_once("..") {
            let a = if a.is_empty() { f64::MIN } else { parse(a).ok_or_else(|| bad(field, v))? };
            let b = if b.is_empty() { f64::MAX } else { parse(b).ok_or_else(|| bad(field, v))? };
            return Ok(Value::Num(a, b));
        }
        let n = parse(v).ok_or_else(|| bad(field, v))?;
        Ok(Value::Num(n, n))
    };
    match field {
        Size => range(&parse_size),
        Duration => range(&parse_duration),
        Width | Height | Fps | Bitrate | Iso | Focal | Year | Pages | Lines => range(&|s| parse_number(s)),
        Res => range(&parse_res),
        Modified | Taken => {
            if let Some((a, b)) = v.split_once("..") {
                let s = if a.is_empty() { i64::MIN } else { parse_time(a).ok_or_else(|| bad(field, v))?.0 };
                let e = if b.is_empty() { i64::MAX } else { parse_time(b).ok_or_else(|| bad(field, v))?.1 };
                return Ok(Value::Time(s, e));
            }
            let (s, e) = parse_time(v).ok_or_else(|| bad(field, v))?;
            Ok(Value::Time(s, e))
        }
        Type => {
            let kinds: Vec<String> = v.split(',').map(|k| type_alias(&fold(k)).to_string()).filter(|k| !k.is_empty()).collect();
            if kinds.is_empty() {
                return Err(bad(field, v));
            }
            Ok(Value::List(kinds))
        }
        Ext => Ok(Value::List(v.split(',').map(|e| fold(e.trim_start_matches('.'))).filter(|e| !e.is_empty()).collect())),
        Gps => Ok(Value::Bool(!matches!(fold(v).as_str(), "no" | "ne" | "false" | "0" | "none"))),
        Orient => {
            let o = match fold(v).as_str() {
                "portrait" | "na vysku" | "vyska" | "p" => "portrait",
                "landscape" | "na sirku" | "sirka" | "l" => "landscape",
                "square" | "ctverec" | "s" => "square",
                _ => return Err(bad(field, v)),
            };
            Ok(Value::List(vec![o.into()]))
        }
        _ => {
            if op != Op::Is {
                return Err(bad(field, v));
            }
            Ok(text_value(v, true))
        }
    }
}

fn type_alias(k: &str) -> &str {
    match k {
        "image" | "images" | "img" | "photo" | "picture" | "obrazek" | "obrazky" | "foto" | "fotka" | "fotky" => "image",
        "raw" => "raw",
        "video" | "videa" | "movie" | "film" => "video",
        "audio" | "music" | "hudba" | "zvuk" | "song" => "audio",
        "doc" | "docs" | "document" | "dokument" | "dokumenty" => "document",
        "pdf" => "pdf",
        "sheet" | "spreadsheet" | "tabulka" | "excel" => "sheet",
        "slides" | "presentation" | "prezentace" => "slides",
        "archive" | "archiv" | "zip" => "archive",
        "code" | "kod" | "source" => "code",
        "text" | "txt" => "text",
        "font" | "pismo" => "font",
        "design" | "grafika" => "design",
        "folder" | "dir" | "directory" | "slozka" | "adresar" => "folder",
        "file" | "soubor" => "file",
        _ => "",
    }
}

fn parse_number(s: &str) -> Option<f64> {
    s.trim().replace(',', ".").parse().ok()
}

pub fn parse_size(s: &str) -> Option<f64> {
    let s = s.trim().to_lowercase().replace(',', ".");
    if s == "empty" || s == "prazdny" {
        return Some(0.0);
    }
    let split = s.find(|c: char| !(c.is_ascii_digit() || c == '.')).unwrap_or(s.len());
    let n: f64 = s[..split].parse().ok()?;
    let mult = match s[split..].trim() {
        "" | "b" => 1.0,
        "k" | "kb" | "kib" => 1024.0,
        "m" | "mb" | "mib" => 1024.0 * 1024.0,
        "g" | "gb" | "gib" => 1024.0 * 1024.0 * 1024.0,
        "t" | "tb" | "tib" => 1024f64.powi(4),
        _ => return None,
    };
    Some(n * mult)
}

pub fn parse_duration(s: &str) -> Option<f64> {
    let s = s.trim().to_lowercase();
    if s.contains(':') {
        let parts: Vec<f64> = s.split(':').map(|p| p.parse().ok()).collect::<Option<_>>()?;
        return Some(parts.iter().fold(0.0, |a, p| a * 60.0 + p));
    }
    let mut total = 0.0;
    let mut num = String::new();
    let mut any = false;
    for c in s.chars() {
        if c.is_ascii_digit() || c == '.' {
            num.push(c);
            continue;
        }
        let n: f64 = num.parse().ok()?;
        num.clear();
        total += n * match c {
            'h' => 3600.0,
            'm' => 60.0,
            's' => 1.0,
            _ => return None,
        };
        any = true;
    }
    if !num.is_empty() {
        total += num.parse::<f64>().ok()?;
        any = true;
    }
    any.then_some(total)
}

/// Resolution classes compare by the short side bucket (portrait 4K = 4K).
fn parse_res(s: &str) -> Option<f64> {
    let s = fold(s);
    Some(match s.as_str() {
        "sd" | "480" | "480p" => 480.0,
        "hd" | "720" | "720p" => 720.0,
        "fhd" | "fullhd" | "1080" | "1080p" => 1080.0,
        "2k" | "qhd" | "1440" | "1440p" => 1440.0,
        "4k" | "uhd" | "2160" | "2160p" => 2160.0,
        "8k" | "4320" | "4320p" => 4320.0,
        _ => return None,
    })
}

fn res_bucket(w: u32, h: u32) -> f64 {
    let short = w.min(h);
    match short {
        0..=699 => 480.0,
        700..=1049 => 720.0,
        1050..=1399 => 1080.0,
        1400..=2099 => 1440.0,
        2100..=4199 => 2160.0,
        _ => 4320.0,
    }
}

/// A time value to a [start, end) range in ms.
pub fn parse_time(s: &str) -> Option<(i64, i64)> {
    let now = now_ms();
    let day = 86_400_000i64;
    let (y, mo, d, ..) = local_parts(now);
    let today = local_naive_to_ms(y, mo, d, 0, 0, 0);
    let f = fold(s);
    let rel = |n: i64| Some((now - n, now + 1));
    match f.as_str() {
        "today" | "dnes" => return Some((today, today + day)),
        "yesterday" | "vcera" => return Some((today - day, today)),
        "week" | "tyden" => return rel(7 * day),
        "month" | "mesic" => return rel(30 * day),
        "year" | "rok" => return rel(365 * day),
        _ => {}
    }
    // Relative: 7d, 2w, 3m (months), 1y, 12h
    if let Some(unit) = f.chars().last().filter(|c| "hdwmy".contains(*c)) {
        if let Ok(n) = f[..f.len() - 1].parse::<i64>() {
            let span = match unit {
                'h' => 3_600_000,
                'd' => day,
                'w' => 7 * day,
                'm' => 30 * day,
                _ => 365 * day,
            };
            return rel(n * span);
        }
    }
    // 2025, 2025-03, 2025-03-14, 14.3.2025, 3.2025
    let nums: Vec<i64> = f.split(|c: char| !c.is_ascii_digit()).filter(|p| !p.is_empty()).filter_map(|p| p.parse().ok()).collect();
    let (yy, mm, dd) = if f.contains('.') {
        match nums[..] {
            [d, m, y] => (y, Some(m), Some(d)),
            [m, y] => (y, Some(m), None),
            _ => return None,
        }
    } else {
        match nums[..] {
            [y] => (y, None, None),
            [y, m] => (y, Some(m), None),
            [y, m, d] => (y, Some(m), Some(d)),
            _ => return None,
        }
    };
    if !(1900..=2200).contains(&yy) || mm.is_some_and(|m| !(1..=12).contains(&m)) || dd.is_some_and(|d| !(1..=31).contains(&d)) {
        return None;
    }
    let y = yy as i32;
    Some(match (mm, dd) {
        (None, _) => (local_naive_to_ms(y, 1, 1, 0, 0, 0), local_naive_to_ms(y + 1, 1, 1, 0, 0, 0)),
        (Some(m), None) => {
            let (ny, nm) = if m == 12 { (y + 1, 1) } else { (y, m as u32 + 1) };
            (local_naive_to_ms(y, m as u32, 1, 0, 0, 0), local_naive_to_ms(ny, nm, 1, 0, 0, 0))
        }
        (Some(m), Some(d)) => {
            let s = local_naive_to_ms(y, m as u32, d as u32, 0, 0, 0);
            (s, s + day)
        }
    })
}

// ---------------------------------------------------------------- folding

/// Lower case without diacritics ("Žluťoučký" -> "zlutoucky").
pub fn fold(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        if c.is_ascii() {
            out.push(c.to_ascii_lowercase());
            continue;
        }
        let base = match c {
            'á' | 'à' | 'â' | 'ä' | 'ã' | 'å' | 'ą' | 'ă' | 'Á' | 'À' | 'Â' | 'Ä' | 'Ã' | 'Å' | 'Ą' | 'Ă' => 'a',
            'č' | 'ć' | 'ç' | 'Č' | 'Ć' | 'Ç' => 'c',
            'ď' | 'đ' | 'Ď' | 'Đ' => 'd',
            'é' | 'è' | 'ê' | 'ë' | 'ě' | 'ę' | 'É' | 'È' | 'Ê' | 'Ë' | 'Ě' | 'Ę' => 'e',
            'í' | 'ì' | 'î' | 'ï' | 'Í' | 'Ì' | 'Î' | 'Ï' => 'i',
            'ľ' | 'ĺ' | 'ł' | 'Ľ' | 'Ĺ' | 'Ł' => 'l',
            'ň' | 'ń' | 'ñ' | 'Ň' | 'Ń' | 'Ñ' => 'n',
            'ó' | 'ò' | 'ô' | 'ö' | 'õ' | 'ő' | 'ø' | 'Ó' | 'Ò' | 'Ô' | 'Ö' | 'Õ' | 'Ő' | 'Ø' => 'o',
            'ř' | 'ŕ' | 'Ř' | 'Ŕ' => 'r',
            'š' | 'ś' | 'ş' | 'ș' | 'Š' | 'Ś' | 'Ş' | 'Ș' => 's',
            'ť' | 'ţ' | 'ț' | 'Ť' | 'Ţ' | 'Ț' => 't',
            'ú' | 'ù' | 'û' | 'ü' | 'ů' | 'ű' | 'Ú' | 'Ù' | 'Û' | 'Ü' | 'Ů' | 'Ű' => 'u',
            'ý' | 'ÿ' | 'Ý' => 'y',
            'ž' | 'ź' | 'ż' | 'Ž' | 'Ź' | 'Ż' => 'z',
            'ß' => {
                out.push_str("ss");
                continue;
            }
            c => {
                out.extend(c.to_lowercase());
                continue;
            }
        };
        out.push(base);
    }
    out
}

// ---------------------------------------------------------------- evaluation

fn eval(n: &Node, s: &dyn Subject) -> bool {
    match n {
        Node::All(v) => v.iter().all(|c| eval(c, s)),
        Node::Any(v) => v.iter().any(|c| eval(c, s)),
        Node::Not(c) => !eval(c, s),
        Node::Term { field, op, value } => term_matches(*field, *op, value, s),
    }
}

fn cmp_num(op: Op, x: f64, v: &Value) -> bool {
    let Value::Num(a, b) = *v else { return false };
    match op {
        Op::Is => x >= a && x <= b,
        Op::Lt => x < a,
        Op::Le => x <= b,
        Op::Gt => x > b,
        Op::Ge => x >= a,
    }
}

fn cmp_time(op: Op, t: i64, v: &Value) -> bool {
    let Value::Time(a, b) = *v else { return false };
    match op {
        Op::Is => t >= a && t < b,
        Op::Lt => t < a,
        Op::Le => t < b,
        Op::Gt => t >= b,
        Op::Ge => t >= a,
    }
}

fn text_matches(hay: &str, v: &Value) -> bool {
    let Value::Text { text, glob } = v else { return false };
    let h = fold(hay);
    match glob {
        Some(re) => re.is_match(&h),
        None => h.contains(text.as_str()),
    }
}

fn opt_text(x: Option<&String>, v: &Value) -> bool {
    x.is_some_and(|s| text_matches(s, v))
}

fn term_matches(field: Field, op: Op, v: &Value, s: &dyn Subject) -> bool {
    use Field::*;
    let ext = || {
        let n = s.name();
        if s.is_dir() {
            String::new()
        } else {
            n.rfind('.').filter(|&i| i > 0).map(|i| n[i + 1..].to_lowercase()).unwrap_or_default()
        }
    };
    let m = || s.meta();
    let num = |x: Option<f64>| x.is_some_and(|x| cmp_num(op, x, v));
    match field {
        Name => text_matches(s.name(), v),
        Path => text_matches(s.path(), v),
        Ext => matches!(v, Value::List(l) if l.contains(&ext())),
        Type => {
            let Value::List(kinds) = v else { return false };
            let k = kind_of(&ext(), s.is_dir());
            kinds.iter().any(|want| want == k || (want == "file" && !s.is_dir()) || (want == "image" && k == "raw") || (want == "document" && matches!(k, "pdf" | "text")))
        }
        Size => !s.is_dir() && cmp_num(op, s.size() as f64, v),
        Modified => cmp_time(op, s.mtime(), v),
        Taken => m().and_then(|m| m.taken).is_some_and(|t| cmp_time(op, t, v)),
        Width => num(m().and_then(|m| m.width).map(f64::from)),
        Height => num(m().and_then(|m| m.height).map(f64::from)),
        Res => num(m().and_then(|m| Some(res_bucket(m.width?, m.height?)))),
        Orient => {
            let Value::List(o) = v else { return false };
            m().and_then(|m| Some((m.width?, m.height?))).is_some_and(|(w, h)| {
                let k = if w > h {
                    "landscape"
                } else if h > w {
                    "portrait"
                } else {
                    "square"
                };
                o.iter().any(|x| x == k)
            })
        }
        Duration => num(m().and_then(|m| m.duration)),
        Fps => num(m().and_then(|m| m.fps).map(f64::from)),
        Bitrate => num(m().and_then(|m| m.bitrate).map(f64::from)),
        Iso => num(m().and_then(|m| m.iso).map(f64::from)),
        Focal => num(m().and_then(|m| m.focal).map(f64::from)),
        Year => num(m().and_then(|m| m.year.map(f64::from).or_else(|| m.taken.map(|t| local_parts(t).0 as f64)))),
        Pages => num(m().and_then(|m| m.pages).map(f64::from)),
        Lines => num(m().and_then(|m| m.lines).map(|l| l as f64)),
        Codec => m().is_some_and(|m| opt_text(m.video_codec.as_ref(), v) || opt_text(m.audio_codec.as_ref(), v)),
        VCodec => m().is_some_and(|m| opt_text(m.video_codec.as_ref(), v)),
        ACodec => m().is_some_and(|m| opt_text(m.audio_codec.as_ref(), v)),
        Camera => m().is_some_and(|m| opt_text(m.camera.as_ref(), v)),
        Lens => m().is_some_and(|m| opt_text(m.lens.as_ref(), v)),
        Artist => m().is_some_and(|m| opt_text(m.artist.as_ref(), v)),
        Album => m().is_some_and(|m| opt_text(m.album.as_ref(), v)),
        Title => m().is_some_and(|m| opt_text(m.title.as_ref(), v)),
        Genre => m().is_some_and(|m| opt_text(m.genre.as_ref(), v)),
        Author => m().is_some_and(|m| opt_text(m.author.as_ref(), v)),
        Gps => matches!(v, Value::Bool(want) if m().is_some_and(|m| m.gps.is_some()) == *want),
        Tag => s.tags().iter().any(|t| text_matches(t, v)),
        Content => matches!(v, Value::Text { text, .. } if s.contains_text(text)),
    }
}

/// A file on disk as a query subject, with metadata read on demand.
pub struct FileSubject {
    pub path: String,
    pub name: String,
    pub is_dir: bool,
    pub size: u64,
    pub mtime: i64,
    meta: std::cell::OnceCell<Option<FileMeta>>,
}

impl FileSubject {
    pub fn new(path: String, is_dir: bool, size: u64, mtime: i64) -> Self {
        let name = std::path::Path::new(&path).file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| path.clone());
        FileSubject { path, name, is_dir, size, mtime, meta: std::cell::OnceCell::new() }
    }
}

impl Subject for FileSubject {
    fn name(&self) -> &str {
        &self.name
    }
    fn path(&self) -> &str {
        &self.path
    }
    fn is_dir(&self) -> bool {
        self.is_dir
    }
    fn size(&self) -> u64 {
        self.size
    }
    fn mtime(&self) -> i64 {
        self.mtime
    }
    fn meta(&self) -> Option<&FileMeta> {
        self.meta
            .get_or_init(|| (!self.is_dir).then(|| crate::meta::get_known(std::path::Path::new(&self.path), self.size, self.mtime)))
            .as_ref()
    }
    fn tags(&self) -> Vec<String> {
        crate::tags::tags_of(&self.path)
    }
    fn contains_text(&self, folded: &str) -> bool {
        crate::meta::text_content(std::path::Path::new(&self.path)).is_some_and(|t| fold(&t).contains(folded))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct S {
        name: &'static str,
        size: u64,
        mtime: i64,
        meta: Option<FileMeta>,
    }
    impl Subject for S {
        fn name(&self) -> &str {
            self.name
        }
        fn path(&self) -> &str {
            self.name
        }
        fn is_dir(&self) -> bool {
            false
        }
        fn size(&self) -> u64 {
            self.size
        }
        fn mtime(&self) -> i64 {
            self.mtime
        }
        fn meta(&self) -> Option<&FileMeta> {
            self.meta.as_ref()
        }
    }

    fn video() -> S {
        S {
            name: "Dovolená Kréta.mp4",
            size: 3 << 30,
            mtime: local_naive_to_ms(2025, 7, 14, 10, 0, 0),
            meta: Some(FileMeta {
                width: Some(2160),
                height: Some(3840),
                duration: Some(754.0),
                video_codec: Some("HEVC".into()),
                ..Default::default()
            }),
        }
    }

    fn q(s: &str) -> bool {
        Query::parse(s).unwrap().matches(&video())
    }

    #[test]
    fn names_and_folding() {
        assert!(q("dovolena"));
        assert!(q("KRÉTA dovol"));
        assert!(q("*.mp4"));
        assert!(!q("*.mov"));
        assert!(q("\"dovolená kréta\""));
        assert!(!q("-kreta"));
    }

    #[test]
    fn fields() {
        assert!(q("type:video"));
        assert!(q("typ:videa ext:mp4,mov"));
        assert!(q("codec:hevc res:4k"));
        assert!(q("res>=1080p"));
        assert!(!q("res>4k"));
        assert!(q("orient:portrait"));
        assert!(q("duration>10m delka<13m"));
        assert!(q("size>2gb size:1g..4g"));
        assert!(q("velikost:>2gb"));
        assert!(q("modified:2025 zmeneno:7.2025 modified:14.7.2025"));
        assert!(q("modified>2025-07-13 modified<2025-08"));
        assert!(!q("modified>2025"));
        assert!(q("width:2000..3000"));
    }

    #[test]
    fn logic() {
        assert!(q("type:image | type:video"));
        assert!(q("(ext:mov OR ext:mp4) -codec:h264"));
        assert!(!q("type:audio OR (type:video codec:av1)"));
        assert!(q("NOT type:audio"));
        assert!(Query::parse("(a").is_err());
        assert!(Query::parse("size:abc").is_err());
        assert!(Query::parse("foo:bar").unwrap().matches(&S { name: "foo:bar", size: 0, mtime: 0, meta: None }));
    }

    #[test]
    fn values() {
        assert_eq!(parse_size("1.5k"), Some(1536.0));
        assert_eq!(parse_duration("1:30"), Some(90.0));
        assert_eq!(parse_duration("1h2m3s"), Some(3723.0));
        assert_eq!(fold("Příliš ŽLUŤOUČKÝ"), "prilis zlutoucky");
    }
}
