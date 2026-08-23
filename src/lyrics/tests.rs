// SPDX-FileCopyrightText: 2026 Miguel Rincon
// SPDX-License-Identifier: GPL-3.0-or-later

use super::*;

fn candidate(title: &str, artist: &str, album: &str, duration: f64, synced: &str) -> WireLyrics {
    WireLyrics {
        track_name: title.into(),
        artist_name: artist.into(),
        album_name: album.into(),
        duration,
        instrumental: false,
        plain_lyrics: Some("plain".into()),
        synced_lyrics: Some(synced.into()),
    }
}

#[test]
fn a_query_needs_a_title_and_artist_and_is_bounded() {
    assert!(Query::new(Some("123"), "", "Artist", "Album", 1).is_none());
    assert!(Query::new(Some("123"), "Song", "", "Album", 1).is_none());
    let long = "x".repeat(TRACK_FIELD_MAX + 50);
    let query = Query::new(Some("123"), &long, "Artist", "Album", 1).unwrap();
    assert_eq!(query.title.chars().count(), TRACK_FIELD_MAX);
    assert_eq!(query.catalog_id.as_deref(), Some("123"));
}

#[test]
fn only_numeric_catalog_ids_reach_apple_lyrics() {
    let query = Query::new(Some("i.library-id"), "Song", "Artist", "Album", 1).unwrap();
    assert_eq!(query.catalog_id, None);
    let query = Query::new(Some("../123"), "Song", "Artist", "Album", 1).unwrap();
    assert_eq!(query.catalog_id, None);
    let query = Query::new(Some(" 1440857781 "), "Song", "Artist", "Album", 1).unwrap();
    assert_eq!(query.catalog_id.as_deref(), Some("1440857781"));
}

#[test]
fn every_provider_requires_its_own_opt_in() {
    assert!(!Providers::default().any());
    assert!(Providers { lrclib: true }.any());
}

#[test]
fn provider_results_keep_attribution_in_the_memory_cache_value() {
    let lrclib = lyrics_from_wire(&candidate("Song", "Artist", "Album", 120.0, "[00:01]Hello"));
    assert_eq!(lrclib.source, Some(Provider::Lrclib));
}

#[test]
fn apple_line_ttml_becomes_synchronized_native_lines() {
    let ttml = r#"<?xml version="1.0" encoding="UTF-8"?>
        <tt xmlns="http://www.w3.org/ns/ttml"
            xmlns:itunes="http://music.apple.com/lyric-ttml-internal"
            itunes:timing="Line">
          <body><div>
            <p begin="00:00:03.250" end="00:00:05.000">First line</p>
            <p begin="00:01:02.500" end="00:01:05.000"><span>Second </span><span>line</span></p>
          </div></body>
        </tt>"#;
    let lyrics = lyrics_from_apple_ttml(ttml).unwrap();
    assert_eq!(lyrics.source, Some(Provider::AppleMusic));
    assert!(lyrics.synced);
    assert_eq!(lyrics.lines.len(), 2);
    assert_eq!(lyrics.lines[0].at_ms, Some(3_250));
    assert_eq!(lyrics.lines[1].at_ms, Some(62_500));
    assert_eq!(lyrics.lines[1].text, "Second line");
}

#[test]
fn apple_none_timing_is_kept_as_complete_plain_lyrics() {
    let ttml = r#"<tt xmlns:itunes="urn:apple" itunes:timing="None">
        <body><div><p>One</p><p>Two</p></div></body></tt>"#;
    let lyrics = lyrics_from_apple_ttml(ttml).unwrap();
    assert!(!lyrics.synced);
    assert_eq!(lyrics.lines.len(), 2);
    assert!(lyrics.lines.iter().all(|line| line.at_ms.is_none()));
}

