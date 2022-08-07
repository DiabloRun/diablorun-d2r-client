import { describe, it } from "mocha";
import { expect } from "expect";
import sharp from "sharp";
import { detectLoading, MIN_IMAGE_SIZE } from "./loading-detection";

class MockCanvasRenderingContext2D {
  private data: Uint8ClampedArray;

  constructor(private buffer: Buffer, private width: number) {
    this.data = Uint8ClampedArray.from(this.buffer);
  }

  getImageData(sx: number, sy: number, sw: number, sh: number) {
    const data = new Uint8ClampedArray(sw * sh * 4);
    let i = 0;

    for (let y = sy; y < sy + sh; ++y) {
      for (let x = sx; x < sx + sw; ++x) {
        data.set(
          this.data.subarray(
            (y * this.width + x) * 4,
            (y * this.width + x) * 4 + 4
          ),
          i
        );

        i += 4;
      }
    }

    return { data } as ImageData;
  }
}

async function expectLoading(filename: string, expectedValue = true) {
  const im = sharp(filename).resize(MIN_IMAGE_SIZE, MIN_IMAGE_SIZE, {
    fit: "inside",
  });
  const { data, info } = await im.raw().toBuffer({ resolveWithObject: true });
  const ctx = new MockCanvasRenderingContext2D(data, info.width);
  const isLoading = detectLoading(ctx as any, info.width, info.height);

  expect(isLoading).toBe(expectedValue);
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
    await expectLoading("test/1280x720-legacy.png", false);
  });

  it("should not be loading with 1280x720", async () => {
    await expectLoading("test/1280x720-dark.png", false);
  });

  // Screen space option ticked
  it("should be door loading with screen space ticked", async () => {
    await expectLoading("test/2560x1440-4-tick-screen-space-door.png");
  });

  it("should be logo loading with screen space ticked", async () => {
    await expectLoading("test/2560x1440-4-tick-screen-space-black-logo.png");
  });

  it("should not be loading with screen space ticked", async () => {
    await expectLoading(
      "test/2560x1440-4-tick-screen-space-dark-area.png",
      false
    );
  });
});
