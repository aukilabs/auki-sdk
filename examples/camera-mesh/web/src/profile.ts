export type CameraResolutionTier = "low" | "medium" | "high";
export type CameraRateTier = "low" | "medium" | "high";

export interface CameraStreamProfile {
  readonly resolution: CameraResolutionTier;
  readonly rate: CameraRateTier;
  readonly width: number;
  readonly height: number;
  readonly rateHz: number;
}

const RESOLUTIONS: Record<CameraResolutionTier, readonly [number, number]> = {
  low: [480, 270],
  medium: [960, 540],
  high: [1_920, 1_080],
};

const RATES: Record<CameraRateTier, number> = {
  low: 5,
  medium: 15,
  high: 30,
};

export const DEFAULT_CAMERA_PROFILE = cameraStreamProfile("low", "low");

export function cameraStreamProfile(
  resolution: CameraResolutionTier,
  rate: CameraRateTier,
): CameraStreamProfile {
  const [width, height] = RESOLUTIONS[resolution];
  return Object.freeze({ resolution, rate, width, height, rateHz: RATES[rate] });
}

export function verifiedCameraProfile(
  width: number,
  height: number,
  rateHz: number,
): CameraStreamProfile {
  const resolution = (Object.entries(RESOLUTIONS) as Array<
    [CameraResolutionTier, readonly [number, number]]
  >).find(([, dimensions]) => dimensions[0] === width && dimensions[1] === height)?.[0];
  const rate = (Object.entries(RATES) as Array<[CameraRateTier, number]>)
    .find(([, value]) => value === rateHz)?.[0];
  if (!resolution || !rate) {
    throw new Error(
      `unsupported Camera Mesh profile ${width}×${height} at ${rateHz} fps`,
    );
  }
  return cameraStreamProfile(resolution, rate);
}

export function cameraProfileLabel(profile: CameraStreamProfile): string {
  return `${profile.width} × ${profile.height} · ${profile.rateHz} fps`;
}
