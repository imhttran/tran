import type { EvidenceRef, EvidenceStatus } from "@/lib/experience-contract";

export type TreePerson = {
  record_id: string;
  person_label: string;
  evidence_status: EvidenceStatus;
  aliases: string[];
};
export type TreeLink = {
  from_id: string;
  to_id: string;
  kind: "parent_child" | "spouse";
  evidence_status: EvidenceStatus;
  evidence: EvidenceRef[];
};
export type TreeGraph = { people: TreePerson[]; links: TreeLink[]; truncated: boolean };
export type PositionedPerson = TreePerson & { x: number; y: number; childCount: number };
export type TreeLayout = {
  people: PositionedPerson[];
  links: TreeLink[];
  width: number;
  height: number;
};

export const CARD_WIDTH = 190;
export const CARD_HEIGHT = 76;
const STEP_X = 250;
const STEP_Y = 126;
/** Disconnected groups wrap instead of forming one unreadable row. */
const MAX_BAND_WIDTH = 1600;
const COMPONENT_GAP_X = 48;
const COMPONENT_GAP_Y = 56;
/** The camera starts (and follows a selected person) at this readable scale. */
export const READABLE_WIDTH = 1100;
export const READABLE_HEIGHT = 720;

/** Accent-insensitive search. The original name remains untouched in display. */
export function normalizedName(value: string): string {
  return value.normalize("NFD").replace(/[\u0300-\u036f]/g, "").replace(/đ/g, "d").replace(/Đ/g, "D").toLowerCase();
}

/** Defensive client-side shape check. Rust remains responsible for access control. */
export function parseTreeGraph(body: unknown): TreeGraph {
  if (!body || typeof body !== "object") throw new Error("Invalid genealogy graph");
  const data = body as Partial<TreeGraph>;
  if (!Array.isArray(data.people) || !Array.isArray(data.links) ||
      typeof data.truncated !== "boolean") throw new Error("Invalid genealogy graph");
  const validStatus = (s: unknown): s is EvidenceStatus =>
    s === "verified" || s === "unverified" || s === "needs_review";
  const people = data.people.filter((p): p is TreePerson =>
    !!p && typeof p.record_id === "string" && p.record_id.length > 0 &&
    typeof p.person_label === "string" && validStatus(p.evidence_status) &&
    Array.isArray(p.aliases) && p.aliases.every((a) => typeof a === "string"));
  const ids = new Set(people.map((p) => p.record_id));
  const links = data.links.filter((l): l is TreeLink =>
    !!l && typeof l.from_id === "string" && typeof l.to_id === "string" &&
    ids.has(l.from_id) && ids.has(l.to_id) &&
    (l.kind === "parent_child" || l.kind === "spouse") &&
    validStatus(l.evidence_status) && Array.isArray(l.evidence) &&
    l.evidence.every((e) => !!e && typeof e.evidence_id === "string" &&
      typeof e.citation === "string"));
  if (people.length !== data.people.length || links.length !== data.links.length) {
    throw new Error("Invalid genealogy graph references");
  }
  return { people, links, truncated: data.truncated };
}

/**
 * The other person's role relative to `selectedId`. A parent_child link is
 * stored parent (from_id) → child (to_id).
 */
export function relationLabel(link: TreeLink, selectedId: string): "Parent" | "Child" | "Spouse" {
  if (link.kind === "spouse") return "Spouse";
  return link.from_id === selectedId ? "Child" : "Parent";
}

/** The person-centred view: one person, their immediate children, and optionally
 * their immediate parents. Spouses and co-parents are included only when they
 * belong to that immediate family. */
export type Focus = { rootId: string; showParents: boolean };

/** A screen-space rectangle, as returned by getBoundingClientRect(). */
export type Box = { left: number; top: number; width: number; height: number };

export type PopupPlacement = {
  left: number;
  top: number;
  placement: "above" | "below" | "left" | "right";
  maxWidth: number;
  maxHeight: number;
};

