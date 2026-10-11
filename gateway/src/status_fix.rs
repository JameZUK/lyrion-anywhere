//! Repairing a player's "now playing" when LMS's status contradicts itself.
//!
//! LMS's `status` leaves out a queue entry it can no longer resolve. That happens, for
//! example, when a rescan gave a queued track a new id. The entries after it then move up
//! one place and are numbered from there, while playback carries on with the real track. So
//! `status - 1` can name the *next* track as the current one (the wrong title, artwork,
//! length and waveform, though the top-level `duration` is still the playing track's), or,
//! when nothing follows it, no current track at all.
//!
//! When the two durations disagree, or the queue has a track at the current position that the
//! status doesn't list, the gateway asks LMS for the path at the current queue
//! position (`playlist path <i> ?`) and that track's details (`songinfo … url:<path>`), and
//! puts them in place of the wrong entry, before the response is filtered as usual (so paths
//! never reach the client). Any failure leaves the status as LMS sent it.

use serde_json::{Map, Value};

/// Durations closer than this (seconds) agree.
const TOLERANCE_S: f64 = 1.0;

fn number(v: &Value) -> Option<f64> {
    match v {
        Value::Number(n) => n.as_f64(),
        Value::String(s) => s.trim().parse().ok(),
        _ => None,
    }
    .filter(|f: &f64| f.is_finite())
}

/// For `status - …`, the current queue position when the first entry contradicts the
/// player's own duration, or is missing though the queue reaches that position (see the
/// module documentation); otherwise `None`.
pub(crate) fn needs_repair(cmd: &[String], result: &Value) -> Option<u64> {
    if cmd.first().map(String::as_str) != Some("status")
        || cmd.get(1).map(String::as_str) != Some("-")
    {
        return None;
    }
    let cur = number(&result["playlist_cur_index"])?;
    if cur < 0.0 || cur.fract() != 0.0 || cur > u32::MAX as f64 {
        return None;
    }
    let first = result["playlist_loop"].get(0).filter(|t| t.is_object());
    let Some(first) = first else {
        let tracks = number(&result["playlist_tracks"])?;
        return (tracks > cur).then_some(cur as u64);
    };
    let playing = number(&result["duration"])?;
    let listed = number(&first["duration"])?;
    ((playing - listed).abs() > TOLERANCE_S).then_some(cur as u64)
}

/// The `tags:` parameter of a command, if any.
pub(crate) fn tags(cmd: &[String]) -> Option<&str> {
    cmd.iter()
        .find(|a| a.starts_with("tags:"))
        .map(String::as_str)
}

/// The path LMS gave for `playlist path <i> ?`.
pub(crate) fn path_of(answer: &Value) -> Option<&str> {
    answer["_path"]
        .as_str()
        .filter(|p| !p.is_empty() && p.len() <= 4096)
}

/// A status-style track entry from a `songinfo` answer (a list of one-key objects), at queue
/// position `index`. `None` if LMS found nothing usable.
pub(crate) fn entry_from_songinfo(answer: &Value, index: u64) -> Option<Value> {
    let mut entry = Map::new();
    for item in answer["songinfo_loop"].as_array()?.iter().take(100) {
        if let Some(obj) = item.as_object() {
            for (k, v) in obj {
                entry.entry(k.clone()).or_insert_with(|| v.clone());
            }
        }
    }
    // Only a real track: it must have an id and a title.
    if entry.get("id").and_then(number).is_none()
        || !entry.get("title").is_some_and(Value::is_string)
    {
        return None;
    }
    entry.insert("playlist index".into(), Value::from(index));
    Some(Value::Object(entry))
}

