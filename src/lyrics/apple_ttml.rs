// SPDX-FileCopyrightText: 2026 Miguel Rincon
// SPDX-License-Identifier: GPL-3.0-or-later

//! Bounded conversion of Apple Music TTML into Jamelade's native lyric model.

use std::collections::HashMap;

use anyhow::Result;
use xml::reader::{EventReader, ParserConfig, ParserConfig2, XmlEvent};

use super::{
    LINE_MAX, LINES_MAX, Line, LyricVariant, LyricVariantKind, Lyrics, MAX_TIMESTAMP_MS, Provider,
};

const KEY_MAX: usize = 256;
const VARIANTS_MAX: usize = 4;

struct LocalizedText {
    depth: usize,
    key: String,
    text: String,
}

struct LocalizationGroup {
    depth: usize,
    kind: LyricVariantKind,
    label: String,
    entries: HashMap<String, String>,
    text: Option<LocalizedText>,
}

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

    let parser = EventReader::new_with_config(raw.as_bytes(), parser_config());

    let mut timing_none = false;
    let mut in_body = false;
    let mut paragraph: Option<(Option<u64>, Option<String>, String)> = None;
    let mut lines = Vec::new();
    let mut line_keys = Vec::new();

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
            XmlEvent::StartElement { name, .. } if name.local_name == "body" => {
                if in_body {
                    anyhow::bail!("Apple Music returned malformed lyrics");
                }
                in_body = true;
            }
            XmlEvent::StartElement {
                name, attributes, ..
            } if in_body && name.local_name == "p" => {
                if paragraph.is_some() {
                    anyhow::bail!("Apple Music returned malformed lyrics");
                }
                let at_ms = attributes
                    .iter()
                    .find(|attribute| attribute.name.local_name == "begin")
                    .and_then(|attribute| parse_ttml_timestamp(&attribute.value));
                let key = attributes
                    .iter()
                    .find(|attribute| attribute.name.local_name == "key")
                    .and_then(|attribute| bounded_attribute(&attribute.value, KEY_MAX));
                paragraph = Some((at_ms, key, String::new()));
            }
            XmlEvent::Characters(text) if paragraph.is_some() => {
                if let Some((_, _, copy)) = paragraph.as_mut() {
                    append_bounded(copy, &text);
                }
            }
            XmlEvent::EndElement { name } if in_body && name.local_name == "p" => {
                let Some((at_ms, key, text)) = paragraph.take() else {
                    anyhow::bail!("Apple Music returned malformed lyrics");
                };
                let text = clean_line(&text);
                if !text.is_empty() && lines.len() < LINES_MAX {
                    lines.push(Line {
                        at_ms: (!timing_none).then_some(at_ms).flatten(),
                        text,
                    });
                    line_keys.push(key);
                }
            }
            XmlEvent::EndElement { name } if name.local_name == "body" => {
                if !in_body || paragraph.is_some() {
                    anyhow::bail!("Apple Music returned malformed lyrics");
                }
                in_body = false;
            }
            _ => {}
        }
    }

    if paragraph.is_some() || in_body {
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
    let variants = parse_localizations(raw, &lines, &line_keys)?;

    Ok(Lyrics {
        lines,
        synced,
        instrumental: false,
        source: Some(Provider::AppleMusic),
        variants,
    })
}

fn parser_config() -> ParserConfig2 {
    let base = ParserConfig::new()
        .trim_whitespace(false)
        .whitespace_to_characters(true)
        .cdata_to_characters(true)
        .ignore_comments(true)
        .coalesce_characters(true);
    ParserConfig2::from(base)
        .allow_multiple_root_elements(false)
        .max_entity_expansion_length(LINE_MAX * LINES_MAX)
        .max_entity_expansion_depth(2)
        .max_attributes(64)
        .max_attribute_length(4 * 1024)
        .max_name_length(256)
        .max_data_length(2 * 1024 * 1024)
}

