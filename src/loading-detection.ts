export const MIN_IMAGE_SIZE = 160;

/**
 * Pixels at or below this channel value are treated as background black.
 * Loading screens have a pure black background, so the threshold can stay low
 * enough to also find the dim gold border after downscaling.
 */
const BLACK_LEVEL = 20;

const BRIGHT_LEVEL = 40;

/** Gold needs a clear warm hue, not just a bright pixel. */
const GOLD_MIN_RED = 20;
const GOLD_MIN_RED_BLUE_DELTA = 6;
const GOLD_MIN_GREEN_BLUE_DELTA = 2;
const GOLD_MIN_RED_GREEN_RATIO = 0.75;

/** Per-edge border coverage and the average across all four edges. */
const MIN_EDGE_BORDER_COVERAGE = 0.6;
const MIN_AVERAGE_BORDER_COVERAGE = 0.8;

/** The border as a whole must still look warm/golden, not white or blue. */
const MIN_BORDER_RED_BLUE_DELTA = 3;

/** The loading screen must be black outside the frame (titlebar excluded). */
const MIN_BACKGROUND_BLACK_RATIO = 0.985;

/** The bottom-right logo screen is almost entirely black. */
const MAX_LOGO_NON_BLACK_RATIO = 0.02;

export interface PixelData {
  data: Uint8ClampedArray | Uint8Array;
  /** Bytes per pixel. Defaults to 4 (RGBA). */
  stride?: number;
}

export interface PixelSource {
  getImageData(
    sx: number,
    sy: number,
    sw: number,
    sh: number
  ): PixelData;
}

/**
 * Adapter that lets the detector work on raw RGB/RGBA frames coming from
 * ffmpeg instead of a browser CanvasRenderingContext2D.
 */
export class RawFrameSource implements PixelSource {
  constructor(
    private readonly frame: Uint8Array,
    private readonly width: number,
    private readonly height: number,
    private readonly pixelStride = 4
  ) {}

  getImageData(sx: number, sy: number, sw: number, sh: number): PixelData {
    // The detector always reads the complete frame. Returning the existing
    // buffer avoids a per-frame allocation and copy.
    if (
      sx === 0 &&
      sy === 0 &&
      sw === this.width &&
      sh === this.height
    ) {
      return { data: this.frame, stride: this.pixelStride };
    }

    const data = new Uint8ClampedArray(sw * sh * this.pixelStride);
    const startX = Math.max(0, Math.min(this.width, sx));
    const copyWidth = Math.max(0, Math.min(sw, this.width - startX));

    if (copyWidth === 0) {
      return { data, stride: this.pixelStride };
    }

    for (let y = 0; y < sh; ++y) {
      const sourceY = sy + y;

      if (sourceY < 0 || sourceY >= this.height) {
        continue;
      }

      const sourceStart =
        (sourceY * this.width + startX) * this.pixelStride;
      const destinationStart =
        (y * sw + (startX - sx)) * this.pixelStride;

      data.set(
        this.frame.subarray(
          sourceStart,
          sourceStart + copyWidth * this.pixelStride
        ),
        destinationStart
      );
    }

    return { data, stride: this.pixelStride };
  }
}

interface Frame {
  data: Uint8ClampedArray | Uint8Array;
  stride: number;
  width: number;
  height: number;
}

interface Bounds {
  left: number;
  top: number;
  right: number;
  bottom: number;
}

interface NonBlackScan {
  nonBlack: number;
  total: number;
  bounds: Bounds;
}

export function detectLoading(
  source: PixelSource,
  width: number,
  height: number
): boolean {
  if (width < 16 || height < 16) {
    return false;
  }

  // Read the frame once and run every check on the same snapshot instead of
  // issuing many small getImageData calls.
  const pixelData = source.getImageData(0, 0, width, height);
  const stride = pixelData.stride ?? 4;

  if (stride < 3 || pixelData.data.length < width * height * stride) {
    return false;
  }

  const frame: Frame = {
    data: pixelData.data,
    stride,
    width,
    height,
  };
  const contentTop = findContentTop(frame);
  const bounds = findCenterFrame(frame, contentTop);

  if (bounds && isCenterLoading(frame, contentTop, bounds)) {
    return true;
  }

  return isLogoLoading(frame, contentTop);
}

function pixelOffset(frame: Frame, x: number, y: number) {
  return (y * frame.width + x) * frame.stride;
}

