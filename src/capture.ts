import { desktopCapturer } from "electron";

export async function getCaptureSourceId() {
  const sources = await desktopCapturer.getSources({
    types: ["window", "screen"],
    thumbnailSize: { width: 0, height: 0 },
  });

  const source = sources.find((s) => s.name === "Diablo II: Resurrected");

  if (!source) {
    return await new Promise<string>((resolve) => {
      setTimeout(async () => {
        resolve(await getCaptureSourceId());
      }, 3000);
    });
  }

  return source.id;
}
