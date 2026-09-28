import { Grid, type View } from "./grid";
const get = <T extends Element = HTMLElement>(id: string): T => document.getElementById(id) as unknown as T;

/** Render source frames independently; only validated alignment exposes the union. */
export class MapViews {
  private partner = new Grid(get<SVGSVGElement>("remote-grid"));
  private combined = new Grid(get<SVGSVGElement>("combined-grid"));
  constructor(private local: Grid, private remove: (name: string) => void) {
    this.clear();
  }
  clear(): void {
    this.local.render([], []);
    this.partner.render([], []);
    this.combined.render([], []);
    get("combined-grid").setAttribute("hidden", "");
    get("remote-frame").textContent = "No partner frame yet";
    get("combined-frame").textContent = "No aligned frame yet";
    get("combined-status").textContent = "Place a shared portal to align the maps.";
    get("portal-list").replaceChildren();
    get("conflict-summary").textContent = "Remove a portal from your map here. Your partner’s original map is preserved.";
  }
  render(view: View): void {
    const peers = [view.local_peer, ...(view.remote_peer ? [view.remote_peer] : [])];
    const conflicts = view.conflicts.map((entry) => entry.name);
    get("empty").hidden = view.local_portals.length > 0;
    get("frame-caption").textContent = `Frame ${view.local_frame.slice(-8)}`;
    get("frame-caption").title = view.local_frame;
    get("portal-count").textContent = `${view.local_portals.length} local portals`;
    this.local.render(view.local_portals, peers, conflicts);
    this.partner.render(view.remote_portals, peers, conflicts);
    const aligned = view.state === "aligned";
    this.combined.render(aligned ? view.portals : [], peers);
    get("combined-grid").toggleAttribute("hidden", !aligned);
    get("remote-frame").textContent = view.remote_frame ?? "No partner frame yet";
    get("combined-frame").textContent = aligned ? view.display_frame : "No aligned frame yet";
    get("combined-status").textContent = aligned
      ? `${view.portals.length} portals · aligned union · read only`
      : view.state === "conflict"
        ? "Combined view unavailable: shared portals imply different offsets. Correct or remove your own placements below."
        : view.reason ?? "Waiting for a shared portal in both maps.";
    get("conflict-summary").textContent = conflicts.length
      ? "Red portals disagree with another shared portal. Neither placement is automatically considered correct. Coordinates below are in your local frame."
      : "Remove a portal from your map here. Your partner’s original map is preserved.";
    const rows = [...view.local_portals].sort((a, b) => a.name.localeCompare(b.name)).map((portal) => {
      const row = document.createElement("div");
      row.className = "portal-row";
      const text = document.createElement("div");
      const name = document.createElement("strong");
      name.textContent = `${portal.name} (${portal.x}, ${portal.y})`;
      text.append(name);
      const conflict = view.conflicts.find((entry) => entry.name === portal.name);
      if (conflict) {
        row.classList.add("conflicting");
        const detail = document.createElement("p");
        detail.textContent = `Incompatible with: ${conflict.disagrees_with.join(", ")}`;
        text.append(detail);
      }
      const remove = document.createElement("button");
      remove.type = "button";
      remove.className = "secondary";
      remove.textContent = "Remove mine";
      remove.setAttribute("aria-label", `Remove ${portal.name} from my map`);
      remove.onclick = () => this.remove(portal.name);
      row.append(text, remove);
      return row;
    });
    get("portal-list").replaceChildren(...rows);
  }
}
