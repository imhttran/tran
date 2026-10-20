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
    let roots = members.filter((id) => !incoming.has(id));
    if (roots.length === 0) roots = [members[0]]; // Cycle; choose deterministic root.
    const queue = roots.map((id) => ({ id, depth: 0 }));
    for (let i = 0; i < queue.length; i++) {
      const { id, depth } = queue[i];
      if (depths.has(id)) continue;
      depths.set(id, { component, depth });
      // Spouses stay at the current generation; they are never inferred parents.
      for (const edge of graph.links) {
        if (edge.kind !== "spouse") continue;
        if (edge.from_id === id && !depths.has(edge.to_id)) queue.push({ id: edge.to_id, depth });
        if (edge.to_id === id && !depths.has(edge.from_id)) queue.push({ id: edge.from_id, depth });
      }
      if (!collapsed.has(id)) {
        for (const child of children.get(id) ?? []) queue.push({ id: child, depth: depth + 1 });
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
  let xOffset = 24;
  for (let c = 0; c < component; c++) {
    const levels = [...rows.keys()].filter((key) => key.startsWith(String(c) + ":"));
    const max = Math.max(1, ...levels.map((key) => rows.get(key)?.length ?? 0));
    for (const key of levels) {
      const depth = Number(key.split(":")[1]);
      (rows.get(key) ?? []).forEach((id, index) => {
        const person = byId.get(id);
        if (!person) return;
        result.push({
          ...person,
          x: xOffset + index * STEP_X,
          y: 24 + depth * STEP_Y,
          childCount: children.get(id)?.length ?? 0,
        });
      });
    }
    xOffset += max * STEP_X + 48;
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
