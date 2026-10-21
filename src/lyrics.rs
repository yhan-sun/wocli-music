use crate::model::{clean, Track};
use unicode_normalization::UnicodeNormalization;

#[derive(Debug, Clone, PartialEq)]
pub struct Line {
    pub time: f64,
    pub text: String,
}

pub fn parse(text: &str) -> Vec<Line> {
    let offset = text
        .lines()
        .find_map(|line| {
            let lower = line.to_ascii_lowercase();
            let start = lower.find("[offset:")? + 8;
            lower[start..]
                .split(']')
                .next()?
                .trim()
                .parse::<f64>()
                .ok()
                .filter(|v| v.is_finite())
                .map(|v| v / 1000.)
        })
        .unwrap_or(0.);
    let mut output = vec![];
    for raw in text.lines() {
        let mut line = raw.trim_start_matches('\u{feff}').trim_start();
        let mut times = vec![];
        while let Some(rest) = line.strip_prefix('[') {
            let Some(end) = rest.find(']') else {
                break;
            };
            if let Some((minute, second)) = rest[..end].split_once(':') {
                if let (Ok(m), Ok(s)) = (
                    minute.parse::<u32>(),
                    second.replacen(':', ".", 1).parse::<f64>(),
                ) {
                    if s.is_finite() && (0. ..60.).contains(&s) {
                        times.push((m as f64 * 60. + s + offset).max(0.));
                    }
                }
            }
            line = &rest[end + 1..];
        }
        let words = clean(line);
        for time in times {
            output.push(Line {
                time,
                text: words.clone(),
            });
        }
    }
    output.sort_by(|a, b| a.time.total_cmp(&b.time));
    output
}
pub fn current(lines: &[Line], position: f64, offset: i64) -> Option<usize> {
    lines
        .partition_point(|v| v.time <= position + offset as f64 / 1000.)
        .checked_sub(1)
}
pub fn title(value: &str) -> String {
    let normalized: String = value.nfkc().collect();
    let mut output = String::new();
    let mut rest = normalized.as_str();
    while let Some(start) = rest.find('(') {
        output.push_str(&rest[..start]);
        let Some(end) = rest[start..].find(')').map(|v| start + v) else {
            output.push_str(&rest[start..]);
            return clean(&output);
        };
        let annotation = rest[start + 1..end].trim().to_lowercase();
        if !["カバー", "cover", "翻唱", "explicit", "clean"]
            .iter()
            .any(|v| {
                annotation == *v
                    || annotation.starts_with(&format!("{v}:"))
                    || annotation == format!("{v} version")
            })
        {
            output.push_str(&rest[start..=end]);
        }
        rest = &rest[end + 1..];
    }
    output.push_str(rest);
    clean(&output)
}
fn normalized(value: &str) -> String {
    value
        .nfkc()
        .flat_map(char::to_lowercase)
        .filter(|v| !v.is_whitespace())
        .collect()
}
pub fn matches(track: &Track, candidate: &str, artists: &[String], duration: f64) -> bool {
    normalized(&title(&track.title)) == normalized(&title(candidate))
        && track.artist.split(['&', '/', '、', ',', '，']).any(|a| {
            artists
                .iter()
                .any(|b| !a.is_empty() && normalized(a) == normalized(b))
        })
        && (track.duration <= 0. || (duration > 0. && (duration - track.duration).abs() <= 8.))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn timestamps_offsets_and_duplicate_stamps() {
        let lines = parse("[offset:-500]\n[00:01.25][01:03.500]一行\n[ar:artist]\n[00:NaN]invalid");
        assert_eq!(
            lines.iter().map(|v| v.time).collect::<Vec<_>>(),
            vec![0.75, 63.]
        );
        assert_eq!(current(&lines, 0.2, 0), None);
        assert_eq!(current(&lines, 63., 0), Some(1));
    }
    #[test]
    fn match_versions_artist_and_duration() {
        let track = Track {
            title: "心做し（カバー）".into(),
            artist: "majiko".into(),
            duration: 273.,
            ..Default::default()
        };
        assert!(matches(&track, "心做し", &["majiko".into()], 273.5));
        assert!(!matches(&track, "心做し (Live)", &["majiko".into()], 273.));
        assert!(!matches(&track, "心做し", &["其他人".into()], 273.));
        assert!(!matches(&track, "心做し", &["majiko".into()], 248.));
    }
}