/// Apple's current `syllable-lyrics` response stores translations and
/// transliterations under TTML metadata. Each localized entry points at the
/// `itunes:key` of a body paragraph. Keep those alternatives separate from
/// the song body and reuse the original timestamps.
fn parse_localizations(
    raw: &str,
    original: &[Line],
    line_keys: &[Option<String>],
) -> Result<Vec<LyricVariant>> {
    if original.is_empty()
        || line_keys.len() != original.len()
        || line_keys.iter().any(Option::is_none)
    {
        return Ok(Vec::new());
    }

    let mut known_keys = HashMap::with_capacity(line_keys.len());
    for (index, key) in line_keys.iter().enumerate() {
        let key = key.as_deref().expect("checked above");
        if known_keys.insert(key.to_owned(), index).is_some() {
            return Ok(Vec::new());
        }
    }

    let parser = EventReader::new_with_config(raw.as_bytes(), parser_config());
    let mut depth = 0usize;
    let mut section: Option<(LyricVariantKind, usize)> = None;
    let mut group: Option<LocalizationGroup> = None;
    let mut variants = Vec::new();

    for event in parser {
        let event = event.map_err(|_| anyhow::anyhow!("Apple Music returned malformed lyrics"))?;
        match event {
            XmlEvent::StartElement {
                name, attributes, ..
            } => {
                depth = depth.saturating_add(1);
                if let Some(group) = group.as_mut() {
                    if group.text.is_none()
                        && group.entries.len() < LINES_MAX
                        && let Some(key) = attributes
                            .iter()
                            .find(|attribute| attribute.name.local_name == "for")
                            .and_then(|attribute| bounded_attribute(&attribute.value, KEY_MAX))
                        && known_keys.contains_key(&key)
                    {
                        group.text = Some(LocalizedText {
                            depth,
                            key,
                            text: String::new(),
                        });
                    }
                } else if let Some((kind, section_depth)) = section {
                    if depth == section_depth.saturating_add(1) && variants.len() < VARIANTS_MAX {
                        group = Some(LocalizationGroup {
                            depth,
                            kind,
                            label: localization_label(kind, &attributes),
                            entries: HashMap::new(),
                            text: None,
                        });
                    }
                } else if let Some(kind) = localization_kind(&name.local_name) {
                    section = Some((kind, depth));
                }
            }
            XmlEvent::Characters(text) => {
                if let Some(text_state) = group.as_mut().and_then(|group| group.text.as_mut()) {
                    append_bounded(&mut text_state.text, &text);
                }
            }
            XmlEvent::EndElement { .. } => {
                if group
                    .as_ref()
                    .and_then(|group| group.text.as_ref())
                    .is_some_and(|text| text.depth == depth)
                    && let Some(text) = group.as_mut().and_then(|group| group.text.take())
                {
                    let value = clean_line(&text.text);
                    if !value.is_empty()
                        && let Some(group) = group.as_mut()
                    {
                        group.entries.entry(text.key).or_insert(value);
                    }
                }

                if group.as_ref().is_some_and(|group| group.depth == depth)
                    && let Some(finished) = group.take()
                    && let Some(variant) = localization_variant(finished, original, line_keys)
                    && !variants
                        .iter()
                        .any(|existing: &LyricVariant| existing.lines == variant.lines)
                {
                    variants.push(variant);
                }
                if section.is_some_and(|(_, section_depth)| section_depth == depth) {
                    section = None;
                }
                depth = depth.saturating_sub(1);
            }
            _ => {}
        }
    }

    Ok(variants)
}

fn localization_kind(name: &str) -> Option<LyricVariantKind> {
    if name.eq_ignore_ascii_case("translations") {
        Some(LyricVariantKind::Translation)
    } else if name.eq_ignore_ascii_case("transliterations") {
        Some(LyricVariantKind::Romanization)
    } else {
        None
    }
}

fn localization_label(
    kind: LyricVariantKind,
    attributes: &[xml::attribute::OwnedAttribute],
) -> String {
    let base = match kind {
        LyricVariantKind::Translation => "Translation",
        LyricVariantKind::Romanization => "Romanized",
    };
    let detail = attributes
        .iter()
        .find(|attribute| {
            matches!(
                attribute.name.local_name.as_str(),
                "lang" | "language" | "locale" | "name"
            )
        })
        .and_then(|attribute| bounded_attribute(&attribute.value, 48));
    detail
        .map(|detail| format!("{base} · {detail}"))
        .unwrap_or_else(|| base.to_owned())
}

fn localization_variant(
    group: LocalizationGroup,
    original: &[Line],
    line_keys: &[Option<String>],
) -> Option<LyricVariant> {
    let mut lines = Vec::with_capacity(original.len());
    for (line, key) in original.iter().zip(line_keys) {
        let text = group.entries.get(key.as_deref()?)?.clone();
        lines.push(Line {
            at_ms: line.at_ms,
            text,
        });
    }
    if lines == original {
        return None;
    }
    Some(LyricVariant {
        kind: group.kind,
        label: group.label,
        synced: !original.is_empty() && original.iter().all(|line| line.at_ms.is_some()),
        lines,
    })
}

fn bounded_attribute(value: &str, max: usize) -> Option<String> {
    let value = value
        .trim()
        .chars()
        .filter(|character| !character.is_control())
        .take(max)
        .collect::<String>();
    (!value.is_empty()).then_some(value)
}

fn append_bounded(target: &mut String, value: &str) {
    let limit = LINE_MAX.saturating_mul(8);
    if target.len() < limit {
        let left = limit.saturating_sub(target.len());
        target.extend(value.chars().take(left));
    }
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