#[test]
fn malformed_or_partially_timed_apple_lyrics_never_claim_live_sync() {
    assert!(lyrics_from_apple_ttml("<tt><body><p>").is_err());
    assert!(
        lyrics_from_apple_ttml(
            r#"<!DOCTYPE tt [<!ENTITY words "private expansion">]><tt><p>&words;</p></tt>"#,
        )
        .is_err()
    );
    let lyrics = lyrics_from_apple_ttml(
        r#"<tt><body><p begin="00:01.0">Timed</p><p>Untimed</p></body></tt>"#,
    )
    .unwrap();
    assert!(!lyrics.synced);
    assert!(lyrics.lines.iter().all(|line| line.at_ms.is_none()));
}

#[test]
fn apple_ttml_timestamps_are_strict_and_bounded() {
    assert_eq!(parse_ttml_timestamp("00:01:02.345"), Some(62_345));
    assert_eq!(parse_ttml_timestamp("62.345s"), Some(62_345));
    assert_eq!(parse_ttml_timestamp("00:60.000"), None);
    assert_eq!(parse_ttml_timestamp("00:00:60.000"), None);
    assert_eq!(parse_ttml_timestamp("00:00:01:12"), None);
}

#[test]
fn synced_lyrics_parse_multiple_timestamps_and_sort() {
    let lines = parse_lrc("[ar:Someone]\n[01:02.50][00:10.25]Chorus\n[00:03.1]Opening\nmalformed");
    assert_eq!(lines.len(), 3);
    assert_eq!(lines[0].at_ms, Some(3_100));
    assert_eq!(lines[1].at_ms, Some(10_250));
    assert_eq!(lines[2].at_ms, Some(62_500));
    assert_eq!(lines[2].text, "Chorus");
}

#[test]
fn search_fallback_accepts_only_the_same_timed_recording() {
    let query = Query::new(None, "Live & Learn", "The Band", "Home", 180_000).unwrap();
    let candidates = vec![
        candidate("Other", "The Band", "Home", 180.0, "[00:01]wrong title"),
        candidate(
            "Live & Learn",
            "Other",
            "Home",
            180.0,
            "[00:01]wrong artist",
        ),
        candidate(
            "Live & Learn",
            "The Band",
            "Home",
            190.0,
            "[00:01]wrong cut",
        ),
        candidate("LIVE & LEARN", "THE BAND", "Single", 181.5, "[00:01]right"),
    ];

    let chosen = choose_synced(&query, candidates).unwrap();
    assert_eq!(chosen.album_name, "Single");
}

#[test]
fn search_fallback_prefers_the_same_album_then_closest_duration() {
    let query = Query::new(None, "Song", "Artist", "Album", 200_000).unwrap();
    let candidates = vec![
        candidate("Song", "Artist", "Single", 200.0, "[00:01]single"),
        candidate("Song", "Artist", "Album", 202.0, "[00:01]album"),
    ];

    let chosen = choose_synced(&query, candidates).unwrap();
    assert_eq!(chosen.album_name, "Album");
}

#[test]
fn malformed_synchronized_candidates_are_not_claimed_as_live() {
    let query = Query::new(None, "Song", "Artist", "Album", 200_000).unwrap();
    let candidates = vec![candidate(
        "Song",
        "Artist",
        "Album",
        200.0,
        "not timestamped",
    )];
    assert!(choose_synced(&query, candidates).is_none());
}

#[test]
fn malformed_or_absurd_timestamps_are_rejected() {
    assert_eq!(parse_timestamp("00:60.0"), None);
    assert_eq!(parse_timestamp("nope"), None);
    assert_eq!(parse_timestamp("99999:00"), None);
}

#[test]
fn track_metadata_is_encoded_as_data_not_as_url_syntax() {
    assert_eq!(
        urlencode("AC/DC & Sigur Rós"),
        "AC%2FDC%20%26%20Sigur%20R%C3%B3s"
    );
}

#[test]
fn plain_lyrics_are_capped_and_empty_lines_are_dropped() {
    let raw = (0..(LINES_MAX + 5))
        .map(|i| {
            if i == 2 {
                String::new()
            } else {
                format!("line {i}")
            }
        })
        .collect::<Vec<_>>()
        .join("\n");
    let lines = parse_plain(&raw);
    assert_eq!(lines.len(), LINES_MAX);
    assert!(lines.iter().all(|line| line.at_ms.is_none()));
}
