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
  CARD_HEIGHT, CARD_WIDTH, READABLE_HEIGHT, READABLE_WIDTH, focusCamera, focusedGraph,
  immediateChildIds, immediateParentIds, layoutTree, normalizedName, parseTreeGraph,
  popupPosition, relationLabel, revealPerson,
} from "@/lib/family-tree";
import type { Focus, PopupPlacement, TreeGraph, TreeLink } from "@/lib/family-tree";

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
type HistoryEntry = {
  focus: Focus | null;
  collapsed: ReadonlySet<string>;
  view: View;
  selected: string | null;
  popupOpen: boolean;
  notice: string | null;
};

/** Long enough to swallow the second click of a double-click, short enough not to feel laggy. */
const SINGLE_CLICK_DELAY_MS = 260;
/** A pointer that moves more than this is a drag, never a selection. */
const DRAG_THRESHOLD_PX = 6;
const EMPTY_COLLAPSED: ReadonlySet<string> = new Set();

function statusClass(status: string) {
  return status === "verified" ? "verified" : status === "needs_review" ? "review" : "unverified";
}
function edgeClass(link: TreeLink) {
  return "family-edge family-edge-" + statusClass(link.evidence_status) +
    (link.kind === "spouse" ? " family-edge-spouse" : "");
}
function readableView(layout: { width: number; height: number }): View {
  return {
    x: 0,
    y: 0,
    width: Math.min(Math.max(layout.width, 600), READABLE_WIDTH),
    height: Math.min(Math.max(layout.height, 370), READABLE_HEIGHT),
  };
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
  const [focus, setFocus] = useState<Focus | null>(null);
  const [history, setHistory] = useState<HistoryEntry[]>([]);
  const [popupOpen, setPopupOpen] = useState(false);
  const [placement, setPlacement] = useState<PopupPlacement | null>(null);
  const [notice, setNotice] = useState<string | null>(null);
  const [frame, setFrame] = useState(0);
  const svgRef = useRef<SVGSVGElement>(null);
  const workspaceRef = useRef<HTMLElement>(null);
  const canvasRef = useRef<HTMLDivElement>(null);
  const popupRef = useRef<HTMLDivElement>(null);
  const cardRefs = useRef(new Map<string, SVGGElement>());
  const clickTimer = useRef<number | null>(null);
  const focusedPopupFor = useRef<string | null>(null);
  const press = useRef<{ id: string; x: number; y: number; pointerType: string } | null>(null);
  const fullTreeView = useRef<View | null>(null);
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

  const activeGraph = useMemo(
    () => (graph && focus ? focusedGraph(graph, focus) : graph),
    [graph, focus],
  );
  const layout = useMemo(
    () => (activeGraph ? layoutTree(activeGraph, focus ? EMPTY_COLLAPSED : collapsed) : null),
    [activeGraph, focus, collapsed],
  );
  const byId = useMemo(() => new Map(layout?.people.map((p) => [p.record_id, p]) ?? []), [layout]);
  const matches = useMemo(() => {
    const term = normalizedName(search.trim());
    if (!graph || !term) return [];
    return graph.people.filter((p) => (normalizedName(p.person_label).includes(term) ||
      p.aliases.some((alias) => normalizedName(alias).includes(term)))).slice(0, 25);
  }, [graph, search]);
  const selectedPerson = activeGraph?.people.find((p) => p.record_id === selected) ?? null;
  const focusRoot = focus ? graph?.people.find((p) => p.record_id === focus.rootId) ?? null : null;
  const childCount = graph && selected ? immediateChildIds(graph, selected).length : 0;
  const parentCount = graph && selected ? immediateParentIds(graph, selected).length : 0;
  const immediateLinks = useMemo(
    () => graph && selected
      ? graph.links.filter((link) => link.from_id === selected || link.to_id === selected)
      : [],
    [graph, selected],
  );

  useEffect(() => {
    if (!graph) return;
    setView(readableView(layoutTree(graph, EMPTY_COLLAPSED)));
  }, [graph]); // Only reset the camera when source data changes.

  useEffect(() => {
    if (!popupOpen && !expanded) return;
    const onKeyDown = (event: KeyboardEvent) => {
      if (event.key !== "Escape") return;
      if (popupOpen) {
        event.preventDefault();
        closePopup(selected);
      } else {
        setExpanded(false);
      }
    };
    window.addEventListener("keydown", onKeyDown);
    return () => window.removeEventListener("keydown", onKeyDown);
  }, [popupOpen, expanded, selected]);

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

  useEffect(() => {
    if (!popupOpen) {
      focusedPopupFor.current = null;
      return;
    }
    // Focus once per opened person, after placement makes the popup visible.
    // Repositioning during pan/zoom must not steal focus from its buttons.
    if (!placement || focusedPopupFor.current === selected) return;
    popupRef.current?.focus({ preventScroll: true });
    focusedPopupFor.current = selected;
  }, [popupOpen, selected, placement]);

  useEffect(() => {
    if (!popupOpen) return;
    let start: { x: number; y: number } | null = null;
    const onPointerDown = (event: PointerEvent) => {
      start = { x: event.clientX, y: event.clientY };
    };
    const onPointerUp = (event: PointerEvent) => {
      const began = start;
      start = null;
      if (!began) return;
      // A drag is a pan, not an outside click: the popup follows the camera.
      if (Math.hypot(event.clientX - began.x, event.clientY - began.y) > DRAG_THRESHOLD_PX) return;
      const target = event.target as Node | null;
      if (!target) return;
      if (popupRef.current?.contains(target)) return;
      // Clicks inside the tree workspace (toolbar, canvas, catalog, legend) are
      // camera or navigation actions, not dismissals. A click elsewhere on the
      // page closes the popup.
      if (workspaceRef.current?.contains(target)) return;
      setPopupOpen(false);
      setPlacement(null);
    };
    document.addEventListener("pointerdown", onPointerDown);
    document.addEventListener("pointerup", onPointerUp);
    return () => {
      document.removeEventListener("pointerdown", onPointerDown);
      document.removeEventListener("pointerup", onPointerUp);
    };
  }, [popupOpen, selected]);

  useEffect(() => {
    if (!popupOpen) return;
    const bump = () => setFrame((current) => current + 1);
    window.addEventListener("resize", bump);
    window.addEventListener("scroll", bump, true);
    let observer: ResizeObserver | null = null;
    if (typeof ResizeObserver !== "undefined") {
      observer = new ResizeObserver(bump);
      if (canvasRef.current) observer.observe(canvasRef.current);
      if (svgRef.current) observer.observe(svgRef.current);
      if (popupRef.current) observer.observe(popupRef.current);
    }
    return () => {
      window.removeEventListener("resize", bump);
      window.removeEventListener("scroll", bump, true);
      observer?.disconnect();
    };
  }, [popupOpen]);

  // Reposition on pan, zoom, resize, and full-screen changes. A null result
  // means the anchor left the visible canvas, so the popup closes.
  useEffect(() => {
    if (!popupOpen) {
      if (placement !== null) setPlacement(null);
      return;
    }
    const svg = svgRef.current;
    const container = canvasRef.current;
    const popup = popupRef.current;
    if (!svg || !container || !popup || !selected || !layout) return;
    const person = layout.people.find((p) => p.record_id === selected);
    if (!person) {
      setPopupOpen(false);
      setPlacement(null);
      return;
    }
    const svgBox = svg.getBoundingClientRect();
    const containerBox = container.getBoundingClientRect();
    const popupBox = popup.getBoundingClientRect();
    const next = popupPosition(
      { x: person.x, y: person.y },
      view,
      { left: svgBox.left, top: svgBox.top, width: svgBox.width, height: svgBox.height },
      { left: containerBox.left, top: containerBox.top, width: containerBox.width, height: containerBox.height },
      { width: popupBox.width, height: popupBox.height },
    );
    if (!next) {
      setPopupOpen(false);
      setPlacement(null);
      return;
    }
    // Reposition without re-rendering when the computed position is unchanged.
    setPlacement((current) => current &&
      Math.abs(current.left - next.left) < 0.5 &&
      Math.abs(current.top - next.top) < 0.5 &&
      current.placement === next.placement &&
      Math.abs(current.maxHeight - next.maxHeight) < 0.5
      ? current
      : next);
  }); // Every camera/viewport change must reposition the popup.

  useEffect(() => () => cancelScheduledOpen(), []);

  function cancelScheduledOpen() {
    if (clickTimer.current !== null) {
      window.clearTimeout(clickTimer.current);
      clickTimer.current = null;
    }
  }

  function openPopup(id: string) {
    cancelScheduledOpen();
    setSelected(id);
    setNotice(null);
    setPlacement(null);
    setPopupOpen(true);
  }

  function closePopup(restoreTo: string | null) {
    setPopupOpen(false);
    setPlacement(null);
    if (restoreTo) cardRefs.current.get(restoreTo)?.focus({ preventScroll: true });
  }

  function scheduleOpen(id: string, pointerType: string) {
    cancelScheduledOpen();
    setSelected(id);
    setNotice(null);
    setPlacement(null);
    if (pointerType === "touch") {
      // No hover or double-tap on touch: a tap opens the popup, and the visible
      // buttons provide the generation navigation.
      setPopupOpen(true);
      return;
    }
    clickTimer.current = window.setTimeout(() => {
      clickTimer.current = null;
      setPopupOpen(true);
    }, SINGLE_CLICK_DELAY_MS);
  }

  function pushHistory() {
    setHistory((entries) => [
      ...entries.slice(-29),
      { focus, collapsed, view, selected, popupOpen, notice },
    ]);
  }

  function cameraFor(next: Focus): View | null {
    if (!graph) return null;
    return focusCamera(layoutTree(focusedGraph(graph, next), EMPTY_COLLAPSED), next.rootId);
  }

  function applyFocus(next: Focus) {
    if (!graph) return;
    if (focus && focus.rootId === next.rootId && focus.showParents === next.showParents) {
      const camera = cameraFor(next);
      if (camera) setView(camera);
      return;
    }
    pushHistory();
    if (!focus) fullTreeView.current = view;
    setFocus(next);
    setCollapsed(new Set());
    setSelected(next.rootId);
    setPopupOpen(true);
    setNotice(null);
    setPlacement(null);
    const camera = cameraFor(next);
    if (camera) setView(camera);
  }

  function focusChildren(id: string) {
    if (!graph) return;
    const person = graph.people.find((p) => p.record_id === id);
    setSelected(id);
    setPopupOpen(true);
    if (immediateChildIds(graph, id).length === 0) {
      setNotice("No children recorded for " + (person?.person_label ?? id) + ".");
      return;
    }
    setNotice(null);
    applyFocus({ rootId: id, showParents: false });
  }

  function showParents(id: string) {
    if (!graph) return;
    const person = graph.people.find((p) => p.record_id === id);
    setSelected(id);
    setPopupOpen(true);
    if (immediateParentIds(graph, id).length === 0) {
      setNotice("No parents recorded for " + (person?.person_label ?? id) + ".");
      return;
    }
    setNotice(null);
    applyFocus({ rootId: id, showParents: true });
  }

  function goBack() {
    if (history.length === 0) return;
    const previous = history[history.length - 1];
    setHistory(history.slice(0, -1));
    setFocus(previous.focus);
    setCollapsed(previous.collapsed);
    setView(previous.view);
    setSelected(previous.selected);
    setPopupOpen(previous.popupOpen);
    setPlacement(null);
    setNotice(previous.notice);
  }

  function showEntireTree() {
    if (!focus || !graph) return;
    pushHistory();
    setFocus(null);
    setCollapsed(new Set());
    setPopupOpen(false);
    setPlacement(null);
    setNotice(null);
    setView(fullTreeView.current ?? readableView(layoutTree(graph, EMPTY_COLLAPSED)));
  }

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
    if (focus) {
      pushHistory();
      setFocus(null);
    }
    // Expand only the hidden ancestor path; unrelated collapsed branches stay.
    const nextCollapsed = revealPerson(graph, collapsed, id);
    setCollapsed(nextCollapsed);
    setSelected(id);
    setSearch("");
    setPopupOpen(true);
    setPlacement(null);
    setNotice(null);
    const positioned = layoutTree(graph, nextCollapsed).people.find((p) => p.record_id === id);
    if (positioned) setView((old) => focusViewFor(positioned, old));
    svgRef.current?.scrollIntoView?.({ block: "nearest" });
  }

  function handleCardPointerUp(event: ReactPointerEvent<SVGGElement>, id: string) {
    const start = press.current;
    press.current = null;
    if (event.currentTarget.hasPointerCapture(event.pointerId)) {
      event.currentTarget.releasePointerCapture(event.pointerId);
    }
    if (!start || start.id !== id) return;
    const moved = Math.hypot(event.clientX - start.x, event.clientY - start.y);
    if (moved > DRAG_THRESHOLD_PX) return; // a drag is never a selection
    scheduleOpen(id, start.pointerType);
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
        <section ref={workspaceRef}
          className={"family-workspace" + (expanded ? " family-workspace-expanded" : "")}
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
              <button type="button" onClick={goBack} disabled={history.length === 0}
                data-testid="family-back">Back</button>
              <button type="button" onClick={showEntireTree} disabled={!focus}
                data-testid="family-show-entire-tree">Show entire tree</button>
              <button type="button" onClick={() => setExpanded((current) => !current)}
                aria-pressed={expanded} data-testid="family-canvas-toggle"
                aria-label={expanded ? "Exit full screen" : "Expand canvas to full screen"}>
                {expanded ? "Exit full screen" : "Expand canvas"}
              </button>
            </div>
          </div>
          {graph.truncated && (
            <p role="status" className="family-limit">
              Showing the first 500 authorized people. This is a partial graph; counts and relatives may be incomplete.
            </p>
          )}
          {focus && (
            <p className="family-focus" role="status" data-testid="family-focus">
              Showing {focusRoot?.person_label ?? focus.rootId} and immediate{" "}
              {focus.showParents ? "parents and children" : "children"}. Use Back to return.
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
              <div className="family-canvas" ref={canvasRef} data-testid="family-canvas">
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
                  {layout.people.map((person) => {
                    const isSelected = selected === person.record_id;
                    return (
                      <g key={person.record_id} className="family-node"
                        ref={(node) => {
                          if (node) cardRefs.current.set(person.record_id, node);
                          else cardRefs.current.delete(person.record_id);
                        }}
                        role="button" tabIndex={0}
                        aria-label={"Select " + person.person_label}
                        aria-pressed={isSelected}
                        aria-haspopup="dialog"
                        aria-expanded={isSelected && popupOpen}
                        onPointerDown={(event) => {
                          event.stopPropagation();
                          press.current = { id: person.record_id, x: event.clientX,
                            y: event.clientY, pointerType: event.pointerType };
                          event.currentTarget.setPointerCapture(event.pointerId);
                        }}
                        onPointerUp={(event) => {
                          event.stopPropagation();
                          handleCardPointerUp(event, person.record_id);
                        }}
                        onPointerCancel={() => { press.current = null; }}
                        onDoubleClick={(event) => {
                          event.stopPropagation();
                          cancelScheduledOpen();
                          focusChildren(person.record_id);
                        }}
                        onKeyDown={(event) => {
                          if (event.key === "Enter" || event.key === " ") {
                            event.preventDefault();
                            openPopup(person.record_id);
                          }
                        }}
                        data-testid="family-person">
                        <rect x={person.x} y={person.y} rx="11"
                          width={CARD_WIDTH} height={CARD_HEIGHT}
                          className={isSelected ? "family-node-selected" : ""} />
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
                    );
                  })}
                </svg>
                {popupOpen && selectedPerson && (
                  <div ref={popupRef} className="family-popup" role="dialog" aria-modal="false"
                    aria-labelledby="family-popup-title" tabIndex={-1}
                    data-testid="family-person-details"
                    data-placement={placement?.placement ?? "below"}
                    style={{
                      left: placement?.left ?? 0,
                      top: placement?.top ?? 0,
                      visibility: placement ? "visible" : "hidden",
                      maxHeight: placement?.maxHeight,
                    }}
                    onDoubleClick={(event) => event.stopPropagation()}>
                    <button type="button" className="family-popup-close" aria-label="Close person details"
                      data-testid="family-popup-close" onClick={() => closePopup(selected)}>×</button>
                    <h2 id="family-popup-title">{selectedPerson.person_label}</h2>
                    <p className="hint">Catalog ID: {selectedPerson.record_id}</p>
                    <EvidenceBadge status={selectedPerson.evidence_status} />
                    <p className="meta" data-testid="family-popup-summary">
                      {childCount} recorded {childCount === 1 ? "child" : "children"} ·{" "}
                      {parentCount} recorded {parentCount === 1 ? "parent" : "parents"}
                    </p>
                    {notice && (
                      <p className="family-popup-notice" role="status" data-testid="family-popup-notice">
                        {notice}
                      </p>
                    )}
                    {graph.truncated && (
                      <p className="hint" data-testid="family-popup-truncated">
                        This graph is partial; counts and relatives may be incomplete.
                      </p>
                    )}
                    <div className="family-popup-actions">
                      {/* The tree payload's record_id is the person key. */}
                      <Link className="family-popup-action"
                        href={`/family/${encodeURIComponent(selectedPerson.record_id)}`}
                        data-testid="open-family-profile">View profile</Link>
                      <button type="button" className="family-popup-action"
                        onClick={() => showParents(selectedPerson.record_id)}
                        data-testid="family-show-parents">Show parents ({parentCount})</button>
                      <button type="button" className="family-popup-action"
                        onClick={() => focusChildren(selectedPerson.record_id)}
                        data-testid="family-show-children">Show children ({childCount})</button>
                    </div>
                    {childCount === 0 && (
                      <p className="hint" data-testid="family-no-children">No children recorded.</p>
                    )}
                    {parentCount === 0 && (
                      <p className="hint" data-testid="family-no-parents">No parents recorded.</p>
                    )}
                    <h3>Recorded relationships</h3>
                    {immediateLinks.length === 0 ? (
                      <p className="hint">
                        No reviewed relationships are recorded for this person yet.
                        Relationship statements that are still unconfirmed appear
                        under recorded assertions with their evidence.
                      </p>
                    ) : (
                      <ul className="family-detail-list">
                        {immediateLinks.map((link, index) => {
                          const otherId = link.from_id === selected ? link.to_id : link.from_id;
                          const other = graph.people.find((candidate) => candidate.record_id === otherId);
                          const inView = activeGraph?.people.some((candidate) => candidate.record_id === otherId);
                          return (
                            <li key={link.kind + ":" + otherId + ":" + index} data-testid="family-relative">
                              <strong className="family-relation" data-testid="family-relation">
                                {selected ? relationLabel(link, selected) : ""}
                              </strong>{" "}
                              <button type="button" className="family-text-button"
                                onClick={() => inView ? openPopup(otherId) : selectPerson(otherId)}>
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
                  </div>
                )}
                <p className="family-canvas-help">
                  Select a person for details. Double-click a card, or use Show children, to focus one
                  generation; Show parents adds the recorded parents. Drag blank space to pan; use ＋/− to zoom.
                </p>
                {!focus && selected && layout.people.some((p) => p.record_id === selected && p.childCount > 0) && (
                  <button type="button" data-testid="family-collapse" onClick={() =>
                    setCollapsed((current) => {
                      const next = new Set(current);
                      if (next.has(selected)) next.delete(selected);
                      else next.add(selected);
                      return next;
                    })}>{collapsed.has(selected) ? "Expand descendants" : "Collapse descendants"}</button>
                )}
              </div>
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