/** Recorded children of `id`, in stable first-seen order, de-duplicated by ID. */
export function immediateChildIds(graph: TreeGraph, id: string): string[] {
  const seen = new Set<string>();
  const children: string[] = [];
  for (const link of graph.links) {
    if (link.kind !== "parent_child" || link.from_id !== id) continue;
    if (seen.has(link.to_id)) continue;
    seen.add(link.to_id);
    children.push(link.to_id);
  }
  return children;
}

/** Recorded parents of `id`, in stable first-seen order, de-duplicated by ID. */
export function immediateParentIds(graph: TreeGraph, id: string): string[] {
  const seen = new Set<string>();
  const parents: string[] = [];
  for (const link of graph.links) {
    if (link.kind !== "parent_child" || link.to_id !== id) continue;
    if (seen.has(link.from_id)) continue;
    seen.add(link.from_id);
    parents.push(link.from_id);
  }
  return parents;
}

/**
 * The authorized subgraph for a focused view.
 *
 * It contains the root, the root's recorded children, and, when requested, the
 * root's recorded parents. A recorded spouse is included when the spouse is the
 * root's spouse or a co-parent of an included child. Nothing is inferred, and
 * only links whose endpoints are both included are kept. Namesakes stay apart
 * because the graph is keyed by stable person IDs.
 */
export function focusedGraph(graph: TreeGraph, focus: Focus): TreeGraph {
  const known = new Set(graph.people.map((person) => person.record_id));
  if (!known.has(focus.rootId)) {
    return { people: [], links: [], truncated: graph.truncated };
  }
  const children = new Set(immediateChildIds(graph, focus.rootId));
  const include = new Set<string>([focus.rootId, ...children]);
  if (focus.showParents) {
    for (const parent of immediateParentIds(graph, focus.rootId)) include.add(parent);
  }
  // The root's recorded spouses.
  for (const link of graph.links) {
    if (link.kind !== "spouse") continue;
    if (link.from_id === focus.rootId) include.add(link.to_id);
    if (link.to_id === focus.rootId) include.add(link.from_id);
  }
  // Other recorded parents of the included children (co-parents).
  for (const link of graph.links) {
    if (link.kind !== "parent_child") continue;
    if (children.has(link.to_id)) include.add(link.from_id);
  }
  for (const id of [...include]) if (!known.has(id)) include.delete(id);

  const links: TreeLink[] = [];
  const seen = new Set<string>();
  for (const link of graph.links) {
    if (!include.has(link.from_id) || !include.has(link.to_id)) continue;
    let relevant: boolean;
    if (link.kind === "spouse") {
      relevant = true;
    } else {
      relevant = link.from_id === focus.rootId ||
        (focus.showParents && link.to_id === focus.rootId) ||
        children.has(link.to_id);
    }
    if (!relevant) continue;
    const key = link.kind === "spouse"
      ? "spouse:" + [link.from_id, link.to_id].sort().join(":")
      : "parent_child:" + link.from_id + ":" + link.to_id;
    if (seen.has(key)) continue;
    seen.add(key);
    links.push(link);
  }
  return {
    people: graph.people.filter((person) => include.has(person.record_id)),
    links,
    truncated: graph.truncated,
  };
}

/**
 * Camera for a focused layout: readable scale, root centred horizontally, and
 * the top of the layout visible so parents above the root are not cut off.
 */
export function focusCamera(layout: TreeLayout, rootId: string): { x: number; y: number; width: number; height: number } {
  const width = Math.min(Math.max(layout.width, 600), READABLE_WIDTH);
  const height = Math.min(Math.max(layout.height, 370), READABLE_HEIGHT);
  const root = layout.people.find((person) => person.record_id === rootId);
  if (!root) return { x: 0, y: 0, width, height };
  return { x: root.x + CARD_WIDTH / 2 - width / 2, y: 0, width, height };
}

