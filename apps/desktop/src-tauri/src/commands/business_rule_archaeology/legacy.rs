use super::adapter::{ArchaeologyAdapterInput, SourcePositionIndex};
use super::contracts::ArchaeologySourceSpan;
use crate::commands::structural_graph::types::{stable_graph_id, StructuralGraphCancellation};

pub(super) const MAX_LEGACY_LINE_BYTES: usize = 64 * 1024;
pub(super) const MAX_LEGACY_TOKENS: usize = 256;

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum LegacyFormat {
    Fixed,
    Free,
}

#[derive(Clone, Copy)]
pub(super) struct LegacyLine<'a> {
    pub number: u64,
    pub start: usize,
    pub end: usize,
    pub logical_start: usize,
    pub logical_end: usize,
    pub text: &'a str,
    pub indicator: Option<u8>,
}

impl<'a> LegacyLine<'a> {
    pub fn logical(self) -> &'a str {
        &self.text[self.logical_start - self.start..self.logical_end - self.start]
    }
    pub fn range(self) -> (usize, usize) {
        (self.start, self.end)
    }
}

#[rustfmt::skip]
pub(super) fn lines(source: &str, format: LegacyFormat) -> impl Iterator<Item = LegacyLine<'_>> {
    let mut byte = 0_usize;
    source.split_inclusive('\n').enumerate().map(move |(index, raw)| {
        let without_newline = raw.strip_suffix('\n').unwrap_or(raw);
        let text = without_newline.strip_suffix('\r').unwrap_or(without_newline);
        let start = byte;
        let end = start + text.len();
        byte += raw.len();
        let (logical_start, logical_end, indicator) = match format {
            LegacyFormat::Free => (start, end, None),
            LegacyFormat::Fixed => {
                let mut logical_start = text.len().min(7);
                while logical_start < text.len() && !text.is_char_boundary(logical_start) { logical_start += 1; }
                let mut logical_end = text.len().min(72);
                while logical_end > logical_start && !text.is_char_boundary(logical_end) { logical_end -= 1; }
                let indicator = if text.len() <= 6 {
                    None
                } else if text.is_char_boundary(6) && text.is_char_boundary(7) {
                    text.as_bytes().get(6).copied()
                } else {
                    Some(0xff)
                };
                (start + logical_start, start + logical_end, indicator)
            },
        };
        LegacyLine { number: index as u64 + 1, start, end, logical_start, logical_end, text, indicator }
    })
}

#[derive(Clone, Copy, Debug)]
pub(super) struct LegacyToken {
    pub start: usize,
    pub end: usize,
}

impl LegacyToken {
    pub fn text(self, source: &str) -> &str {
        &source[self.start..self.end]
    }
    pub fn is(self, source: &str, expected: &str) -> bool {
        self.text(source).eq_ignore_ascii_case(expected)
    }
}

/// Bounded single-line scanner shared by COBOL and Assembly fallbacks.
/// It keeps quoted literals whole and never uses regex/backtracking.
#[rustfmt::skip]
pub(super) fn tokens(source: &str, line: LegacyLine<'_>) -> Result<Vec<LegacyToken>, &'static str> {
    if line.end - line.start > MAX_LEGACY_LINE_BYTES {
        return Err("legacy source line exceeds the byte bound");
    }
    let bytes = source.as_bytes();
    let mut result = Vec::new();
    let mut cursor = line.logical_start;
    while cursor < line.logical_end {
        while cursor < line.logical_end && bytes[cursor].is_ascii_whitespace() { cursor += 1; }
        if cursor == line.logical_end { break; }
        let start = cursor;
        let byte = bytes[cursor];
        if matches!(byte, b'\'' | b'"') {
            cursor += 1;
            let mut closed = false;
            while cursor < line.logical_end {
                if bytes[cursor] == byte {
                    cursor += 1;
                    if cursor < line.logical_end && bytes[cursor] == byte { cursor += 1; continue; }
                    closed = true;
                    break;
                }
                cursor += 1;
            }
            if !closed { return Err("legacy quoted literal is unterminated"); }
        } else if matches!(byte, b'<' | b'>' | b'=') {
            cursor += 1;
            if cursor < line.logical_end && bytes[cursor] == b'=' { cursor += 1; }
        } else if matches!(byte, b'.' | b',' | b'(' | b')' | b'+' | b'*' | b'/') {
            cursor += 1;
        } else {
            cursor += 1;
            while cursor < line.logical_end && !bytes[cursor].is_ascii_whitespace()
                && !matches!(bytes[cursor], b'\'' | b'"' | b'<' | b'>' | b'=' | b'.' | b',' | b'(' | b')' | b'+' | b'*' | b'/') {
                cursor += 1;
            }
        }
        if result.len() == MAX_LEGACY_TOKENS { return Err("legacy source line exceeds the token bound"); }
        result.push(LegacyToken { start, end: cursor });
    }
    Ok(result)
}

#[rustfmt::skip]
pub(super) fn checked_span(
    input: &ArchaeologyAdapterInput<'_>, source: &str, parser_id: &str,
    range: (usize, usize), positions: &SourcePositionIndex,
) -> Result<ArchaeologySourceSpan, String> {
    if range.0 >= range.1 || range.1 > source.len() {
        return Err("Legacy adapter produced an invalid source range".to_string());
    }
    Ok(ArchaeologySourceSpan {
        span_id: archaeology_id("span", input, parser_id, &format!("{}\0{}", range.0, range.1)),
        source_unit_id: input.unit.identity.source_unit_id.clone(),
        revision_sha: input.unit.identity.revision_sha.clone(),
        start: positions.position(source, range.0).ok_or("Legacy span start is not a UTF-8 boundary")?,
        end: positions.position(source, range.1).ok_or("Legacy span end is not a UTF-8 boundary")?,
    })
}

#[rustfmt::skip]
pub(super) fn archaeology_id(kind: &str, input: &ArchaeologyAdapterInput<'_>, parser_id: &str, local: &str) -> String {
    stable_graph_id(&format!("archaeology-{kind}"), &format!(
        "{}\0{}\0{parser_id}\0{local}",
        input.unit.identity.repository_id, input.unit.identity.source_unit_id,
    ))
}

pub(super) fn check_cancelled(cancellation: &StructuralGraphCancellation) -> Result<(), String> {
    if cancellation.is_cancelled() {
        Err("Legacy archaeology adapter cancelled".to_string())
    } else {
        Ok(())
    }
}
