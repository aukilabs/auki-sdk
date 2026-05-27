/** Last `/`-separated segment of a sensor_id — e.g.
 * `"K1-AABBCCDDEEFF/head_left_cam"` → `"head_left_cam"`. The full id
 * stays in tooltips. */
export function shortName(sensor_id: string): string {
  return sensor_id.split("/").pop() ?? sensor_id;
}
