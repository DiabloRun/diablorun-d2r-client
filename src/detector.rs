//! Heuristic loading-screen detector.
//!
//! This is a direct port of `src/loading-detection.ts` from the Linux client.
//! It deliberately has no Windows or GPU dependencies so it can be unit tested
//! with plain RGB/RGBA buffers.

/// Pixels at or below this channel value are treated as background black.
const BLACK_LEVEL: u8 = 20;

const BRIGHT_LEVEL: u8 = 40;

/// Gold needs a clear warm hue, not just a bright pixel.
const GOLD_MIN_RED: u8 = 20;
const GOLD_MIN_RED_BLUE_DELTA: i32 = 6;
const GOLD_MIN_GREEN_BLUE_DELTA: i32 = 2;
const GOLD_MIN_RED_GREEN_RATIO: f32 = 0.75;

/// Per-edge border coverage and the average across all four edges.
const MIN_EDGE_BORDER_COVERAGE: f32 = 0.6;
const MIN_AVERAGE_BORDER_COVERAGE: f32 = 0.8;

/// The border as a whole must still look warm/golden, not white or blue.
const MIN_BORDER_RED_BLUE_DELTA: i32 = 3;

/// The loading screen must be at least 98.5% black outside the frame
/// (titlebar excluded), i.e. at most 15 non-black pixels per thousand.
const MAX_BACKGROUND_NON_BLACK_PER_THOUSAND: usize = 15;

/// The bottom-right logo screen is almost entirely black: at most 2%.
const MAX_LOGO_NON_BLACK_PERCENT: usize = 2;

/// Run the detector on a tightly packed RGBA frame (stride = 4).
pub fn detect_loading(data: &[u8], width: usize, height: usize) -> bool {
    detect_loading_stride(data, width, height, 4)
}

/// Run the detector on a frame with an arbitrary pixel stride of at least 3
/// bytes. The fourth (alpha) byte, when present, is ignored.
pub fn detect_loading_stride(data: &[u8], width: usize, height: usize, stride: usize) -> bool {
    if width < 16 || height < 16 || stride < 3 {
        return false;
    }

    let needed = match width
        .checked_mul(height)
        .and_then(|pixels| pixels.checked_mul(stride))
    {
        Some(value) => value,
        None => return false,
    };

    if data.len() < needed {
        return false;
    }

    let frame = Frame {
        data,
        stride,
        width,
        height,
    };
    let content_top = find_content_top(&frame);
    if let Some(bounds) = find_center_frame(&frame, content_top) {
        if is_center_loading(&frame, content_top, bounds) {
            return true;
        }
    }

    is_logo_loading(&frame, content_top)
}

struct Frame<'a> {
    data: &'a [u8],
    stride: usize,
    width: usize,
    height: usize,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Bounds {
    left: usize,
    top: usize,
    right: usize,
    bottom: usize,
}

struct NonBlackScan {
    non_black: usize,
    bounds: Bounds,
}

#[inline]
fn pixel_offset(frame: &Frame<'_>, x: usize, y: usize) -> usize {
    (y * frame.width + x) * frame.stride
}

#[inline]
fn max_channel(frame: &Frame<'_>, x: usize, y: usize) -> u8 {
    let index = pixel_offset(frame, x, y);
    frame.data[index]
        .max(frame.data[index + 1])
        .max(frame.data[index + 2])
}

#[inline]
fn is_content(frame: &Frame<'_>, x: usize, y: usize) -> bool {
    max_channel(frame, x, y) > BLACK_LEVEL
}

#[inline]
fn is_black(frame: &Frame<'_>, x: usize, y: usize) -> bool {
    max_channel(frame, x, y) <= BLACK_LEVEL
}

#[inline]
fn is_gold(frame: &Frame<'_>, x: usize, y: usize) -> bool {
    let index = pixel_offset(frame, x, y);
    let red = i32::from(frame.data[index]);
    let green = i32::from(frame.data[index + 1]);
    let blue = i32::from(frame.data[index + 2]);

    red > i32::from(GOLD_MIN_RED)
        && red - blue >= GOLD_MIN_RED_BLUE_DELTA
        && green - blue >= GOLD_MIN_GREEN_BLUE_DELTA
        && red as f32 >= green as f32 * GOLD_MIN_RED_GREEN_RATIO
}

/// ffmpeg's fast bilinear scaling can desaturate a thin gold line into gray.
/// Border coverage therefore accepts any bright, non-cold pixel; the average
/// color of the whole border is checked separately.
#[inline]
fn is_border_pixel(frame: &Frame<'_>, x: usize, y: usize) -> bool {
    let index = pixel_offset(frame, x, y);
    let red = f32::from(frame.data[index]);
    let green = f32::from(frame.data[index + 1]);
    let blue = f32::from(frame.data[index + 2]);

    red > f32::from(GOLD_MIN_RED) && red >= green * 0.7 && green >= blue * 0.7
}

/// Finds the top of the game content inside a windowed D2R window. Fullscreen
/// frames start with black rows, so this is normally zero.
fn find_content_top(frame: &Frame<'_>) -> usize {
    let x = frame.width >> 1;
    let max_titlebar = (frame.height as f32 * 0.2).round() as usize;

    for y in 0..frame.height {
        if is_black(frame, x, y) {
            // A dark row far below the top is game scenery, not a titlebar.
            return if y <= max_titlebar { y } else { 0 };
        }
    }

    0
}