function maxChannel(frame: Frame, x: number, y: number) {
  const index = pixelOffset(frame, x, y);
  return Math.max(
    frame.data[index],
    frame.data[index + 1],
    frame.data[index + 2]
  );
}

function isContent(frame: Frame, x: number, y: number) {
  return maxChannel(frame, x, y) > BLACK_LEVEL;
}

function isBlack(frame: Frame, x: number, y: number) {
  return maxChannel(frame, x, y) <= BLACK_LEVEL;
}

function isGold(frame: Frame, x: number, y: number) {
  const index = pixelOffset(frame, x, y);
  const red = frame.data[index];
  const green = frame.data[index + 1];
  const blue = frame.data[index + 2];

  return (
    red > GOLD_MIN_RED &&
    red - blue >= GOLD_MIN_RED_BLUE_DELTA &&
    green - blue >= GOLD_MIN_GREEN_BLUE_DELTA &&
    red >= green * GOLD_MIN_RED_GREEN_RATIO
  );
}

/**
 * ffmpeg's fast bilinear scaling can desaturate a thin gold line into gray.
 * Border coverage therefore accepts any bright, non-cold pixel; the average
 * color of the whole border is checked separately below.
 */
function isBorderPixel(frame: Frame, x: number, y: number) {
  const index = pixelOffset(frame, x, y);
  const red = frame.data[index];
  const green = frame.data[index + 1];
  const blue = frame.data[index + 2];

  return red > GOLD_MIN_RED && red >= green * 0.7 && green >= blue * 0.7;
}

/**
 * Finds the top of the game content inside a windowed D2R window. Fullscreen
 * frames start with black rows, so this is normally zero.
 */
function findContentTop(frame: Frame) {
  const x = frame.width >> 1;
  const maxTitlebar = Math.round(frame.height * 0.2);

  for (let y = 0; y < frame.height; ++y) {
    if (isBlack(frame, x, y)) {
      // A dark row far below the top is game scenery, not a window titlebar.
      return y <= maxTitlebar ? y : 0;
    }
  }

  return 0;
}

/**
 * Finds the bright centered rectangle (the door/legacy loading frame).
 *
 * Several scan lines are used instead of one so a cursor, animated element, or
 * compression artifact cannot move a margin. The results must also be stable
 * across the scan lines, which rejects ordinary gameplay scenes quickly.
 */
function findCenterFrame(
  frame: Frame,
  contentTop: number
): Bounds | null {
  const innerHeight = frame.height - contentTop;

  if (innerHeight < 12) {
    return null;
  }

  const tops: number[] = [];
  const bottoms: number[] = [];
  const lefts: number[] = [];
  const rights: number[] = [];

  for (const fraction of [0.44, 0.47, 0.5, 0.53, 0.56]) {
    const x = Math.min(
      frame.width - 1,
      Math.round((frame.width - 1) * fraction)
    );
    const range = scanColumn(frame, x, contentTop);

    if (range) {
      tops.push(range[0]);
      bottoms.push(range[1]);
    }
  }

  for (const fraction of [0.34, 0.42, 0.5, 0.58, 0.66]) {
    const y =
      contentTop +
      Math.min(
        innerHeight - 1,
        Math.round((innerHeight - 1) * fraction)
      );
    const range = scanRow(frame, y);

    if (range) {
      lefts.push(range[0]);
      rights.push(range[1]);
    }
  }

  if (
    tops.length < 3 ||
    lefts.length < 3 ||
    !isStable(tops, Math.max(3, Math.round(innerHeight * 0.12))) ||
    !isStable(bottoms, Math.max(3, Math.round(innerHeight * 0.12))) ||
    !isStable(lefts, Math.max(3, Math.round(frame.width * 0.12))) ||
    !isStable(rights, Math.max(3, Math.round(frame.width * 0.12)))
  ) {
    return null;
  }

  // Use the outermost detected content. Thin, dim frame edges can disappear
  // from one scan line after ffmpeg's bilinear scaling; taking the extreme
  // keeps the complete border inside the bounds.
  const bounds = {
    top: Math.min(...tops),
    bottom: Math.max(...bottoms),
    left: Math.min(...lefts),
    right: Math.max(...rights),
  };

  if (
    bounds.right <= bounds.left ||
    bounds.bottom <= bounds.top
  ) {
    return null;
  }

  return bounds;
}

