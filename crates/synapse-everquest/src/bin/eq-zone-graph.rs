use std::{env, ffi::OsStr, fs, path::PathBuf, process};

use synapse_everquest::{
    EverQuestMapCoord, EverQuestNearestLandmark, EverQuestZoneEdge, EverQuestZoneGraph,
    EverQuestZoneLandmark, build_zone_graph_from_root,
};

const MAX_EXIT_SAMPLES: usize = 32;
const MAX_LANDMARK_SAMPLES: usize = 32;
const MAX_SKIPPED_MAP_SAMPLES: usize = 16;
const DEFAULT_NEAREST_LIMIT: usize = 8;

fn main() {
    let args = parse_args().unwrap_or_else(|error| {
        eprintln!("{error}");
        eprintln!(
            "usage: eq-zone-graph --root <everquest-install-root> --zone <short-name> [--nearest x,y,z] [--summary-out <path>]"
        );
        process::exit(2);
    });

    match build_zone_graph_from_root(&args.root) {
        Ok(graph) => {
            let summary = format_zone_summary(&graph, &args);
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
    root: PathBuf,
    zone: String,
    nearest: Option<EverQuestMapCoord>,
    summary_out: Option<PathBuf>,
}

fn parse_args() -> Result<Args, String> {
    let mut args = env::args_os();
    let _program = args.next();
    let mut root = None;
    let mut zone = None;
    let mut nearest = None;
    let mut summary_out = None;

    while let Some(flag) = args.next() {
        if flag == OsStr::new("--root") {
            root = Some(required_path(&mut args, "--root")?);
        } else if flag == OsStr::new("--zone") {
            zone = Some(required_string(&mut args, "--zone")?);
        } else if flag == OsStr::new("--nearest") {
            nearest = Some(parse_coord(&required_string(&mut args, "--nearest")?)?);
        } else if flag == OsStr::new("--summary-out") {
            summary_out = Some(required_path(&mut args, "--summary-out")?);
        } else {
            return Err(format!(
                "unknown argument {}",
                PathBuf::from(flag).display()
            ));
        }
    }

    Ok(Args {
        root: root.ok_or_else(|| "--root is required".to_owned())?,
        zone: zone.ok_or_else(|| "--zone is required".to_owned())?,
        nearest,
        summary_out,
    })
}

fn required_path(args: &mut env::ArgsOs, flag: &str) -> Result<PathBuf, String> {
    args.next()
        .map(PathBuf::from)
        .ok_or_else(|| format!("{flag} requires a path"))
}

fn required_string(args: &mut env::ArgsOs, flag: &str) -> Result<String, String> {
    let value = args
        .next()
        .ok_or_else(|| format!("{flag} requires a value"))?;
    value
        .into_string()
        .map_err(|_| format!("{flag} value must be valid UTF-8"))
}

fn parse_coord(value: &str) -> Result<EverQuestMapCoord, String> {
    let fields = value.split(',').map(str::trim).collect::<Vec<_>>();
    if fields.len() != 3 {
        return Err("--nearest requires x,y,z".to_owned());
    }
    Ok(EverQuestMapCoord {
        x: parse_f64(fields[0], "nearest.x")?,
        y: parse_f64(fields[1], "nearest.y")?,
        z: parse_f64(fields[2], "nearest.z")?,
    })
}

fn parse_f64(value: &str, label: &str) -> Result<f64, String> {
    let parsed = value
        .parse::<f64>()
        .map_err(|_| format!("{label} must be a number"))?;
    if parsed.is_finite() {
        Ok(parsed)
    } else {
        Err(format!("{label} must be finite"))
    }
}

fn format_zone_summary(graph: &EverQuestZoneGraph, args: &Args) -> String {
    let exits = graph.exits_for_zone(&args.zone);
    let landmarks = graph.landmarks_for_zone(&args.zone);
    let mut out = String::new();
    push_line(&mut out, format!("root={}", args.root.display()));
    push_line(&mut out, format!("zone_query={}", args.zone));
    push_line(&mut out, format!("zone_count={}", graph.nodes.len()));
    push_line(
        &mut out,
        format!("graph_landmark_count={}", graph.landmarks.len()),
    );
    push_line(&mut out, format!("graph_edge_count={}", graph.edges.len()));
    push_line(
        &mut out,
        format!("unresolved_edge_count={}", graph.unresolved_edge_count),
    );
    push_line(
        &mut out,
        format!("skipped_map_count={}", graph.skipped_maps.len()),
    );
    for (index, skipped) in graph
        .skipped_maps
        .iter()
        .take(MAX_SKIPPED_MAP_SAMPLES)
        .enumerate()
    {
        push_line(
            &mut out,
            format!("skipped_map[{index}].path={}", skipped.path.display()),
        );
        push_line(
            &mut out,
            format!("skipped_map[{index}].error={}", skipped.error),
        );
    }
    push_line(
        &mut out,
        format!(
            "skipped_map_samples_truncated={}",
            graph.skipped_maps.len() > MAX_SKIPPED_MAP_SAMPLES
        ),
    );
    match graph.node(&args.zone) {
        Some(node) => {
            push_line(&mut out, "zone_found=true");
            push_line(
                &mut out,
                format!("zone.short_name={}", node.zone_short_name),
            );
            push_line(
                &mut out,
                format!(
                    "zone.display_name={}",
                    node.display_name.as_deref().unwrap_or("unknown")
                ),
            );
            push_line(
                &mut out,
                format!("zone.source_path={}", node.source_path.display()),
            );
            push_line(&mut out, format!("zone.len_bytes={}", node.len_bytes));
        }
        None => push_line(&mut out, "zone_found=false"),
    }

    push_line(&mut out, format!("zone_exit_count={}", exits.len()));
    for (index, edge) in exits.iter().take(MAX_EXIT_SAMPLES).enumerate() {
        format_edge(&mut out, index, edge);
    }
    push_line(
        &mut out,
        format!(
            "zone_exit_samples_truncated={}",
            exits.len() > MAX_EXIT_SAMPLES
        ),
    );

    push_line(&mut out, format!("zone_landmark_count={}", landmarks.len()));
    for (index, landmark) in landmarks.iter().take(MAX_LANDMARK_SAMPLES).enumerate() {
        format_landmark(&mut out, index, landmark);
    }
    push_line(
        &mut out,
        format!(
            "zone_landmark_samples_truncated={}",
            landmarks.len() > MAX_LANDMARK_SAMPLES
        ),
    );

    if let Some(location) = &args.nearest {
        push_line(
            &mut out,
            format!("nearest_query={},{},{}", location.x, location.y, location.z),
        );
        let nearest = graph.nearest_landmarks(&args.zone, location, DEFAULT_NEAREST_LIMIT);
        push_line(&mut out, format!("nearest_count={}", nearest.len()));
        for (index, item) in nearest.iter().enumerate() {
            format_nearest(&mut out, index, item);
        }
    }
    out
}

fn format_edge(out: &mut String, index: usize, edge: &EverQuestZoneEdge) {
    push_line(out, format!("exit[{index}].label={}", edge.label));
    push_line(
        out,
        format!("exit[{index}].target_hint={}", edge.target_hint),
    );
    push_line(
        out,
        format!(
            "exit[{index}].target_zone_short_name={}",
            edge.target_zone_short_name
                .as_deref()
                .unwrap_or("unresolved")
        ),
    );
    push_line(
        out,
        format!(
            "exit[{index}].target_display_name={}",
            edge.target_display_name.as_deref().unwrap_or("unknown")
        ),
    );
    push_line(out, format!("exit[{index}].confidence={}", edge.confidence));
    push_line(
        out,
        format!("exit[{index}].resolution={:?}", edge.resolution),
    );
    push_line(
        out,
        format!("exit[{index}].source_path={}", edge.source_path.display()),
    );
    push_line(
        out,
        format!(
            "exit[{index}].source_line_number={}",
            edge.source_line_number
        ),
    );
    push_line(
        out,
        format!(
            "exit[{index}].xyz={},{},{}",
            edge.location.x, edge.location.y, edge.location.z
        ),
    );
}

fn format_landmark(out: &mut String, index: usize, landmark: &EverQuestZoneLandmark) {
    push_line(out, format!("landmark[{index}].label={}", landmark.label));
    push_line(
        out,
        format!(
            "landmark[{index}].xyz={},{},{}",
            landmark.location.x, landmark.location.y, landmark.location.z
        ),
    );
    push_line(
        out,
        format!(
            "landmark[{index}].source_line_number={}",
            landmark.source_line_number
        ),
    );
}

fn format_nearest(out: &mut String, index: usize, item: &EverQuestNearestLandmark) {
    push_line(
        out,
        format!("nearest[{index}].label={}", item.landmark.label),
    );
    push_line(out, format!("nearest[{index}].distance={}", item.distance));
    push_line(
        out,
        format!(
            "nearest[{index}].source_line_number={}",
            item.landmark.source_line_number
        ),
    );
}

fn push_line(out: &mut String, line: impl AsRef<str>) {
    out.push_str(line.as_ref());
    out.push('\n');
}