/// Finds the bright centered rectangle (the door/legacy loading frame).
///
/// Several scan lines are used instead of one so a cursor, animated element, or
/// compression artifact cannot move a margin. The results must also be stable
/// across the scan lines, which rejects ordinary gameplay scenes quickly.
fn find_center_frame(frame: &Frame<'_>, content_top: usize) -> Option<Bounds> {
    let inner_height = frame.height.saturating_sub(content_top);
    if inner_height < 12 {
        return None;
    }

    let mut tops = [0usize; 5];
    let mut bottoms = [0usize; 5];
    let mut lefts = [0usize; 5];
    let mut rights = [0usize; 5];
    let mut top_count = 0usize;
    let mut left_count = 0usize;

    for fraction in [0.44_f32, 0.47, 0.5, 0.53, 0.56] {
        let x = ((frame.width - 1) as f32 * fraction).round() as usize;
        let x = x.min(frame.width - 1);
        if let Some((first, last)) = scan_column(frame, x, content_top) {
            tops[top_count] = first;
            bottoms[top_count] = last;
            top_count += 1;
        }
    }

    for fraction in [0.34_f32, 0.42, 0.5, 0.58, 0.66] {
        let offset = ((inner_height - 1) as f32 * fraction).round() as usize;
        let y = content_top + offset.min(inner_height - 1);
        if let Some((first, last)) = scan_row(frame, y) {
            lefts[left_count] = first;
            rights[left_count] = last;
            left_count += 1;
        }
    }

    let vertical_tolerance = 3.max((inner_height as f32 * 0.12).round() as usize);
    let horizontal_tolerance = 3.max((frame.width as f32 * 0.12).round() as usize);

    if top_count < 3
        || left_count < 3
        || !is_stable(&tops[..top_count], vertical_tolerance)
        || !is_stable(&bottoms[..top_count], vertical_tolerance)
        || !is_stable(&lefts[..left_count], horizontal_tolerance)
        || !is_stable(&rights[..left_count], horizontal_tolerance)
    {
        return None;
    }

    // Use the outermost detected content. Thin, dim frame edges can disappear
    // from one scan line after ffmpeg's bilinear scaling; taking the extreme
    // keeps the complete border inside the bounds.
    let bounds = Bounds {
        top: *tops[..top_count].iter().min().unwrap(),
        bottom: *bottoms[..top_count].iter().max().unwrap(),
        left: *lefts[..left_count].iter().min().unwrap(),
        right: *rights[..left_count].iter().max().unwrap(),
    };

    if bounds.right <= bounds.left || bounds.bottom <= bounds.top {
        return None;
    }

    Some(bounds)
}

fn scan_column(frame: &Frame<'_>, x: usize, start_y: usize) -> Option<(usize, usize)> {
    let mut first = None;
    let mut last = 0;

    for y in start_y..frame.height {
        if is_content(frame, x, y) {
            if first.is_none() {
                first = Some(y);
            }
            last = y;
        }
    }

    first.map(|first| (first, last))
}

fn scan_row(frame: &Frame<'_>, y: usize) -> Option<(usize, usize)> {
    let mut first = None;
    let mut last = 0;

    for x in 0..frame.width {
        if is_content(frame, x, y) {
            if first.is_none() {
                first = Some(x);
            }
            last = x;
        }
    }

    first.map(|first| (first, last))
}

fn is_stable(values: &[usize], tolerance: usize) -> bool {
    let Some((&first, rest)) = values.split_first() else {
        return false;
    };
    let mut min = first;
    let mut max = first;

    for &value in rest {
        min = min.min(value);
        max = max.max(value);
    }

    max - min <= tolerance
}

fn is_center_loading(frame: &Frame<'_>, content_top: usize, bounds: Bounds) -> bool {
    let frame_width = bounds.right - bounds.left + 1;
    let frame_height = bounds.bottom - bounds.top + 1;
    let inner_height = frame.height.saturating_sub(content_top);

    if frame_width < 16 || frame_height < 16 {
        return false;
    }

    let center_x = (bounds.left + bounds.right + 1) as f32 / 2.0;
    let center_y = (bounds.top + bounds.bottom + 1) as f32 / 2.0;
    let expected_center_y = content_top as f32 + inner_height as f32 / 2.0;

    if (center_x - frame.width as f32 / 2.0).abs() > 2.0_f32.max(frame.width as f32 * 0.02)
        || (center_y - expected_center_y).abs() > 2.0_f32.max(inner_height as f32 * 0.03)
    {
        return false;
    }

    let aspect = frame_width as f32 / frame_height as f32;
    let width_ratio = frame_width as f32 / inner_height as f32;
    let height_ratio = frame_height as f32 / inner_height as f32;

    // D2R's door loading frame is ~1.5:1. The legacy frame is ~1:1.
    let door = in_range(aspect, 1.32, 1.68)
        && in_range(width_ratio, 0.55, 0.86)
        && in_range(height_ratio, 0.36, 0.6);
    let legacy = in_range(aspect, 0.85, 1.15)
        && in_range(width_ratio, 0.3, 0.55)
        && in_range(height_ratio, 0.3, 0.55);

    if !door && !legacy {
        return false;
    }

    // The black background is the strongest in-game guard and can reject a
    // gameplay frame after only a small fraction of the pixels have been read.
    if !has_black_background(frame, content_top, bounds) {
        return false;
    }

    let edges = border_stats(frame, bounds);
    if !is_normal_border(&edges) {
        return false;
    }

    let Some(artwork) = loading_artwork_stats(frame, bounds) else {
        return false;
    };

    artwork.is_loading() || (artwork.is_early() && is_strong_border(&edges))
}

