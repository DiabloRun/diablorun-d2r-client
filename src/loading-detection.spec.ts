import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { describe, it } from "node:test";
import sharp from "sharp";
import {
  detectLoading,
  MIN_IMAGE_SIZE,
  PixelSource,
  RawFrameSource,
} from "./loading-detection";

class BufferPixelSource implements PixelSource {
  private readonly data: Uint8ClampedArray;

  constructor(buffer: Buffer, private readonly width: number) {
    this.data = Uint8ClampedArray.from(buffer);
  }

  getImageData(sx: number, sy: number, sw: number, sh: number) {
    const data = new Uint8ClampedArray(sw * sh * 4);
    let index = 0;

    for (let y = sy; y < sy + sh; ++y) {
      for (let x = sx; x < sx + sw; ++x) {
        data.set(
          this.data.subarray(
            (y * this.width + x) * 4,
            (y * this.width + x) * 4 + 4
          ),
          index
        );

        index += 4;
      }
    }

    return { data };
  }
}

async function loadImage(filename: string) {
  const image = sharp(filename).resize(MIN_IMAGE_SIZE, MIN_IMAGE_SIZE, {
    fit: "inside",
  });

  return image.raw().toBuffer({ resolveWithObject: true });
}

function setPixel(
  buffer: Buffer,
  width: number,
  x: number,
  y: number,
  red: number,
  green: number,
  blue: number
) {
  const index = (y * width + x) * 4;
  buffer[index] = red;
  buffer[index + 1] = green;
  buffer[index + 2] = blue;
  buffer[index + 3] = 255;
}

async function expectLoading(filename: string, expectedValue = true) {
  const { data, info } = await loadImage(filename);
  const source = new BufferPixelSource(data, info.width);
  const isLoading = detectLoading(source, info.width, info.height);

  assert.equal(
    isLoading,
    expectedValue,
    `${filename} should ${expectedValue ? "be" : "not be"} loading`
  );
}

function makeBlackFrame(width: number, height: number) {
  const frame = Buffer.alloc(width * height * 4);

  for (let i = 0; i < frame.length; i += 4) {
    frame[i + 3] = 255;
  }

  return frame;
}

function paintRect(
  frame: Buffer,
  width: number,
  x0: number,
  y0: number,
  x1: number,
  y1: number,
  red: number,
  green: number,
  blue: number
) {
  for (let y = y0; y <= y1; ++y) {
    for (let x = x0; x <= x1; ++x) {
      setPixel(frame, width, x, y, red, green, blue);
    }
  }
}

class CountingPixelSource implements PixelSource {
  calls = 0;
  bytes = 0;

  constructor(private readonly inner: PixelSource) {}

  getImageData(sx: number, sy: number, sw: number, sh: number) {
    this.calls += 1;
    const result = this.inner.getImageData(sx, sy, sw, sh);
    this.bytes += result.data.length;
    return result;
  }
}