function scanColumn(
  frame: Frame,
  x: number,
  startY: number
): [number, number] | null {
  let first = -1;
  let last = -1;

  for (let y = startY; y < frame.height; ++y) {
    if (isContent(frame, x, y)) {
      if (first < 0) {
        first = y;
      }
      last = y;
    }
  }

  return first < 0 ? null : [first, last];
}

function scanRow(frame: Frame, y: number): [number, number] | null {
  let first = -1;
  let last = -1;

  for (let x = 0; x < frame.width; ++x) {
    if (isContent(frame, x, y)) {
      if (first < 0) {
        first = x;
      }
      last = x;
    }
  }

  return first < 0 ? null : [first, last];
}

function isStable(values: number[], tolerance: number) {
  let min = values[0];
  let max = values[0];

  for (const value of values) {
    min = Math.min(min, value);
    max = Math.max(max, value);
  }

  return max - min <= tolerance;
}

function isCenterLoading(
  frame: Frame,
  contentTop: number,
  bounds: Bounds
): boolean {
  const frameWidth = bounds.right - bounds.left + 1;
  const frameHeight = bounds.bottom - bounds.top + 1;
  const innerHeight = frame.height - contentTop;

  if (frameWidth < 16 || frameHeight < 16) {
    return false;
  }

  const centerX = (bounds.left + bounds.right + 1) / 2;
  const centerY = (bounds.top + bounds.bottom + 1) / 2;
  const expectedCenterY = contentTop + innerHeight / 2;

  if (
    Math.abs(centerX - frame.width / 2) >
      Math.max(2, frame.width * 0.02) ||
    Math.abs(centerY - expectedCenterY) >
      Math.max(2, innerHeight * 0.03)
  ) {
    return false;
  }

  const aspect = frameWidth / frameHeight;
  const widthRatio = frameWidth / innerHeight;
  const heightRatio = frameHeight / innerHeight;

  // D2R's door loading frame is ~1.5:1. The legacy frame is ~1:1.
  const door =
    inRange(aspect, 1.32, 1.68) &&
    inRange(widthRatio, 0.55, 0.86) &&
    inRange(heightRatio, 0.36, 0.6);
  const legacy =
    inRange(aspect, 0.85, 1.15) &&
    inRange(widthRatio, 0.32, 0.55) &&
    inRange(heightRatio, 0.32, 0.55);

  if (!door && !legacy) {
    return false;
  }

  if (!hasFrameBorder(frame, bounds)) {
    return false;
  }

  if (!hasLoadingArtwork(frame, bounds)) {
    return false;
  }

  return hasBlackBackground(frame, contentTop, bounds);
}

function inRange(value: number, min: number, max: number) {
  return value >= min && value <= max;
}

/**
 * The loading frame has a continuous ornate border. Checking the whole border,
 * rather than a few points, rejects gold-colored gameplay objects and dark UI
 * panels that happen to line up with the frame geometry.
 */
function hasFrameBorder(frame: Frame, bounds: Bounds) {
  const frameWidth = bounds.right - bounds.left + 1;
  const frameHeight = bounds.bottom - bounds.top + 1;
  const insetX = Math.max(1, Math.round(frameWidth * 0.12));
  const insetY = Math.max(1, Math.round(frameHeight * 0.12));

  const edges = [
    edgeBorderStats(
      frame,
      bounds.left,
      bounds.top + insetY,
      bounds.left,
      bounds.bottom - insetY,
      "x"
    ),
    edgeBorderStats(
      frame,
      bounds.right,
      bounds.top + insetY,
      bounds.right,
      bounds.bottom - insetY,
      "x"
    ),
    edgeBorderStats(
      frame,
      bounds.left + insetX,
      bounds.top,
      bounds.right - insetX,
      bounds.top,
      "y"
    ),
    edgeBorderStats(
      frame,
      bounds.left + insetX,
      bounds.bottom,
      bounds.right - insetX,
      bounds.bottom,
      "y"
    ),
  ];

  if (
    edges.some((edge) => edge.coverage < MIN_EDGE_BORDER_COVERAGE)
  ) {
    return false;
  }

  const coverage =
    edges.reduce((sum, edge) => sum + edge.coverage, 0) / edges.length;

  if (coverage < MIN_AVERAGE_BORDER_COVERAGE) {
    return false;
  }

  const samples = edges.reduce((sum, edge) => sum + edge.samples, 0);
  const red = edges.reduce((sum, edge) => sum + edge.red, 0) / samples;
  const green = edges.reduce((sum, edge) => sum + edge.green, 0) / samples;
  const blue = edges.reduce((sum, edge) => sum + edge.blue, 0) / samples;

  // The border is desaturated by ffmpeg in places, but the border as a whole
  // is still warm. This rejects white, blue, and green UI frames.
  return (
    red - blue >= MIN_BORDER_RED_BLUE_DELTA &&
    red >= green * 0.95 &&
    green >= blue
  );
}

