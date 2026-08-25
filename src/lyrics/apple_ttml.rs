// SPDX-FileCopyrightText: 2026 Miguel Rincon
// SPDX-License-Identifier: GPL-3.0-or-later

//! Bounded conversion of Apple Music TTML into Jamelade's native lyric model.

use anyhow::Result;
use xml::reader::{EventReader, ParserConfig, ParserConfig2, XmlEvent};

use super::{LINE_MAX, LINES_MAX, Line, Lyrics, MAX_TIMESTAMP_MS, Provider};

/// Parse the bounded TTML string returned by Apple Music.
///
/// `xml-rs` is a streaming parser and owns no URL resolver. We read only `tt`,
/// `p`, text and `begin`; styling, agents and Apple-specific word spans are
/// ignored. Remote XML therefore never reaches GTK or a browser.
pub(crate) fn parse(raw: &str) -> Result<Lyrics> {
    // Apple TTML needs no DTD. Rejecting one up front removes the only reason
    // to expand custom entities at all, even though xml-rs also carries its own
    // entity-expansion limits.
    if raw
        .as_bytes()
        .windows(b"<!DOCTYPE".len())
        .any(|window| window.eq_ignore_ascii_case(b"<!DOCTYPE"))
    {
        anyhow::bail!("Apple Music returned an unsupported lyrics document");
    }

    let base = ParserConfig::new()
        .trim_whitespace(false)
        .whitespace_to_characters(true)
        .cdata_to_characters(true)
        .ignore_comments(true)
        .coalesce_characters(true);
    let config = ParserConfig2::from(base)
        .allow_multiple_root_elements(false)
        .max_entity_expansion_length(LINE_MAX * LINES_MAX)
        .max_entity_expansion_depth(2)
        .max_attributes(64)
        .max_attribute_length(4 * 1024)
        .max_name_length(256)
        .max_data_length(2 * 1024 * 1024);
    let parser = EventReader::new_with_config(raw.as_bytes(), config);

    let mut timing_none = false;
    let mut paragraph: Option<(Option<u64>, String)> = None;
    let mut lines = Vec::new();

    for event in parser {
        let event = event.map_err(|_| anyhow::anyhow!("Apple Music returned malformed lyrics"))?;
        match event {
            XmlEvent::StartElement {
                name, attributes, ..
            } if name.local_name == "tt" => {
                timing_none = attributes.iter().any(|attribute| {
                    attribute.name.local_name == "timing"
                        && attribute.value.eq_ignore_ascii_case("none")
                });
            }
            XmlEvent::StartElement {
                name, attributes, ..
            } if name.local_name == "p" => {
                if paragraph.is_some() {
                    anyhow::bail!("Apple Music returned malformed lyrics");
                }
                let at_ms = attributes
                    .iter()
                    .find(|attribute| attribute.name.local_name == "begin")
                    .and_then(|attribute| parse_ttml_timestamp(&attribute.value));
                paragraph = Some((at_ms, String::new()));
            }
            XmlEvent::Characters(text) if paragraph.is_some() => {
                if let Some((_, copy)) = paragraph.as_mut()
                    && copy.len() < LINE_MAX.saturating_mul(8)
                {
                    let left = LINE_MAX.saturating_mul(8).saturating_sub(copy.len());
                    copy.extend(text.chars().take(left));
                }
            }
            XmlEvent::EndElement { name } if name.local_name == "p" => {
                let Some((at_ms, text)) = paragraph.take() else {
                    anyhow::bail!("Apple Music returned malformed lyrics");
                };
                let text = clean_line(&text);
                if !text.is_empty() {
                    lines.push(Line {
                        at_ms: (!timing_none).then_some(at_ms).flatten(),
                        text,
                    });
                }
                if lines.len() >= LINES_MAX {
                    break;
                }
            }
            _ => {}
        }
    }

    if paragraph.is_some() {
        anyhow::bail!("Apple Music returned malformed lyrics");
    }

    // A line-timed document is useful only when every displayed line has a
    // timestamp. Mixed state would make untimed lines vanish from navigation;
    // fall back to a complete plain lyric rather than claiming broken sync.
    let synced = !lines.is_empty() && lines.iter().all(|line| line.at_ms.is_some());
    if synced {
        lines.sort_by_key(|line| line.at_ms);
    } else {
        for line in &mut lines {
            line.at_ms = None;
        }
    }

    Ok(Lyrics {
        lines,
        synced,
        instrumental: false,
        source: Some(Provider::AppleMusic),
        ..Lyrics::default()
    })
}

fn clean_line(value: &str) -> String {
    value
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .chars()
        .take(LINE_MAX)
        .collect()
}

/// TTML commonly uses `hh:mm:ss.mmm`; a time offset ending in `s` is accepted
/// as well. Frame and tick expressions depend on document metadata Jamelade
/// does not consume, so rejecting them is safer than pretending they are time.
pub(super) fn parse_ttml_timestamp(raw: &str) -> Option<u64> {
    let raw = raw.trim();
    if let Some(seconds) = raw.strip_suffix('s') {
        let seconds: f64 = seconds.parse().ok()?;
        if !seconds.is_finite() || seconds < 0.0 {
            return None;
        }
        let millis = (seconds * 1_000.0).round();
        return (millis <= MAX_TIMESTAMP_MS as f64).then_some(millis as u64);
    }

    let parts: Vec<&str> = raw.split(':').collect();
    let (hours, minutes, seconds): (u64, u64, f64) = match parts.as_slice() {
        [minutes, seconds] => (0_u64, minutes.parse().ok()?, seconds.parse().ok()?),
        [hours, minutes, seconds] => (
            hours.parse().ok()?,
            minutes.parse().ok()?,
            seconds.parse().ok()?,
        ),
        _ => return None,
    };
    if !seconds.is_finite() || !(0.0..60.0).contains(&seconds) || minutes >= 60 {
        return None;
    }
    let millis = hours
        .checked_mul(3_600_000)?
        .checked_add(minutes.checked_mul(60_000)?)?
        .checked_add((seconds * 1_000.0).round() as u64)?;
    (millis <= MAX_TIMESTAMP_MS).then_some(millis)
}
