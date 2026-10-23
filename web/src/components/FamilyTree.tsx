"use client";

import { useEffect, useMemo, useRef, useState } from "react";
import Link from "next/link";
import type { PointerEvent as ReactPointerEvent, WheelEvent as ReactWheelEvent } from "react";

import { DenialView } from "@/components/DenialView";
import { EvidenceBadge, EvidenceList } from "@/components/EvidenceBadge";
import { ExperienceShell } from "@/components/ExperienceShell";
import { PageHeader } from "@/components/PageHeader";
import { PageTitle } from "@/components/PageTitle";
import { SourceDialog } from "@/components/SourceDetails";
import type { CitedFact, OpenSource } from "@/components/SourceDetails";
import type { EvidenceRef } from "@/lib/experience-contract";
import {
  CARD_HEIGHT, CARD_WIDTH, READABLE_HEIGHT, READABLE_WIDTH, layoutTree, normalizedName,
  parseTreeGraph, relationLabel, revealPerson,
} from "@/lib/family-tree";
import type { TreeGraph, TreeLink } from "@/lib/family-tree";

type PersonAssertion = {
  assertion_id: number;
  kind: string;
  key: string;
  value_text: string | null;
  review_status: string;
  evidence: EvidenceRef[];
};
type PersonDetail = {
  record_id: string;
  person_label: string | null;
  assertions: PersonAssertion[];
};
type View = { x: number; y: number; width: number; height: number };
type Denial = { kind: "denied" | "no_access"; reason: string };

function statusClass(status: string) {
  return status === "verified" ? "verified" : status === "needs_review" ? "review" : "unverified";
}
function edgeClass(link: TreeLink) {
  return "family-edge family-edge-" + statusClass(link.evidence_status) +
    (link.kind === "spouse" ? " family-edge-spouse" : "");
}

export default function GenealogyPage() {
  return (
    <ExperienceShell>
      {() => <FamilyTree />}
    </ExperienceShell>
  );
}