interface Rgb {
  red: number;
  green: number;
  blue: number;
}

interface EdgeBorderStats {
  coverage: number;
  samples: number;
  red: number;
  green: number;
  blue: number;
}

function edgeBorderStats(
  frame: Frame,
  startX: number,
  startY: number,
  endX: number,
  endY: number,
  perpendicular: "x" | "y"
): EdgeBorderStats {
  const horizontal = perpendicular === "y";
  const length = horizontal ? endX - startX + 1 : endY - startY + 1;

  if (length <= 0) {
    return {
      coverage: 0,
      samples: 0,
      red: 0,
      green: 0,
      blue: 0,
    };
  }

  let matches = 0;
  let red = 0;
  let green = 0;
  let blue = 0;

  for (let index = 0; index < length; ++index) {
    const x = horizontal ? startX + index : startX;
    const y = horizontal ? startY : startY + index;
    const pixel = brightestBorderPixel(frame, x, y, perpendicular);

    if (pixel) {
      matches += 1;
      red += pixel.red;
      green += pixel.green;
      blue += pixel.blue;
    }
  }

  // Every matched border pixel contributes one sample to the color average.
  if (matches === 0) {
    return {
      coverage: 0,
      samples: 0,
      red: 0,
      green: 0,
      blue: 0,
    };
  }

  return {
    coverage: matches / length,
    samples: matches,
    red,
    green,
    blue,
  };
}

function brightestBorderPixel(
  frame: Frame,
  x: number,
  y: number,
  axis: "x" | "y"
): Rgb | null {
  let best: Rgb | null = null;
  let bestValue = -1;

  // The border can move by a pixel after scaling. Search perpendicular to the
  // edge so the strong, warm interior does not count as border coverage.
  for (let offset = -2; offset <= 2; ++offset) {
    const sampleX =
      axis === "x"
        ? Math.max(0, Math.min(frame.width - 1, x + offset))
        : x;
    const sampleY =
      axis === "y"
        ? Math.max(0, Math.min(frame.height - 1, y + offset))
        : y;

    if (!isBorderPixel(frame, sampleX, sampleY)) {
      continue;
    }

    const index = pixelOffset(frame, sampleX, sampleY);
    const value = Math.max(
      frame.data[index],
      frame.data[index + 1],
      frame.data[index + 2]
    );

    if (value > bestValue) {
      bestValue = value;
      best = {
        red: frame.data[index],
        green: frame.data[index + 1],
        blue: frame.data[index + 2],
      };
    }
  }

  return best;
}

function hasLoadingArtwork(frame: Frame, bounds: Bounds) {
  const frameWidth = bounds.right - bounds.left + 1;
  const frameHeight = bounds.bottom - bounds.top + 1;
  const inset = Math.max(
    2,
    Math.round(Math.min(frameWidth, frameHeight) * 0.05)
  );
  const startX = bounds.left + inset;
  const endX = bounds.right - inset;
  const startY = bounds.top + inset;
  const endY = bounds.bottom - inset;

  if (startX >= endX || startY >= endY) {
    return false;
  }

  let brightness = 0;
  let brightPixels = 0;
  let warmPixels = 0;
  let pixels = 0;

  for (let y = startY; y <= endY; ++y) {
    for (let x = startX; x <= endX; ++x) {
      const value = maxChannel(frame, x, y);
      brightness += value;
      pixels += 1;

      if (value > BRIGHT_LEVEL) {
        brightPixels += 1;
      }

      if (isGold(frame, x, y)) {
        warmPixels += 1;
      }
    }
  }

  // Both loading artworks contain a large, bright, warm scene. A black or
  // cold UI panel inside a gold border should not pause the timer.
  return (
    brightness / pixels >= 20 &&
    brightPixels / pixels >= 0.1 &&
    warmPixels / pixels >= 0.1
  );
}

