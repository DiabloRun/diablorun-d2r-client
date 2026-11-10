export function formatTime(milliseconds, decimals = 2) {
  const places = Math.max(0, Math.min(3, Math.floor(decimals)));
  const value = Number.isFinite(milliseconds) ? Math.max(0, milliseconds) : 0;
  const total = Math.floor(value);

  const totalSeconds = Math.floor(total / 1000);
  const fraction = total % 1000;
  const hours = Math.floor(totalSeconds / 3600);
  const minutes = Math.floor((totalSeconds % 3600) / 60);
  const seconds = totalSeconds % 60;

  const fractionString = String(fraction).padStart(3, "0").slice(0, places);
  const suffix = places > 0 ? `.${fractionString}` : "";
  const secondsString = String(seconds).padStart(2, "0");

  if (hours > 0) {
    return `${hours}:${String(minutes).padStart(2, "0")}:${secondsString}${suffix}`;
  }

  return `${minutes}:${secondsString}${suffix}`;
}

export function readNumberParam(params, name, fallback, min, max) {
  const raw = params.get(name);

  if (raw === null) {
    return fallback;
  }

  const value = Number(raw);

  if (!Number.isFinite(value)) {
    return fallback;
  }

  return Math.min(max, Math.max(min, value));
}
