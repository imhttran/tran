"use client";

import { useEffect, useMemo, useRef, useState } from "react";
import Link from "next/link";
import type {
  PointerEvent as ReactPointerEvent,
  WheelEvent as ReactWheelEvent,
} from "react";

import { DenialView } from "@/components/DenialView";
import { EvidenceBadge, EvidenceList } from "@/components/EvidenceBadge";
import { ExperienceShell } from "@/components/ExperienceShell";
import { PageHeader } from "@/components/PageHeader";
import { PageTitle } from "@/components/PageTitle";
import {
  PersonalHistoryDialog,
  type HistoryPerson,
} from "@/components/PersonalHistoryDialog";
import { SourceDialog } from "@/components/SourceDetails";
import type { CitedFact, OpenSource } from "@/components/SourceDetails";
import type { EvidenceRef } from "@/lib/experience-contract";
import { cardLifespanLabel, lifespanLabel } from "@/lib/dates";
import {
  CARD_HEIGHT,
  CARD_WIDTH,
  READABLE_HEIGHT,
  READABLE_WIDTH,
  focusCamera,
  focusedGraph,
  immediateChildIds,
  immediateParentIds,
  layoutTree,
  normalizedName,
  parseTreeGraph,
  popupPosition,
  relationLabel,
  revealPerson,
} from "@/lib/family-tree";
import type {
  Focus,
  PopupPlacement,
  TreeGraph,
  TreeLink,
  TreePerson,
} from "@/lib/family-tree";
import { initialLetter } from "@/lib/person-display";

