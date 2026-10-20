//! Video container parsers: ISO BMFF (MP4/MOV/3GP), Matroska/WebM, AVI.
//! Only headers are read: duration, resolution, frame rate and codecs.

use super::FileMeta;
use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::Path;

pub fn video(path: &Path, ext: &str, m: &mut FileMeta) {
    let Ok(mut f) = File::open(path) else { return };
    let len = f.metadata().map(|md| md.len()).unwrap_or(0);
    let mut head = [0u8; 12];
    if f.read_exact(&mut head).is_err() {
        return;
    }
    let _ = f.seek(SeekFrom::Start(0));
    if &head[4..8] == b"ftyp" || matches!(&head[4..8], b"moov" | b"mdat" | b"wide" | b"free" | b"skip") {
        mp4(&mut f, len, m);
    } else if head[..4] == [0x1A, 0x45, 0xDF, 0xA3] {
        mkv(&mut f, m);
    } else if &head[..4] == b"RIFF" && &head[8..12] == b"AVI " {
        avi(&mut f, m);
    } else if ext == "mp4" || ext == "mov" {
        mp4(&mut f, len, m);
    }
    if let (Some(d), None) = (m.duration, m.bitrate) {
        if d > 0.0 {
            m.bitrate = Some((len as f64 * 8.0 / d / 1000.0).round() as u32);
        }
    }
}

pub fn codec_name(id: &str) -> String {
    let n = match id.trim_end_matches('\0').trim() {
        "avc1" | "avc3" | "H264" | "h264" | "X264" | "x264" | "V_MPEG4/ISO/AVC" => "H.264",
        "hvc1" | "hev1" | "HEVC" | "V_MPEGH/ISO/HEVC" => "HEVC",
        "av01" | "V_AV1" => "AV1",
        "vp09" | "V_VP9" => "VP9",
        "vp08" | "V_VP8" => "VP8",
        "mp4v" | "V_MPEG4/ISO/SP" | "V_MPEG4/ISO/ASP" | "FMP4" => "MPEG-4",
        "XVID" | "xvid" => "Xvid",
        "DIVX" | "divx" | "DX50" | "DIV3" => "DivX",
        "mp2v" | "V_MPEG2" | "mpg2" => "MPEG-2",
        "apcn" | "apch" | "apcs" | "apco" | "ap4h" | "ap4x" => "ProRes",
        "MJPG" | "mjpa" | "jpeg" | "V_MJPEG" => "MJPEG",
        "dvh1" | "dvhe" => "Dolby Vision",
        "mp4a" | "A_AAC" | "A_AAC/MPEG4/LC" | "A_AAC/MPEG2/LC" => "AAC",
        "ac-3" | "A_AC3" => "AC-3",
        "ec-3" | "A_EAC3" => "E-AC-3",
        "A_DTS" | "dtsc" | "dtsh" => "DTS",
        "Opus" | "A_OPUS" => "Opus",
        "A_VORBIS" => "Vorbis",
        "fLaC" | "A_FLAC" => "FLAC",
        "alac" | "A_ALAC" => "ALAC",
        ".mp3" | "A_MPEG/L3" => "MP3",
        "lpcm" | "sowt" | "twos" | "in24" | "ipcm" | "A_PCM/INT/LIT" | "A_PCM/INT/BIG" | "A_PCM/FLOAT/IEEE" => "PCM",
        "A_TRUEHD" | "mlpa" => "TrueHD",
        other => return other.to_string(),
    };
    n.to_string()
}

fn be16(b: &[u8], i: usize) -> u32 {
    b.get(i..i + 2).map(|s| u16::from_be_bytes([s[0], s[1]]) as u32).unwrap_or(0)
}
fn be32(b: &[u8], i: usize) -> u32 {
    b.get(i..i + 4).map(|s| u32::from_be_bytes([s[0], s[1], s[2], s[3]])).unwrap_or(0)
}
fn be64(b: &[u8], i: usize) -> u64 {
    b.get(i..i + 8).map(|s| u64::from_be_bytes(s.try_into().unwrap())).unwrap_or(0)
}
fn le16(b: &[u8], i: usize) -> u32 {
    b.get(i..i + 2).map(|s| u16::from_le_bytes([s[0], s[1]]) as u32).unwrap_or(0)
}
fn le32(b: &[u8], i: usize) -> u32 {
    b.get(i..i + 4).map(|s| u32::from_le_bytes([s[0], s[1], s[2], s[3]])).unwrap_or(0)
}