#[inline]
fn in_range(value: f32, min: f32, max: f32) -> bool {
    value >= min && value <= max
}

/// Collect the four edge samples once so the normal and strong border checks
/// can share them.
fn border_stats(frame: &Frame<'_>, bounds: Bounds) -> [EdgeBorderStats; 4] {
    let frame_width = bounds.right - bounds.left + 1;
    let frame_height = bounds.bottom - bounds.top + 1;
    let inset_x = 1.max((frame_width as f32 * 0.12).round() as usize);
    let inset_y = 1.max((frame_height as f32 * 0.12).round() as usize);

    [
        edge_border_stats(
            frame,
            bounds.left,
            bounds.top + inset_y,
            bounds.left,
            bounds.bottom - inset_y,
            Axis::X,
        ),
        edge_border_stats(
            frame,
            bounds.right,
            bounds.top + inset_y,
            bounds.right,
            bounds.bottom - inset_y,
            Axis::X,
        ),
        edge_border_stats(
            frame,
            bounds.left + inset_x,
            bounds.top,
            bounds.right - inset_x,
            bounds.top,
            Axis::Y,
        ),
        edge_border_stats(
            frame,
            bounds.left + inset_x,
            bounds.bottom,
            bounds.right - inset_x,
            bounds.bottom,
            Axis::Y,
        ),
    ]
}

/// The loading frame has a continuous ornate border. Checking the whole border,
/// rather than a few points, rejects gold-colored gameplay objects and dark UI
/// panels that happen to line up with the frame geometry.
fn is_normal_border(edges: &[EdgeBorderStats; 4]) -> bool {
    if edges
        .iter()
        .any(|edge| edge.coverage < MIN_EDGE_BORDER_COVERAGE)
    {
        return false;
    }

    let coverage = edges.iter().map(|edge| edge.coverage).sum::<f32>() / edges.len() as f32;
    if coverage < MIN_AVERAGE_BORDER_COVERAGE {
        return false;
    }

    let samples: usize = edges.iter().map(|edge| edge.samples).sum();
    if samples == 0 {
        return false;
    }

    let red = edges.iter().map(|edge| edge.red).sum::<i32>() as f32 / samples as f32;
    let green = edges.iter().map(|edge| edge.green).sum::<i32>() as f32 / samples as f32;
    let blue = edges.iter().map(|edge| edge.blue).sum::<i32>() as f32 / samples as f32;

    // The border is desaturated by ffmpeg in places, but the border as a whole
    // is still warm. This rejects white, blue, and green UI frames.
    red - blue >= MIN_BORDER_RED_BLUE_DELTA as f32 && red >= green * 0.95 && green >= blue
}

/// Stricter border check used only for early (dark-interior) door frames.
fn is_strong_border(edges: &[EdgeBorderStats; 4]) -> bool {
    if edges.iter().any(|edge| edge.coverage < 0.9) {
        return false;
    }

    edges.iter().map(|edge| edge.coverage).sum::<f32>() / edges.len() as f32 >= 0.95
}

#[derive(Debug)]
struct EdgeBorderStats {
    coverage: f32,
    samples: usize,
    red: i32,
    green: i32,
    blue: i32,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Axis {
    X,
    Y,
}

fn edge_border_stats(
    frame: &Frame<'_>,
    start_x: usize,
    start_y: usize,
    end_x: usize,
    end_y: usize,
    perpendicular: Axis,
) -> EdgeBorderStats {
    let horizontal = perpendicular == Axis::Y;
    let length = if horizontal {
        end_x.saturating_sub(start_x) + 1
    } else {
        end_y.saturating_sub(start_y) + 1
    };

    if length == 0 {
        return EdgeBorderStats {
            coverage: 0.0,
            samples: 0,
            red: 0,
            green: 0,
            blue: 0,
        };
    }

    let mut matches = 0usize;
    let mut red = 0i32;
    let mut green = 0i32;
    let mut blue = 0i32;

    for index in 0..length {
        let x = if horizontal { start_x + index } else { start_x };
        let y = if horizontal { start_y } else { start_y + index };
        if let Some((r, g, b)) = brightest_border_pixel(frame, x, y, perpendicular) {
            matches += 1;
            red += i32::from(r);
            green += i32::from(g);
            blue += i32::from(b);
        }
    }

    EdgeBorderStats {
        coverage: matches as f32 / length as f32,
        samples: matches,
        red,
        green,
        blue,
    }
}

fn brightest_border_pixel(
    frame: &Frame<'_>,
    x: usize,
    y: usize,
    axis: Axis,
) -> Option<(u8, u8, u8)> {
    let mut best = None;
    let mut best_value = -1i32;

    // The border can move by a pixel after scaling. Search perpendicular to the
    // edge so the strong, warm interior does not count as border coverage.
    for offset in -2i32..=2 {
        let sample_x = if axis == Axis::X {
            (x as i32 + offset).clamp(0, frame.width as i32 - 1) as usize
        } else {
            x
        };
        let sample_y = if axis == Axis::Y {
            (y as i32 + offset).clamp(0, frame.height as i32 - 1) as usize
        } else {
            y
        };

        if !is_border_pixel(frame, sample_x, sample_y) {
            continue;
        }

        let index = pixel_offset(frame, sample_x, sample_y);
        let value = i32::from(
            frame.data[index]
                .max(frame.data[index + 1])
                .max(frame.data[index + 2]),
        );

        if value > best_value {
            best_value = value;
            best = Some((
                frame.data[index],
                frame.data[index + 1],
                frame.data[index + 2],
            ));
        }
    }

    best
}

struct ArtworkStats {
    brightness: f32,
    bright_ratio: f32,
    warm_ratio: f32,
}

impl ArtworkStats {
    /// Both loading artworks contain a large, bright, warm scene. A black or
    /// cold UI panel inside a gold border should not pause the timer.
    fn is_loading(&self) -> bool {
        self.brightness >= 20.0 && self.bright_ratio >= 0.1 && self.warm_ratio >= 0.1
    }