// Only an explicit http(s) URL becomes a link. An alias, a bare handle, or an
// executable scheme stays plain text.
function safeHref(value: string): string | null {
  const trimmed = value.trim();
  if (!/^https?:\/\//i.test(trimmed)) return null;
  try {
    const url = new URL(trimmed);
    return url.protocol === "http:" || url.protocol === "https:"
      ? url.href
      : null;
  } catch {
    return null;
  }
}

// Inline zoom icons. The button keeps the accessible name, so the glyph is
// decorative and hidden from assistive technology.
function ZoomInIcon() {
  return (
    <svg
      viewBox="0 0 20 20"
      width="16"
      height="16"
      aria-hidden="true"
      focusable="false"
    >
      <circle
        cx="8.5"
        cy="8.5"
        r="5.5"
        fill="none"
        stroke="currentColor"
        strokeWidth="2"
      />
      <line
        x1="12.8"
        y1="12.8"
        x2="17"
        y2="17"
        stroke="currentColor"
        strokeWidth="2"
        strokeLinecap="round"
      />
      <line
        x1="8.5"
        y1="5.6"
        x2="8.5"
        y2="11.4"
        stroke="currentColor"
        strokeWidth="2"
        strokeLinecap="round"
      />
      <line
        x1="5.6"
        y1="8.5"
        x2="11.4"
        y2="8.5"
        stroke="currentColor"
        strokeWidth="2"
        strokeLinecap="round"
      />
    </svg>
  );
}
function ZoomOutIcon() {
  return (
    <svg
      viewBox="0 0 20 20"
      width="16"
      height="16"
      aria-hidden="true"
      focusable="false"
    >
      <circle
        cx="8.5"
        cy="8.5"
        r="5.5"
        fill="none"
        stroke="currentColor"
        strokeWidth="2"
      />
      <line
        x1="12.8"
        y1="12.8"
        x2="17"
        y2="17"
        stroke="currentColor"
        strokeWidth="2"
        strokeLinecap="round"
      />
      <line
        x1="5.6"
        y1="8.5"
        x2="11.4"
        y2="8.5"
        stroke="currentColor"
        strokeWidth="2"
        strokeLinecap="round"
      />
    </svg>
  );
}

type PersonAssertion = {
  assertion_id: number;
  kind: string;
  key: string;
  value_text: string | null;
  review_status: string;
  /** Structured date for birth_date/death_date assertions, when present. */
  date?: {
    start: string;
    end: string | null;
    precision: string;
    qualifier: string;
  } | null;
  evidence: EvidenceRef[];
};
type PersonDetail = {
  record_id: string;
  person_label: string | null;
  assertions: PersonAssertion[];
};
type View = { x: number; y: number; width: number; height: number };
type Denial = { kind: "denied" | "no_access"; reason: string };
type Unavailable = { reason: string };
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
/** Popup fade/slide duration; must match the CSS transition. */
const POPUP_ANIMATION_MS = 180;
/** A pointer that moves more than this is a drag, never a selection. */
const DRAG_THRESHOLD_PX = 6;
const EMPTY_COLLAPSED: ReadonlySet<string> = new Set();

function statusClass(status: string) {
  return status === "verified"
    ? "verified"
    : status === "needs_review"
      ? "review"
      : "unverified";
}
function edgeClass(link: TreeLink) {
  return (
    "family-edge family-edge-" +
    statusClass(link.evidence_status) +
    (link.kind === "spouse" ? " family-edge-spouse" : "")
  );
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
  return <ExperienceShell>{() => <FamilyTree />}</ExperienceShell>;
}

function FamilyTree() {
  const [graph, setGraph] = useState<TreeGraph | null>(null);
  const [denial, setDenial] = useState<Denial | null>(null);
  const [unavailable, setUnavailable] = useState<Unavailable | null>(null);
  const [search, setSearch] = useState("");
  const [selected, setSelected] = useState<string | null>(null);
  const [collapsed, setCollapsed] = useState<ReadonlySet<string>>(
    () => new Set(),
  );
  const [view, setView] = useState<View>({
    x: 0,
    y: 0,
    width: 850,
    height: 530,
  });
  const [detail, setDetail] = useState<PersonDetail | null>(null);
  const [detailError, setDetailError] = useState<string | null>(null);
  const [openSource, setOpenSource] = useState<OpenSource | null>(null);
  const [historyPerson, setHistoryPerson] = useState<HistoryPerson | null>(
    null,
  );
  const [expanded, setExpanded] = useState(false);
  const [helpOpen, setHelpOpen] = useState(false);
  const [isTouch, setIsTouch] = useState(false);
  const [focus, setFocus] = useState<Focus | null>(null);
  const [history, setHistory] = useState<HistoryEntry[]>([]);
  const [popupOpen, setPopupOpen] = useState(false);
  const [popupMounted, setPopupMounted] = useState(false);
  const [placement, setPlacement] = useState<PopupPlacement | null>(null);
  const [notice, setNotice] = useState<string | null>(null);
  const [frame, setFrame] = useState(0);
  const treeRequest = useRef<AbortController | null>(null);
  const svgRef = useRef<SVGSVGElement>(null);
  const workspaceRef = useRef<HTMLElement>(null);
  const canvasRef = useRef<HTMLDivElement>(null);
  const popupRef = useRef<HTMLDivElement>(null);
  const helpRef = useRef<HTMLElement>(null);
  const helpButtonRef = useRef<HTMLButtonElement>(null);
  // Either the card's history link (an SVG group) or the popup's Personal
  // history button can open the dialog, so both element kinds are accepted.
  const historyReturnFocus = useRef<HTMLElement | SVGGElement | null>(null);
  const cardRefs = useRef(new Map<string, SVGGElement>());
  const clickTimer = useRef<number | null>(null);
  const focusedPopupFor = useRef<string | null>(null);
  const popupExitTimer = useRef<number | null>(null);
  const reducedMotion = useRef(false);
  const press = useRef<{
    id: string;
    x: number;
    y: number;
    pointerType: string;
  } | null>(null);
  const fullTreeView = useRef<View | null>(null);
  const dragging = useRef<{ x: number; y: number; view: View } | null>(null);

  // A tree failure that is not an authorization outcome. It is retryable, so
  // it stays separate from the denial view; the loader can be re-invoked.
  function loadTree(signal: AbortSignal) {
    fetch("/api/genealogy/tree", { cache: "no-store", signal })
      .then(async (response) => {
        if (response.status === 401 || response.status === 403) {
          setDenial({
            kind: "denied",
            reason: "Your account is not authorized to view this family tree.",
          });
          return;
        }
        if (response.status === 404) {
          setDenial({
            kind: "no_access",
            reason: "No authorized family tree is available.",
          });
          return;
        }
        if (!response.ok) {
          if (response.status >= 500) {
            throw new Error(
              "Family tree service returned HTTP " + response.status,
            );
          }
          setDenial({
            kind: "denied",
            reason: "Family tree service returned HTTP " + response.status,
          });
          return;
        }
        let parsed: TreeGraph;
        try {
          parsed = parseTreeGraph(await response.json());
        } catch (error) {
          // A malformed or self-inconsistent payload is not a service outage.
          // Keep the existing denial behaviour rather than offering a retry that
          // cannot make the same payload valid.
          if (!signal.aborted) {
            setDenial({
              kind: "denied",
              reason:
                error instanceof Error
                  ? error.message
                  : "Invalid genealogy graph",
            });
          }
          return;
        }
        if (!signal.aborted) {
          setDenial(null);
          setUnavailable(null);
          setGraph(parsed);
          setSelected(parsed.people[0]?.record_id ?? null);
        }
      })
      .catch((error: unknown) => {
        if (!signal.aborted) {
          setUnavailable({
            reason:
              error instanceof Error
                ? error.message
                : "Family tree is unavailable.",
          });
        }
      });
  }

  function retryTree() {
    setUnavailable(null);
    treeRequest.current?.abort();
    const controller = new AbortController();
    treeRequest.current = controller;
    loadTree(controller.signal);
  }

  useEffect(() => {
    const controller = new AbortController();
    treeRequest.current = controller;
    loadTree(controller.signal);
    return () => controller.abort();
  }, []); // Load once; retryTree re-issues the request.

  const activeGraph = useMemo(
    () => (graph && focus ? focusedGraph(graph, focus) : graph),
    [graph, focus],
  );
  const layout = useMemo(
    () =>
      activeGraph
        ? layoutTree(activeGraph, focus ? EMPTY_COLLAPSED : collapsed)
        : null,
    [activeGraph, focus, collapsed],
  );
  const byId = useMemo(
    () => new Map(layout?.people.map((p) => [p.record_id, p]) ?? []),
    [layout],
  );
  const matches = useMemo(() => {
    const term = normalizedName(search.trim());
    if (!graph || !term) return [];
    return graph.people
      .filter(
        (p) =>
          normalizedName(p.person_label).includes(term) ||
          p.aliases.some((alias) => normalizedName(alias).includes(term)),
      )
      .slice(0, 25);
  }, [graph, search]);
  const selectedPerson =
    activeGraph?.people.find((p) => p.record_id === selected) ?? null;
  const selectedLifespan = selectedPerson
    ? lifespanLabel(selectedPerson.birth, selectedPerson.death)
    : "";
  const focusRoot = focus
    ? (graph?.people.find((p) => p.record_id === focus.rootId) ?? null)
    : null;
  const childCount =
    graph && selected ? immediateChildIds(graph, selected).length : 0;
  const parentCount =
    graph && selected ? immediateParentIds(graph, selected).length : 0;
  const immediateLinks = useMemo(
    () =>
      graph && selected
        ? graph.links.filter(
            (link) => link.from_id === selected || link.to_id === selected,
          )
        : [],
    [graph, selected],
  );

  useEffect(() => {
    if (!graph) return;
    setView(readableView(layoutTree(graph, EMPTY_COLLAPSED)));
  }, [graph]); // Only reset the camera when source data changes.

  useEffect(() => {
    if (!helpOpen && !popupOpen && !expanded) return;
    const onKeyDown = (event: KeyboardEvent) => {
      if (event.key !== "Escape") return;
      // A modal personal-history dialog owns Escape while it is open.
      if (historyPerson) return;
      if (helpOpen) {
        event.preventDefault();
        closeHelp();
      } else if (popupOpen) {
        event.preventDefault();
        closePopup(selected);
      } else {
        setExpanded(false);
      }
    };
    window.addEventListener("keydown", onKeyDown);
    return () => window.removeEventListener("keydown", onKeyDown);
  }, [helpOpen, popupOpen, expanded, selected, historyPerson]);

  useEffect(() => {
    if (!helpOpen) return;
    helpRef.current?.focus({ preventScroll: true });
    helpRef.current?.scrollIntoView?.({ block: "nearest" });
  }, [helpOpen]);

  useEffect(() => {
    const query = window.matchMedia("(prefers-reduced-motion: reduce)");
    reducedMotion.current = query.matches;
    const onChange = (event: MediaQueryListEvent) => {
      reducedMotion.current = event.matches;
    };
    query.addEventListener?.("change", onChange);
    return () => query.removeEventListener?.("change", onChange);
  }, []);

  useEffect(() => {
    // Touch devices get tap wording; a mouse gets click wording. This reads the
    // pointer capability once, after hydration, so the server render is stable.
    const coarse =
      typeof window !== "undefined" &&
      window.matchMedia?.("(pointer: coarse)").matches;
    setIsTouch(
      Boolean(coarse) ||
        (typeof navigator !== "undefined" && navigator.maxTouchPoints > 0),
    );
  }, []);

  useEffect(() => {
    if (!selected) {
      setDetail(null);
      return;
    }
    const controller = new AbortController();
    setDetail(null);
    setDetailError(null);
    fetch("/api/genealogy/person/" + encodeURIComponent(selected), {
      cache: "no-store",
      signal: controller.signal,
    })
      .then(async (response) => {
        if (!response.ok)
          throw new Error(
            response.status === 401 || response.status === 403
              ? "Person details are not authorized."
              : "Person details are unavailable.",
          );
        const body = (await response.json()) as PersonDetail;
        if (
          !controller.signal.aborted &&
          body.record_id === selected &&
          Array.isArray(body.assertions)
        )
          setDetail(body);
      })
      .catch((error: unknown) => {
        if (!controller.signal.aborted) {
          setDetailError(
            error instanceof Error
              ? error.message
              : "Person details are unavailable.",
          );
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
      if (
        Math.hypot(event.clientX - began.x, event.clientY - began.y) >
        DRAG_THRESHOLD_PX
      )
        return;
      const target = event.target as Node | null;
      if (!target) return;
      if (popupRef.current?.contains(target)) return;
      // Clicks inside the tree workspace (toolbar, canvas, catalog, legend) are
      // camera or navigation actions, not dismissals. A click elsewhere on the
      // page closes the popup.
      if (workspaceRef.current?.contains(target)) return;
      dismissPopup(null);
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
    if (!popupOpen) return; // keep the last placement while the exit animation runs
    const svg = svgRef.current;
    const container = canvasRef.current;
    const popup = popupRef.current;
    if (!svg || !container || !popup || !selected || !layout) return;
    const person = layout.people.find((p) => p.record_id === selected);
    if (!person) {
      setPopupOpen(false);
      setPopupMounted(false);
      setPlacement(null);
      return;
    }
    const svgBox = svg.getBoundingClientRect();
    const containerBox = container.getBoundingClientRect();
    const popupBox = popup.getBoundingClientRect();
    const next = popupPosition(
      { x: person.x, y: person.y },
      view,
      {
        left: svgBox.left,
        top: svgBox.top,
        width: svgBox.width,
        height: svgBox.height,
      },
      {
        left: containerBox.left,
        top: containerBox.top,
        width: containerBox.width,
        height: containerBox.height,
      },
      { width: popupBox.width, height: popupBox.height },
    );
    if (!next) {
      // The anchor left the visible canvas: unmount immediately, no exit slide.
      setPopupOpen(false);
      setPopupMounted(false);
      setPlacement(null);
      return;
    }
    // Reposition without re-rendering when the computed position is unchanged.
    setPlacement((current) =>
      current &&
      Math.abs(current.left - next.left) < 0.5 &&
      Math.abs(current.top - next.top) < 0.5 &&
      current.placement === next.placement &&
      Math.abs(current.maxHeight - next.maxHeight) < 0.5
        ? current
        : next,
    );
  }); // Every camera/viewport change must reposition the popup.

  useEffect(
    () => () => {
      cancelScheduledOpen();
      if (popupExitTimer.current !== null)
        window.clearTimeout(popupExitTimer.current);
    },
    [],
  );

  function closeHelp() {
    setHelpOpen(false);
    helpButtonRef.current?.focus({ preventScroll: true });
  }

  function openPersonalHistory(
    person: TreePerson,
    trigger: HTMLElement | SVGGElement,
  ) {
    cancelScheduledOpen();
    historyReturnFocus.current = trigger;
    setHistoryPerson({
      person_key: person.record_id,
      person_label: person.person_label,
    });
  }

  function closePersonalHistory() {
    setHistoryPerson(null);
    const target = historyReturnFocus.current;
    window.setTimeout(() => target?.focus({ preventScroll: true }), 0);
  }

  function cancelScheduledOpen() {
    if (clickTimer.current !== null) {
      window.clearTimeout(clickTimer.current);
      clickTimer.current = null;
    }
  }

  /** Show the popup (or keep it mounted) and cancel any running exit timer. */
  function showPopup() {
    if (popupExitTimer.current !== null) {
      window.clearTimeout(popupExitTimer.current);
      popupExitTimer.current = null;
    }
    setPopupMounted(true);
    setPopupOpen(true);
  }

  /**
   * Close the popup immediately for interaction purposes, then unmount it after
   * the fade. Clicks, focus, and Escape are never delayed by the animation.
   */
  function dismissPopup(restoreTo: string | null) {
    setPopupOpen(false);
    if (restoreTo)
      cardRefs.current.get(restoreTo)?.focus({ preventScroll: true });
    if (popupExitTimer.current !== null)
      window.clearTimeout(popupExitTimer.current);
    const delay = reducedMotion.current ? 0 : POPUP_ANIMATION_MS;
    if (delay === 0) {
      setPopupMounted(false);
      setPlacement(null);
      return;
    }
    popupExitTimer.current = window.setTimeout(() => {
      popupExitTimer.current = null;
      setPopupMounted(false);
      setPlacement(null);
    }, delay);
  }

  function openPopup(id: string) {
    cancelScheduledOpen();
    setSelected(id);
    setNotice(null);
    setPlacement(null);
    showPopup();
  }

  function closePopup(restoreTo: string | null) {
    dismissPopup(restoreTo);
  }

  function scheduleOpen(id: string, pointerType: string) {
    cancelScheduledOpen();
    setSelected(id);
    setNotice(null);
    setPlacement(null);
    if (pointerType === "touch") {
      // No hover or double-tap on touch: a tap opens the popup, and the visible
      // buttons provide the generation navigation.
      showPopup();
      return;
    }
    clickTimer.current = window.setTimeout(() => {
      clickTimer.current = null;
      showPopup();
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
    return focusCamera(
      layoutTree(focusedGraph(graph, next), EMPTY_COLLAPSED),
      next.rootId,
    );
  }

  function applyFocus(next: Focus) {
    if (!graph) return;
    if (
      focus &&
      focus.rootId === next.rootId &&
      focus.showParents === next.showParents
    ) {
      const camera = cameraFor(next);
      if (camera) setView(camera);
      return;
    }
    pushHistory();
    if (!focus) fullTreeView.current = view;
    setFocus(next);
    setCollapsed(new Set());
    setSelected(next.rootId);
    showPopup();
    setNotice(null);
    setPlacement(null);
    const camera = cameraFor(next);
    if (camera) setView(camera);
  }

  function focusChildren(id: string) {
    if (!graph) return;
    const person = graph.people.find((p) => p.record_id === id);
    setSelected(id);
    showPopup();
    if (immediateChildIds(graph, id).length === 0) {
      setNotice(
        "No children recorded for " + (person?.person_label ?? id) + ".",
      );
      return;
    }
    setNotice(null);
    applyFocus({ rootId: id, showParents: false });
  }

  function showParents(id: string) {
    if (!graph) return;
    const person = graph.people.find((p) => p.record_id === id);
    setSelected(id);
    showPopup();
    if (immediateParentIds(graph, id).length === 0) {
      setNotice(
        "No parents recorded for " + (person?.person_label ?? id) + ".",
      );
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
    if (previous.popupOpen) {
      showPopup();
    } else {
      setPopupOpen(false);
      setPopupMounted(false);
      setPlacement(null);
    }
    setNotice(previous.notice);
  }

  function showEntireTree() {
    if (!focus || !graph) return;
    pushHistory();
    setFocus(null);
    setCollapsed(new Set());
    dismissPopup(null);
    setNotice(null);
    setView(
      fullTreeView.current ?? readableView(layoutTree(graph, EMPTY_COLLAPSED)),
    );
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
    setView({
      x: 0,
      y: 0,
      width: Math.max(600, layout.width),
      height: Math.max(370, layout.height),
    });
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
    showPopup();
    setPlacement(null);
    setNotice(null);
    const positioned = layoutTree(graph, nextCollapsed).people.find(
      (p) => p.record_id === id,
    );
    if (positioned) setView((old) => focusViewFor(positioned, old));
    svgRef.current?.scrollIntoView?.({ block: "nearest" });
  }

  function handleCardPointerUp(
    event: ReactPointerEvent<SVGGElement>,
    id: string,
  ) {
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
    const name = (id: string) =>
      graph.people.find((p) => p.record_id === id)?.person_label ?? "Unknown";
    const cites = (refs: EvidenceRef[]) =>
      refs.some((ref) => ref.evidence_id === evidence.evidence_id);
    const citedFor: CitedFact[] = [
      ...graph.links
        .filter((link) => cites(link.evidence))
        .map((link) => ({
          label:
            link.kind === "spouse"
              ? name(link.from_id) + " and " + name(link.to_id) + " — spouses"
              : name(link.from_id) + " — parent of " + name(link.to_id),
          status: link.evidence_status,
        })),
      ...(detail?.assertions ?? [])
        .filter((a) => cites(a.evidence))
        .map((a) => ({
          label:
            (detail?.person_label ?? "Person") +
            " — " +
            a.kind.replace("_", " ") +
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
      return {
        x: old.x + (old.width - width) / 2,
        y: old.y + (old.height - height) / 2,
        width,
        height,
      };
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
    setView({
      ...start.view,
      x:
        start.view.x -
        ((event.clientX - start.x) * start.view.width) / bounds.width,
      y:
        start.view.y -
        ((event.clientY - start.y) * start.view.height) / bounds.height,
    });
  }
  function onWheel(event: ReactWheelEvent<SVGSVGElement>) {
    if (event.ctrlKey || event.metaKey) return;
    scaleView(event.deltaY > 0 ? 1.12 : 0.89);
  }

  return (
    <>
      <PageTitle title="Family tree" />
      <PageHeader
        title="Family tree"
        subtitle="Explore people, relationships, and their sources. Evidence status is never inferred."
      />
      {denial ? (
        <DenialView kind={denial.kind} reason={denial.reason} />
      ) : unavailable ? (
        <div
          className="family-unavailable"
          role="alert"
          data-testid="genealogy-unavailable"
        >
          <h2>The family tree is temporarily unavailable</h2>
          <p className="hint">
            The archive could not load the tree. This is not an access decision.
            Check your connection and try again.
          </p>
          <p className="hint" data-testid="genealogy-unavailable-reason">
            {unavailable.reason}
          </p>
          <button
            type="button"
            onClick={retryTree}
            data-testid="genealogy-unavailable-retry"
          >
            Retry
          </button>
        </div>
      ) : !graph || !layout ? (
        <p className="hint" data-testid="genealogy-loading">
          Loading family tree…
        </p>
      ) : (
        <section
          ref={workspaceRef}
          className={
            "family-workspace" + (expanded ? " family-workspace-expanded" : "")
          }
          data-testid="genealogy-tree"
          data-expanded={expanded ? "true" : "false"}
        >
          <div className="family-toolbar">
            <div className="family-search">
              <label htmlFor="family-person-search">Find a person</label>
              <input
                id="family-person-search"
                type="search"
                value={search}
                placeholder="Search Vietnamese names or aliases…"
                onChange={(event) => setSearch(event.target.value)}
                data-testid="family-search"
              />
              {search.trim() && (
                <ul
                  className="family-suggestions"
                  data-testid="family-search-results"
                >
                  {matches.length ? (
                    matches.map((person) => (
                      <li key={person.record_id}>
                        <button
                          type="button"
                          onClick={() => selectPerson(person.record_id)}
                        >
                          {person.person_label}
                        </button>
                      </li>
                    ))
                  ) : (
                    <li className="hint">No matching authorized people.</li>
                  )}
                </ul>
              )}
            </div>
            <div className="family-controls">
              <div
                className="family-control-group"
                role="group"
                aria-label="Zoom and view"
              >
                <button
                  type="button"
                  className="family-control family-control-icon"
                  onClick={() => scaleView(0.8)}
                  aria-label="Zoom in"
                >
                  <ZoomInIcon />
                </button>
                <button
                  type="button"
                  className="family-control family-control-icon"
                  onClick={() => scaleView(1.25)}
                  aria-label="Zoom out"
                >
                  <ZoomOutIcon />
                </button>
                <button
                  type="button"
                  className="family-control family-control-primary"
                  onClick={fitAll}
                  data-testid="family-fit-all"
                >
                  Fit all
                </button>
                <button
                  type="button"
                  className="family-control"
                  onClick={focusSelected}
                  disabled={!selected}
                  data-testid="family-focus-selected"
                >
                  Focus selected
                </button>
                <button
                  type="button"
                  className="family-control"
                  onClick={() => setCollapsed(new Set())}
                >
                  Expand all
                </button>
              </div>
              <div
                className="family-control-group"
                role="group"
                aria-label="Navigation"
              >
                <button
                  type="button"
                  className="family-control"
                  onClick={goBack}
                  disabled={history.length === 0}
                  data-testid="family-back"
                >
                  Back
                </button>
                <button
                  type="button"
                  className="family-control"
                  onClick={showEntireTree}
                  disabled={!focus}
                  data-testid="family-show-entire-tree"
                >
                  Show entire tree
                </button>
                <button
                  type="button"
                  className="family-control"
                  onClick={() => setExpanded((current) => !current)}
                  aria-pressed={expanded}
                  data-testid="family-canvas-toggle"
                  aria-label={
                    expanded
                      ? "Exit full screen"
                      : "Expand canvas to full screen"
                  }
                >
                  {expanded ? "Exit full screen" : "Expand canvas"}
                </button>
              </div>
            </div>
          </div>
          {helpOpen && (
            <section
              id="family-help-panel"
              ref={helpRef}
              className="family-help"
              role="dialog"
              aria-modal="false"
              aria-labelledby="family-help-title"
              tabIndex={-1}
              data-testid="family-help-panel"
            >
              <div className="family-help-head">
                <h2 id="family-help-title">Family tree help</h2>
                <button
                  type="button"
                  className="family-help-close"
                  onClick={closeHelp}
                  data-testid="family-help-close"
                >
                  Close help
                </button>
              </div>
              <div className="family-help-grid">
                <div>
                  <h3>Person details</h3>
                  <p>
                    Click a person to open their details. A card you reached
                    with Tab opens with Enter or Space.
                  </p>
                  <h3>Children and parents</h3>
                  <p>
                    Double-click a person, or choose{" "}
                    <strong>Show children</strong>, to focus that person and
                    their recorded children. <strong>Show parents</strong> adds
                    the recorded parents above. On touch, tap the person and
                    choose the button. Each step moves one generation; choose
                    another card to continue.
                  </p>
                  <h3>Back</h3>
                  <p>
                    <strong>Back</strong> restores the previous focus, expansion
                    state, and camera.
                    <strong> Show entire tree</strong> returns to the whole
                    graph.
                  </p>
                </div>
                <div>
                  <h3>Search</h3>
                  <p>
                    Search ignores Vietnamese accents: <em>Nguyen</em> finds{" "}
                    <em>Nguyễn</em>, and recorded aliases match too.
                  </p>
                  <h3>Canvas controls</h3>
                  <p>
                    <strong>Zoom in</strong> and <strong>Zoom out</strong>{" "}
                    change the scale.
                    <strong> Fit all</strong> shows the whole graph.{" "}
                    <strong>Focus selected</strong> centres the selected card.
                    <strong> Expand canvas</strong> uses the full window; Escape
                    leaves it.
                  </p>
                  <h3>Evidence status</h3>
                  <p>
                    <span className="family-key family-key-verified" />{" "}
                    <strong>Verified</strong> has reviewed, source-backed
                    evidence. <span className="family-key family-key-review" />{" "}
                    <strong>Needs review</strong> is recorded but not confirmed.{" "}
                    <span className="family-key family-key-unverified" />{" "}
                    <strong>Unverified</strong> has no reviewed evidence. Edges
                    use the same status, and citations open the source details.
                  </p>
                </div>
              </div>
              <p
                className="family-help-note"
                data-testid="family-help-no-children"
              >
                <strong>“No children recorded”</strong> means the archive has no
                available child links for that person. It does not establish
                that the person had no children.
              </p>
              <p className="family-help-note">
                Keyboard: Tab moves between controls, Enter or Space activates a
                focused control, and Escape closes this help or the person
                details. When neither is open, Escape leaves expanded canvas.
              </p>
            </section>
          )}
          {graph.truncated && (
            <p role="status" className="family-limit">
              Showing the first 500 authorized people. This is a partial graph;
              counts and relatives may be incomplete.
            </p>
          )}
          {focus && (
            <p
              className="family-focus"
              role="status"
              data-testid="family-focus"
            >
              Showing {focusRoot?.person_label ?? focus.rootId} and immediate{" "}
              {focus.showParents ? "parents and children" : "children"}. Use
              Back to return.
            </p>
          )}
          {graph.links.length === 0 && (
            <p className="hint" data-testid="genealogy-no-links">
              No relationships are available yet, so every authorized person is
              shown on their own. Relationship statements appear on each
              person&apos;s evidence list until a reviewer confirms them.
            </p>
          )}
          {graph.people.length === 0 ? (
            <p className="hint" data-testid="genealogy-empty">
              No genealogy records are available to your role.
            </p>
          ) : (
            <div className="family-main">
              <div
                className="family-canvas"
                ref={canvasRef}
                data-testid="family-canvas"
              >
                <svg
                  ref={svgRef}
                  role="group"
                  aria-label="Interactive family tree"
                  data-testid="family-tree-canvas"
                  viewBox={[view.x, view.y, view.width, view.height].join(" ")}
                  onPointerDown={onPointerDown}
                  onPointerMove={onPointerMove}
                  onPointerUp={(event) => {
                    dragging.current = null;
                    if (event.currentTarget.hasPointerCapture(event.pointerId))
                      event.currentTarget.releasePointerCapture(
                        event.pointerId,
                      );
                  }}
                  onPointerCancel={() => {
                    dragging.current = null;
                  }}
                  onWheel={onWheel}
                >
                  <rect
                    x={view.x}
                    y={view.y}
                    width={view.width}
                    height={view.height}
                    fill="transparent"
                    pointerEvents="all"
                  />
                  {layout.links.map((edge, i) => {
                    const from = byId.get(edge.from_id);
                    const to = byId.get(edge.to_id);
                    if (!from || !to) return null;
                    const x1 = from.x + CARD_WIDTH / 2;
                    const y1 =
                      edge.kind === "spouse"
                        ? from.y + CARD_HEIGHT / 2
                        : from.y + CARD_HEIGHT;
                    const x2 = to.x + CARD_WIDTH / 2;
                    const y2 =
                      edge.kind === "spouse" ? to.y + CARD_HEIGHT / 2 : to.y;
                    return (
                      <path
                        key={
                          edge.kind +
                          ":" +
                          edge.from_id +
                          ":" +
                          edge.to_id +
                          ":" +
                          i
                        }
                        d={
                          "M " +
                          x1 +
                          " " +
                          y1 +
                          " C " +
                          x1 +
                          " " +
                          (y1 + y2) / 2 +
                          ", " +
                          x2 +
                          " " +
                          (y1 + y2) / 2 +
                          ", " +
                          x2 +
                          " " +
                          y2
                        }
                        className={edgeClass(edge)}
                        data-testid="family-relationship"
                        data-status={edge.evidence_status}
                        data-kind={edge.kind}
                      >
                        <title>
                          {edge.kind.replace("_", " ") +
                            " — " +
                            edge.evidence_status.replace("_", " ")}
                        </title>
                      </path>
                    );
                  })}
                  {layout.people.map((person) => {
                    const isSelected = selected === person.record_id;
                    // A lifespan sits beside the status only when it fits the
                    // fixed card width; the popup always shows the full value.
                    const lifespan = cardLifespanLabel(
                      person.birth,
                      person.death,
                    );
                    return (
                      <g
                        key={person.record_id}
                        className="family-node"
                        role="group"
                        aria-label={person.person_label}
                        data-testid="family-person"
                        data-record-id={person.record_id}
                      >
                        <g
                          className="family-node-card"
                          ref={(node) => {
                            if (node)
                              cardRefs.current.set(person.record_id, node);
                            else cardRefs.current.delete(person.record_id);
                          }}
                          role="button"
                          tabIndex={0}
                          aria-label={"Select " + person.person_label}
                          aria-pressed={isSelected}
                          aria-haspopup="dialog"
                          aria-expanded={isSelected && popupOpen}
                          onPointerDown={(event) => {
                            event.stopPropagation();
                            press.current = {
                              id: person.record_id,
                              x: event.clientX,
                              y: event.clientY,
                              pointerType: event.pointerType,
                            };
                            event.currentTarget.setPointerCapture(
                              event.pointerId,
                            );
                          }}
                          onPointerUp={(event) => {
                            event.stopPropagation();
                            handleCardPointerUp(event, person.record_id);
                          }}
                          onPointerCancel={() => {
                            press.current = null;
                          }}
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
                        >
                          <rect
                            x={person.x}
                            y={person.y}
                            rx="11"
                            width={CARD_WIDTH}
                            height={CARD_HEIGHT}
                            className={isSelected ? "family-node-selected" : ""}
                          />
                          {/* Decorative initials avatar: the card's aria-label is
                              the authoritative accessible name. */}
                          <g
                            className="family-node-avatar"
                            aria-hidden="true"
                            data-testid="family-person-avatar"
                          >
                            <circle
                              cx={person.x + 25}
                              cy={person.y + 27}
                              r="15"
                            />
                            <text
                              x={person.x + 25}
                              y={person.y + 27}
                              textAnchor="middle"
                              dominantBaseline="central"
                              className="family-node-avatar-letter"
                            >
                              {initialLetter(person.person_label)}
                            </text>
                          </g>
                          <text
                            x={person.x + 48}
                            y={person.y + 24}
                            className="family-node-name"
                          >
                            {person.person_label.length > 17
                              ? person.person_label.slice(0, 16) + "…"
                              : person.person_label}
                          </text>
                          <text
                            x={person.x + 48}
                            y={person.y + 42}
                            className={
                              "family-node-status family-node-status-" +
                              statusClass(person.evidence_status)
                            }
                          >
                            {person.evidence_status.replace("_", " ")}
                          </text>
                          {lifespan && (
                            <text
                              x={person.x + 48}
                              y={person.y + 58}
                              className="family-node-life"
                              data-testid="family-person-life"
                            >
                              {lifespan}
                            </text>
                          )}
                        </g>
                        {person.history_passages > 0 && (
                          <g
                            className="family-node-history"
                            role="link"
                            tabIndex={0}
                            aria-label={
                              "Personal history for " + person.person_label
                            }
                            aria-haspopup="dialog"
                            data-testid="family-person-history"
                            onPointerDown={(event) => event.stopPropagation()}
                            onPointerUp={(event) => event.stopPropagation()}
                            onClick={(event) => {
                              event.stopPropagation();
                              openPersonalHistory(person, event.currentTarget);
                            }}
                            onDoubleClick={(event) => event.stopPropagation()}
                            onKeyDown={(event) => {
                              if (event.key !== "Enter" && event.key !== " ")
                                return;
                              event.preventDefault();
                              event.stopPropagation();
                              openPersonalHistory(person, event.currentTarget);
                            }}
                          >
                            <rect
                              x={person.x + 8}
                              y={person.y + 54}
                              rx="5"
                              width={CARD_WIDTH - 16}
                              height={18}
                              className="family-node-history-hit"
                            />
                            <text
                              x={person.x + 12}
                              y={person.y + 67}
                              className="family-node-history-label"
                            >
                              Personal history
                            </text>
                          </g>
                        )}
                        <title>{person.person_label}</title>
                      </g>
                    );
                  })}
                </svg>
                {popupMounted && selectedPerson && (
                  <div
                    ref={popupRef}
                    className="family-popup"
                    role="dialog"
                    aria-modal="false"
                    aria-labelledby="family-popup-title"
                    tabIndex={-1}
                    aria-hidden={popupOpen ? undefined : true}
                    data-testid="family-person-details"
                    data-visible={
                      popupOpen && placement !== null ? "true" : "false"
                    }
                    data-placement={placement?.placement ?? "below"}
                    style={{
                      left: placement?.left ?? 0,
                      top: placement?.top ?? 0,
                      visibility: placement ? "visible" : "hidden",
                      maxWidth: placement?.maxWidth,
                      maxHeight: placement?.maxHeight,
                    }}
                    onDoubleClick={(event) => event.stopPropagation()}
                  >
                    <button
                      type="button"
                      className="family-popup-close"
                      aria-label="Close person details"
                      data-testid="family-popup-close"
                      onClick={() => closePopup(selected)}
                    >
                      ×
                    </button>
                    <header
                      className="family-popup-head"
                      data-testid="family-popup-header"
                    >
                      <h2 id="family-popup-title">
                        {selectedPerson.person_label}
                      </h2>
                      <p className="hint">
                        Catalog ID: {selectedPerson.record_id}
                      </p>
                      <EvidenceBadge status={selectedPerson.evidence_status} />
                      {selectedLifespan && (
                        <p className="meta" data-testid="family-popup-lifespan">
                          {selectedLifespan}
                        </p>
                      )}
                      <p className="meta" data-testid="family-popup-summary">
                        {childCount} recorded{" "}
                        {childCount === 1 ? "child" : "children"} ·{" "}
                        {parentCount} recorded{" "}
                        {parentCount === 1 ? "parent" : "parents"}
                      </p>
                    </header>
                    {notice && (
                      <p
                        className="family-popup-notice"
                        role="status"
                        data-testid="family-popup-notice"
                      >
                        {notice}
                      </p>
                    )}
                    {graph.truncated && (
                      <p className="hint" data-testid="family-popup-truncated">
                        This graph is partial; counts and relatives may be
                        incomplete.
                      </p>
                    )}
                    <div
                      className="family-popup-actions"
                      data-testid="family-popup-actions"
                    >
                      {/* The tree payload's record_id is the person key. */}
                      <Link
                        className="family-popup-action"
                        href={`/family/${encodeURIComponent(selectedPerson.record_id)}`}
                        data-testid="open-family-profile"
                      >
                        View profile
                      </Link>
                      <button
                        type="button"
                        className="family-popup-action"
                        onClick={() => showParents(selectedPerson.record_id)}
                        data-testid="family-show-parents"
                      >
                        Show parents ({parentCount})
                      </button>
                      <button
                        type="button"
                        className="family-popup-action"
                        onClick={() => focusChildren(selectedPerson.record_id)}
                        data-testid="family-show-children"
                      >
                        Show children ({childCount})
                      </button>
                      {/* Opens the same dialog as the card link, on demand only. */}
                      {selectedPerson.history_passages > 0 && (
                        <button
                          type="button"
                          className="family-popup-action family-popup-action-secondary"
                          data-testid="family-popup-history"
                          onClick={(event) =>
                            openPersonalHistory(
                              selectedPerson,
                              event.currentTarget,
                            )
                          }
                        >
                          Personal history
                        </button>
                      )}
                    </div>
                    {childCount === 0 && (
                      <p className="hint" data-testid="family-no-children">
                        No children recorded.
                      </p>
                    )}
                    {parentCount === 0 && (
                      <p className="hint" data-testid="family-no-parents">
                        No parents recorded.
                      </p>
                    )}
                    <section
                      className="family-popup-section"
                      data-testid="family-popup-relationships"
                    >
                      <h3>Recorded relationships</h3>
                      {immediateLinks.length === 0 ? (
                        <p className="hint">
                          No reviewed relationships are recorded for this person
                          yet. Relationship statements that are still
                          unconfirmed appear under recorded facts with their
                          evidence.
                        </p>
                      ) : (
                        <ul className="family-detail-list">
                          {immediateLinks.map((link, index) => {
                            const otherId =
                              link.from_id === selected
                                ? link.to_id
                                : link.from_id;
                            const other = graph.people.find(
                              (candidate) => candidate.record_id === otherId,
                            );
                            const inView = activeGraph?.people.some(
                              (candidate) => candidate.record_id === otherId,
                            );
                            return (
                              <li
                                key={link.kind + ":" + otherId + ":" + index}
                                data-testid="family-relative"
                              >
                                <strong
                                  className="family-relation"
                                  data-testid="family-relation"
                                >
                                  {selected
                                    ? relationLabel(link, selected)
                                    : ""}
                                </strong>{" "}
                                <button
                                  type="button"
                                  className="family-text-button"
                                  onClick={() =>
                                    inView
                                      ? openPopup(otherId)
                                      : selectPerson(otherId)
                                  }
                                >
                                  {other?.person_label ?? "Unknown"}
                                </button>
                                <p className="meta">
                                  {link.evidence_status.replace("_", " ")}
                                </p>
                                <EvidenceList
                                  evidence={link.evidence}
                                  onOpen={openCitation}
                                />
                              </li>
                            );
                          })}
                        </ul>
                      )}
                    </section>
                    <section
                      className="family-popup-section"
                      data-testid="family-popup-facts"
                    >
                      <h3>Recorded facts</h3>
                      {detailError && (
                        <p role="alert" className="hint">
                          {detailError}
                        </p>
                      )}
                      {!detail && !detailError && (
                        <p className="hint">Loading evidence…</p>
                      )}
                      {detail && detail.assertions.length === 0 && (
                        <p className="hint">No assertions available.</p>
                      )}
                      {detail && (
                        <ul
                          className="family-detail-list"
                          data-testid="family-assertions"
                        >
                          {detail.assertions.map((assertion) => (
                            <li key={assertion.assertion_id}>
                              <strong>
                                {assertion.kind.replace("_", " ")}
                              </strong>
                              {assertion.kind === "social_link" &&
                              assertion.value_text &&
                              safeHref(assertion.value_text) ? (
                                <p>
                                  <a
                                    href={safeHref(assertion.value_text)!}
                                    target="_blank"
                                    rel="noopener noreferrer"
                                    data-testid="family-social-link"
                                  >
                                    {assertion.value_text}
                                  </a>
                                </p>
                              ) : (
                                assertion.value_text && (
                                  <p>{assertion.value_text}</p>
                                )
                              )}
                              <span className="meta">
                                {assertion.review_status.replace("_", " ")}
                              </span>
                              <EvidenceList
                                evidence={assertion.evidence}
                                onOpen={openCitation}
                              />
                            </li>
                          ))}
                        </ul>
                      )}
                    </section>
                    <section
                      className="family-popup-section"
                      data-testid="family-popup-sources"
                    >
                      <h3>Sources</h3>
                      <p className="hint">
                        Each citation above opens its source details. Only
                        citation metadata is shown; no original file is served.
                      </p>
                    </section>
                  </div>
                )}
                <div className="family-canvas-footer">
                  <p
                    className="family-canvas-help"
                    data-testid="family-navigation-hint"
                  >
                    {isTouch
                      ? "Tap a person for details, then choose Show children or Show parents."
                      : "Click for details. Double-click to explore children. Drag empty space to pan."}
                  </p>
                  <button
                    type="button"
                    ref={helpButtonRef}
                    onClick={() => setHelpOpen(true)}
                    className="button-secondary family-help-button"
                    aria-expanded={helpOpen}
                    aria-controls="family-help-panel"
                    data-testid="family-help"
                  >
                    Help
                  </button>
                </div>
                {!focus &&
                  selected &&
                  layout.people.some(
                    (p) => p.record_id === selected && p.childCount > 0,
                  ) && (
                    <button
                      type="button"
                      data-testid="family-collapse"
                      onClick={() =>
                        setCollapsed((current) => {
                          const next = new Set(current);
                          if (next.has(selected)) next.delete(selected);
                          else next.add(selected);
                          return next;
                        })
                      }
                    >
                      {collapsed.has(selected)
                        ? "Expand descendants"
                        : "Collapse descendants"}
                    </button>
                  )}
              </div>
            </div>
          )}
          <p className="family-legend">
            <span className="family-key family-key-verified" /> Verified
            <span className="family-key family-key-review" /> Needs review
            <span className="family-key family-key-unverified" /> Unverified
          </p>
          <SourceDialog
            source={openSource}
            onClose={() => setOpenSource(null)}
          />
          <PersonalHistoryDialog
            person={historyPerson}
            onClose={closePersonalHistory}
            onOpenSource={setOpenSource}
          />
          <section className="card">
            <h2>Catalog view</h2>
            <ul className="family-catalog" data-testid="genealogy-records">
              {graph.people.map((person) => (
                <li
                  key={person.record_id}
                  data-testid="genealogy-record"
                  data-record-id={person.record_id}
                >
                  <button
                    type="button"
                    className="family-text-button"
                    onClick={() => selectPerson(person.record_id)}
                  >
                    {person.person_label}
                  </button>
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
