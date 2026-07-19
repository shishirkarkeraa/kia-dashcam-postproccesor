use crate::TimestampSource;
use chrono::{DateTime, NaiveDate, NaiveDateTime, TimeZone, Utc};
use regex::Regex;
use std::collections::HashMap;
use std::path::Path;
use std::sync::LazyLock;
use std::time::SystemTime;

static COMPACT_TIMESTAMP: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?x)(?P<y>20\d{2})(?P<m>0[1-9]|1[0-2])(?P<d>0[1-9]|[12]\d|3[01])[^0-9]?(?P<h>[01]\d|2[0-3])(?P<min>[0-5]\d)(?P<s>[0-5]\d)").unwrap()
});

static SEPARATED_TIMESTAMP: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?x)(?P<y>20\d{2})[-_.](?P<m>0[1-9]|1[0-2])[-_.](?P<d>0[1-9]|[12]\d|3[01])(?:[T_-]|\s)+(?P<h>[01]\d|2[0-3])[-_.:](?P<min>[0-5]\d)[-_.:](?P<s>[0-5]\d)").unwrap()
});

pub(crate) fn choose_timestamp(
    format_tags: Option<&HashMap<String, String>>,
    video_tags: Option<&HashMap<String, String>>,
    path: &Path,
    modified: Option<SystemTime>,
) -> (Option<DateTime<Utc>>, Option<TimestampSource>) {
    if let Some(value) = find_creation_time(format_tags).and_then(parse_metadata_timestamp) {
        return (Some(value), Some(TimestampSource::ContainerMetadata));
    }
    if let Some(value) = find_creation_time(video_tags).and_then(parse_metadata_timestamp) {
        return (Some(value), Some(TimestampSource::VideoMetadata));
    }
    if let Some(value) = timestamp_from_filename(path) {
        return (Some(value), Some(TimestampSource::Filename));
    }
    if let Some(value) = modified.map(DateTime::<Utc>::from) {
        return (Some(value), Some(TimestampSource::ModifiedTime));
    }
    (None, None)
}

fn find_creation_time(tags: Option<&HashMap<String, String>>) -> Option<&str> {
    tags?.iter().find_map(|(key, value)| {
        key.eq_ignore_ascii_case("creation_time")
            .then_some(value.as_str())
    })
}

fn parse_metadata_timestamp(value: &str) -> Option<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(value)
        .map(|value| value.with_timezone(&Utc))
        .or_else(|_| {
            DateTime::parse_from_str(value, "%Y-%m-%d %H:%M:%S%#z").map(|v| v.with_timezone(&Utc))
        })
        .or_else(|_| {
            NaiveDateTime::parse_from_str(value, "%Y-%m-%d %H:%M:%S")
                .map(|value| Utc.from_utc_datetime(&value))
        })
        .ok()
}

fn timestamp_from_filename(path: &Path) -> Option<DateTime<Utc>> {
    let filename = path.file_stem()?.to_string_lossy();
    for regex in [&*SEPARATED_TIMESTAMP, &*COMPACT_TIMESTAMP] {
        if let Some(captures) = regex.captures(&filename) {
            let value = NaiveDate::from_ymd_opt(
                captures.name("y")?.as_str().parse().ok()?,
                captures.name("m")?.as_str().parse().ok()?,
                captures.name("d")?.as_str().parse().ok()?,
            )?
            .and_hms_opt(
                captures.name("h")?.as_str().parse().ok()?,
                captures.name("min")?.as_str().parse().ok()?,
                captures.name("s")?.as_str().parse().ok()?,
            )?;
            return Some(Utc.from_utc_datetime(&value));
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_compact_filename_timestamp() {
        let path = Path::new("REC_20260719_143501.AVI");
        assert_eq!(
            timestamp_from_filename(path).unwrap().to_rfc3339(),
            "2026-07-19T14:35:01+00:00"
        );
    }

    #[test]
    fn parses_separated_filename_timestamp() {
        let path = Path::new("KIA 2026-07-19 14-35-01.avi");
        assert_eq!(
            timestamp_from_filename(path).unwrap().to_rfc3339(),
            "2026-07-19T14:35:01+00:00"
        );
    }
}
