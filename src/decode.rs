//! Robust decoding of LaTeX source bytes.
//!
//! Real-world `.tex` files are not always UTF-8: older documents are
//! frequently Latin-1 / Windows-1252 (`\usepackage[latin1]{inputenc}`),
//! some editors write UTF-16 with a byte-order mark, and classic Mac files
//! use bare CR line endings. Everything is normalized here so the lexer only
//! ever sees UTF-8 text with `\n` line endings.

use std::io;
use std::path::Path;

/// Windows-1252 code points for bytes 0x80..=0x9F (undefined bytes map to
/// their C1 control equivalents, matching the WHATWG decoder).
const CP1252_HIGH: [char; 32] = [
    '\u{20AC}', '\u{0081}', '\u{201A}', '\u{0192}', '\u{201E}', '\u{2026}', '\u{2020}', '\u{2021}',
    '\u{02C6}', '\u{2030}', '\u{0160}', '\u{2039}', '\u{0152}', '\u{008D}', '\u{017D}', '\u{008F}',
    '\u{0090}', '\u{2018}', '\u{2019}', '\u{201C}', '\u{201D}', '\u{2022}', '\u{2013}', '\u{2014}',
    '\u{02DC}', '\u{2122}', '\u{0161}', '\u{203A}', '\u{0153}', '\u{009D}', '\u{017E}', '\u{0178}',
];

/// Decode raw bytes into normalized text. Never fails: every byte sequence
/// maps to *some* text.
pub fn decode_bytes(bytes: &[u8]) -> String {
    let text = if let Some(rest) = bytes.strip_prefix(&[0xEF, 0xBB, 0xBF]) {
        String::from_utf8_lossy(rest).into_owned()
    } else if let Some(rest) = bytes.strip_prefix(&[0xFF, 0xFE]) {
        decode_utf16(rest, u16::from_le_bytes)
    } else if let Some(rest) = bytes.strip_prefix(&[0xFE, 0xFF]) {
        decode_utf16(rest, u16::from_be_bytes)
    } else {
        match std::str::from_utf8(bytes) {
            Ok(text) => text.to_string(),
            Err(_) if looks_like_utf16(bytes) => {
                if bytes.first() == Some(&0) {
                    decode_utf16(bytes, u16::from_be_bytes)
                } else {
                    decode_utf16(bytes, u16::from_le_bytes)
                }
            }
            Err(_) => decode_cp1252(bytes),
        }
    };
    normalize_text(&text)
}

/// Read and decode a file.
pub fn read_file(path: &Path) -> io::Result<String> {
    std::fs::read(path).map(|bytes| decode_bytes(&bytes))
}

/// Normalize line endings (CRLF and bare CR to LF) and drop characters that
/// TeX ignores or that would corrupt Markdown output (NUL, DEL).
pub fn normalize_text(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut chars = text.chars().peekable();
    while let Some(ch) = chars.next() {
        match ch {
            '\r' => {
                if chars.peek() == Some(&'\n') {
                    chars.next();
                }
                out.push('\n');
            }
            '\u{0}' | '\u{7F}' | '\u{FEFF}' => {}
            // Form feeds (common in old sources as page separators) act as
            // whitespace
            '\u{0C}' | '\u{0B}' => out.push('\n'),
            _ => out.push(ch),
        }
    }
    out
}

fn decode_utf16(bytes: &[u8], from: fn([u8; 2]) -> u16) -> String {
    let units = bytes.chunks_exact(2).map(|pair| from([pair[0], pair[1]]));
    char::decode_utf16(units)
        .map(|r| r.unwrap_or(char::REPLACEMENT_CHARACTER))
        .collect()
}

/// BOM-less UTF-16 heuristic: ASCII text in UTF-16 has a NUL in every other
/// byte.
fn looks_like_utf16(bytes: &[u8]) -> bool {
    let sample = &bytes[..bytes.len().min(512)];
    if sample.len() < 4 {
        return false;
    }
    let nul_even = sample.iter().step_by(2).filter(|&&b| b == 0).count();
    let nul_odd = sample.iter().skip(1).step_by(2).filter(|&&b| b == 0).count();
    let half = sample.len() / 2;
    nul_even * 10 > half * 7 || nul_odd * 10 > half * 7
}

fn decode_cp1252(bytes: &[u8]) -> String {
    bytes
        .iter()
        .map(|&b| match b {
            0x80..=0x9F => CP1252_HIGH[(b - 0x80) as usize],
            _ => b as char,
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn utf8_passthrough() {
        assert_eq!(decode_bytes("héllo".as_bytes()), "héllo");
    }

    #[test]
    fn utf8_bom_stripped() {
        assert_eq!(decode_bytes(b"\xEF\xBB\xBFabc"), "abc");
    }

    #[test]
    fn latin1_fallback() {
        assert_eq!(decode_bytes(b"caf\xE9 \x93q\x94"), "café \u{201C}q\u{201D}");
    }

    #[test]
    fn utf16_with_bom() {
        let le: Vec<u8> = [0xFF, 0xFE].into_iter().chain("Grüß".encode_utf16().flat_map(|u| u.to_le_bytes())).collect();
        assert_eq!(decode_bytes(&le), "Grüß");
        let be: Vec<u8> = [0xFE, 0xFF].into_iter().chain("Grüß".encode_utf16().flat_map(|u| u.to_be_bytes())).collect();
        assert_eq!(decode_bytes(&be), "Grüß");
    }

    #[test]
    fn utf16_without_bom() {
        let le: Vec<u8> = "\\section{x}".encode_utf16().flat_map(|u| u.to_le_bytes()).collect();
        assert_eq!(decode_bytes(&le), "\\section{x}");
    }

    #[test]
    fn line_endings_normalized() {
        assert_eq!(decode_bytes(b"a\r\nb\rc\n"), "a\nb\nc\n");
    }
}