// ---------------------------------------------------------------- MP4 / MOV

/// Iterates boxes in `buf`: (type, body).
fn boxes(buf: &[u8]) -> impl Iterator<Item = (&[u8], &[u8])> {
    let mut i = 0usize;
    std::iter::from_fn(move || {
        if i + 8 > buf.len() {
            return None;
        }
        let mut size = be32(buf, i) as usize;
        let typ = &buf[i + 4..i + 8];
        let mut hdr = 8;
        if size == 1 {
            size = be64(buf, i + 8) as usize;
            hdr = 16;
        } else if size == 0 {
            size = buf.len() - i;
        }
        if size < hdr || i + size > buf.len() {
            // Truncated box: return what is there.
            let body = &buf[(i + hdr).min(buf.len())..];
            i = buf.len();
            return Some((typ, body));
        }
        let body = &buf[i + hdr..i + size];
        i += size;
        Some((typ, body))
    })
}

fn child<'a>(buf: &'a [u8], name: &[u8]) -> Option<&'a [u8]> {
    boxes(buf).find(|(t, _)| *t == name).map(|(_, b)| b)
}

fn mp4(f: &mut File, len: u64, m: &mut FileMeta) {
    // Find the top-level moov box (may sit at the end of the file).
    let mut pos = 0u64;
    let mut moov = None;
    let mut hdr = [0u8; 16];
    for _ in 0..64 {
        if pos + 8 > len || f.seek(SeekFrom::Start(pos)).is_err() || f.read_exact(&mut hdr[..8]).is_err() {
            break;
        }
        let mut size = be32(&hdr, 0) as u64;
        let mut h = 8;
        if size == 1 {
            if f.read_exact(&mut hdr[8..16]).is_err() {
                break;
            }
            size = be64(&hdr, 8);
            h = 16;
        } else if size == 0 {
            size = len - pos;
        }
        if size < h {
            break;
        }
        if &hdr[4..8] == b"moov" {
            let n = (size - h).min(64 << 20) as usize;
            let mut body = vec![0u8; n];
            if f.read_exact(&mut body).is_ok() {
                moov = Some(body);
            }
            break;
        }
        pos += size;
    }
    let Some(moov) = moov else { return };

    if let Some(mvhd) = child(&moov, b"mvhd") {
        let (ts, dur) = if mvhd.first() == Some(&1) { (be32(mvhd, 20), be64(mvhd, 24)) } else { (be32(mvhd, 12), be32(mvhd, 16) as u64) };
        if ts > 0 && dur > 0 {
            m.duration = Some((dur as f64 / ts as f64 * 100.0).round() / 100.0);
        }
    }
    for (t, trak) in boxes(&moov) {
        if t != b"trak" {
            continue;
        }
        let Some(mdia) = child(trak, b"mdia") else { continue };
        let handler = child(mdia, b"hdlr").and_then(|h| h.get(8..12)).unwrap_or(b"    ");
        let (ts, dur) = child(mdia, b"mdhd")
            .map(|h| if h.first() == Some(&1) { (be32(h, 20), be64(h, 24)) } else { (be32(h, 12), be32(h, 16) as u64) })
            .unwrap_or((0, 0));
        let stbl = child(mdia, b"minf").and_then(|x| child(x, b"stbl"));
        let codec = stbl.and_then(|s| child(s, b"stsd")).and_then(|s| s.get(12..16)).map(|c| String::from_utf8_lossy(c).into_owned());
        match handler {
            b"vide" if m.video_codec.is_none() => {
                m.video_codec = codec.map(|c| codec_name(&c));
                if let Some(tkhd) = child(trak, b"tkhd") {
                    let off = if tkhd.first() == Some(&1) { 88 } else { 76 };
                    let (w, h) = (be32(tkhd, off) >> 16, be32(tkhd, off + 4) >> 16);
                    if w > 0 && h > 0 {
                        m.width = Some(w);
                        m.height = Some(h);
                    }
                }
                // Visual sample entry carries the coded size too.
                if m.width.is_none() {
                    if let Some(stsd) = stbl.and_then(|s| child(s, b"stsd")) {
                        let (w, h) = (be16(stsd, 8 + 32), be16(stsd, 8 + 34));
                        if w > 0 && h > 0 {
                            m.width = Some(w);
                            m.height = Some(h);
                        }
                    }
                }
                let samples: u64 = stbl
                    .and_then(|s| child(s, b"stts"))
                    .map(|stts| (0..be32(stts, 4) as usize).map(|i| be32(stts, 8 + i * 8) as u64).sum())
                    .unwrap_or(0);
                if samples > 0 && dur > 0 && ts > 0 {
                    m.fps = Some(((samples as f64 * ts as f64 / dur as f64) * 100.0).round() as f32 / 100.0);
                }
            }
            b"soun" if m.audio_codec.is_none() => {
                m.audio_codec = codec.map(|c| codec_name(&c));
                if let Some(stsd) = stbl.and_then(|s| child(s, b"stsd")) {
                    // Audio sample entry (starting at 8): v0/v1 channels at +24 and
                    // 16.16 rate at +32; QuickTime v2 has f64 rate at +40, u32 channels at +48.
                    let (ch, rate) = if be16(stsd, 8 + 16) == 2 {
                        let r = stsd.get(48..56).map(|b| f64::from_be_bytes(b.try_into().unwrap())).unwrap_or(0.0);
                        (be32(stsd, 8 + 48), if r.is_finite() && r > 0.0 && r < 1e7 { r as u32 } else { 0 })
                    } else {
                        (be16(stsd, 8 + 24), be32(stsd, 8 + 32) >> 16)
                    };
                    if (1..=32).contains(&ch) {
                        m.channels = Some(ch as u8);
                    }
                    if rate > 0 {
                        m.sample_rate = Some(rate);
                    }
                }
            }
            _ => {}
        }
    }
}

