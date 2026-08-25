// SPDX-FileCopyrightText: 2026 Jamelade contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! Local playlist exports. No Apple credentials or library identifiers leave
//! the process; only the public metadata already visible on the playlist page
//! is written to the file the user explicitly chooses.

use crate::music::types::Track;

const MAX_TRACKS: usize = 1_000;
const MAX_TEXT_CHARS: usize = 512;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Format {
    M3u8,
    Csv,
    Json,
}

impl Format {
    pub const ALL: [Self; 3] = [Self::M3u8, Self::Csv, Self::Json];

    pub const fn label(self) -> &'static str {
        match self {
            Self::M3u8 => "M3U8 playlist",
            Self::Csv => "CSV table",
            Self::Json => "JSON data",
        }
    }

    pub const fn extension(self) -> &'static str {
        match self {
            Self::M3u8 => "m3u8",
            Self::Csv => "csv",
            Self::Json => "json",
        }
    }
}

/// A filename hint, never a path. GTK's save portal decides the destination.
pub fn suggested_name(title: &str, format: Format) -> String {
    let stem: String = title
        .chars()
        .map(|ch| {
            if ch.is_alphanumeric() || matches!(ch, ' ' | '-' | '_') {
                ch
            } else {
                '_'
            }
        })
        .take(80)
        .collect::<String>()
        .trim()
        .to_owned();
    let stem = if stem.is_empty() { "playlist" } else { &stem };
    format!("{stem}.{}", format.extension())
}

pub fn render(title: &str, tracks: &[Track], format: Format) -> anyhow::Result<Vec<u8>> {
    let text = match format {
        Format::M3u8 => render_m3u8(tracks),
        Format::Csv => render_csv(tracks),
        Format::Json => render_json(title, tracks)?,
    };
    Ok(text.into_bytes())
}

fn public_url(track: &Track) -> Option<String> {
    track
        .share_url
        .as_deref()
        .and_then(crate::apple_link::canonical)
}

fn bounded_text(value: &str) -> String {
    value
        .chars()
        .filter(|ch| !ch.is_control())
        .take(MAX_TEXT_CHARS)
        .collect()
}

fn render_m3u8(tracks: &[Track]) -> String {
    let mut out = String::from("#EXTM3U\n");
    for track in tracks.iter().take(MAX_TRACKS) {
        let seconds = track.duration_ms / 1_000;
        out.push_str(&format!(
            "#EXTINF:{seconds},{} - {}\n",
            m3u_text(&bounded_text(&track.artist)),
            m3u_text(&bounded_text(&track.title))
        ));
        if let Some(url) = public_url(track) {
            out.push_str(&url);
        } else {
            // An unresolved library upload has no public Apple Music URL. A
            // comment preserves the row without inventing a playable address.
            out.push_str("# No public Apple Music link available");
        }
        out.push('\n');
    }
    out
}

fn m3u_text(value: &str) -> String {
    value.replace(['\r', '\n'], " ")
}

fn csv_cell(value: &str) -> String {
    let mut value = bounded_text(value);
    // Spreadsheet applications can evaluate a quoted CSV cell beginning with
    // one of these characters. Treat remote Apple metadata as data, not a
    // formula, by prefixing the conventional literal marker.
    if matches!(
        value.trim_start().chars().next(),
        Some('=' | '+' | '-' | '@')
    ) {
        value.insert(0, '\'');
    }
    format!("\"{}\"", value.replace('"', "\"\""))
}

fn render_csv(tracks: &[Track]) -> String {
    let mut out = String::from("position,title,artist,album,duration_ms,apple_music_url\r\n");
    for (index, track) in tracks.iter().take(MAX_TRACKS).enumerate() {
        let url = public_url(track).unwrap_or_default();
        out.push_str(&format!(
            "{},{},{},{},{},{}\r\n",
            index + 1,
            csv_cell(&track.title),
            csv_cell(&track.artist),
            csv_cell(&track.album),
            track.duration_ms,
            csv_cell(&url),
        ));
    }
    out
}

#[derive(serde::Serialize)]
struct JsonExport {
    format: &'static str,
    title: String,
    tracks: Vec<JsonTrack>,
}

#[derive(serde::Serialize)]
struct JsonTrack {
    position: usize,
    title: String,
    artist: String,
    album: String,
    duration_ms: u64,
    apple_music_url: String,
}

fn render_json(title: &str, tracks: &[Track]) -> anyhow::Result<String> {
    let export = JsonExport {
        format: "jamelade-playlist-v1",
        title: bounded_text(title),
        tracks: tracks
            .iter()
            .take(MAX_TRACKS)
            .enumerate()
            .map(|(index, track)| JsonTrack {
                position: index + 1,
                title: bounded_text(&track.title),
                artist: bounded_text(&track.artist),
                album: bounded_text(&track.album),
                duration_ms: track.duration_ms,
                apple_music_url: public_url(track).unwrap_or_default(),
            })
            .collect(),
    };
    Ok(serde_json::to_string_pretty(&export)? + "\n")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::music::types::{Track, TrackId};

    fn track(title: &str, url: Option<&str>) -> Track {
        Track {
            date_added: String::new(),
            year: String::new(),
            favorite: false,
            in_library: false,
            library_id: None,
            id: TrackId("synthetic-id".into()),
            catalog_id: Some("1000000001".into()),
            title: title.into(),
            artist: "Example Artist".into(),
            album: "Example Album".into(),
            duration_ms: 123_456,
            track_number: 1,
            artwork: None,
            share_url: url.map(str::to_owned),
        }
    }

    #[test]
    fn formats_are_bounded_to_public_metadata() {
        let tracks = [track(
            "A, \"quoted\" song",
            Some("https://music.apple.com/x"),
        )];
        let csv = String::from_utf8(render("Example", &tracks, Format::Csv).unwrap()).unwrap();
        assert!(csv.contains("\"A, \"\"quoted\"\" song\""));
        assert!(csv.contains("https://music.apple.com/x"));
        assert!(!csv.contains("synthetic-id"));

        let json = String::from_utf8(render("Example", &tracks, Format::Json).unwrap()).unwrap();
        assert!(json.contains("jamelade-playlist-v1"));
        assert!(!json.contains("synthetic-id"));
    }

    #[test]
    fn m3u_keeps_unlinked_library_uploads_as_comments() {
        let out =
            String::from_utf8(render("Example", &[track("Upload", None)], Format::M3u8).unwrap())
                .unwrap();
        assert!(out.starts_with("#EXTM3U\n#EXTINF:123,"));
        assert!(out.contains("# No public Apple Music link available"));
    }

    #[test]
    fn filename_hints_never_contain_path_separators() {
        assert_eq!(
            suggested_name("../My/List", Format::Json),
            "___My_List.json"
        );
        assert_eq!(suggested_name("", Format::M3u8), "playlist.m3u8");
    }

    #[test]
    fn csv_never_turns_remote_metadata_into_a_formula() {
        let out = String::from_utf8(
            render(
                "Example",
                &[track("=HYPERLINK(\"bad\")", None)],
                Format::Csv,
            )
            .unwrap(),
        )
        .unwrap();
        assert!(out.contains("\"'=HYPERLINK(\"\"bad\"\")\""));
    }
}
