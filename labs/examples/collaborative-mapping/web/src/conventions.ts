import { convertConventionPoint } from "../pkg-web/auki_collaborative_mapping_web.js";
export type Convention = "x_right" | "x_up" | "x_left" | "x_down";
export const conventions: Record<
  Convention,
  { label: string; right: string; up: string }
> = {
  x_right: { label: "+X right · +Y up", right: "+X", up: "+Y" },
  x_up: { label: "+X up · +Y left", right: "−Y", up: "+X" },
  x_left: { label: "+X left · +Y down", right: "−X", up: "−Y" },
  x_down: {
    label: "+X down · +Y right",
    right: "+Y",
    up: "−X",
  },
};
export function toCanvas(
  convention: Convention,
  x: number,
  y: number,
): [number, number] {
  const [px, py] = convertConventionPoint(convention, "x_right", x, y);
  return [px, py];
}
export function fromCanvas(
  convention: Convention,
  x: number,
  y: number,
): [number, number] {
  const [px, py] = convertConventionPoint("x_right", convention, x, y);
  return [px, py];
}
