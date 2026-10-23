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