/**
 * Outside the frame the door and legacy loading screens are pure black. This is
 * the strongest in-game guard: normal gameplay and inventory/quest panels leave
 * HUD elements or lit scenery around the screen.
 */
function hasBlackBackground(
  frame: Frame,
  contentTop: number,
  bounds: Bounds
) {
  const innerHeight = frame.height - contentTop;
  const padding = Math.max(
    2,
    Math.round(Math.min(frame.width, frame.height) * 0.03)
  );
  const scanTop = contentTop + Math.round(innerHeight * 0.1);
  const left = Math.max(0, bounds.left - padding);
  const right = Math.min(frame.width - 1, bounds.right + padding);
  const top = Math.max(0, bounds.top - padding);
  const bottom = Math.min(frame.height - 1, bounds.bottom + padding);

  let blackPixels = 0;
  let pixels = 0;

  for (let y = scanTop; y < frame.height; ++y) {
    for (let x = 0; x < frame.width; ++x) {
      if (x >= left && x <= right && y >= top && y <= bottom) {
        continue;
      }

      pixels += 1;

      if (isBlack(frame, x, y)) {
        blackPixels += 1;
      }
    }
  }

  return (
    pixels > 0 &&
    blackPixels / pixels >= MIN_BACKGROUND_BLACK_RATIO
  );
}

/**
 * The waypoint/area logo screen is an almost completely black frame with the
 * fiery D2R logo in the bottom-right corner. Position, size, and warm colors are
 * all checked so a bright HUD element or a dark in-game scene cannot match.
 */
function isLogoLoading(frame: Frame, contentTop: number) {
  const innerHeight = frame.height - contentTop;

  if (innerHeight < 16) {
    return false;
  }

  // Skip the titlebar area in windowed mode. The logo sits far below it.
  const scanTop = contentTop + Math.round(innerHeight * 0.1);
  const scan = scanNonBlack(frame, scanTop);

  if (
    scan.nonBlack < 8 ||
    scan.nonBlack / scan.total > MAX_LOGO_NON_BLACK_RATIO
  ) {
    return false;
  }

  const { bounds } = scan;
  const frameWidth = bounds.right - bounds.left + 1;
  const frameHeight = bounds.bottom - bounds.top + 1;
  const aspect = frameWidth / frameHeight;
  const leftRatio = bounds.left / frame.width;
  const rightRatio = (bounds.right + 1) / frame.width;
  const topRatio = (bounds.top - contentTop) / innerHeight;
  const bottomRatio = (bounds.bottom + 1 - contentTop) / innerHeight;
  const widthRatio = frameWidth / frame.width;
  const heightRatio = frameHeight / innerHeight;

  if (
    !inRange(leftRatio, 0.86, 0.935) ||
    !inRange(rightRatio, 0.95, 0.99) ||
    !inRange(topRatio, 0.78, 0.87) ||
    !inRange(bottomRatio, 0.87, 0.94) ||
    !inRange(widthRatio, 0.05, 0.095) ||
    !inRange(heightRatio, 0.05, 0.11) ||
    !inRange(aspect, 1.1, 2.7)
  ) {
    return false;
  }

  let brightPixels = 0;
  let warmPixels = 0;
  let pixels = 0;

  for (let y = bounds.top; y <= bounds.bottom; ++y) {
    for (let x = bounds.left; x <= bounds.right; ++x) {
      const value = maxChannel(frame, x, y);
      pixels += 1;

      if (value > BRIGHT_LEVEL) {
        brightPixels += 1;
      }

      if (isGold(frame, x, y)) {
        warmPixels += 1;
      }
    }
  }

  return (
    brightPixels / pixels >= 0.2 &&
    warmPixels / pixels >= 0.25
  );
}

function scanNonBlack(frame: Frame, startY: number): NonBlackScan {
  let nonBlack = 0;
  let total = 0;
  let left = frame.width;
  let top = frame.height;
  let right = -1;
  let bottom = -1;

  for (let y = startY; y < frame.height; ++y) {
    for (let x = 0; x < frame.width; ++x) {
      total += 1;

      if (isContent(frame, x, y)) {
        nonBlack += 1;
        left = Math.min(left, x);
        top = Math.min(top, y);
        right = Math.max(right, x);
        bottom = Math.max(bottom, y);
      }
    }
  }

  return {
    nonBlack,
    total,
    bounds: { left, top, right, bottom },
  };
}
