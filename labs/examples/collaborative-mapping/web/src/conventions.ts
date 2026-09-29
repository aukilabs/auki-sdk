export type Convention = "x_right" | "x_up" | "x_left" | "x_down";
export const conventions: Record<
  Convention,
  { label: string; right: string; up: string; basis: [number, number] }
> = {
  x_right: { label: "+X right · +Y up", right: "+X", up: "+Y", basis: [1, 0] },
  x_up: { label: "+X up · +Y left", right: "−Y", up: "+X", basis: [0, 1] },
  x_left: { label: "+X left · +Y down", right: "−X", up: "−Y", basis: [-1, 0] },
  x_down: {
    label: "+X down · +Y right",
    right: "+Y",
    up: "−X",
    basis: [0, -1],
  },
};
export function toCanvas(
  convention: Convention,
  x: number,
  y: number,
): [number, number] {
  const [c, s] = conventions[convention].basis;
  return [c * x - s * y, s * x + c * y];
}
export function fromCanvas(
  convention: Convention,
  x: number,
  y: number,
): [number, number] {
  const [c, s] = conventions[convention].basis;
  return [c * x + s * y, -s * x + c * y];
}
export function inversePoint(
  t: { translation: number[]; rotation_wxyz: number[] },
  x: number,
  y: number,
): [number, number] {
  const [w, , , z] = t.rotation_wxyz;
  const c = 1 - 2 * z * z,
    s = 2 * w * z;
  x -= t.translation[0];
  y -= t.translation[1];
  return [Math.round(c * x + s * y), Math.round(-s * x + c * y)];
}