describe("Loading detection", () => {
  // 1280x720
  it("should be door loading with 1280x720", async () => {
    await expectLoading("test/1280x720-door.png");
  });

  it("should be logo loading with 1280x720", async () => {
    await expectLoading("test/1280x720-logo.png");
  });

  it("should be legacy loading with 1280x720", async () => {
    await expectLoading("test/1280x720-legacy.png");
  });

  it("should not be loading with 1280x720", async () => {
    await expectLoading("test/1280x720-dark.png", false);
  });

  // Screen space option ticked
  it("should be door loading with screen space ticked", async () => {
    await expectLoading("test/2560x1440-4-tick-screen-space-door.png");
  });

  it("should be logo loading with screen space ticked", async () => {
    await expectLoading(
      "test/2560x1440-4-tick-screen-space-black-logo.png"
    );
  });

  it("should not be loading with screen space ticked", async () => {
    await expectLoading(
      "test/2560x1440-4-tick-screen-space-dark-area.png",
      false
    );
  });

  it("should be door loading at 2560x1440", async () => {
    await expectLoading("test/2560x1440-door.png");
  });

  it("should be logo loading at 2560x1440", async () => {
    await expectLoading("test/2560x1440-logo.png");
  });

  it("should work through the raw frame adapter", async () => {
    const { data, info } = await loadImage("test/1280x720-door.png");
    const source = new RawFrameSource(data, info.width, info.height);

    assert.equal(detectLoading(source, info.width, info.height), true);
  });

  it("should work with RGB24 frames", async () => {
    const { data, info } = await loadImage("test/1280x720-door.png");
    const rgb24 = Buffer.alloc(info.width * info.height * 3);

    for (
      let sourceIndex = 0, destination = 0;
      sourceIndex < data.length;
      sourceIndex += 4, destination += 3
    ) {
      rgb24[destination] = data[sourceIndex];
      rgb24[destination + 1] = data[sourceIndex + 1];
      rgb24[destination + 2] = data[sourceIndex + 2];
    }

    const source = new RawFrameSource(
      rgb24,
      info.width,
      info.height,
      3
    );

    assert.equal(detectLoading(source, info.width, info.height), true);
  });

  it("should detect real ffmpeg fast_bilinear output", () => {
    // Captured at 160x90 with:
    // ffmpeg -vf scale=160:90:flags=fast_bilinear -pix_fmt rgb24
    // Bilinear scaling dims and desaturates the thin frame border, so these
    // fixtures guard against over-tight color/aspect thresholds.
    const loading = [
      "test/ffmpeg-160x90-door.rgb",
      "test/ffmpeg-160x90-logo.rgb",
      "test/ffmpeg-160x90-legacy.rgb",
      "test/ffmpeg-160x90-screen-space-door.rgb",
    ];
    const gameplay = [
      "test/ffmpeg-160x90-dark.rgb",
      "test/ffmpeg-160x90-dark-area.rgb",
    ];

    for (const filename of loading) {
      const data = readFileSync(filename);
      const source = new RawFrameSource(data, 160, 90, 3);

      assert.equal(detectLoading(source, 160, 90), true, filename);
    }

    for (const filename of gameplay) {
      const data = readFileSync(filename);
      const source = new RawFrameSource(data, 160, 90, 3);

      assert.equal(detectLoading(source, 160, 90), false, filename);
    }
  });

  it("should ignore the animated center of a door loading frame", async () => {
    const { data, info } = await loadImage("test/1280x720-door.png");
    const frame = Buffer.from(data);

    let seed = 12345;
    const random = () => {
      seed = (seed * 1664525 + 1013904223) >>> 0;
      return seed & 255;
    };

    // The door frame is roughly x=46..113, y=22..67 at 160x90. Replace only
    // the interior, leaving the golden border untouched.
    for (let y = 27; y <= 62; ++y) {
      for (let x = 52; x <= 107; ++x) {
        setPixel(frame, info.width, x, y, random(), random(), random());
      }
    }

    const source = new RawFrameSource(frame, info.width, info.height);
    assert.equal(detectLoading(source, info.width, info.height), true);
  });

  it("should reject a framed image without a golden border", async () => {
    const { data, info } = await loadImage("test/1280x720-door.png");
    const frame = Buffer.from(data);

    // Bright enough for the frame scan, but blue instead of gold.
    paintRect(frame, info.width, 44, 20, 48, 69, 60, 60, 255);
    paintRect(frame, info.width, 111, 20, 115, 69, 60, 60, 255);
    paintRect(frame, info.width, 44, 20, 115, 24, 60, 60, 255);
    paintRect(frame, info.width, 44, 65, 115, 69, 60, 60, 255);

    const source = new RawFrameSource(frame, info.width, info.height);
    assert.equal(detectLoading(source, info.width, info.height), false);
  });

  it("should reject a framed image when the outside is not black", async () => {
    const { data, info } = await loadImage("test/1280x720-door.png");
    const frame = Buffer.from(data);

    paintRect(frame, info.width, 40, 42, 44, 46, 80, 80, 80);
    paintRect(frame, info.width, 115, 42, 119, 46, 80, 80, 80);
    paintRect(frame, info.width, 77, 16, 81, 20, 80, 80, 80);
    paintRect(frame, info.width, 77, 69, 81, 73, 80, 80, 80);

    const source = new RawFrameSource(frame, info.width, info.height);
    assert.equal(detectLoading(source, info.width, info.height), false);
  });

  it("should ignore a windowed titlebar", async () => {
    const { data, info } = await loadImage("test/1280x720-door.png");
    const titlebar = 8;
    const height = info.height + titlebar;
    const frame = makeBlackFrame(info.width, height);

    paintRect(
      frame,
      info.width,
      0,
      0,
      info.width - 1,
      titlebar - 1,
      60,
      60,
      60
    );

    for (let y = 0; y < info.height; ++y) {
      for (let x = 0; x < info.width; ++x) {
        const index = (y * info.width + x) * 4;
        setPixel(
          frame,
          info.width,
          x,
          y + titlebar,
          data[index],
          data[index + 1],
          data[index + 2]
        );
      }
    }

    const source = new RawFrameSource(frame, info.width, height);
    assert.equal(detectLoading(source, info.width, height), true);
  });

  it("should read the frame only once", async () => {
    const cases = [
      { filename: "test/1280x720-door.png", expected: true },
      { filename: "test/1280x720-dark.png", expected: false },
    ];

    for (const { filename, expected } of cases) {
      const { data, info } = await loadImage(filename);
      const source = new CountingPixelSource(
        new RawFrameSource(data, info.width, info.height)
      );

      assert.equal(
        detectLoading(source, info.width, info.height),
        expected
      );
      assert.equal(source.calls, 1);
      assert.equal(source.bytes, info.width * info.height * 4);
    }
  });

  it("should reject a cold logo-shaped patch on a black frame", () => {
    const width = 160;
    const height = 90;
    const frame = makeBlackFrame(width, height);

    paintRect(frame, width, 143, 74, 155, 81, 40, 70, 255);

    const source = new RawFrameSource(frame, width, height);
    assert.equal(detectLoading(source, width, height), false);
  });

  it("should reject a warm patch that is not logo-shaped", () => {
    const width = 160;
    const height = 90;
    const frame = makeBlackFrame(width, height);

    paintRect(frame, width, 143, 60, 155, 81, 255, 120, 20);

    const source = new RawFrameSource(frame, width, height);
    assert.equal(detectLoading(source, width, height), false);
  });

  it("should reject a gold-framed panel with a dark interior", () => {
    const width = 160;
    const height = 90;
    const frame = makeBlackFrame(width, height);
    const gold = [90, 75, 45] as const;

    paintRect(frame, width, 46, 22, 113, 23, ...gold);
    paintRect(frame, width, 46, 66, 113, 67, ...gold);
    paintRect(frame, width, 46, 22, 47, 67, ...gold);
    paintRect(frame, width, 112, 22, 113, 67, ...gold);

    const source = new RawFrameSource(frame, width, height);
    assert.equal(detectLoading(source, width, height), false);
  });

  it("should reject an in-game frame with a warm logo-area patch", async () => {
    const { data, info } = await loadImage("test/1280x720-dark.png");
    const frame = Buffer.from(data);

    paintRect(frame, info.width, 143, 74, 155, 81, 255, 120, 20);

    const source = new RawFrameSource(frame, info.width, info.height);
    assert.equal(detectLoading(source, info.width, info.height), false);
  });
});