/// Put `entry` in place of the first queue entry of a status result, or make it the only
/// entry when the status listed none.
pub(crate) fn replace_current(result: &mut Value, entry: Value) {
    let Some(map) = result.as_object_mut() else {
        return;
    };
    match map.get_mut("playlist_loop").and_then(Value::as_array_mut) {
        Some(list) if !list.is_empty() => list[0] = entry,
        _ => {
            map.insert("playlist_loop".into(), Value::Array(vec![entry]));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn cmd(parts: &[&str]) -> Vec<String> {
        parts.iter().map(|s| s.to_string()).collect()
    }

    fn status(duration: Value, listed: Value) -> Value {
        json!({"playlist_cur_index": "13", "duration": duration,
               "playlist_loop": [{"playlist index": 13, "id": 2, "title": "Next", "duration": listed}]})
    }

    #[test]
    fn a_status_whose_durations_disagree_is_repaired() {
        let c = cmd(&["status", "-", "1", "tags:adc"]);
        assert_eq!(
            needs_repair(&c, &status(json!(300.5), json!(95.25))),
            Some(13)
        );
    }

    #[test]
    fn agreeing_or_missing_durations_are_left_alone() {
        let c = cmd(&["status", "-", "1", "tags:adc"]);
        assert_eq!(needs_repair(&c, &status(json!(95.3), json!("95.25"))), None);
        // Radio and other streams: no duration at the top level.
        assert_eq!(needs_repair(&c, &status(Value::Null, json!(98.0))), None);
        assert_eq!(needs_repair(&c, &status(json!(300), Value::Null)), None);
        assert_eq!(
            needs_repair(
                &c,
                &json!({"duration": 300, "playlist_cur_index": 0, "playlist_tracks": 1,
                                     "playlist_loop": [{"title": "no duration"}]})
            ),
            None
        );
    }

    #[test]
    fn only_the_current_track_status_is_repaired() {
        let s = status(json!(300.0), json!(98.0));
        for c in [
            cmd(&["status", "0", "50"]),
            cmd(&["serverstatus", "-"]),
            cmd(&["status"]),
        ] {
            assert_eq!(needs_repair(&c, &s), None, "{c:?}");
        }
    }

    #[test]
    fn odd_positions_are_refused() {
        let c = cmd(&["status", "-", "1"]);
        for cur in [
            json!(-1),
            json!(1.5),
            json!("x"),
            json!(1e12),
            Value::Null,
            json!(f64::MAX),
        ] {
            let mut s = status(json!(300.0), json!(98.0));
            s["playlist_cur_index"] = cur.clone();
            assert_eq!(needs_repair(&c, &s), None, "{cur}");
        }
    }

    #[test]
    fn songinfo_becomes_a_status_entry() {
        let a = json!({"songinfo_loop": [
            {"id": 4021}, {"title": "Playing Track"}, {"artist": "Some Band"},
            {"duration": 300.5}, {"coverid": "abc"}, {"url": "file:///srv/music/x.flac"},
            {"title": "ignored duplicate"}, "junk", 7
        ]});
        let e = entry_from_songinfo(&a, 13).unwrap();
        assert_eq!(e["id"], 4021);
        assert_eq!(e["title"], "Playing Track");
        assert_eq!(e["playlist index"], 13);
        assert_eq!(
            e["url"], "file:///srv/music/x.flac",
            "filtered later with the rest"
        );
    }

    #[test]
    fn songinfo_without_a_track_is_refused() {
        for a in [
            json!({}),
            json!({"songinfo_loop": []}),
            json!({"songinfo_loop": [{"title": "x"}]}),
            json!({"songinfo_loop": [{"id": 3}]}),
            json!({"songinfo_loop": "nope"}),
            json!({"songinfo_loop": [{"id": "abc"}, {"title": "x"}]}),
        ] {
            assert!(entry_from_songinfo(&a, 0).is_none(), "{a}");
        }
    }

    #[test]
    fn replacing_swaps_the_first_entry_or_adds_the_only_one() {
        let mut s = status(json!(300.0), json!(98.0));
        replace_current(&mut s, json!({"id": 9}));
        assert_eq!(s["playlist_loop"][0]["id"], 9);
        for mut listed_none in [json!({"playlist_loop": []}), json!({"mode": "play"})] {
            replace_current(&mut listed_none, json!({"id": 9}));
            assert_eq!(listed_none["playlist_loop"], json!([{"id": 9}]));
        }
        let mut not_an_object = json!([1]);
        replace_current(&mut not_an_object, json!({"id": 9}));
        assert_eq!(not_an_object, json!([1]));
    }

    #[test]
    fn a_status_listing_no_current_track_is_repaired_when_the_queue_has_one() {
        let c = cmd(&["status", "-", "1", "tags:adc"]);
        // Regression: LMS listed nothing while the playing track sat at position 13 of 25.
        let s = json!({"mode": "play", "duration": 300.5, "playlist_cur_index": "13",
                       "playlist_tracks": 25});
        assert_eq!(needs_repair(&c, &s), Some(13));
        let s = json!({"playlist_cur_index": 2, "playlist_tracks": 3, "playlist_loop": []});
        assert_eq!(needs_repair(&c, &s), Some(2));
        // An empty queue, or a position past its end: nothing to look up.
        for s in [
            json!({"playlist_tracks": 0}),
            json!({"playlist_cur_index": 0, "playlist_tracks": 0}),
            json!({"playlist_cur_index": 3, "playlist_tracks": 3}),
            json!({"playlist_cur_index": 1}),
            json!({"playlist_cur_index": 0, "playlist_tracks": "x", "playlist_loop": [null]}),
        ] {
            assert_eq!(needs_repair(&c, &s), None, "{s}");
        }
    }

    #[test]
    fn paths_and_tags_are_read_carefully() {
        assert_eq!(
            path_of(&json!({"_path": "file:///a.flac"})),
            Some("file:///a.flac")
        );
        assert_eq!(path_of(&json!({"_path": ""})), None);
        assert_eq!(path_of(&json!({"_path": 5})), None);
        assert_eq!(path_of(&json!({"_path": "x".repeat(5000)})), None);
        assert_eq!(
            tags(&cmd(&["status", "-", "1", "tags:ad"])),
            Some("tags:ad")
        );
        assert_eq!(tags(&cmd(&["status", "-", "1"])), None);
    }

    #[test]
    fn arbitrary_status_answers_never_panic() {
        let c = cmd(&["status", "-", "1"]);
        let samples = [
            json!(null),
            json!([]),
            json!("s"),
            json!({"playlist_loop": 3}),
            json!({"playlist_loop": [null], "duration": 1, "playlist_cur_index": 0}),
            json!({"playlist_loop": [{"duration": {}}], "duration": [], "playlist_cur_index": {}}),
        ];
        for mut s in samples {
            let _ = needs_repair(&c, &s);
            replace_current(&mut s, json!({}));
            let _ = entry_from_songinfo(&s, 0);
            let _ = path_of(&s);
        }
    }
}