function FamilyTree() {
  const [graph, setGraph] = useState<TreeGraph | null>(null);
  const [denial, setDenial] = useState<Denial | null>(null);
  const [search, setSearch] = useState("");
  const [selected, setSelected] = useState<string | null>(null);
  const [collapsed, setCollapsed] = useState<ReadonlySet<string>>(() => new Set());
  const [view, setView] = useState<View>({ x: 0, y: 0, width: 850, height: 530 });
  const [detail, setDetail] = useState<PersonDetail | null>(null);
  const [detailError, setDetailError] = useState<string | null>(null);
  const [openSource, setOpenSource] = useState<OpenSource | null>(null);
  const [expanded, setExpanded] = useState(false);
  const svgRef = useRef<SVGSVGElement>(null);
  const dragging = useRef<{ x: number; y: number; view: View } | null>(null);

  useEffect(() => {
    const controller = new AbortController();
    fetch("/api/genealogy/tree", { cache: "no-store", signal: controller.signal })
      .then(async (response) => {
        if (response.status === 401 || response.status === 403) {
          setDenial({ kind: "denied", reason: "Your account is not authorized to view this family tree." });
          return;
        }
        if (response.status === 404) {
          setDenial({ kind: "no_access", reason: "No authorized family tree is available." });
          return;
        }
        if (!response.ok) throw new Error("Family tree service returned HTTP " + response.status);
        const parsed = parseTreeGraph(await response.json());
        if (!controller.signal.aborted) {
          setGraph(parsed);
          setSelected(parsed.people[0]?.record_id ?? null);
        }
      })
      .catch((error: unknown) => {
        if (!controller.signal.aborted) {
          setDenial({
            kind: "denied",
            reason: error instanceof Error ? error.message : "Family tree is unavailable.",
          });
        }
      });
    return () => controller.abort();
  }, []);

  const layout = useMemo(() => graph ? layoutTree(graph, collapsed) : null, [graph, collapsed]);
  const byId = useMemo(() => new Map(layout?.people.map((p) => [p.record_id, p]) ?? []), [layout]);
  const matches = useMemo(() => {
    const term = normalizedName(search.trim());
    if (!graph || !term) return [];
    return graph.people.filter((p) => (normalizedName(p.person_label).includes(term) ||
      p.aliases.some((alias) => normalizedName(alias).includes(term)))).slice(0, 25);
  }, [graph, search]);
  const selectedPerson = graph?.people.find((p) => p.record_id === selected) ?? null;

  useEffect(() => {
    if (!layout) return;
    // Open at a readable scale. A large disconnected catalog is taller than it
    // is wide, so fitting everything at once would shrink each card to a few
    // pixels; the user can pan or press Fit.
    setView({
      x: 0,
      y: 0,
      width: Math.min(Math.max(layout.width, 600), READABLE_WIDTH),
      height: Math.min(Math.max(layout.height, 370), READABLE_HEIGHT),
    });
  }, [graph]); // Only reset the camera when source data changes, not on collapse.

  useEffect(() => {
    if (!expanded) return;
    const close = (event: KeyboardEvent) => {
      if (event.key === "Escape") setExpanded(false);
    };
    window.addEventListener("keydown", close);
    return () => window.removeEventListener("keydown", close);
  }, [expanded]);

  useEffect(() => {
    if (!selected) {
      setDetail(null);
      return;
    }
    const controller = new AbortController();
    setDetail(null);
    setDetailError(null);
    fetch("/api/genealogy/person/" + encodeURIComponent(selected), {
      cache: "no-store", signal: controller.signal,
    }).then(async (response) => {
      if (!response.ok) throw new Error(
        response.status === 401 || response.status === 403
          ? "Person details are not authorized."
          : "Person details are unavailable.");
      const body = await response.json() as PersonDetail;
      if (!controller.signal.aborted && body.record_id === selected &&
          Array.isArray(body.assertions)) setDetail(body);
    }).catch((error: unknown) => {
      if (!controller.signal.aborted) {
        setDetailError(error instanceof Error ? error.message : "Person details are unavailable.");
      }
    });
    return () => controller.abort();
  }, [selected]);

  /**
   * Centre a card at a readable scale. A tighter zoom the user already chose is
   * kept, but a whole-canvas view is never inherited: that is what shrank the
   * selected card to nothing.
   */
  function focusViewFor(positioned: { x: number; y: number }, old: View): View {
    const width = Math.min(old.width, READABLE_WIDTH);
    const height = Math.min(old.height, READABLE_HEIGHT);
    return {
      x: positioned.x + CARD_WIDTH / 2 - width / 2,
      y: positioned.y + CARD_HEIGHT / 2 - height / 2,
      width,
      height,
    };
  }

  /** Show the whole graph. A large catalog is necessarily scaled down here. */
  function fitAll() {
    if (!layout) return;
    setView({ x: 0, y: 0, width: Math.max(600, layout.width),
      height: Math.max(370, layout.height) });
  }

  /** Return to the selected person at a readable scale. */
  function focusSelected() {
    if (!layout || !selected) return;
    const positioned = layout.people.find((p) => p.record_id === selected);
    if (!positioned) return;
    setView((old) => focusViewFor(positioned, old));
  }

  function selectPerson(id: string) {
    if (!graph) return;
    // Expand only the hidden ancestor path; unrelated collapsed branches stay.
    const nextCollapsed = revealPerson(graph, collapsed, id);
    setCollapsed(nextCollapsed);
    setSelected(id);
    setSearch("");
    const positioned = layoutTree(graph, nextCollapsed).people.find((p) => p.record_id === id);
    if (positioned) setView((old) => focusViewFor(positioned, old));
    svgRef.current?.scrollIntoView?.({ block: "nearest" });
  }

  // Facts in this authorized view that cite exactly this evidence.
  function openCitation(evidence: EvidenceRef) {
    if (!graph) return;
    const name = (id: string) => graph.people.find((p) => p.record_id === id)?.person_label ?? "Unknown";
    const cites = (refs: EvidenceRef[]) => refs.some((ref) => ref.evidence_id === evidence.evidence_id);
    const citedFor: CitedFact[] = [
      ...graph.links.filter((link) => cites(link.evidence)).map((link) => ({
        label: link.kind === "spouse"
          ? name(link.from_id) + " and " + name(link.to_id) + " — spouses"
          : name(link.from_id) + " — parent of " + name(link.to_id),
        status: link.evidence_status,
      })),
      ...(detail?.assertions ?? []).filter((a) => cites(a.evidence)).map((a) => ({
        label: (detail?.person_label ?? "Person") + " — " + a.kind.replace("_", " ") +
          (a.value_text ? ": " + a.value_text : ""),
        status: a.review_status,
      })),
    ];
    setOpenSource({ evidence, citedFor });
  }

  function scaleView(factor: number) {
    setView((old) => {
      const width = Math.max(280, Math.min(9000, old.width * factor));
      const height = Math.max(175, Math.min(6000, old.height * factor));
      return { x: old.x + (old.width - width) / 2,
        y: old.y + (old.height - height) / 2, width, height };
    });
  }
  function onPointerDown(event: ReactPointerEvent<SVGSVGElement>) {
    dragging.current = { x: event.clientX, y: event.clientY, view };
    event.currentTarget.setPointerCapture(event.pointerId);
  }
  function onPointerMove(event: ReactPointerEvent<SVGSVGElement>) {
    if (!dragging.current || !svgRef.current) return;
    const bounds = svgRef.current.getBoundingClientRect();
    if (!bounds.width || !bounds.height) return;
    const start = dragging.current;
    setView({ ...start.view,
      x: start.view.x - (event.clientX - start.x) * start.view.width / bounds.width,
      y: start.view.y - (event.clientY - start.y) * start.view.height / bounds.height });
  }
  function onWheel(event: ReactWheelEvent<SVGSVGElement>) {
    if (event.ctrlKey || event.metaKey) return;
    scaleView(event.deltaY > 0 ? 1.12 : 0.89);
  }

  return (
    <>
      <PageTitle title="Family tree" />
      <PageHeader title="Family tree"
        subtitle="Explore people, relationships, and their sources. Evidence status is never inferred." />
      {denial ? <DenialView kind={denial.kind} reason={denial.reason} /> : !graph || !layout ? (
        <p className="hint" data-testid="genealogy-loading">Loading family tree…</p>
      ) : (
        <section className={"family-workspace" + (expanded ? " family-workspace-expanded" : "")}
          data-testid="genealogy-tree" data-expanded={expanded ? "true" : "false"}>
          <div className="family-toolbar">
            <div className="family-search">
              <label htmlFor="family-person-search">Find a person</label>
              <input id="family-person-search" type="search" value={search}
                placeholder="Search Vietnamese names or aliases…"
                onChange={(event) => setSearch(event.target.value)}
                data-testid="family-search" />
              {search.trim() && (
                <ul className="family-suggestions" data-testid="family-search-results">
                  {matches.length ? matches.map((person) => (
                    <li key={person.record_id}>
                      <button type="button" onClick={() => selectPerson(person.record_id)}>
                        {person.person_label}
                      </button>
                    </li>
                  )) : <li className="hint">No matching authorized people.</li>}
                </ul>
              )}
            </div>
            <div className="family-controls">
              <button type="button" onClick={() => scaleView(0.8)} aria-label="Zoom in">＋</button>
              <button type="button" onClick={() => scaleView(1.25)} aria-label="Zoom out">−</button>
              <button type="button" onClick={fitAll} data-testid="family-fit-all">Fit all</button>
              <button type="button" onClick={focusSelected} disabled={!selected}
                data-testid="family-focus-selected">Focus selected</button>
              <button type="button" onClick={() => setCollapsed(new Set())}>Expand all</button>
              <button type="button" onClick={() => setExpanded((current) => !current)}
                aria-pressed={expanded} data-testid="family-canvas-toggle"
                aria-label={expanded ? "Exit full screen" : "Expand canvas to full screen"}>
                {expanded ? "Exit full screen" : "Expand canvas"}
              </button>
            </div>
          </div>
          {graph.truncated && (
            <p role="status" className="family-limit">
              Showing the first 500 authorized people. This is a partial graph; not all relatives are shown.
            </p>
          )}
          {graph.links.length === 0 && (
            <p className="hint" data-testid="genealogy-no-links">
              No relationships are available yet, so every authorized person is shown on
              their own. Relationship statements appear on each person&apos;s evidence list
              until a reviewer confirms them.
            </p>
          )}
          {graph.people.length === 0 ? (
            <p className="hint" data-testid="genealogy-empty">No genealogy records are available to your role.</p>
          ) : (
            <div className="family-main">
              <div className="family-canvas">
                <svg ref={svgRef} role="group" aria-label="Interactive family tree"
                  data-testid="family-tree-canvas" viewBox={[
                    view.x, view.y, view.width, view.height,
                  ].join(" ")} onPointerDown={onPointerDown} onPointerMove={onPointerMove}
                  onPointerUp={(event) => { dragging.current = null;
                    if (event.currentTarget.hasPointerCapture(event.pointerId))
                      event.currentTarget.releasePointerCapture(event.pointerId); }}
                  onPointerCancel={() => { dragging.current = null; }}
                  onWheel={onWheel}>
                  <rect x={view.x} y={view.y} width={view.width} height={view.height} fill="transparent"
                    pointerEvents="all" />
                  {layout.links.map((edge, i) => {
                    const from = byId.get(edge.from_id);
                    const to = byId.get(edge.to_id);
                    if (!from || !to) return null;
                    const x1 = from.x + CARD_WIDTH / 2;
                    const y1 = edge.kind === "spouse" ? from.y + CARD_HEIGHT / 2 : from.y + CARD_HEIGHT;
                    const x2 = to.x + CARD_WIDTH / 2;
                    const y2 = edge.kind === "spouse" ? to.y + CARD_HEIGHT / 2 : to.y;
                    return (
                      <path key={edge.kind + ":" + edge.from_id + ":" + edge.to_id + ":" + i}
                        d={"M " + x1 + " " + y1 + " C " + x1 + " " + ((y1 + y2) / 2) +
                          ", " + x2 + " " + ((y1 + y2) / 2) + ", " + x2 + " " + y2}
                        className={edgeClass(edge)} data-testid="family-relationship"
                        data-status={edge.evidence_status} data-kind={edge.kind}>
                        <title>{edge.kind.replace("_", " ") + " — " +
                          edge.evidence_status.replace("_", " ")}</title>
                      </path>
                    );
                  })}
                  {layout.people.map((person) => (
                    <g key={person.record_id} className="family-node"
                      role="button" tabIndex={0}
                      aria-label={"Select " + person.person_label}
                      aria-pressed={selected === person.record_id}
                      onPointerDown={(event) => event.stopPropagation()}
                      onClick={() => setSelected(person.record_id)}
                      onKeyDown={(event) => {
                        if (event.key === "Enter" || event.key === " ") {
                          event.preventDefault(); setSelected(person.record_id);
                        }
                      }}
                      data-testid="family-person">
                      <rect x={person.x} y={person.y} rx="11"
                        width={CARD_WIDTH} height={CARD_HEIGHT}
                        className={selected === person.record_id ? "family-node-selected" : ""} />
                      <text x={person.x + 12} y={person.y + 26}
                        className="family-node-name">
                        {person.person_label.length > 23
                          ? person.person_label.slice(0, 22) + "…" : person.person_label}
                      </text>
                      <text x={person.x + 12} y={person.y + 51}
                        className={"family-node-status family-node-status-" + statusClass(person.evidence_status)}>
                        {person.evidence_status.replace("_", " ")}
                      </text>
                      <title>{person.person_label}</title>
                    </g>
                  ))}
                </svg>
                <p className="family-canvas-help">
                  Select a person to inspect evidence. Drag blank space to pan; use ＋/− to zoom,
                  Fit all to see everyone, and Focus selected to return to the person.
                </p>
                {selected && layout.people.some((p) => p.record_id === selected && p.childCount > 0) && (
                  <button type="button" data-testid="family-collapse" onClick={() =>
                    setCollapsed((current) => {
                      const next = new Set(current);
                      if (next.has(selected)) next.delete(selected);
                      else next.add(selected);
                      return next;
                    })}>{collapsed.has(selected) ? "Expand descendants" : "Collapse descendants"}</button>
                )}
              </div>
              <aside className="family-detail" data-testid="family-person-details">
                {selectedPerson ? (
                  <>
                    <h2>{selectedPerson.person_label}</h2>
                    <p className="hint">Catalog ID: {selectedPerson.record_id}</p>
                    <EvidenceBadge status={selectedPerson.evidence_status} />
                    <p>
                      {/* The tree payload's record_id is the person key. */}
                      <Link
                        className="family-text-button"
                        href={`/family/${encodeURIComponent(selectedPerson.record_id)}`}
                        data-testid="open-family-profile"
                      >
                        View family profile
                      </Link>
                    </p>
                    <h3>Relatives</h3>
                    {graph.links.filter((link) =>
                      link.from_id === selected || link.to_id === selected).length === 0 ? (
                      <p className="hint">
                        No reviewed relationships are recorded for this person yet.
                        Relationship statements that are still unconfirmed appear
                        under recorded assertions with their evidence.
                      </p>
                    ) : (
                      <ul className="family-detail-list">
                        {graph.links.filter((link) =>
                          link.from_id === selected || link.to_id === selected).map((link, i) => {
                          const other = graph.people.find((p) => p.record_id ===
                            (link.from_id === selected ? link.to_id : link.from_id));
                          return (
                            <li key={i} data-testid="family-relative">
                              <strong className="family-relation" data-testid="family-relation">
                                {selected ? relationLabel(link, selected) : ""}
                              </strong>{" "}
                              <button type="button" className="family-text-button"
                                onClick={() => { if (other) selectPerson(other.record_id); }}>
                                {other?.person_label ?? "Unknown"}
                              </button>
                              <p className="meta">{link.evidence_status.replace("_", " ")}</p>
                              <EvidenceList evidence={link.evidence} onOpen={openCitation} />
                            </li>
                          );
                        })}
                      </ul>
                    )}
                    <h3>Recorded assertions</h3>
                    {detailError && <p role="alert" className="hint">{detailError}</p>}
                    {!detail && !detailError && <p className="hint">Loading evidence…</p>}
                    {detail && detail.assertions.length === 0 && (
                      <p className="hint">No assertions available.</p>
                    )}
                    {detail && (
                      <ul className="family-detail-list" data-testid="family-assertions">
                        {detail.assertions.map((assertion) => (
                          <li key={assertion.assertion_id}>
                            <strong>{assertion.kind.replace("_", " ")}</strong>
                            {assertion.value_text && <p>{assertion.value_text}</p>}
                            <span className="meta">{assertion.review_status.replace("_", " ")}</span>
                            <EvidenceList evidence={assertion.evidence} onOpen={openCitation} />
                          </li>
                        ))}
                      </ul>
                    )}
                  </>
                ) : <p className="hint">Select a person.</p>}
              </aside>
            </div>
          )}
          <p className="family-legend">
            <span className="family-key family-key-verified" /> Verified
            <span className="family-key family-key-review" /> Needs review
            <span className="family-key family-key-unverified" /> Unverified
          </p>
          <SourceDialog source={openSource} onClose={() => setOpenSource(null)} />
          <section className="card">
            <h2>Catalog view</h2>
            <ul className="family-catalog" data-testid="genealogy-records">
              {graph.people.map((person) => (
                <li key={person.record_id} data-testid="genealogy-record">
                  <button type="button" className="family-text-button"
                    onClick={() => selectPerson(person.record_id)}>{person.person_label}</button>
                  <EvidenceBadge status={person.evidence_status} />
                </li>
              ))}
            </ul>
          </section>
        </section>
      )}
    </>
  );
}