// ---------------------------------------------------------------- Matroska

fn vint(b: &[u8], i: &mut usize, keep_marker: bool) -> Option<u64> {
    let first = *b.get(*i)?;
    let n = first.leading_zeros() as usize + 1;
    if n > 8 || *i + n > b.len() {
        return None;
    }
    let mut v = if keep_marker { first as u64 } else { (first as u64) & (0xFF >> n) };
    for k in 1..n {
        v = (v << 8) | b[*i + k] as u64;
    }
    // All ones = unknown size.
    if !keep_marker && v == (1u64 << (7 * n)) - 1 {
        v = u64::MAX;
    }
    *i += n;
    Some(v)
}

fn ebml_float(b: &[u8]) -> f64 {
    match b.len() {
        4 => f32::from_be_bytes(b.try_into().unwrap()) as f64,
        8 => f64::from_be_bytes(b.try_into().unwrap()),
        _ => 0.0,
    }
}

fn ebml_uint(b: &[u8]) -> u64 {
    b.iter().take(8).fold(0, |a, &x| (a << 8) | x as u64)
}

/// Walks EBML elements in `b`, calling `f(id, body)`; master elements listed
/// in `masters` are entered instead of reported.
fn ebml_walk(b: &[u8], masters: &[u64], f: &mut dyn FnMut(u64, &[u8]) -> bool) -> bool {
    let mut i = 0;
    while i < b.len() {
        let Some(id) = vint(b, &mut i, true) else { return true };
        let Some(size) = vint(b, &mut i, false) else { return true };
        let end = if size == u64::MAX { b.len() } else { (i as u64 + size).min(b.len() as u64) as usize };
        let body = &b[i..end];
        if id == 0x1F43_B675 {
            return false; // Cluster: headers are over.
        }
        if masters.contains(&id) {
            if !ebml_walk(body, masters, f) {
                return false;
            }
        } else if !f(id, body) {
            return false;
        }
        i = end;
    }
    true
}

