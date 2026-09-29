import { Grid, peerColor, type View, type Portal } from "./grid";
const get = <T extends Element = HTMLElement>(id: string): T =>
  document.getElementById(id) as unknown as T;
export class MapViews {
  private latest?: View;
  private hidden = new Set<string>();
  private coordinate = "combined";
  private frame = "";
  readonly statuses = new Map<string, string>();
  constructor(
    private board: Grid,
    private inspect: (portal: Portal) => void,
  ) {
    get<HTMLSelectElement>("coordinate-frame").onchange = () => {
      this.coordinate = get<HTMLSelectElement>("coordinate-frame").value;
      if (this.latest) this.render(this.latest);
    };
  }
  get placementFrame(): string {
    return this.frame;
  }
  get editable(): boolean {
    if (!this.latest) return false;
    const local = this.latest.layers.find(
      (l) => l.peer === this.latest!.local_peer,
    )!;
    return (
      this.frame === local.frame ||
      (!!local.to_display &&
        this.latest.layers.some((l) => l.frame === this.frame && l.to_display))
    );
  }
  clear(): void {
    this.board.cancelMove();
    this.latest = undefined;
    this.frame = "";
    this.coordinate = "combined";
    this.hidden.clear();
    this.statuses.clear();
    this.board.render([]);
    this.board.fit();
    get("layers").replaceChildren();
    get("portal-list").replaceChildren();
    get("conflict-panel").hidden = true;
    get<HTMLSelectElement>("coordinate-frame").replaceChildren(
      new Option("Combined view", "combined"),
    );
    get("frame-caption").textContent = "Start a session to place portals";
    get("portal-count").textContent = "0 portals";
    get("session-count").textContent = "Not connected";
  }
  render(view: View): void {
    this.latest = view;
    if (
      this.coordinate !== "combined" &&
      !view.layers.some((l) => l.peer === this.coordinate)
    )
      this.coordinate = "combined";
    const selected = view.layers.find((l) => l.peer === this.coordinate);
    const frame = selected?.frame ?? view.display_frame;
    const frameChanged = frame !== this.frame;
    if (frameChanged) this.board.cancelMove();
    this.frame = frame;
    const coordinate = get<HTMLSelectElement>("coordinate-frame");
    coordinate.replaceChildren(
      new Option("Combined · aligned layers", "combined"),
      ...view.layers.map(
        (l) =>
          new Option(
            l.peer === view.local_peer
              ? "My original coordinates"
              : `Peer ${l.peer.slice(-6)} · original coordinates`,
            l.peer,
          ),
      ),
    );
    coordinate.value = this.coordinate;
    const offset = selected?.to_display?.translation ?? [0, 0, 0];
    const union = new Map<string, Portal>();
    const rows = view.layers.map((layer) => {
      const own = layer.peer === view.local_peer;
      const compatible =
        layer.frame === frame ||
        (!!layer.to_display && (!selected || !!selected.to_display));
      const row = document.createElement("div");
      row.className = "layer-row";
      const label = document.createElement("label"),
        checkbox = document.createElement("input");
      checkbox.type = "checkbox";
      checkbox.checked = !this.hidden.has(layer.peer);
      checkbox.disabled = !compatible;
      checkbox.onchange = () => {
        if (checkbox.checked) this.hidden.delete(layer.peer);
        else this.hidden.add(layer.peer);
        this.render(view);
      };
      const dot = document.createElement("i");
      dot.className = "peer-dot";
      dot.style.background = peerColor(layer.peer);
      const text = document.createElement("span");
      const title = document.createElement("strong");
      title.textContent = own ? "My map" : `Peer ${layer.peer.slice(-6)}`;
      const sub = document.createElement("small");
      sub.textContent = `${layer.portals.length} portals · ${own ? "you" : (this.statuses.get(layer.peer) ?? "Live")} · #${layer.sequence}`;
      text.append(title, sub);
      label.append(checkbox, dot, text);
      row.append(label);
      if (!compatible) {
        const inspect = document.createElement("button");
        inspect.className = "text-button";
        inspect.textContent =
          layer.state === "conflict" ? "Conflict · view" : "Unaligned · view";
        inspect.onclick = () => {
          this.coordinate = layer.peer;
          this.render(view);
        };
        row.append(inspect);
      }
      if (compatible && !this.hidden.has(layer.peer)) {
        const portals =
          layer.frame === frame
            ? layer.portals
            : (layer.aligned_portals ?? []).map((p) => ({
                ...p,
                x: p.x - offset[0],
                y: p.y - offset[1],
              }));
        for (const portal of portals) {
          const prior = union.get(portal.id);
          if (prior) prior.contributors.push(...portal.contributors);
          else
            union.set(portal.id, {
              ...portal,
              contributors: [...portal.contributors],
            });
        }
      }
      return row;
    });
    for (const [peer, status] of this.statuses) {
      if (view.layers.some((layer) => layer.peer === peer)) continue;
      const row = document.createElement("div");
      row.className = "layer-row";
      const title = document.createElement("strong");
      title.textContent = `Peer ${peer.slice(-6)}`;
      const detail = document.createElement("small");
      detail.textContent = status;
      row.append(title, detail);
      rows.push(row);
    }
    get("layers").replaceChildren(...rows);
    const conflicts = view.conflicts.map((c) => c.name);
    this.board.render([...union.values()], [], conflicts);
    if (frameChanged) this.board.fit();
    get("frame-caption").textContent =
      `${selected ? "Original frame" : "Aligned frame"} ${frame.slice(-8)} · XY · meters`;
    get("frame-caption").title = frame;
    get("portal-count").textContent = `${union.size} visible portals`;
    get("session-count").textContent =
      `${view.layers.length} peer${view.layers.length === 1 ? "" : "s"}`;
    get("map-help").textContent = this.editable
      ? "Drag to pan · ⌘-drag your pin to move · double-click to place"
      : "Unaligned peer frame · read only · drag to pan";
    get<HTMLButtonElement>("add-pin").disabled = !this.editable;
    const local = view.layers.find((l) => l.peer === view.local_peer)!;
    get("conflict-panel").hidden = !view.conflicts.length;
    get("conflict-summary").textContent = view.conflicts.length
      ? "Shared placements disagree. Inspect a portal to move or remove your own copy."
      : "";
    const notes = view.conflicts.map((c) => {
      const item = document.createElement("p");
      item.textContent = `${c.name} disagrees with ${c.disagrees_with.join(", ")} (peers ${c.peers.map((p) => (p === view.local_peer ? "you" : p.slice(-6))).join(" / ")})`;
      return item;
    });
    get("conflict-list").replaceChildren(...notes);
    get("portal-list").replaceChildren(
      ...local.portals.map((portal) => {
        const item = document.createElement("button");
        item.className = "portal-item";
        item.textContent = `${portal.name}  ·  ${portal.x}, ${portal.y}`;
        if (
          view.conflicts.some(
            (c) => c.name === portal.name && c.peers.includes(view.local_peer),
          )
        )
          item.classList.add("conflicting");
        item.onclick = () => this.inspect(portal);
        return item;
      }),
    );
  }
}
