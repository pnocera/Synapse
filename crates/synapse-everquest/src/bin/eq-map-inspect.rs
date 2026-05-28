use std::{env, ffi::OsStr, fs, path::PathBuf, process};

use synapse_everquest::{EverQuestMapFile, EverQuestMapRecord, parse_map_file};

const MAX_POINT_SAMPLES: usize = 32;

fn main() {
    let args = parse_args().unwrap_or_else(|error| {
        eprintln!("{error}");
        eprintln!("usage: eq-map-inspect <map-file> [--summary-out <path>]");
        process::exit(2);
    });

    match parse_map_file(&args.map_file) {
        Ok(map) => {
            let summary = format_map(&map);
            print!("{summary}");
            if let Some(summary_out) = args.summary_out {
                if let Err(source) = fs::write(&summary_out, summary) {
                    eprintln!("error=failed to write {}: {source}", summary_out.display());
                    process::exit(1);
                }
                println!("summary_out={}", summary_out.display());
            }
        }
        Err(error) => {
            eprintln!("error={error}");
            process::exit(1);
        }
    }
}

struct Args {
    map_file: PathBuf,
    summary_out: Option<PathBuf>,
}

fn parse_args() -> Result<Args, String> {
    let mut args = env::args_os();
    let _program = args.next();
    let map_file = args
        .next()
        .map(PathBuf::from)
        .ok_or_else(|| "missing map-file argument".to_owned())?;

    let mut summary_out = None;
    while let Some(flag) = args.next() {
        if flag != OsStr::new("--summary-out") {
            return Err(format!(
                "unknown argument {}",
                PathBuf::from(flag).display()
            ));
        }
        if summary_out.is_some() {
            return Err("--summary-out was provided more than once".to_owned());
        }
        let out = args
            .next()
            .map(PathBuf::from)
            .ok_or_else(|| "--summary-out requires a path".to_owned())?;
        summary_out = Some(out);
    }

    Ok(Args {
        map_file,
        summary_out,
    })
}

fn format_map(map: &EverQuestMapFile) -> String {
    let mut out = String::new();
    push_line(&mut out, format!("path={}", map.source.path.display()));
    push_line(
        &mut out,
        format!("zone_short_name={}", map.source.zone_short_name),
    );
    push_line(&mut out, format!("len_bytes={}", map.source.len_bytes));
    push_line(
        &mut out,
        format!(
            "last_modified_unix_ms={}",
            map.source
                .last_modified_unix_ms
                .map_or_else(|| "unknown".to_owned(), |value| value.to_string())
        ),
    );
    push_line(&mut out, format!("line_count={}", map.line_count));
    push_line(&mut out, format!("segment_count={}", map.segment_count));
    push_line(&mut out, format!("point_count={}", map.point_count));

    let mut emitted = 0_usize;
    for record in &map.records {
        let EverQuestMapRecord::Point(point) = record else {
            continue;
        };
        if emitted == MAX_POINT_SAMPLES {
            push_line(&mut out, "point_samples_truncated=true");
            return out;
        }
        push_line(
            &mut out,
            format!("point[{emitted}].line_number={}", point.source_line_number),
        );
        push_line(
            &mut out,
            format!(
                "point[{emitted}].xyz={},{},{}",
                point.location.x, point.location.y, point.location.z
            ),
        );
        push_line(
            &mut out,
            format!(
                "point[{emitted}].rgb={},{},{}",
                point.color.r, point.color.g, point.color.b
            ),
        );
        push_line(&mut out, format!("point[{emitted}].layer={}", point.layer));
        push_line(&mut out, format!("point[{emitted}].label={}", point.label));
        emitted += 1;
    }
    push_line(&mut out, "point_samples_truncated=false");
    out
}

fn push_line(out: &mut String, line: impl AsRef<str>) {
    out.push_str(line.as_ref());
    out.push('\n');
}
