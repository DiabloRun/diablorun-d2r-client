export const MIN_IMAGE_SIZE = 160;

export function detectLoading(
  ctx: CanvasRenderingContext2D,
  width: number,
  height: number
) {
  // Get titlebar height
  const vertical = ctx.getImageData(Math.floor(width / 2), 0, 1, height);
  const verticalOffset = getImageDataLightStartMargin(vertical);
  const innerHeight = height - verticalOffset;

  // Check for center loading box and bottom-right loading logo
  // both from two spots in case the cursor is on a line
  return (
    detectCenterLoading(
      ctx,
      width,
      height,
      Math.floor(width * 0.375),
      Math.floor(height * 0.375),
      verticalOffset
    ) ||
    detectCenterLoading(
      ctx,
      width,
      height,
      Math.floor(width * 0.625),
      Math.floor(height * 0.625),
      verticalOffset
    ) ||
    detectBottomRightLoading(
      ctx,
      width,
      height,
      Math.floor(width * 0.91),
      verticalOffset + Math.floor(innerHeight * 0.85),
      verticalOffset
    ) ||
    detectBottomRightLoading(
      ctx,
      width,
      height,
      Math.floor(width * 0.95),
      verticalOffset + Math.floor(innerHeight * 0.88),
      verticalOffset
    )
  );
}

function detectCenterLoading(
  ctx: CanvasRenderingContext2D,
  width: number,
  height: number,
  x: number,
  y: number,
  verticalOffset: number
) {
  const cm = getMargin(ctx, width, height, x, y, verticalOffset);
  const innerHeight = height - verticalOffset;

  console.log(
    cm.t / innerHeight,
    cm.b / innerHeight,
    (width - cm.l - cm.r) / innerHeight
  );

  return (
    equalWithin(cm.t / innerHeight, 0.3, 0.075) &&
    equalWithin(cm.b / innerHeight, 0.3, 0.075) &&
    (equalWithin((width - cm.l - cm.r) / innerHeight, 0.7375, 0.1) || // D2R
      equalWithin((width - cm.l - cm.r) / innerHeight, 0.425, 0.1)) // Legacy
  );
}

function detectBottomRightLoading(
  ctx: CanvasRenderingContext2D,
  width: number,
  height: number,
  x: number,
  y: number,
  verticalOffset: number
) {
  const innerHeight = height - verticalOffset;
  const lm = getMargin(ctx, width, height, x, y, verticalOffset);

  return (
    equalWithin(lm.t / innerHeight, 0.82, 0.03) &&
    equalWithin(lm.b / innerHeight, 0.1, 0.03) &&
    equalWithin(lm.l / width, 0.9, 0.03) &&
    equalWithin(lm.r / width, 0.045, 0.03)
  );
}

function equalWithin(a: number, b: number, e: number) {
  return Math.abs(a - b) < e;
}

function getMargin(
  ctx: CanvasRenderingContext2D,
  width: number,
  height: number,
  x: number,
  y: number,
  verticalOffset: number
) {
  const vertical = ctx.getImageData(x, 0, 1, height);
  const horizontal = ctx.getImageData(0, y, width, 1);

  return {
    t: getImageDataStartMargin(vertical, verticalOffset) - verticalOffset,
    b: getImageDataEndMargin(vertical),
    l: getImageDataStartMargin(horizontal),
    r: getImageDataEndMargin(horizontal),
  };
}

function getImageDataLightStartMargin(imageData: ImageData) {
  const size = imageData.data.length;

  for (let i = 0; i < size; ++i) {
    if (imageData.data[i * 4] <= 20) {
      return i;
    }
  }

  return size / 4;
}

function getImageDataStartMargin(imageData: ImageData, offset = 1) {
  const size = imageData.data.length;

  for (let i = offset; i < size; ++i) {
    if (imageData.data[i * 4] > 20) {
      return i;
    }
  }

  return size / 4;
}

function getImageDataEndMargin(imageData: ImageData, offset = 1) {
  const size = imageData.data.length;

  for (let i = offset; i < size; ++i) {
    if (imageData.data[size - i * 4] > 20) {
      return i;
    }
  }

  return size / 4;
}
