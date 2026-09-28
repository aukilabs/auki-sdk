export interface Portal {
  id: string;
  name: string;
  x: number;
  y: number;
  contributors: string[];
}
export interface Reference {
  sequence: number;
  product: { peer_id: string; product_id: string; manifest_hash: string };
}
export interface View {
  state: string;
  reason: string | null;
  local_peer: string;
  remote_peer: string | null;
  local_frame: string;
  remote_frame: string | null;
  conflicts: { name: string; disagrees_with: string[] }[];
  display_frame: string;
  local_to_display: {
    from_frame_id: string;
    to_frame_id: string;
    translation: number[];
  };
  local_reference: Reference;
  remote_reference: Reference | null;
  shared_names: string[];
  portals: Portal[];
  local_portals: Portal[];
  remote_portals: Portal[];
}
const NS = "http://www.w3.org/2000/svg";
function svg<K extends keyof SVGElementTagNameMap>(
  tag: K,
  attrs: Record<string, string | number> = {},
): SVGElementTagNameMap[K] {
  const element = document.createElementNS(NS, tag);
  Object.entries(attrs).forEach(([key, value]) =>
    element.setAttribute(key, String(value)),
  );
  return element;
}
export class Grid {
  constructor(
    private readonly root: SVGSVGElement,
    private readonly selected?: (x: number, y: number) => void,
  ) {
    root.addEventListener("click", (event) => {
      if (!selected) return;
      const matrix = root.getScreenCTM();
      if (!matrix) return;
      const point = new DOMPoint(event.clientX, event.clientY).matrixTransform(
        matrix.inverse(),
      );
      selected(Math.round(point.x), Math.round(-point.y));
    });
  }
  render(portals: Portal[], peers: string[], conflicts: string[] = []): void {
    const sortedPeers = [...peers].sort();
    const xs = portals.map((p) => p.x),
      ys = portals.map((p) => p.y);
    const minX = Math.min(-8, ...xs),
      maxX = Math.max(8, ...xs);
    const minY = Math.min(-8, ...ys),
      maxY = Math.max(8, ...ys);
    const size = Math.max(24, maxX - minX + 8, maxY - minY + 8);
    const left = (minX + maxX - size) / 2,
      top = -(minY + maxY + size) / 2;
    this.root.setAttribute("viewBox", `${left} ${top} ${size} ${size}`);
    const unit = size / 24;
    const step = Math.max(1, 10 ** Math.floor(Math.log10(unit)));
    const patternId = `${this.root.id}-cells`;
    const defs = svg("defs"),
      pattern = svg("pattern", {
        id: patternId,
        width: step,
        height: step,
        patternUnits: "userSpaceOnUse",
      });
    pattern.append(
      svg("path", {
        d: `M ${step} 0 H 0 V ${step}`,
        fill: "none",
        stroke: "#e9e5f0",
        "stroke-width": 0.022 * unit,
      }),
    );
    defs.append(pattern);
    this.root.replaceChildren(
      defs,
      svg("rect", {
        x: left - size * 2,
        y: top - size * 2,
        width: size * 5,
        height: size * 5,
        fill: `url(#${patternId})`,
      }),
      svg("path", {
        d: `M ${left - size * 2} 0 H ${left + size * 3} M 0 ${top - size * 2} V ${top + size * 3}`,
        stroke: "#cfc7de",
        "stroke-width": 0.03 * unit,
        "stroke-dasharray": `${0.14 * unit} ${0.15 * unit}`,
      }),
    );
    for (const [label, x, y] of [
      ["0", 0.2 * unit, 0.5 * unit],
      ["+X", left + size - unit, 0.5 * unit],
      ["+Y", 0.2 * unit, top + unit],
    ] as const) {
      const axis = svg("text", { x, y, fill: "#81758f", "font-size": 0.4 * unit });
      axis.textContent = label;
      this.root.append(axis);
    }
    for (const portal of portals) {
      const conflicting = conflicts.includes(portal.name);
      const color = conflicting ? "#bd4558" :
        portal.contributors.length > 1
          ? "#52a897"
          : portal.contributors[0] === sortedPeers[0]
            ? "#7960d0"
            : "#dd965d";
      const group = svg("g", {
        transform: `translate(${portal.x} ${-portal.y})`,
        "data-portal": portal.name,
        "data-conflict": String(conflicting),
        "data-x": portal.x,
        "data-y": portal.y,
      });
      const title = svg("title");
      title.textContent = `${portal.name} · (${portal.x}, ${portal.y}) · ${portal.contributors.length} contributor(s)`;
      group.append(
        title,
        svg("circle", { r: 0.47 * unit, fill: color, opacity: 0.12 }),
        svg("circle", {
          r: 0.19 * unit,
          fill: color,
          stroke: "#fff",
          "stroke-width": 0.055 * unit,
        }),
      );
      const text = svg("text", {
        x: 0.48 * unit,
        y: -0.32 * unit,
        fill: color,
        "font-size": 0.36 * unit,
        "font-weight": 600,
        "paint-order": "stroke",
        stroke: "#fdfcfe",
        "stroke-width": 0.12 * unit,
        "stroke-linejoin": "round",
      });
      text.textContent = portal.name;
      group.append(text);
      this.root.append(group);
    }
  }
}
