//! Turns the API's finished-transcript text into txt, srt, vtt, markdown or JSON.
//!
//! The API key routes return a finished transcript only as text, one block per
//! paragraph:
//!
//! ```text
//! [00:00:01.240] Speaker 0:
//! First paragraph.
//!
//! [00:00:09.800] Speaker 1:
//! Second paragraph.
//! ```
//!
//! That gives a start time, a speaker and the words, but no end time, so a
//! paragraph ends where the next one starts.
use serde_json::{json, Value};

#[derive(Debug, Clone, Copy, PartialEq, Eq, clap::ValueEnum)]
pub enum Format {
    Txt,
    Srt,
    Vtt,
    Md,
    Json,
}

impl Format {
    pub const NAMES: [&'static str; 5] = ["txt", "srt", "vtt", "md", "json"];

    pub fn parse(name: &str) -> Option<Format> {
        match name {
            "txt" => Some(Format::Txt),
            "srt" => Some(Format::Srt),
            "vtt" => Some(Format::Vtt),
            "md" => Some(Format::Md),
            "json" => Some(Format::Json),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub struct FormatOptions {
    pub speakers: bool,
    pub timecodes: bool,
}

impl Default for FormatOptions {
    fn default() -> Self {
        FormatOptions {
            speakers: true,
            timecodes: true,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Paragraph {
    pub start: f64,
    pub end: f64,
    pub speaker: Option<String>,
    pub text: String,
}

/// Speaking rate used to give the last paragraph an end time.
const WORDS_PER_SECOND: f64 = 2.5;

/// `[HH:MM:SS.mmm] Speaker: ` → (seconds, speaker). `None` for any other line.
fn parse_header(line: &str) -> Option<(f64, Option<String>)> {
    let rest = line.strip_prefix('[')?;
    let (clock, after) = rest.split_once(']')?;
    let mut parts = clock.split(':');
    let hours: f64 = parts.next()?.parse().ok()?;
    let minutes: f64 = parts.next()?.parse().ok()?;
    let seconds: f64 = parts.next()?.parse().ok()?;
    // The API rounds seconds to three places, so 59.9996 arrives as "60.000".
    if parts.next().is_some() || !(0.0..60.0).contains(&minutes) || !(0.0..=60.0).contains(&seconds)
    {
        return None;
    }
    // Anything after the bracket that is not "name:" means this is a line of
    // speech that happens to start with a timecode, not a header.
    let label = after.trim();
    let speaker = match label.strip_suffix(':') {
        // A paragraph without a speaker arrives labelled "undefined" or "null".
        Some(name) if matches!(name.trim(), "" | "undefined" | "null") => None,
        Some(name) => Some(name.trim().to_string()),
        None if label.is_empty() => None,
        None => return None,
    };
    Some((hours * 3600.0 + minutes * 60.0 + seconds, speaker))
}

/// Parses the API's transcript text into paragraphs.
pub fn parse_transcript(text: &str) -> Vec<Paragraph> {
    let mut paragraphs: Vec<Paragraph> = Vec::new();
    let mut saw_header = false;

    for line in text.lines() {
        if let Some((start, speaker)) = parse_header(line) {
            saw_header = true;
            paragraphs.push(Paragraph {
                start,
                end: start,
                speaker,
                text: String::new(),
            });
        } else if saw_header {
            let current = paragraphs.last_mut().expect("a header was seen");
            if !line.trim().is_empty() {
                if !current.text.is_empty() {
                    current.text.push(' ');
                }
                current.text.push_str(line.trim());
            }
        }
    }

    if !saw_header {
        // Text without timecodes: keep the paragraph breaks, nothing else is known.
        return text
            .split("\n\n")
            .map(str::trim)
            .filter(|block| !block.is_empty())
            .map(|block| Paragraph {
                start: 0.0,
                end: 0.0,
                speaker: None,
                text: block.to_string(),
            })
            .collect();
    }

    paragraphs.retain(|p| !p.text.is_empty());
    for i in 0..paragraphs.len() {
        let estimated = paragraphs[i].start
            + (paragraphs[i].text.split_whitespace().count() as f64 / WORDS_PER_SECOND).max(1.0);
        paragraphs[i].end = match paragraphs.get(i + 1) {
            Some(next) if next.start > paragraphs[i].start => next.start,
            _ => estimated,
        };
    }
    paragraphs
}

/// Renders a finished project (as returned by `Client::get_project`).
pub fn format_project(project: &Value, format: Format, opts: FormatOptions) -> String {
    let text = project.get("text").and_then(Value::as_str).unwrap_or("");
    let paragraphs = parse_transcript(text);
    match format {
        Format::Txt => to_text(&paragraphs, opts),
        Format::Srt => to_srt(&paragraphs, opts),
        Format::Vtt => to_vtt(&paragraphs, opts),
        Format::Md => {
            let name = project
                .get("name")
                .and_then(Value::as_str)
                .filter(|n| !n.is_empty());
            to_markdown(name.unwrap_or("Transcript"), &paragraphs, opts)
        }
        Format::Json => to_json(project, &paragraphs, opts),
    }
}

fn to_text(paragraphs: &[Paragraph], o: FormatOptions) -> String {
    paragraphs
        .iter()
        .map(|p| {
            let mut parts: Vec<String> = Vec::new();
            if o.timecodes {
                parts.push(format!("[{}]", clock(p.start)));
            }
            if let Some(name) = p.speaker.as_ref().filter(|_| o.speakers) {
                parts.push(format!("{name}:"));
            }
            parts.push(p.text.clone());
            parts.join(" ")
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn to_markdown(title: &str, paragraphs: &[Paragraph], o: FormatOptions) -> String {
    let mut lines = vec![format!("# {title}"), String::new()];
    let mut last_speaker: Option<&str> = None;
    for p in paragraphs {
        if let Some(name) = p.speaker.as_deref().filter(|_| o.speakers) {
            if last_speaker != Some(name) {
                lines.push(String::new());
                lines.push(format!("**{name}**"));
                last_speaker = Some(name);
            }
        }
        if o.timecodes {
            lines.push(format!("{} `{}`", p.text, clock(p.start)));
        } else {
            lines.push(p.text.clone());
        }
    }
    lines.join("\n")
}

fn to_srt(paragraphs: &[Paragraph], o: FormatOptions) -> String {
    paragraphs
        .iter()
        .enumerate()
        .map(|(i, p)| {
            let text = match p.speaker.as_ref().filter(|_| o.speakers) {
                Some(name) => format!("{name}: {}", p.text),
                None => p.text.clone(),
            };
            format!(
                "{}\n{} --> {}\n{}\n",
                i + 1,
                stamp(p.start, ','),
                stamp(p.end, ','),
                text
            )
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn to_vtt(paragraphs: &[Paragraph], o: FormatOptions) -> String {
    let cues = paragraphs
        .iter()
        .map(|p| {
            let text = match p.speaker.as_ref().filter(|_| o.speakers) {
                Some(name) => format!("<v {name}>{}", p.text),
                None => p.text.clone(),
            };
            format!(
                "{} --> {}\n{}\n",
                stamp(p.start, '.'),
                stamp(p.end, '.'),
                text
            )
        })
        .collect::<Vec<_>>()
        .join("\n");
    format!("WEBVTT\n\n{cues}")
}

fn to_json(project: &Value, paragraphs: &[Paragraph], o: FormatOptions) -> String {
    let rows: Vec<Value> = paragraphs
        .iter()
        .map(|p| {
            let mut row = serde_json::Map::new();
            if o.speakers {
                row.insert("speaker".into(), json!(p.speaker));
            }
            if o.timecodes {
                row.insert("start".into(), json!(round_ms(p.start)));
                row.insert("end".into(), json!(round_ms(p.end)));
            }
            row.insert("text".into(), json!(p.text));
            Value::Object(row)
        })
        .collect();
    let field = |key: &str| project.get(key).cloned().unwrap_or(Value::Null);
    crate::output::pretty(&json!({
        "id": field("id"),
        "name": field("name"),
        "youtubeId": field("youtubeId"),
        "language": field("language"),
        "transcription": rows,
    }))
}

fn round_ms(seconds: f64) -> f64 {
    (seconds * 1000.0).round() / 1000.0
}

/// Seconds → `HH:MM:SS,mmm` (or with `.` for VTT).
fn stamp(seconds: f64, sep: char) -> String {
    let total_ms = (seconds.max(0.0) * 1000.0).round() as u64;
    let (s, ms) = (total_ms / 1000, total_ms % 1000);
    format!(
        "{:02}:{:02}:{:02}{sep}{ms:03}",
        s / 3600,
        (s % 3600) / 60,
        s % 60
    )
}

/// Seconds → `HH:MM:SS`.
fn clock(seconds: f64) -> String {
    let s = seconds.max(0.0).floor() as u64;
    format!("{:02}:{:02}:{:02}", s / 3600, (s % 3600) / 60, s % 60)
}

#[cfg(test)]
mod tests {
    use super::*;

    const TEXT: &str =
        "[00:00:00.000] Alice: \nHello world.\n\n[00:00:05.250] Speaker 1: \nHi there.";

    fn project() -> Value {
        json!({"id": "p1", "name": "Test Video", "youtubeId": "dQw4w9WgXcQ", "language": "en", "text": TEXT})
    }

    #[test]
    fn parses_start_speaker_and_text() {
        let p = parse_transcript(TEXT);
        assert_eq!(p.len(), 2);
        assert_eq!(p[0].speaker.as_deref(), Some("Alice"));
        assert_eq!(p[0].text, "Hello world.");
        assert_eq!(p[0].start, 0.0);
        assert_eq!(p[1].speaker.as_deref(), Some("Speaker 1"));
        assert_eq!(p[1].start, 5.25);
    }

    #[test]
    fn a_paragraph_ends_where_the_next_one_starts() {
        let p = parse_transcript(TEXT);
        assert_eq!(p[0].end, 5.25);
        // Last paragraph: two words at 2.5 words/s is under the 1 s minimum.
        assert_eq!(p[1].end, 6.25);
    }

    #[test]
    fn hours_and_long_paragraphs_are_handled() {
        let words = vec!["word"; 50].join(" ");
        let p = parse_transcript(&format!("[01:02:03.500] Speaker 0: \n{words}"));
        assert_eq!(p[0].start, 3723.5);
        assert_eq!(p[0].end, 3743.5);
    }

    #[test]
    fn a_header_without_a_speaker_is_accepted() {
        let p = parse_transcript("[00:00:02.000] \nNo speaker here.");
        assert_eq!(p.len(), 1);
        assert_eq!(p[0].speaker, None);
        assert_eq!(p[0].text, "No speaker here.");
    }

    #[test]
    fn speech_that_starts_with_a_bracket_is_not_a_header() {
        let p = parse_transcript(
            "[00:00:02.000] Speaker 0: \n[00:10:00] is when we meet\n[music] plays",
        );
        assert_eq!(p.len(), 1);
        assert_eq!(p[0].text, "[00:10:00] is when we meet [music] plays");
    }

    #[test]
    fn text_without_timecodes_keeps_its_paragraphs() {
        let p = parse_transcript("First one.\n\nSecond one.");
        assert_eq!(p.len(), 2);
        assert_eq!(p[1].text, "Second one.");
        assert_eq!(p[1].speaker, None);
    }

    /// The fixture was produced by the backend's own formatting code.
    #[test]
    fn parses_text_exactly_as_the_backend_writes_it() {
        let text = include_str!("../tests/fixtures/detail-text-timecode-speaker.txt");
        let p = parse_transcript(text);
        assert_eq!(p.len(), 5);
        assert_eq!(p[0].start, 1.24);
        assert_eq!(
            p[0].text,
            "Welcome back to the channel. Today we look at [00:10:00] exactly."
        );
        assert_eq!(p[0].end, 9.8);
        assert_eq!(p[1].speaker.as_deref(), Some("Speaker 1"));
        // "[00:00:60.000]" is 59.9996 s rounded up by the backend.
        assert_eq!(p[2].start, 60.0);
        assert_eq!(p[2].speaker.as_deref(), Some("Jodie Smith"));
        assert_eq!(p[2].text, "A name instead of a number: fine.");
        assert_eq!(p[3].start, 3725.5);
        assert_eq!(p[4].speaker, None);
        assert_eq!(p[4].text, "No speaker on this one.");

        let srt = to_srt(&p, FormatOptions::default());
        assert!(srt.contains("3\n00:01:00,000 --> 01:02:05,500\nJodie Smith: A name"));
        assert!(srt.contains("5\n01:02:11,000 --> 01:02:13,000\nNo speaker on this one.\n"));
    }

    #[test]
    fn empty_text_gives_no_paragraphs() {
        assert!(parse_transcript("").is_empty());
        assert_eq!(
            format_project(
                &json!({"text": null}),
                Format::Txt,
                FormatOptions::default()
            ),
            ""
        );
    }

    #[test]
    fn txt_has_timecodes_and_speaker_labels() {
        let out = format_project(&project(), Format::Txt, FormatOptions::default());
        assert_eq!(
            out,
            "[00:00:00] Alice: Hello world.\n[00:00:05] Speaker 1: Hi there."
        );
    }

    #[test]
    fn txt_omits_speakers_and_timecodes_on_request() {
        let out = format_project(
            &project(),
            Format::Txt,
            FormatOptions {
                speakers: false,
                timecodes: true,
            },
        );
        assert_eq!(out, "[00:00:00] Hello world.\n[00:00:05] Hi there.");
        let out = format_project(
            &project(),
            Format::Txt,
            FormatOptions {
                speakers: false,
                timecodes: false,
            },
        );
        assert_eq!(out, "Hello world.\nHi there.");
    }

    #[test]
    fn srt_has_sequence_numbers_and_comma_timestamps() {
        let out = format_project(&project(), Format::Srt, FormatOptions::default());
        assert_eq!(
            out,
            "1\n00:00:00,000 --> 00:00:05,250\nAlice: Hello world.\n\n2\n00:00:05,250 --> 00:00:06,250\nSpeaker 1: Hi there.\n"
        );
    }

    #[test]
    fn vtt_has_the_header_and_voice_tags() {
        let out = format_project(&project(), Format::Vtt, FormatOptions::default());
        assert_eq!(
            out,
            "WEBVTT\n\n00:00:00.000 --> 00:00:05.250\n<v Alice>Hello world.\n\n00:00:05.250 --> 00:00:06.250\n<v Speaker 1>Hi there.\n"
        );
    }

    #[test]
    fn markdown_has_the_title_and_speaker_headings() {
        let out = format_project(&project(), Format::Md, FormatOptions::default());
        assert_eq!(
            out,
            "# Test Video\n\n\n**Alice**\nHello world. `00:00:00`\n\n**Speaker 1**\nHi there. `00:00:05`"
        );
    }

    #[test]
    fn markdown_does_not_repeat_the_same_speaker() {
        let text = "[00:00:00.000] Speaker 0: \nOne.\n\n[00:00:03.000] Speaker 0: \nTwo.";
        let out = format_project(&json!({"text": text}), Format::Md, FormatOptions::default());
        assert_eq!(out.matches("**Speaker 0**").count(), 1);
        assert!(out.starts_with("# Transcript\n"));
    }

    #[test]
    fn json_is_parseable_and_keeps_project_fields() {
        let out = format_project(&project(), Format::Json, FormatOptions::default());
        let parsed: Value = serde_json::from_str(&out).unwrap();
        assert_eq!(parsed["id"], "p1");
        assert_eq!(parsed["language"], "en");
        assert_eq!(parsed["transcription"].as_array().unwrap().len(), 2);
        assert_eq!(parsed["transcription"][1]["start"], 5.25);
        assert_eq!(parsed["transcription"][1]["speaker"], "Speaker 1");
    }

    #[test]
    fn timestamps_do_not_lose_a_millisecond_to_float_rounding() {
        // 1.234 has no exact binary form; truncating would print ,233.
        assert_eq!(stamp(1.234, ','), "00:00:01,234");
        assert_eq!(stamp(3661.999, '.'), "01:01:01.999");
        assert_eq!(stamp(-3.0, ','), "00:00:00,000");
    }

    #[test]
    fn format_names_round_trip() {
        for name in Format::NAMES {
            assert!(Format::parse(name).is_some());
        }
        assert_eq!(Format::parse("docx"), None);
    }
}