/**
 * Place a popup next to a card, in coordinates relative to `container`.
 *
 * The SVG viewBox is mapped with its default `xMidYMid meet`, so letterboxing
 * is part of the calculation. Returns `null` when the card has left the visible
 * canvas, which the caller treats as "close the popup". The result is clamped
 * inside the canvas, and `maxHeight` keeps a long popup scrollable instead of
 * escaping the canvas.
 */
export function popupPosition(
  card: { x: number; y: number },
  view: { x: number; y: number; width: number; height: number },
  svg: Box,
  container: Box,
  popup: { width: number; height: number },
  gap = 10,
  padding = 12,
): PopupPlacement | null {
  if (view.width <= 0 || view.height <= 0 || svg.width <= 0 || svg.height <= 0) return null;
  const scale = Math.min(svg.width / view.width, svg.height / view.height);
  if (scale <= 0) return null;
  const contentWidth = view.width * scale;
  const contentHeight = view.height * scale;
  const originX = svg.left + (svg.width - contentWidth) / 2;
  const originY = svg.top + (svg.height - contentHeight) / 2;

  const left = originX + (card.x - view.x) * scale;
  const top = originY + (card.y - view.y) * scale;
  const right = left + CARD_WIDTH * scale;
  const bottom = top + CARD_HEIGHT * scale;
  // The anchor must be inside the drawn graph. The popup itself may use the
  // letterboxed canvas around the SVG, but never leaves the canvas container.
  if (
    right < originX ||
    left > originX + contentWidth ||
    bottom < originY ||
    top > originY + contentHeight
  )
    return null;

  const clipLeft = container.left;
  const clipTop = container.top;
  const clipRight = container.left + container.width;
  const clipBottom = container.top + container.height;
  if (clipRight <= clipLeft || clipBottom <= clipTop) return null;

  const maxWidth = Math.max(0, clipRight - clipLeft - padding * 2);
  const maxHeight = Math.max(0, clipBottom - clipTop - padding * 2);
  if (maxWidth <= 0 || maxHeight <= 0) return null;
  const width = Math.min(popup.width, maxWidth);
  const height = Math.min(popup.height, maxHeight);
  const centreX = (left + right) / 2 - width / 2;
  // Side popups start at the card's top (clamped), so a tall popup still fits
  // beside a card near the top of the canvas.
  const sideTop = Math.max(clipTop + padding,
    Math.min(top, clipBottom - padding - height));
  const sideFitsVertically = sideTop >= clipTop + padding &&
    sideTop + height <= clipBottom - padding;
  const rightSpace = clipRight - padding - (right + gap);
  const leftSpace = left - gap - (clipLeft + padding);
  // A side popup may shrink a little to stay beside the card. In a parent-focused
  // view that keeps the children below uncovered, which matters more than the
  // full width.
  const minimumSideWidth = 220;

  if (sideFitsVertically && rightSpace >= minimumSideWidth) {
    const sideWidth = Math.min(popup.width, rightSpace);
    return {
      left: right + gap - container.left,
      top: sideTop - container.top,
      placement: "right",
      maxWidth: sideWidth,
      maxHeight,
    };
  }
  if (sideFitsVertically && leftSpace >= minimumSideWidth) {
    const sideWidth = Math.min(popup.width, leftSpace);
    return {
      left: left - gap - sideWidth - container.left,
      top: sideTop - container.top,
      placement: "left",
      maxWidth: sideWidth,
      maxHeight,
    };
  }

  const candidates: { placement: "below" | "above"; left: number; top: number }[] = [
    { placement: "below", left: centreX, top: bottom + gap },
    { placement: "above", left: centreX, top: top - gap - height },
  ];
  for (const candidate of candidates) {
    const fitsHorizontally = candidate.left >= clipLeft + padding &&
      candidate.left + width <= clipRight - padding;
    const fitsVertically = candidate.top >= clipTop + padding &&
      candidate.top + height <= clipBottom - padding;
    if (fitsHorizontally && fitsVertically) {
      return {
        left: candidate.left - container.left,
        top: candidate.top - container.top,
        placement: candidate.placement,
        maxWidth: width,
        maxHeight,
      };
    }
  }

  // Nothing fits whole: fall back to below, clamped into the visible canvas.
  const topPos = Math.max(clipTop + padding,
    Math.min(bottom + gap, clipBottom - padding - height));
  const leftPos = Math.max(clipLeft + padding,
    Math.min(centreX, clipRight - padding - width));
  return {
    left: leftPos - container.left,
    top: topPos - container.top,
    placement: "below",
    maxWidth: width,
    maxHeight,
  };
}