    /// The first door frame is already a complete gold frame with the LOADING
    /// text, but the doorway animation has not lit up yet.
    fn is_early(&self) -> bool {
        self.brightness >= 8.0 && self.bright_ratio >= 0.02 && self.warm_ratio >= 0.01
    }
}

fn loading_artwork_stats(frame: &Frame<'_>, bounds: Bounds) -> Option<ArtworkStats> {
    let frame_width = bounds.right - bounds.left + 1;
    let frame_height = bounds.bottom - bounds.top + 1;
    let inset = 2.max((frame_width.min(frame_height) as f32 * 0.05).round() as usize);
    let start_x = bounds.left + inset;
    let end_x = bounds.right.saturating_sub(inset);
    let start_y = bounds.top + inset;
    let end_y = bounds.bottom.saturating_sub(inset);

    if start_x >= end_x || start_y >= end_y {
        return None;
    }

    let mut brightness = 0u64;
    let mut bright_pixels = 0u64;
    let mut warm_pixels = 0u64;
    let mut pixels = 0u64;

    for y in start_y..=end_y {
        for x in start_x..=end_x {
            let value = max_channel(frame, x, y);
            brightness += u64::from(value);
            pixels += 1;

            if value > BRIGHT_LEVEL {
                bright_pixels += 1;
            }

            if is_gold(frame, x, y) {
                warm_pixels += 1;
            }
        }
    }

    if pixels == 0 {
        return None;
    }

    Some(ArtworkStats {
        brightness: brightness as f32 / pixels as f32,
        bright_ratio: bright_pixels as f32 / pixels as f32,
        warm_ratio: warm_pixels as f32 / pixels as f32,
    })
}

/// Outside the frame the door and legacy loading screens are pure black. This is
/// the strongest in-game guard: normal gameplay and inventory/quest panels leave
/// HUD elements or lit scenery around the screen.
fn has_black_background(frame: &Frame<'_>, content_top: usize, bounds: Bounds) -> bool {
    let inner_height = frame.height.saturating_sub(content_top);
    let padding = 2.max((frame.width.min(frame.height) as f32 * 0.03).round() as usize);
    let scan_top = content_top + (inner_height as f32 * 0.1).round() as usize;
    let left = bounds.left.saturating_sub(padding);
    let right = (bounds.right + padding).min(frame.width - 1);
    let top = bounds.top.saturating_sub(padding);
    let bottom = (bounds.bottom + padding).min(frame.height - 1);

    let total = frame
        .width
        .saturating_mul(frame.height.saturating_sub(scan_top));
    let rect_top = top.max(scan_top);
    let excluded = if rect_top <= bottom && left <= right {
        (bottom - rect_top + 1).saturating_mul(right - left + 1)
    } else {
        0
    };
    let outside = total.saturating_sub(excluded);
    if outside == 0 {
        return false;
    }

    // The background must be at least 98.5% black, so more than 1.5% non-black
    // pixels is already a rejection. Bail out instead of scanning the whole
    // frame; this is the common path for gameplay frames.
    let max_non_black = outside.saturating_mul(MAX_BACKGROUND_NON_BLACK_PER_THOUSAND) / 1000;
    let mut non_black = 0usize;

    for y in scan_top..frame.height {
        if y >= top && y <= bottom {
            if !count_non_black(frame, y, 0, left, &mut non_black, max_non_black) {
                return false;
            }
            if !count_non_black(
                frame,
                y,
                right + 1,
                frame.width,
                &mut non_black,
                max_non_black,
            ) {
                return false;
            }
        } else if !count_non_black(frame, y, 0, frame.width, &mut non_black, max_non_black) {
            return false;
        }
    }

    true
}

#[inline]
fn count_non_black(
    frame: &Frame<'_>,
    y: usize,
    start_x: usize,
    end_x: usize,
    non_black: &mut usize,
    max_non_black: usize,
) -> bool {
    for x in start_x..end_x {
        if !is_black(frame, x, y) {
            *non_black += 1;
            if *non_black > max_non_black {
                return false;
            }
        }
    }
    true
}

/// The waypoint/area logo screen is an almost completely black frame with the
/// fiery D2R logo in the bottom-right corner. Position, size, and warm colors are
/// all checked so a bright HUD element or a dark in-game scene cannot match.
fn is_logo_loading(frame: &Frame<'_>, content_top: usize) -> bool {
    let inner_height = frame.height.saturating_sub(content_top);
    if inner_height < 16 {
        return false;
    }

    // Skip the titlebar area in windowed mode. The logo sits far below it.
    let scan_top = content_top + (inner_height as f32 * 0.1).round() as usize;
    let Some(scan) = scan_non_black(frame, scan_top) else {
        return false;
    };

    if scan.non_black < 8 {
        return false;
    }

    let bounds = scan.bounds;
    let frame_width = bounds.right - bounds.left + 1;
    let frame_height = bounds.bottom - bounds.top + 1;
    let aspect = frame_width as f32 / frame_height as f32;
    let left_ratio = bounds.left as f32 / frame.width as f32;
    let right_ratio = (bounds.right + 1) as f32 / frame.width as f32;
    let top_ratio = (bounds.top - content_top) as f32 / inner_height as f32;
    let bottom_ratio = (bounds.bottom + 1 - content_top) as f32 / inner_height as f32;
    let width_ratio = frame_width as f32 / frame.width as f32;
    let height_ratio = frame_height as f32 / inner_height as f32;

    if !in_range(left_ratio, 0.86, 0.935)
        || !in_range(right_ratio, 0.95, 0.99)
        || !in_range(top_ratio, 0.78, 0.87)
        || !in_range(bottom_ratio, 0.87, 0.94)
        || !in_range(width_ratio, 0.05, 0.095)
        || !in_range(height_ratio, 0.05, 0.11)
        || !in_range(aspect, 1.1, 2.7)
    {
        return false;
    }

    let mut bright_pixels = 0u64;
    let mut warm_pixels = 0u64;
    let mut pixels = 0u64;

    for y in bounds.top..=bounds.bottom {
        for x in bounds.left..=bounds.right {
            let value = max_channel(frame, x, y);
            pixels += 1;

            if value > BRIGHT_LEVEL {
                bright_pixels += 1;
            }

            if is_gold(frame, x, y) {
                warm_pixels += 1;
            }
        }
    }

    pixels > 0
        && bright_pixels as f32 / pixels as f32 >= 0.2
        && warm_pixels as f32 / pixels as f32 >= 0.25
}

/// Scan the frame below `start_y`. Returns `None` as soon as the non-black
/// ratio exceeds the logo screen's allowance; gameplay frames reject after a
/// small fraction of the scan instead of reading the whole frame.
fn scan_non_black(frame: &Frame<'_>, start_y: usize) -> Option<NonBlackScan> {
    let total = frame
        .width
        .saturating_mul(frame.height.saturating_sub(start_y));
    if total == 0 {
        return None;
    }
    let max_non_black = total.saturating_mul(MAX_LOGO_NON_BLACK_PERCENT) / 100;

    let mut non_black = 0usize;
    let mut left = frame.width;
    let mut top = frame.height;
    let mut right = 0usize;
    let mut bottom = 0usize;

    for y in start_y..frame.height {
        for x in 0..frame.width {
            if is_content(frame, x, y) {
                non_black += 1;
                if non_black > max_non_black {
                    return None;
                }
                left = left.min(x);
                top = top.min(y);
                right = right.max(x);
                bottom = bottom.max(y);
            }
        }
    }

    Some(NonBlackScan {
        non_black,
        bounds: Bounds {
            left,
            top,
            right,
            bottom,
        },
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    use std::path::PathBuf;

    /// Fixtures are normalized to a 160-pixel long side before detection.
    const FIXTURE_SIZE: usize = 160;

    struct RgbaImage {
        data: Vec<u8>,
        width: usize,
        height: usize,
    }

    fn fixture_path(name: &str) -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("test")
            .join(name)
    }

    fn load_png(name: &str, target_width: usize, target_height: usize) -> RgbaImage {
        let path = fixture_path(name);
        let file = std::fs::File::open(&path)
            .unwrap_or_else(|error| panic!("open {}: {error}", path.display()));
        let decoder = png::Decoder::new(std::io::BufReader::new(file));
        let mut reader = decoder.read_info().expect("read png info");
        let mut buffer = vec![0; reader.output_buffer_size().expect("PNG output buffer size")];
        let info = reader.next_frame(&mut buffer).expect("decode png");

        let source = match info.color_type {
            png::ColorType::Rgba => RgbaImage {
                data: buffer[..info.buffer_size()].to_vec(),
                width: info.width as usize,
                height: info.height as usize,
            },
            png::ColorType::Rgb => {
                let pixels = info.width as usize * info.height as usize;
                let mut data = Vec::with_capacity(pixels * 4);
                for index in 0..pixels {
                    data.extend_from_slice(&buffer[index * 3..index * 3 + 3]);
                    data.push(255);
                }
                RgbaImage {
                    data,
                    width: info.width as usize,
                    height: info.height as usize,
                }
            }
            other => panic!("unsupported PNG color type {other:?}"),
        };

        resize_rgba(&source, target_width, target_height)
    }

    /// Bilinear resize to fit inside the requested box, preserving aspect.
    fn resize_rgba(source: &RgbaImage, box_width: usize, box_height: usize) -> RgbaImage {
        let scale_x = box_width as f64 / source.width as f64;
        let scale_y = box_height as f64 / source.height as f64;
        let scale = scale_x.min(scale_y);
        let width = ((source.width as f64 * scale).round() as usize).max(1);
        let height = ((source.height as f64 * scale).round() as usize).max(1);
        let mut out = vec![0u8; width * height * 4];

        let x_ratio = source.width as f64 / width as f64;
        let y_ratio = source.height as f64 / height as f64;

        for y in 0..height {
            let source_y =
                ((y as f64 + 0.5) * y_ratio - 0.5).clamp(0.0, source.height as f64 - 1.0);
            let y0 = source_y.floor() as usize;
            let y1 = (y0 + 1).min(source.height - 1);
            let fy = source_y - y0 as f64;

            for x in 0..width {
                let source_x =
                    ((x as f64 + 0.5) * x_ratio - 0.5).clamp(0.0, source.width as f64 - 1.0);
                let x0 = source_x.floor() as usize;
                let x1 = (x0 + 1).min(source.width - 1);
                let fx = source_x - x0 as f64;

                for channel in 0..4 {
                    let p00 = f64::from(source.data[(y0 * source.width + x0) * 4 + channel]);
                    let p10 = f64::from(source.data[(y0 * source.width + x1) * 4 + channel]);
                    let p01 = f64::from(source.data[(y1 * source.width + x0) * 4 + channel]);
                    let p11 = f64::from(source.data[(y1 * source.width + x1) * 4 + channel]);
                    let top = p00 + (p10 - p00) * fx;
                    let bottom = p01 + (p11 - p01) * fx;
                    let value = top + (bottom - top) * fy;
                    out[(y * width + x) * 4 + channel] = value.round().clamp(0.0, 255.0) as u8;
                }
            }
        }

        RgbaImage {
            data: out,
            width,
            height,
        }
    }

    fn expect_loading(name: &str, expected: bool) {
        let image = load_png(name, FIXTURE_SIZE, FIXTURE_SIZE);
        let actual = detect_loading(&image.data, image.width, image.height);
        assert_eq!(
            actual,
            expected,
            "{name} should {} loading",
            if expected { "be" } else { "not be" }
        );
    }

    fn make_black_frame(width: usize, height: usize) -> Vec<u8> {
        let mut frame = vec![0u8; width * height * 4];
        for pixel in frame.chunks_exact_mut(4) {
            pixel[3] = 255;
        }
        frame
    }

    #[allow(clippy::too_many_arguments)]
    fn paint_rect(
        frame: &mut [u8],
        width: usize,
        x0: usize,
        y0: usize,
        x1: usize,
        y1: usize,
        red: u8,
        green: u8,
        blue: u8,
    ) {
        for y in y0..=y1 {
            for x in x0..=x1 {
                let index = (y * width + x) * 4;
                frame[index] = red;
                frame[index + 1] = green;
                frame[index + 2] = blue;
                frame[index + 3] = 255;
            }
        }
    }

    #[test]
    fn detects_door_at_1280x720() {
        expect_loading("1280x720-door.png", true);
    }

    #[test]
    fn detects_logo_at_1280x720() {
        expect_loading("1280x720-logo.png", true);
    }

    #[test]
    fn detects_legacy_at_1280x720() {
        expect_loading("1280x720-legacy.png", true);
    }

    #[test]
    fn rejects_dark_gameplay_at_1280x720() {
        expect_loading("1280x720-dark.png", false);
    }

    #[test]
    fn detects_door_with_screen_space_enabled() {
        expect_loading("2560x1440-4-tick-screen-space-door.png", true);
    }

    #[test]
    fn detects_logo_with_screen_space_enabled() {
        expect_loading("2560x1440-4-tick-screen-space-black-logo.png", true);
    }

    #[test]
    fn rejects_dark_area_with_screen_space_enabled() {
        expect_loading("2560x1440-4-tick-screen-space-dark-area.png", false);
    }

    #[test]
    fn detects_door_at_2560x1440() {
        expect_loading("2560x1440-door.png", true);
    }

    #[test]
    fn detects_logo_at_2560x1440() {
        expect_loading("2560x1440-logo.png", true);
    }

    #[test]
    fn detects_door_at_3840x2160() {
        expect_loading("3840x2160-door.png", true);
    }

    #[test]
    fn detects_legacy_at_3840x2160() {
        expect_loading("3840x2160-legacy.png", true);
    }

    #[test]
    fn detects_logo_at_3840x2160() {
        expect_loading("3840x2160-logo.png", true);
    }

    #[test]
    fn detects_closed_door_at_3840x2160() {
        // This is the first frame of the door sequence: the ornate frame and
        // LOADING text are visible, but the animated doorway is still dark.
        expect_loading("3840x2160-door-closed.png", true);
    }

    #[test]
    fn supported_resolution_and_aspect_matrix() {
        // The 1920x1080 fixtures are resizes of the 4K captures. The 16:10 and
        // 4:3 fixtures are derived from the 16:9 captures: door/legacy/dark use
        // a centered letterbox, while the logo variant keeps the logo anchored
        // at the same bottom-right position. This exercises the relative geometry
        // checks rather than native 4:3/16:10 game renders.
        let cases = [
            ("1920x1080-door.png", true),
            ("1920x1080-door-closed.png", true),
            ("1920x1080-legacy.png", true),
            ("1920x1080-logo.png", true),
            ("1920x1080-dark.png", false),
            ("1920x1200-door.png", true),
            ("1920x1200-door-closed.png", true),
            ("1920x1200-legacy.png", true),
            ("1920x1200-logo.png", true),
            ("1920x1200-dark.png", false),
            ("2048x1536-door.png", true),
            ("2048x1536-door-closed.png", true),
            ("2048x1536-legacy.png", true),
            ("2048x1536-logo.png", true),
            ("2048x1536-dark.png", false),
        ];

        for (name, expected) in cases {
            expect_loading(name, expected);
        }
    }

    #[test]
    fn detects_real_ffmpeg_fast_bilinear_output() {
        let loading = [
            "ffmpeg-160x90-door.rgb",
            "ffmpeg-160x90-logo.rgb",
            "ffmpeg-160x90-legacy.rgb",
            "ffmpeg-160x90-screen-space-door.rgb",
        ];
        let gameplay = ["ffmpeg-160x90-dark.rgb", "ffmpeg-160x90-dark-area.rgb"];

        for name in loading {
            let data = std::fs::read(fixture_path(name)).expect("read fixture");
            assert_eq!(detect_loading_stride(&data, 160, 90, 3), true, "{name}");
        }

        for name in gameplay {
            let data = std::fs::read(fixture_path(name)).expect("read fixture");
            assert_eq!(detect_loading_stride(&data, 160, 90, 3), false, "{name}");
        }
    }

    #[test]
    fn works_with_rgb24_frames() {
        let image = load_png("1280x720-door.png", FIXTURE_SIZE, FIXTURE_SIZE);
        let mut rgb24 = Vec::with_capacity(image.width * image.height * 3);
        for pixel in image.data.chunks_exact(4) {
            rgb24.extend_from_slice(&pixel[..3]);
        }
        assert!(detect_loading_stride(&rgb24, image.width, image.height, 3));
    }

    #[test]
    fn ignores_the_animated_center_of_a_door_loading_frame() {
        let image = load_png("1280x720-door.png", FIXTURE_SIZE, FIXTURE_SIZE);
        let mut frame = image.data.clone();
        let mut seed = 12345u32;
        let mut random = || {
            seed = seed.wrapping_mul(1664525).wrapping_add(1013904223);
            (seed & 255) as u8
        };

        // At 160x90 the door frame is roughly x=46..113, y=22..67. Replace only
        // the interior, leaving the golden border untouched.
        for y in 27..=62 {
            for x in 52..=107 {
                let index = (y * image.width + x) * 4;
                frame[index] = random();
                frame[index + 1] = random();
                frame[index + 2] = random();
            }
        }

        assert!(detect_loading(&frame, image.width, image.height));
    }

    #[test]
    fn rejects_a_framed_image_without_a_golden_border() {
        let image = load_png("1280x720-door.png", FIXTURE_SIZE, FIXTURE_SIZE);
        let mut frame = image.data.clone();
        let width = image.width;

        // Bright enough for the frame scan, but blue instead of gold.
        paint_rect(&mut frame, width, 44, 20, 48, 69, 60, 60, 255);
        paint_rect(&mut frame, width, 111, 20, 115, 69, 60, 60, 255);
        paint_rect(&mut frame, width, 44, 20, 115, 24, 60, 60, 255);
        paint_rect(&mut frame, width, 44, 65, 115, 69, 60, 60, 255);

        assert!(!detect_loading(&frame, image.width, image.height));
    }

    #[test]
    fn rejects_a_framed_image_when_the_outside_is_not_black() {
        let image = load_png("1280x720-door.png", FIXTURE_SIZE, FIXTURE_SIZE);
        let mut frame = image.data.clone();
        let width = image.width;

        paint_rect(&mut frame, width, 40, 42, 44, 46, 80, 80, 80);
        paint_rect(&mut frame, width, 115, 42, 119, 46, 80, 80, 80);
        paint_rect(&mut frame, width, 77, 16, 81, 20, 80, 80, 80);
        paint_rect(&mut frame, width, 77, 69, 81, 73, 80, 80, 80);

        assert!(!detect_loading(&frame, image.width, image.height));
    }

    #[test]
    fn ignores_a_windowed_titlebar() {
        let image = load_png("1280x720-door.png", FIXTURE_SIZE, FIXTURE_SIZE);
        let titlebar = 8;
        let height = image.height + titlebar;
        let mut frame = make_black_frame(image.width, height);

        paint_rect(
            &mut frame,
            image.width,
            0,
            0,
            image.width - 1,
            titlebar - 1,
            60,
            60,
            60,
        );

        for y in 0..image.height {
            for x in 0..image.width {
                let source = (y * image.width + x) * 4;
                let destination = ((y + titlebar) * image.width + x) * 4;
                frame[destination..destination + 4]
                    .copy_from_slice(&image.data[source..source + 4]);
            }
        }

        assert!(detect_loading(&frame, image.width, height));
    }

    #[test]
    fn rejects_a_cold_logo_shaped_patch_on_a_black_frame() {
        let width = 160;
        let height = 90;
        let mut frame = make_black_frame(width, height);
        paint_rect(&mut frame, width, 143, 74, 155, 81, 40, 70, 255);
        assert!(!detect_loading(&frame, width, height));
    }

    #[test]
    fn rejects_a_warm_patch_that_is_not_logo_shaped() {
        let width = 160;
        let height = 90;
        let mut frame = make_black_frame(width, height);
        paint_rect(&mut frame, width, 143, 60, 155, 81, 255, 120, 20);
        assert!(!detect_loading(&frame, width, height));
    }

    #[test]
    fn rejects_a_gold_framed_panel_with_a_dark_interior() {
        let width = 160;
        let height = 90;
        let mut frame = make_black_frame(width, height);
        let gold = (90, 75, 45);

        paint_rect(&mut frame, width, 46, 22, 113, 23, gold.0, gold.1, gold.2);
        paint_rect(&mut frame, width, 46, 66, 113, 67, gold.0, gold.1, gold.2);
        paint_rect(&mut frame, width, 46, 22, 47, 67, gold.0, gold.1, gold.2);
        paint_rect(&mut frame, width, 112, 22, 113, 67, gold.0, gold.1, gold.2);

        assert!(!detect_loading(&frame, width, height));
    }

    #[test]
    fn rejects_an_in_game_frame_with_a_warm_logo_area_patch() {
        let image = load_png("1280x720-dark.png", FIXTURE_SIZE, FIXTURE_SIZE);
        let mut frame = image.data.clone();
        paint_rect(&mut frame, image.width, 143, 74, 155, 81, 255, 120, 20);
        assert!(!detect_loading(&frame, image.width, image.height));
    }

    #[test]
    #[ignore]
    fn debug_metrics() {
        let names = [
            "3840x2160-door.png",
            "3840x2160-door-closed.png",
            "3840x2160-legacy.png",
            "3840x2160-logo.png",
            "1920x1200-door.png",
            "1920x1200-door-closed.png",
            "1920x1200-legacy.png",
            "1920x1200-logo.png",
            "2048x1536-door.png",
            "2048x1536-door-closed.png",
            "2048x1536-legacy.png",
            "2048x1536-logo.png",
            "1280x720-dark.png",
            "1920x1200-dark.png",
            "2048x1536-dark.png",
            "2560x1440-4-tick-screen-space-dark-area.png",
        ];

        for name in names {
            let image = load_png(name, FIXTURE_SIZE, FIXTURE_SIZE);
            let frame = Frame {
                data: &image.data,
                stride: 4,
                width: image.width,
                height: image.height,
            };
            let content_top = find_content_top(&frame);
            let bounds = find_center_frame(&frame, content_top);
            println!(
                "\n{name}: {}x{} top={content_top} bounds={bounds:?} logo={}",
                image.width,
                image.height,
                is_logo_loading(&frame, content_top)
            );

            if bounds.is_none() && name.contains("logo") {
                let inner_height = frame.height - content_top;
                let scan_top = content_top + (inner_height as f32 * 0.1).round() as usize;
                let scan = match scan_non_black(&frame, scan_top) {
                    Some(scan) => scan,
                    None => {
                        println!("  logo scan rejected: too much non-black content");
                        continue;
                    }
                };
                let total = frame.width * (frame.height - scan_top);
                println!(
                    "  logo scan nonblack={} total={} ratio={:.4} bounds={:?}",
                    scan.non_black,
                    total,
                    scan.non_black as f32 / total.max(1) as f32,
                    scan.bounds
                );
                let bounds = scan.bounds;
                let mut bright = 0u64;
                let mut warm = 0u64;
                let mut pixels = 0u64;
                for y in bounds.top..=bounds.bottom {
                    for x in bounds.left..=bounds.right {
                        let value = max_channel(&frame, x, y);
                        pixels += 1;
                        if value > BRIGHT_LEVEL {
                            bright += 1;
                        }
                        if is_gold(&frame, x, y) {
                            warm += 1;
                        }
                    }
                }
                println!(
                    "  logo stats bright={:.3} warm={:.3}",
                    bright as f32 / pixels.max(1) as f32,
                    warm as f32 / pixels.max(1) as f32
                );
            }

            if let Some(bounds) = bounds {
                let frame_width = bounds.right - bounds.left + 1;
                let frame_height = bounds.bottom - bounds.top + 1;
                let inner_height = frame.height - content_top;
                let inset = 2.max((frame_width.min(frame_height) as f32 * 0.05).round() as usize);
                let start_x = bounds.left + inset;
                let end_x = bounds.right.saturating_sub(inset);
                let start_y = bounds.top + inset;
                let end_y = bounds.bottom.saturating_sub(inset);
                let mut brightness = 0u64;
                let mut bright = 0u64;
                let mut warm = 0u64;
                let mut pixels = 0u64;
                for y in start_y..=end_y {
                    for x in start_x..=end_x {
                        let value = max_channel(&frame, x, y);
                        brightness += u64::from(value);
                        pixels += 1;
                        if value > BRIGHT_LEVEL {
                            bright += 1;
                        }
                        if is_gold(&frame, x, y) {
                            warm += 1;
                        }
                    }
                }

                let mut black = 0u64;
                let mut background = 0u64;
                let padding = 2.max((frame.width.min(frame.height) as f32 * 0.03).round() as usize);
                let scan_top = content_top + (inner_height as f32 * 0.1).round() as usize;
                let left = bounds.left.saturating_sub(padding);
                let right = (bounds.right + padding).min(frame.width - 1);
                let top = bounds.top.saturating_sub(padding);
                let bottom = (bounds.bottom + padding).min(frame.height - 1);
                for y in scan_top..frame.height {
                    for x in 0..frame.width {
                        if x >= left && x <= right && y >= top && y <= bottom {
                            continue;
                        }
                        background += 1;
                        if is_black(&frame, x, y) {
                            black += 1;
                        }
                    }
                }

                let edges = border_stats(&frame, bounds);

                println!(
                    "  aspect={:.2} wr={:.2} hr={:.2} border={} artwork={} bg={}",
                    frame_width as f32 / frame_height as f32,
                    frame_width as f32 / inner_height as f32,
                    frame_height as f32 / inner_height as f32,
                    is_normal_border(&edges),
                    loading_artwork_stats(&frame, bounds).is_some_and(|stats| stats.is_loading()),
                    has_black_background(&frame, content_top, bounds)
                );
                println!(
                    "  brightness={:.1} bright={:.3} warm={:.3} bg_black={:.4} edges={:?}",
                    brightness as f32 / pixels as f32,
                    bright as f32 / pixels as f32,
                    warm as f32 / pixels as f32,
                    black as f32 / background as f32,
                    edges
                        .iter()
                        .map(|edge| (edge.coverage, edge.red, edge.green, edge.blue))
                        .collect::<Vec<_>>()
                );
            }
        }
    }
}
