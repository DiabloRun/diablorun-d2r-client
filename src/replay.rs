//! `--replay-frames` developer tool: replay a `--save-frames` dump through the
//! detector and report exactly which frame first classifies as loading.

use std::fs;
use std::path::{Path, PathBuf};

use crate::detector;

struct ReplayFrame {
    index: u64,
    file: PathBuf,
    timestamp: i64,
    width: usize,
    height: usize,
}

pub fn run(directory: &Path) -> Result<(), String> {
    let frames = load_manifest(directory)?;
    if frames.is_empty() {
        return Err(format!("no frames found in {}", directory.display()));
    }

    println!(
        "Replaying {} frames from {}",
        frames.len(),
        directory.display()
    );

    let mut previous: Option<bool> = None;
    let mut first_loading: Option<(u64, i64, PathBuf)> = None;
    let mut first_gameplay_after_loading: Option<(u64, i64, PathBuf)> = None;

    for frame in &frames {
        let data = fs::read(&frame.file)
            .map_err(|error| format!("read {}: {error}", frame.file.display()))?;
        let detected = detector::detect_loading(&data, frame.width, frame.height);

        if previous != Some(detected) {
            let state = if detected { "LOADING" } else { "gameplay" };
            println!(
                "  frame {:06} timestamp={} {} ({}x{}): {}",
                frame.index,
                frame.timestamp,
                state,
                frame.width,
                frame.height,
                frame.file.file_name().unwrap_or_default().to_string_lossy()
            );

            if detected && first_loading.is_none() {
                first_loading = Some((frame.index, frame.timestamp, frame.file.clone()));
            }
            if !detected && first_loading.is_some() && first_gameplay_after_loading.is_none() {
                first_gameplay_after_loading =
                    Some((frame.index, frame.timestamp, frame.file.clone()));
            }
            previous = Some(detected);
        }
    }

    if let Some((index, timestamp, file)) = first_loading {
        println!(
            "First loading frame: {:06} timestamp={} ({})",
            index,
            timestamp,
            file.file_name().unwrap_or_default().to_string_lossy()
        );
    } else {
        println!("No loading frame was detected.");
    }

    if let Some((index, timestamp, file)) = first_gameplay_after_loading {
        println!(
            "First gameplay frame after loading: {:06} timestamp={} ({})",
            index,
            timestamp,
            file.file_name().unwrap_or_default().to_string_lossy()
        );
    }

    Ok(())
}

fn load_manifest(directory: &Path) -> Result<Vec<ReplayFrame>, String> {
    let manifest = directory.join("frames.csv");
    if manifest.is_file() {
        return load_csv(&manifest);
    }

    let mut frames = Vec::new();
    let entries = fs::read_dir(directory)
        .map_err(|error| format!("read {}: {error}", directory.display()))?;
    for entry in entries {
        let entry = entry.map_err(|error| error.to_string())?;
        let path = entry.path();
        if path.extension().and_then(|value| value.to_str()) != Some("rgba") {
            continue;
        }
        let data = fs::metadata(&path).map_err(|error| error.to_string())?;
        let pixels = data.len() as usize / 4;
        let width = 160.min(pixels.max(1));
        let height = (pixels / width).max(1);
        frames.push(ReplayFrame {
            index: frames.len() as u64,
            file: path,
            timestamp: 0,
            width,
            height,
        });
    }
    frames.sort_by(|a, b| a.file.cmp(&b.file));
    Ok(frames)
}

fn load_csv(path: &Path) -> Result<Vec<ReplayFrame>, String> {
    let text =
        fs::read_to_string(path).map_err(|error| format!("read {}: {error}", path.display()))?;
    let directory = path.parent().unwrap_or_else(|| Path::new("."));
    let mut frames = Vec::new();

    for (line_number, line) in text.lines().enumerate() {
        if line_number == 0 || line.trim().is_empty() {
            continue;
        }
        let fields: Vec<&str> = line.split(',').collect();
        if fields.len() < 5 {
            continue;
        }
        let index = fields[0].parse::<u64>().unwrap_or(0);
        let file = directory.join(fields[1]);
        let timestamp = fields[2].parse::<i64>().unwrap_or(0);
        let (width, height) = if fields.len() >= 7 {
            (
                fields[5].parse::<usize>().unwrap_or(160),
                fields[6].parse::<usize>().unwrap_or(90),
            )
        } else {
            infer_dimensions(&file)
        };
        frames.push(ReplayFrame {
            index,
            file,
            timestamp,
            width,
            height,
        });
    }

    Ok(frames)
}

fn infer_dimensions(path: &Path) -> (usize, usize) {
    let pixels = fs::metadata(path)
        .map(|metadata| metadata.len() as usize / 4)
        .unwrap_or(160 * 90);
    let width = 160.min(pixels.max(1));
    (width, (pixels / width).max(1))
}
