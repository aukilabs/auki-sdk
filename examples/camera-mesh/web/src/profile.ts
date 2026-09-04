export type CameraQualityTier = "low" | "medium" | "high";

export interface CameraStreamProfile {
  readonly quality: CameraQualityTier;
  readonly width: number;
  readonly height: number;
  readonly rateHz: number;
}

export const CAMERA_QUALITY_TIERS: readonly CameraQualityTier[] = [
  "low",
  "medium",
  "high",
];

const PROFILES: Readonly<Record<CameraQualityTier, CameraStreamProfile>> = {
  low: Object.freeze({ quality: "low", width: 480, height: 270, rateHz: 5 }),
  medium: Object.freeze({ quality: "medium", width: 960, height: 540, rateHz: 15 }),
  high: Object.freeze({ quality: "high", width: 1_920, height: 1_080, rateHz: 30 }),
};

const ADD_ALL_LIMITS: Readonly<Record<CameraQualityTier, number>> = {
  low: 16,
  medium: 8,
  high: 1,
};

export const DEFAULT_CAMERA_PROFILE = PROFILES.low;

export function cameraStreamProfile(
  quality: CameraQualityTier,
): CameraStreamProfile {
  return PROFILES[quality];
}

/** Conservative batch size for the Camera Mesh demo's measured relay path. */
export function cameraAddAllLimit(quality: CameraQualityTier): number {
  return ADD_ALL_LIMITS[quality];
}

export function verifiedCameraProfile(
  width: number,
  height: number,
  rateHz: number,
): CameraStreamProfile {
  const profile = CAMERA_QUALITY_TIERS
    .map((quality) => PROFILES[quality])
    .find((candidate) =>
      candidate.width === width
      && candidate.height === height
      && candidate.rateHz === rateHz);
  if (!profile) {
    throw new Error(
      `unsupported Camera Mesh profile ${width}×${height} at ${rateHz} fps`,
    );
  }
  return profile;
}

export function cameraProfileLabel(profile: CameraStreamProfile): string {
  return `${profile.width} × ${profile.height} · ${profile.rateHz} fps`;
}

export function cameraQualityLabel(quality: CameraQualityTier): string {
  return `${quality.charAt(0).toUpperCase()}${quality.slice(1)} · ${cameraProfileLabel(PROFILES[quality])}`;
}

export function isCameraQualityTier(value: string): value is CameraQualityTier {
  return value === "low" || value === "medium" || value === "high";
}