/**
 * Collapse set that makes `targetId` visible while keeping every unrelated
 * collapsed branch. layoutTree hides anything below a collapsed node, so every
 * collapsed ancestor is expanded. The upward walk keeps a visited set, so
 * cycles, shared ancestors, and multiple parents terminate.
 */
export function revealPerson(
  graph: TreeGraph, collapsed: ReadonlySet<string>, targetId: string,
): ReadonlySet<string> {
  const parents = new Map<string, string[]>();
  for (const edge of graph.links) {
    if (edge.kind === "parent_child") parents.set(edge.to_id, [...(parents.get(edge.to_id) ?? []), edge.from_id]);
  }
  const ancestors = new Set<string>();
  const queue = [targetId];
  for (let i = 0; i < queue.length; i++) {
    for (const parent of parents.get(queue[i]) ?? []) {
      if (!ancestors.has(parent) && parent !== targetId) {
        ancestors.add(parent);
        queue.push(parent);
      }
    }
  }
  if (![...ancestors].some((id) => collapsed.has(id))) return collapsed;
  return new Set([...collapsed].filter((id) => !ancestors.has(id)));
}

/**
 * Place every node once, even with cycles, multiple parents, or disconnected
 * components. Collapse controls the traversal, never the authoritative data.
 */
export function layoutTree(graph: TreeGraph, collapsed: ReadonlySet<string>): TreeLayout {
  const byId = new Map(graph.people.map((p) => [p.record_id, p]));
  const children = new Map<string, string[]>();
  const adjacent = new Map<string, string[]>();
  const incoming = new Set<string>();
  for (const p of graph.people) {
    children.set(p.record_id, []);
    adjacent.set(p.record_id, []);
  }
  for (const edge of graph.links) {
    adjacent.get(edge.from_id)?.push(edge.to_id);
    adjacent.get(edge.to_id)?.push(edge.from_id);
    if (edge.kind === "parent_child") {
      children.get(edge.from_id)?.push(edge.to_id);
      incoming.add(edge.to_id);
    }
  }
  const componentSeen = new Set<string>();
  const depths = new Map<string, { component: number; depth: number }>();
  let component = 0;
  for (const person of graph.people) {
    if (componentSeen.has(person.record_id)) continue;
    const members: string[] = [];
    const componentQueue = [person.record_id];
    componentSeen.add(person.record_id);
    for (let i = 0; i < componentQueue.length; i++) {
      const id = componentQueue[i];
      members.push(id);
      for (const n of adjacent.get(id) ?? []) {
        if (!componentSeen.has(n)) {
          componentSeen.add(n);
          componentQueue.push(n);
        }
      }
    }
    // A collapse hides reachable descendants, but not the collapsed node
    // itself. Compute that separately from cycle recovery so we never
    // resurrect deliberately hidden branches.
    const hidden = new Set<string>();
    for (const source of members) {
      if (!collapsed.has(source)) continue;
      const seen = new Set([source]);
      const stack = [source];
      while (stack.length) {
        const id = stack.pop()!;
        for (const child of children.get(id) ?? []) {
          if (seen.has(child)) continue;
          seen.add(child);
          hidden.add(child);
          stack.push(child);
        }
      }
    }

    let roots = members.filter((id) => !incoming.has(id) && !hidden.has(id));
    if (roots.length === 0 && members.some((id) => !hidden.has(id))) {
      roots = [members.find((id) => !hidden.has(id))!];
    }
    const queue = roots.map((id) => ({ id, depth: 0 }));
    let processed = 0;
    const visit = () => {
      for (let i = processed; i < queue.length; i++) {
        const { id, depth } = queue[i];
        if (depths.has(id) || hidden.has(id)) continue;
        depths.set(id, { component, depth });
        // Spouses stay at the current generation; they are never inferred parents.
        for (const edge of graph.links) {
          if (edge.kind !== "spouse") continue;
          if (edge.from_id === id && !depths.has(edge.to_id))
            queue.push({ id: edge.to_id, depth });
          if (edge.to_id === id && !depths.has(edge.from_id))
            queue.push({ id: edge.from_id, depth });
        }
        if (!collapsed.has(id)) {
          for (const child of children.get(id) ?? [])
            queue.push({ id: child, depth: depth + 1 });
        }
      }
      processed = queue.length;
    };
    visit();
    // A root elsewhere in a weak component can mask a separate ancestor
    // cycle (A→B→A, B→C, D→C). Seed every unreached, non-hidden node.
    for (const id of members) {
      if (!depths.has(id) && !hidden.has(id)) {
        queue.push({ id, depth: 0 });
        visit();
      }
    }
    component++;
  }
  const rows = new Map<string, string[]>();
  for (const person of graph.people) {
    const position = depths.get(person.record_id);
    if (!position) continue;
    const key = String(position.component) + ":" + String(position.depth);
    rows.set(key, [...(rows.get(key) ?? []), person.record_id]);
  }
  const result: PositionedPerson[] = [];
  // Components are packed into bands. A catalog with no relationships is one
  // component per person, so a single row would make the canvas hundreds of
  // thousands of pixels wide and scale every card down to nothing.
  let cursorX = 24;
  let bandY = 24;
  let bandHeight = 0;
  for (let c = 0; c < component; c++) {
    const levels = [...rows.keys()].filter((key) => key.startsWith(String(c) + ":"));
    const max = Math.max(1, ...levels.map((key) => rows.get(key)?.length ?? 0));
    const componentWidth = max * STEP_X - (STEP_X - CARD_WIDTH);
    const depthCount = Math.max(1, ...levels.map((key) => Number(key.split(":")[1]) + 1));
    const componentHeight = depthCount * STEP_Y - (STEP_Y - CARD_HEIGHT);
    if (cursorX > 24 && cursorX + componentWidth > MAX_BAND_WIDTH) {
      cursorX = 24;
      bandY += bandHeight + COMPONENT_GAP_Y;
      bandHeight = 0;
    }
    for (const key of levels) {
      const depth = Number(key.split(":")[1]);
      (rows.get(key) ?? []).forEach((id, index) => {
        const person = byId.get(id);
        if (!person) return;
        result.push({
          ...person,
          x: cursorX + index * STEP_X,
          y: bandY + depth * STEP_Y,
          childCount: children.get(id)?.length ?? 0,
        });
      });
    }
    cursorX += componentWidth + COMPONENT_GAP_X;
    bandHeight = Math.max(bandHeight, componentHeight);
  }
  const shown = new Set(result.map((p) => p.record_id));
  return {
    people: result,
    links: graph.links.filter((edge) =>
      shown.has(edge.from_id) && shown.has(edge.to_id) &&
      (edge.kind === "spouse" || !collapsed.has(edge.from_id))),
    width: Math.max(600, ...result.map((p) => p.x + CARD_WIDTH + 24)),
    height: Math.max(320, ...result.map((p) => p.y + CARD_HEIGHT + 24)),
  };
}
