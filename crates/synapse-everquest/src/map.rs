use std::{
    fs,
    path::{Path, PathBuf},
    time::UNIX_EPOCH,
};

use serde::{Deserialize, Serialize};
use thiserror::Error;

const MAP_DIR_NAME: &str = "maps";

pub const DEFAULT_MAX_MAP_FILE_BYTES: u64 = 8 * 1024 * 1024;

#[derive(Debug, Error)]
pub enum EverQuestMapError {
    #[error("EverQuest map path {path} is invalid: {reason}")]
    InvalidPath { path: PathBuf, reason: String },
    #[error("I/O error while reading EverQuest map path {path}: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("EverQuest map file {path} is empty")]
    Empty { path: PathBuf },
    #[error("EverQuest map file {path} is {len_bytes} bytes, exceeding the {max_bytes} byte limit")]
    TooLarge {
        path: PathBuf,
        len_bytes: u64,
        max_bytes: u64,
    },
    #[error("EverQuest map record at {path}:{line_number} has unknown type {record_type:?}")]
    UnknownRecord {
        path: PathBuf,
        line_number: usize,
        record_type: String,
    },
    #[error(
        "EverQuest map record at {path}:{line_number} type {record_type:?} is malformed: {reason}"
    )]
    MalformedRecord {
        path: PathBuf,
        line_number: usize,
        record_type: String,
        reason: String,
    },
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct EverQuestMapSource {
    pub path: PathBuf,
    pub zone_short_name: String,
    pub len_bytes: u64,
    pub last_modified_unix_ms: Option<i64>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct EverQuestMapFile {
    pub source: EverQuestMapSource,
    pub line_count: usize,
    pub segment_count: usize,
    pub point_count: usize,
    pub records: Vec<EverQuestMapRecord>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EverQuestMapRecord {
    Line(EverQuestMapLine),
    Point(EverQuestMapPoint),
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct EverQuestMapLine {
    pub source_path: PathBuf,
    pub source_line_number: usize,
    pub start: EverQuestMapCoord,
    pub end: EverQuestMapCoord,
    pub color: EverQuestMapColor,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct EverQuestMapPoint {
    pub source_path: PathBuf,
    pub source_line_number: usize,
    pub location: EverQuestMapCoord,
    pub color: EverQuestMapColor,
    pub layer: i32,
    pub label: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct EverQuestMapCoord {
    pub x: f64,
    pub y: f64,
    pub z: f64,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct EverQuestMapColor {
    pub r: u8,
    pub g: u8,
    pub b: u8,
}

/// Discovers `EverQuest` map files below an install root.
///
/// # Errors
///
/// Returns [`EverQuestMapError::InvalidPath`] when the root has no `maps`
/// directory and [`EverQuestMapError::Io`] when the directory or file metadata
/// cannot be read.
pub fn discover_map_files(root: &Path) -> Result<Vec<EverQuestMapSource>, EverQuestMapError> {
    let map_dir = root.join(MAP_DIR_NAME);
    if !map_dir.is_dir() {
        return Err(EverQuestMapError::InvalidPath {
            path: map_dir,
            reason: "maps directory is absent".to_owned(),
        });
    }

    let mut files = Vec::new();
    for entry in fs::read_dir(&map_dir).map_err(|source| EverQuestMapError::Io {
        path: map_dir.clone(),
        source,
    })? {
        let entry = entry.map_err(|source| EverQuestMapError::Io {
            path: map_dir.clone(),
            source,
        })?;
        let path = entry.path();
        if !is_map_text_path(&path) {
            continue;
        }
        let metadata = entry.metadata().map_err(|source| EverQuestMapError::Io {
            path: path.clone(),
            source,
        })?;
        if metadata.is_file() {
            files.push(source_from_metadata(&path, &metadata)?);
        }
    }
    files.sort_by(|left, right| left.path.cmp(&right.path));
    Ok(files)
}

/// Parses one `EverQuest` map file using the default size limit.
///
/// # Errors
///
/// Returns [`EverQuestMapError`] for missing files, invalid paths, empty files,
/// oversized files, unknown record types, and malformed record fields.
pub fn parse_map_file(path: &Path) -> Result<EverQuestMapFile, EverQuestMapError> {
    parse_map_file_with_limit(path, DEFAULT_MAX_MAP_FILE_BYTES)
}

/// Parses one `EverQuest` map file using a caller-supplied byte limit.
///
/// # Errors
///
/// Returns [`EverQuestMapError`] for missing files, invalid paths, empty files,
/// oversized files, unknown record types, and malformed record fields.
pub fn parse_map_file_with_limit(
    path: &Path,
    max_bytes: u64,
) -> Result<EverQuestMapFile, EverQuestMapError> {
    let metadata = fs::metadata(path).map_err(|source| EverQuestMapError::Io {
        path: path.to_path_buf(),
        source,
    })?;
    if !metadata.is_file() {
        return Err(EverQuestMapError::InvalidPath {
            path: path.to_path_buf(),
            reason: "path is not a file".to_owned(),
        });
    }
    if metadata.len() == 0 {
        return Err(EverQuestMapError::Empty {
            path: path.to_path_buf(),
        });
    }
    if metadata.len() > max_bytes {
        return Err(EverQuestMapError::TooLarge {
            path: path.to_path_buf(),
            len_bytes: metadata.len(),
            max_bytes,
        });
    }

    let text = fs::read_to_string(path).map_err(|source| EverQuestMapError::Io {
        path: path.to_path_buf(),
        source,
    })?;
    let mut records = Vec::new();
    for (index, line) in text.lines().enumerate() {
        if let Some(record) = parse_map_record(path, index + 1, line)? {
            records.push(record);
        }
    }
    if records.is_empty() {
        return Err(EverQuestMapError::Empty {
            path: path.to_path_buf(),
        });
    }

    let segment_count = records
        .iter()
        .filter(|record| matches!(record, EverQuestMapRecord::Line(_)))
        .count();
    let point_count = records
        .iter()
        .filter(|record| matches!(record, EverQuestMapRecord::Point(_)))
        .count();

    Ok(EverQuestMapFile {
        source: source_from_metadata(path, &metadata)?,
        line_count: text.lines().count(),
        segment_count,
        point_count,
        records,
    })
}

/// Parses one `EverQuest` map record line.
///
/// # Errors
///
/// Returns [`EverQuestMapError::UnknownRecord`] for unsupported record types
/// and [`EverQuestMapError::MalformedRecord`] for invalid field counts or
/// values.
pub fn parse_map_record(
    path: &Path,
    line_number: usize,
    line: &str,
) -> Result<Option<EverQuestMapRecord>, EverQuestMapError> {
    let trimmed = line.trim();
    if trimmed.is_empty() {
        return Ok(None);
    }
    let mut chars = trimmed.chars();
    let record_type = chars.next().unwrap_or_default();
    let body = chars.as_str().trim_start();
    match record_type {
        'L' => parse_line_record(path, line_number, body).map(Some),
        'P' => parse_point_record(path, line_number, body).map(Some),
        _ => Err(EverQuestMapError::UnknownRecord {
            path: path.to_path_buf(),
            line_number,
            record_type: record_type.to_string(),
        }),
    }
}

fn parse_line_record(
    path: &Path,
    line_number: usize,
    body: &str,
) -> Result<EverQuestMapRecord, EverQuestMapError> {
    let fields = split_exact_fields(path, line_number, "L", body, 9)?;
    Ok(EverQuestMapRecord::Line(EverQuestMapLine {
        source_path: path.to_path_buf(),
        source_line_number: line_number,
        start: EverQuestMapCoord {
            x: parse_f64(path, line_number, "L", "x1", fields[0])?,
            y: parse_f64(path, line_number, "L", "y1", fields[1])?,
            z: parse_f64(path, line_number, "L", "z1", fields[2])?,
        },
        end: EverQuestMapCoord {
            x: parse_f64(path, line_number, "L", "x2", fields[3])?,
            y: parse_f64(path, line_number, "L", "y2", fields[4])?,
            z: parse_f64(path, line_number, "L", "z2", fields[5])?,
        },
        color: EverQuestMapColor {
            r: parse_color(path, line_number, "L", "r", fields[6])?,
            g: parse_color(path, line_number, "L", "g", fields[7])?,
            b: parse_color(path, line_number, "L", "b", fields[8])?,
        },
    }))
}

fn parse_point_record(
    path: &Path,
    line_number: usize,
    body: &str,
) -> Result<EverQuestMapRecord, EverQuestMapError> {
    let fields: Vec<_> = body.splitn(8, ',').map(str::trim).collect();
    if fields.len() != 8 {
        return malformed(
            path,
            line_number,
            "P",
            format!("expected 8 comma-separated fields, found {}", fields.len()),
        );
    }
    if fields[7].is_empty() {
        return malformed(path, line_number, "P", "label is empty".to_owned());
    }
    Ok(EverQuestMapRecord::Point(EverQuestMapPoint {
        source_path: path.to_path_buf(),
        source_line_number: line_number,
        location: EverQuestMapCoord {
            x: parse_f64(path, line_number, "P", "x", fields[0])?,
            y: parse_f64(path, line_number, "P", "y", fields[1])?,
            z: parse_f64(path, line_number, "P", "z", fields[2])?,
        },
        color: EverQuestMapColor {
            r: parse_color(path, line_number, "P", "r", fields[3])?,
            g: parse_color(path, line_number, "P", "g", fields[4])?,
            b: parse_color(path, line_number, "P", "b", fields[5])?,
        },
        layer: parse_layer(path, line_number, "P", fields[6])?,
        label: fields[7].to_owned(),
    }))
}

fn split_exact_fields<'a>(
    path: &Path,
    line_number: usize,
    record_type: &str,
    body: &'a str,
    expected: usize,
) -> Result<Vec<&'a str>, EverQuestMapError> {
    let fields: Vec<_> = body.split(',').map(str::trim).collect();
    if fields.len() != expected {
        return malformed(
            path,
            line_number,
            record_type,
            format!(
                "expected {expected} comma-separated fields, found {}",
                fields.len()
            ),
        );
    }
    Ok(fields)
}

fn parse_f64(
    path: &Path,
    line_number: usize,
    record_type: &str,
    field_name: &str,
    value: &str,
) -> Result<f64, EverQuestMapError> {
    let parsed = value.parse::<f64>().map_err(|_| {
        malformed_error(
            path,
            line_number,
            record_type,
            format!("{field_name} value {value:?} is not a number"),
        )
    })?;
    if !parsed.is_finite() {
        return malformed(
            path,
            line_number,
            record_type,
            format!("{field_name} value {value:?} is not finite"),
        );
    }
    Ok(parsed)
}

fn parse_color(
    path: &Path,
    line_number: usize,
    record_type: &str,
    field_name: &str,
    value: &str,
) -> Result<u8, EverQuestMapError> {
    let parsed = value.parse::<u16>().map_err(|_| {
        malformed_error(
            path,
            line_number,
            record_type,
            format!("{field_name} color value {value:?} is not an integer"),
        )
    })?;
    u8::try_from(parsed).map_err(|_| {
        malformed_error(
            path,
            line_number,
            record_type,
            format!("{field_name} color value {value:?} is outside 0..=255"),
        )
    })
}

fn parse_layer(
    path: &Path,
    line_number: usize,
    record_type: &str,
    value: &str,
) -> Result<i32, EverQuestMapError> {
    let parsed = value.parse::<i32>().map_err(|_| {
        malformed_error(
            path,
            line_number,
            record_type,
            format!("layer value {value:?} is not an integer"),
        )
    })?;
    if parsed < 0 {
        return malformed(
            path,
            line_number,
            record_type,
            format!("layer value {value:?} is negative"),
        );
    }
    Ok(parsed)
}

fn malformed<T>(
    path: &Path,
    line_number: usize,
    record_type: &str,
    reason: String,
) -> Result<T, EverQuestMapError> {
    Err(malformed_error(path, line_number, record_type, reason))
}

fn malformed_error(
    path: &Path,
    line_number: usize,
    record_type: &str,
    reason: String,
) -> EverQuestMapError {
    EverQuestMapError::MalformedRecord {
        path: path.to_path_buf(),
        line_number,
        record_type: record_type.to_owned(),
        reason,
    }
}

fn source_from_metadata(
    path: &Path,
    metadata: &fs::Metadata,
) -> Result<EverQuestMapSource, EverQuestMapError> {
    let zone_short_name = path
        .file_stem()
        .and_then(|value| value.to_str())
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| EverQuestMapError::InvalidPath {
            path: path.to_path_buf(),
            reason: "file stem is absent or not valid UTF-8".to_owned(),
        })?
        .to_owned();
    let last_modified_unix_ms = metadata
        .modified()
        .ok()
        .and_then(|modified| modified.duration_since(UNIX_EPOCH).ok())
        .and_then(|duration| i64::try_from(duration.as_millis()).ok());

    Ok(EverQuestMapSource {
        path: path.to_path_buf(),
        zone_short_name,
        len_bytes: metadata.len(),
        last_modified_unix_ms,
    })
}

fn is_map_text_path(path: &Path) -> bool {
    path.extension()
        .and_then(|value| value.to_str())
        .is_some_and(|extension| extension.eq_ignore_ascii_case("txt"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_line_and_point_records() -> Result<(), EverQuestMapError> {
        let temp = tempfile::tempdir().map_err(io_error)?;
        let path = temp.path().join("nektulos.txt");
        fs::write(
            &path,
            "L 1.0, 2.5, 3, 4, 5, 6, 10, 20, 30\nP 7, 8, 9, 40, 50, 60, 3, To_Neriak\n",
        )
        .map_err(io_error)?;

        let map = parse_map_file(&path)?;

        assert_eq!(map.source.zone_short_name, "nektulos");
        assert_eq!(map.line_count, 2);
        assert_eq!(map.segment_count, 1);
        assert_eq!(map.point_count, 1);
        match &map.records[0] {
            EverQuestMapRecord::Line(line) => {
                assert_eq!(line.source_path, path);
                assert_eq!(line.source_line_number, 1);
                assert!((line.start.x - 1.0).abs() < f64::EPSILON);
                assert!((line.end.z - 6.0).abs() < f64::EPSILON);
                assert_eq!(line.color.b, 30);
            }
            EverQuestMapRecord::Point(_) => panic!("expected line record"),
        }
        match &map.records[1] {
            EverQuestMapRecord::Point(point) => {
                assert_eq!(point.source_line_number, 2);
                assert!((point.location.y - 8.0).abs() < f64::EPSILON);
                assert_eq!(point.layer, 3);
                assert_eq!(point.label, "To_Neriak");
            }
            EverQuestMapRecord::Line(_) => panic!("expected point record"),
        }
        Ok(())
    }

    #[test]
    fn preserves_point_label_commas() -> Result<(), EverQuestMapError> {
        let temp = tempfile::tempdir().map_err(io_error)?;
        let path = temp.path().join("neriaka.txt");
        fs::write(&path, "P 1, 2, 3, 0, 0, 0, 3, Hall, With, Commas\n").map_err(io_error)?;

        let map = parse_map_file(&path)?;

        match &map.records[0] {
            EverQuestMapRecord::Point(point) => assert_eq!(point.label, "Hall, With, Commas"),
            EverQuestMapRecord::Line(_) => panic!("expected point record"),
        }
        Ok(())
    }

    #[test]
    fn fails_closed_on_unknown_record_type() {
        let result = parse_map_record(Path::new("synthetic.txt"), 1, "Q 1,2,3");

        assert!(matches!(
            result,
            Err(EverQuestMapError::UnknownRecord { line_number: 1, .. })
        ));
    }

    #[test]
    fn fails_closed_on_malformed_numeric_field() {
        let result = parse_map_record(
            Path::new("synthetic.txt"),
            1,
            "L 1, two, 3, 4, 5, 6, 7, 8, 9",
        );

        assert!(matches!(
            result,
            Err(EverQuestMapError::MalformedRecord { line_number: 1, .. })
        ));
    }

    #[test]
    fn fails_closed_on_empty_file() -> Result<(), EverQuestMapError> {
        let temp = tempfile::tempdir().map_err(io_error)?;
        let path = temp.path().join("empty.txt");
        fs::write(&path, "").map_err(io_error)?;

        let result = parse_map_file(&path);

        assert!(matches!(result, Err(EverQuestMapError::Empty { .. })));
        Ok(())
    }

    #[test]
    fn fails_closed_on_oversized_file() -> Result<(), EverQuestMapError> {
        let temp = tempfile::tempdir().map_err(io_error)?;
        let path = temp.path().join("large.txt");
        fs::write(&path, "P 1, 2, 3, 0, 0, 0, 3, Label\n").map_err(io_error)?;

        let result = parse_map_file_with_limit(&path, 4);

        assert!(matches!(result, Err(EverQuestMapError::TooLarge { .. })));
        Ok(())
    }

    fn io_error(source: std::io::Error) -> EverQuestMapError {
        EverQuestMapError::Io {
            path: PathBuf::from("test"),
            source,
        }
    }
}