fn mkv(f: &mut File, m: &mut FileMeta) {
    let mut buf = Vec::new();
    if f.take(4 << 20).read_to_end(&mut buf).is_err() {
        return;
    }
    const SEGMENT: u64 = 0x1853_8067;
    const INFO: u64 = 0x1549_A966;
    const TRACKS: u64 = 0x1654_AE6B;
    const ENTRY: u64 = 0xAE;
    const VIDEO: u64 = 0xE0;
    const AUDIO: u64 = 0xE1;
    let mut scale = 1_000_000u64;
    let mut duration = 0.0f64;
    // Per track state, flushed when a new TrackEntry begins.
    #[derive(Default)]
    struct T {
        kind: u64,
        codec: String,
        w: u32,
        h: u32,
        rate: f64,
        ch: u64,
        frame_ns: u64,
    }
    let mut tracks: Vec<T> = Vec::new();
    ebml_walk(&buf, &[SEGMENT, INFO, TRACKS, VIDEO, AUDIO], &mut |id, body| {
        match id {
            0x2A_D7B1 => scale = ebml_uint(body).max(1),
            0x4489 => duration = ebml_float(body),
            ENTRY => {
                let mut t = T::default();
                ebml_walk(body, &[VIDEO, AUDIO], &mut |id, b| {
                    match id {
                        0x83 => t.kind = ebml_uint(b),
                        0x86 => t.codec = String::from_utf8_lossy(b).trim_end_matches('\0').to_string(),
                        0xB0 => t.w = ebml_uint(b) as u32,
                        0xBA => t.h = ebml_uint(b) as u32,
                        0xB5 => t.rate = ebml_float(b),
                        0x9F => t.ch = ebml_uint(b),
                        0x23_E383 => t.frame_ns = ebml_uint(b),
                        _ => {}
                    }
                    true
                });
                tracks.push(t);
            }
            _ => {}
        }
        true
    });
    if duration > 0.0 {
        m.duration = Some((duration * scale as f64 / 1e9 * 100.0).round() / 100.0);
    }
    if let Some(v) = tracks.iter().find(|t| t.kind == 1) {
        m.video_codec = Some(codec_name(&v.codec));
        if v.w > 0 && v.h > 0 {
            m.width = Some(v.w);
            m.height = Some(v.h);
        }
        if v.frame_ns > 0 {
            m.fps = Some((1e9 / v.frame_ns as f64 * 100.0).round() as f32 / 100.0);
        }
    }
    if let Some(a) = tracks.iter().find(|t| t.kind == 2) {
        m.audio_codec = Some(codec_name(&a.codec));
        if a.rate > 0.0 {
            m.sample_rate = Some(a.rate as u32);
        }
        if a.ch > 0 {
            m.channels = Some(a.ch.min(64) as u8);
        }
    }
}

// ---------------------------------------------------------------- AVI

