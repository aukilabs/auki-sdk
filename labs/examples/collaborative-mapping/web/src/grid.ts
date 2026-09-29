export interface Portal {
  id: string;
  name: string;
  x: number;
  y: number;
  contributors: string[];
}
export interface Layer {
  peer: string;
  frame: string;
  sequence: number;
  state: string;
  portals: Portal[];
  aligned_portals: Portal[] | null;
  to_display: {
    from_frame_id: string;
    to_frame_id: string;
    translation: number[];
  } | null;
}
export interface View {
  local_peer: string;
  display_frame: string;
  layers: Layer[];
  portals: Portal[];
  conflicts: { name: string; disagrees_with: string[]; peers: string[] }[];
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
export const peerColor = (peer: string): string => {
  let hash = 0;
  for (const char of peer) hash = (hash * 31 + char.charCodeAt(0)) | 0;
  return `hsl(${Math.abs(hash) % 360} 52% 44%)`;
};
/** Persistent camera: incoming snapshots never reset the user's pan or zoom. */
export class Grid {
  private camera = { x: -12, y: -12, size: 24 };
  private portals: Portal[] = [];
  private conflicts: string[] = [];
  private drag?: {
    id: number;
    x: number;
    y: number;
    startX: number;
    startY: number;
  };
  private move?: {
    portal: Portal;
    start: DOMPoint;
    x: number;
    y: number;
    commit: (x: number, y: number) => void;
  };
  private dragged = false;
  private pressed?: Portal;
  private draft?: { x: number; y: number };
  constructor(
    private root: SVGSVGElement,
    private selected?: (x: number, y: number) => void,
    private inspect?: (portal: Portal) => void,
    private beginMove?: (
      portal: Portal,
    ) => ((x: number, y: number) => void) | undefined,
  ) {
    new ResizeObserver(() => this.draw()).observe(root);
    root.addEventListener("pointerdown", (event) => {
      if (this.drag || !event.isPrimary || event.button !== 0) return;
      this.dragged = false;
      const marker = (event.target as Element).closest<SVGGElement>(
        "[data-index]",
      );
      this.pressed = marker
        ? this.portals[Number(marker.dataset.index)]
        : undefined;
      if (event.metaKey && this.pressed) {
        const commit = this.beginMove?.(this.pressed);
        const start = this.point(event.clientX, event.clientY);
        // A modifier drag on a read-only pin neither edits it nor pans the map.
        if (!commit || !start) {
          this.dragged = true;
          this.pressed = undefined;
          return;
        }
        this.move = {
          portal: this.pressed,
          start,
          x: this.pressed.x,
          y: this.pressed.y,
          commit,
        };
        event.preventDefault();
        root.focus();
      }
      this.drag = {
        id: event.pointerId,
        x: event.clientX,
        y: event.clientY,
        startX: event.clientX,
        startY: event.clientY,
      };
      root.setPointerCapture(event.pointerId);
      root.classList.add("dragging");
    });
    root.addEventListener("pointermove", (event) => {
      if (!this.drag || this.drag.id !== event.pointerId) return;
      const before = this.point(this.drag.x, this.drag.y),
        after = this.point(event.clientX, event.clientY);
      if (!before || !after) return;
      if (
        Math.hypot(
          event.clientX - this.drag.startX,
          event.clientY - this.drag.startY,
        ) > 4
      )
        this.dragged = true;
      if (this.move) {
        this.move.x = Math.round(
          this.move.portal.x + after.x - this.move.start.x,
        );
        this.move.y = Math.round(
          this.move.portal.y - after.y + this.move.start.y,
        );
      } else {
        this.camera.x -= after.x - before.x;
        this.camera.y -= after.y - before.y;
      }
      this.drag.x = event.clientX;
      this.drag.y = event.clientY;
      this.draw();
    });
    root.addEventListener("pointerup", (event) => {
      if (this.drag?.id !== event.pointerId) return;
      const move = this.move;
      const changed =
        this.dragged &&
        move &&
        (move.x !== move.portal.x || move.y !== move.portal.y);
      this.finishGesture();
      if (changed) move.commit(move.x, move.y);
    });
    root.addEventListener("pointercancel", () => this.cancelMove());
    root.addEventListener("lostpointercapture", () => {
      if (this.drag) this.cancelMove();
    });
    root.addEventListener("dblclick", (event) => {
      event.preventDefault();
      if (this.dragged) return;
      const point = this.point(event.clientX, event.clientY);
      if (point) this.selected?.(Math.round(point.x), Math.round(-point.y));
    });
    root.addEventListener("click", (event) => {
      if (this.dragged || event.detail > 1) return;
      const marker = (event.target as Element).closest<SVGGElement>(
        "[data-index]",
      );
      const portal = marker
        ? this.portals[Number(marker.dataset.index)]
        : this.pressed;
      if (portal) this.inspect?.(portal);
      this.pressed = undefined;
    });
    root.addEventListener(
      "wheel",
      (event) => {
        event.preventDefault();
        this.zoom(
          Math.exp(Math.max(-1, Math.min(1, event.deltaY / 400))),
          event.clientX,
          event.clientY,
        );
      },
      { passive: false },
    );
    root.addEventListener("keydown", (event) => {
      if (this.move) {
        if (event.key === "Escape") {
          event.preventDefault();
          this.cancelMove();
        }
        return;
      }
      const step = this.camera.size / 10;
      if (event.key === "ArrowLeft") this.camera.x -= step;
      else if (event.key === "ArrowRight") this.camera.x += step;
      else if (event.key === "ArrowUp") this.camera.y -= step;
      else if (event.key === "ArrowDown") this.camera.y += step;
      else if (event.key === "+" || event.key === "=") this.zoom(0.8);
      else if (event.key === "-") this.zoom(1.25);
      else if (event.key === "Enter") this.dropAtCenter();
      else return;
      event.preventDefault();
      this.draw();
    });
  }
  private finishGesture(): void {
    const hadMove = !!this.move;
    const id = this.drag?.id;
    this.drag = undefined;
    this.move = undefined;
    this.root.classList.remove("dragging");
    if (id !== undefined && this.root.hasPointerCapture(id))
      this.root.releasePointerCapture(id);
    if (hadMove) this.draw();
  }
  cancelMove(): void {
    this.dragged = true;
    this.pressed = undefined;
    this.finishGesture();
  }
  private point(x: number, y: number): DOMPoint | undefined {
    const matrix = this.root.getScreenCTM();
    return matrix
      ? new DOMPoint(x, y).matrixTransform(matrix.inverse())
      : undefined;
  }
  zoom(factor: number, clientX?: number, clientY?: number): void {
    if (this.move) return;
    const anchor =
      clientX !== undefined && clientY !== undefined
        ? this.point(clientX, clientY)
        : undefined;
    const center = anchor ?? {
      x: this.camera.x + this.camera.size / 2,
      y: this.camera.y + this.camera.size / 2,
    };
    const next = Math.max(6, Math.min(50000, this.camera.size * factor));
    const ratio = next / this.camera.size;
    this.camera.x = center.x - (center.x - this.camera.x) * ratio;
    this.camera.y = center.y - (center.y - this.camera.y) * ratio;
    this.camera.size = next;
    this.draw();
  }
  dropAtCenter(): void {
    this.selected?.(
      Math.round(this.camera.x + this.camera.size / 2),
      Math.round(-this.camera.y - this.camera.size / 2),
    );
  }
  setDraft(point?: { x: number; y: number }): void {
    this.draft = point;
    this.draw();
  }
  fit(): void {
    const xs = this.portals.map((p) => p.x),
      ys = this.portals.map((p) => -p.y);
    const minX = Math.min(-4, ...xs),
      maxX = Math.max(4, ...xs),
      minY = Math.min(-4, ...ys),
      maxY = Math.max(4, ...ys);
    const size = Math.max(24, maxX - minX + 10, maxY - minY + 10);
    this.camera = {
      x: (minX + maxX - size) / 2,
      y: (minY + maxY - size) / 2,
      size,
    };
    this.draw();
  }
  render(
    portals: Portal[],
    _peers: string[] = [],
    conflicts: string[] = [],
  ): void {
    this.portals = portals;
    this.conflicts = conflicts;
    this.draw();
  }
  private draw(): void {
    const { x: left, y: top, size } = this.camera;
    this.root.setAttribute("viewBox", `${left} ${top} ${size} ${size}`);
    const unit =
        (size /
          (Math.min(this.root.clientWidth, this.root.clientHeight) || 720)) *
        24,
      step = Math.max(1, 10 ** Math.floor(Math.log10(unit)));
    const patternId = `${this.root.id}-cells`,
      defs = svg("defs");
    const pattern = svg("pattern", {
      id: patternId,
      width: step,
      height: step,
      patternUnits: "userSpaceOnUse",
    });
    pattern.append(
      svg("path", {
        d: `M ${step} 0 H 0 V ${step}`,
        fill: "none",
        stroke: "#e3e7e8",
        "stroke-width": 0.02 * unit,
      }),
    );
    defs.append(pattern);
    this.root.replaceChildren(
      defs,
      svg("rect", {
        x: left - size * 3,
        y: top - size * 3,
        width: size * 7,
        height: size * 7,
        fill: `url(#${patternId})`,
      }),
      svg("path", {
        d: `M ${left - size * 3} 0 H ${left + size * 4} M 0 ${top - size * 3} V ${top + size * 4}`,
        stroke: "#bac7c8",
        "stroke-width": 0.035 * unit,
      }),
    );
    for (const [label, x, y] of [
      ["0", unit * 0.2, unit * 0.55],
      ["+X", left + size - unit, unit * 0.55],
      ["+Y", unit * 0.2, top + unit],
    ] as const) {
      const text = svg("text", {
        x,
        y,
        fill: "#788889",
        "font-size": unit * 0.4,
      });
      text.textContent = label;
      this.root.append(text);
    }
    const display = [
      ...this.portals.map((p) =>
        this.move?.portal.id === p.id
          ? { ...p, x: this.move.x, y: this.move.y }
          : p,
      ),
      ...(this.draft
        ? [{ id: "draft", name: "New portal", ...this.draft, contributors: [] }]
        : []),
    ];
    display.forEach((portal, index) => {
      const conflict = this.conflicts.includes(portal.name),
        draft = index === this.portals.length;
      const color = draft
        ? "#283e40"
        : conflict
          ? "#bf4858"
          : portal.contributors.length > 1
            ? "#248476"
            : peerColor(portal.contributors[0]);
      const group = svg("g", {
        transform: `translate(${portal.x} ${-portal.y})`,
        "data-portal": portal.name,
        "data-index": index,
        "data-conflict": String(conflict),
        "data-x": portal.x,
        "data-y": portal.y,
      });
      if (draft) group.removeAttribute("data-index");
      const title = svg("title");
      title.textContent = `${portal.name} · (${portal.x}, ${portal.y})`;
      const pin = svg("path", {
        d: "M 0 0 C -.12 -.22 -.5 -.55 -.5 -.9 A .5 .5 0 1 1 .5 -.9 C .5 -.55 .12 -.22 0 0 Z",
        transform: `scale(${unit})`,
        fill: color,
        stroke: "white",
        "stroke-width": 0.06,
      });
      const label = svg("text", {
        x: 0.65 * unit,
        y: -0.7 * unit,
        fill: color,
        "font-size": 0.44 * unit,
        "font-weight": 600,
        "paint-order": "stroke",
        stroke: "#fafcfb",
        "stroke-width": 0.14 * unit,
      });
      label.textContent = portal.name;
      group.append(title, pin, label);
      this.root.append(group);
    });
  }
}