fn avi(f: &mut File, m: &mut FileMeta) {
    let mut buf = Vec::new();
    if f.take(256 * 1024).read_to_end(&mut buf).is_err() {
        return;
    }
    let Some(avih) = buf.windows(4).position(|w| w == b"avih") else { return };
    let h = &buf[avih + 8..];
    let usec = le32(h, 0);
    let frames = le32(h, 16);
    let (w, hh) = (le32(h, 32), le32(h, 36));
    if usec > 0 {
        let fps = 1e6 / usec as f64;
        m.fps = Some((fps * 100.0).round() as f32 / 100.0);
        if frames > 0 {
            m.duration = Some((frames as f64 / fps * 100.0).round() / 100.0);
        }
    }
    if w > 0 && hh > 0 {
        m.width = Some(w);
        m.height = Some(hh);
    }
    // Stream headers: 'strh' + size + fccType + fccHandler; 'strf' follows.
    let mut i = 0;
    while let Some(p) = buf[i..].windows(4).position(|w| w == b"strh") {
        let s = i + p + 8;
        let typ = buf.get(s..s + 4).unwrap_or(b"    ");
        let strf = buf[s..].windows(4).position(|w| w == b"strf").map(|q| s + q + 8);
        if typ == b"vids" && m.video_codec.is_none() {
            // BITMAPINFOHEADER.biCompression at +16
            let fourcc = strf.and_then(|q| buf.get(q + 16..q + 20)).or_else(|| buf.get(s + 4..s + 8));
            m.video_codec = fourcc.map(|c| codec_name(&String::from_utf8_lossy(c)));
        } else if typ == b"auds" && m.audio_codec.is_none() {
            if let Some(q) = strf {
                // WAVEFORMATEX: wFormatTag, nChannels, nSamplesPerSec
                m.audio_codec = Some(
                    match le16(&buf, q) {
                        0x0001 => "PCM",
                        0x0055 => "MP3",
                        0x0050 => "MP2",
                        0x2000 => "AC-3",
                        0x2001 => "DTS",
                        0x00FF | 0x1610 => "AAC",
                        _ => "?",
                    }
                    .to_string(),
                );
                m.channels = Some(le16(&buf, q + 2).min(64) as u8);
                m.sample_rate = Some(le32(&buf, q + 4));
            }
        }
        i = s;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bx(typ: &[u8], body: &[u8]) -> Vec<u8> {
        let mut v = ((body.len() + 8) as u32).to_be_bytes().to_vec();
        v.extend_from_slice(typ);
        v.extend_from_slice(body);
        v
    }

    #[test]
    fn mp4_headers() {
        let mut mvhd = vec![0u8; 100];
        mvhd[12..16].copy_from_slice(&1000u32.to_be_bytes());
        mvhd[16..20].copy_from_slice(&90_500u32.to_be_bytes());
        let mut tkhd = vec![0u8; 84];
        tkhd[76..80].copy_from_slice(&(1920u32 << 16).to_be_bytes());
        tkhd[80..84].copy_from_slice(&(1080u32 << 16).to_be_bytes());
        let mut hdlr = vec![0u8; 24];
        hdlr[8..12].copy_from_slice(b"vide");
        let mut mdhd = vec![0u8; 24];
        mdhd[12..16].copy_from_slice(&25u32.to_be_bytes());
        mdhd[16..20].copy_from_slice(&2262u32.to_be_bytes());
        let mut stsd = vec![0u8; 16];
        stsd[12..16].copy_from_slice(b"hvc1");
        let mut stts = vec![0u8; 16];
        stts[4..8].copy_from_slice(&1u32.to_be_bytes());
        stts[8..12].copy_from_slice(&2262u32.to_be_bytes());
        let stbl = bx(b"stbl", &[bx(b"stsd", &stsd), bx(b"stts", &stts)].concat());
        let mdia = bx(b"mdia", &[bx(b"hdlr", &hdlr), bx(b"mdhd", &mdhd), bx(b"minf", &stbl)].concat());
        let trak = bx(b"trak", &[bx(b"tkhd", &tkhd), mdia].concat());
        let file = [bx(b"ftyp", b"isom\0\0\0\0"), bx(b"mdat", &[0; 32]), bx(b"moov", &[bx(b"mvhd", &mvhd), trak].concat())].concat();
        let p = std::env::temp_dir().join(format!("dualion-{}.mp4", uuid::Uuid::new_v4()));
        std::fs::write(&p, file).unwrap();
        let mut m = FileMeta::default();
        video(&p, "mp4", &mut m);
        std::fs::remove_file(&p).unwrap();
        assert_eq!(m.duration, Some(90.5));
        assert_eq!((m.width, m.height), (Some(1920), Some(1080)));
        assert_eq!(m.video_codec.as_deref(), Some("HEVC"));
        assert_eq!(m.fps, Some(25.0));
    }

    #[test]
    fn mkv_headers() {
        // Minimal EBML: header, Segment(unknown size){Info{scale,duration}, Tracks{Entry{type 1, codec, video{w,h}}}}, Cluster
        fn el(id: &[u8], body: &[u8]) -> Vec<u8> {
            let mut v = id.to_vec();
            v.push(0x80 | body.len() as u8);
            v.extend_from_slice(body);
            v
        }
        let info = el(&[0x15, 0x49, 0xA9, 0x66], &[el(&[0x2A, 0xD7, 0xB1], &[0x0F, 0x42, 0x40]), el(&[0x44, 0x89], &12_000.0f64.to_be_bytes())].concat());
        let vid = el(&[0xE0], &[el(&[0xB0], &[0x0F, 0x00]), el(&[0xBA], &[0x08, 0x70])].concat());
        let entry = el(&[0xAE], &[el(&[0x83], &[1]), el(&[0x86], b"V_VP9"), vid].concat());
        let tracks = el(&[0x16, 0x54, 0xAE, 0x6B], &entry);
        let mut seg = vec![0x18, 0x53, 0x80, 0x67, 0x01, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF];
        seg.extend(info);
        seg.extend(tracks);
        seg.extend([0x1F, 0x43, 0xB6, 0x75, 0x80]);
        let file = [el(&[0x1A, 0x45, 0xDF, 0xA3], &[]), seg].concat();
        let p = std::env::temp_dir().join(format!("dualion-{}.mkv", uuid::Uuid::new_v4()));
        std::fs::write(&p, file).unwrap();
        let mut m = FileMeta::default();
        video(&p, "mkv", &mut m);
        std::fs::remove_file(&p).unwrap();
        assert_eq!(m.duration, Some(12.0));
        assert_eq!((m.width, m.height), (Some(3840), Some(2160)));
        assert_eq!(m.video_codec.as_deref(), Some("VP9"));
    }
}
