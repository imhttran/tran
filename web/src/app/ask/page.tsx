"use client";

import { useEffect, useRef, useState } from "react";
import type { FormEvent } from "react";

import { ExperienceShell } from "@/components/ExperienceShell";
import { PageHeader } from "@/components/PageHeader";
import { PageTitle } from "@/components/PageTitle";
import { PersonAnswerView } from "@/components/PersonAnswer";
import { ApiRequestError, askQuestion, getQuerySuggestions } from "@/lib/api";
import type { QueryResponse } from "@/lib/api";

// Family-facing question page over the existing /api/query service. Rust
// authorizes, retrieves, and refuses; this page only renders its answer and
// citations. Suggestions come from approved archive sections, never from a
// hard-coded list, so an empty archive produces an explicit empty state.
type SuggestionsState =
  | { kind: "loading" }
  | { kind: "ready"; questions: string[] }
  | { kind: "empty" }
  | { kind: "error" };

type AskState =
  | { kind: "idle" }
  | { kind: "loading" }
  | { kind: "answered"; answer: QueryResponse }
  | { kind: "expired" }
  | { kind: "denied" }
  | { kind: "error"; message: string };

// One inline line-icon set for the page states. Text labels always carry the
// meaning; these marks are decorative and hidden from assistive technology.
type AskIconName = "search" | "alert" | "help" | "loading";

function AskIcon({ name }: { name: AskIconName }) {
  return (
    <svg
      className="ask-icon-svg"
      width="14"
      height="14"
      viewBox="0 0 24 24"
      fill="none"
      stroke="currentColor"
      strokeWidth="2"
      strokeLinecap="round"
      strokeLinejoin="round"
      aria-hidden="true"
      focusable="false"
    >
      {name === "search" && (
        <>
          <circle cx="11" cy="11" r="6" />
          <path d="m16 16 4 4" />
        </>
      )}
      {name === "alert" && (
        <>
          <circle cx="12" cy="12" r="9" />
          <path d="M12 7v6" />
          <path d="M12 16.5h.01" />
        </>
      )}
      {name === "help" && (
        <>
          <circle cx="12" cy="12" r="9" />
          <path d="M9.5 9a2.5 2.5 0 1 1 3.6 2.3c-.6.3-1.1.8-1.1 1.5" />
          <path d="M12 16.5h.01" />
        </>
      )}
      {name === "loading" && <path d="M12 3a9 9 0 1 0 9 9" />}
    </svg>
  );
}

function failureState(error: unknown): AskState {
  if (error instanceof ApiRequestError) {
    if (error.status === 401) return { kind: "expired" };
    if (error.status === 403) return { kind: "denied" };
    if (error.status === 400 && error.body)
      return { kind: "error", message: error.body.message };
  }
  return {
    kind: "error",
    message:
      "The archive could not answer right now. Please try again shortly.",
  };
}

export default function AskPage() {
  return <ExperienceShell>{() => <Ask />}</ExperienceShell>;
}

function Ask() {
  const [question, setQuestion] = useState("");
  const [state, setState] = useState<AskState>({ kind: "idle" });
  const [suggestions, setSuggestions] = useState<SuggestionsState>({
    kind: "loading",
  });
  const input = useRef<HTMLInputElement>(null);

  useEffect(() => {
    const controller = new AbortController();
    getQuerySuggestions(controller.signal)
      .then((body) => {
        if (controller.signal.aborted) return;
        setSuggestions(
          body.questions.length > 0
            ? { kind: "ready", questions: body.questions }
            : { kind: "empty" },
        );
      })
      .catch(() => {
        if (!controller.signal.aborted) setSuggestions({ kind: "error" });
      });
    return () => controller.abort();
  }, []);

  async function onAsk(event: FormEvent<HTMLFormElement>) {
    event.preventDefault();
    const trimmed = question.trim();
    if (state.kind === "loading" || trimmed === "") return;
    setState({ kind: "loading" });
    try {
      setState({ kind: "answered", answer: await askQuestion(trimmed) });
    } catch (error) {
      setState(failureState(error));
    }
  }

  // Choosing from a "several possible matches" list resolves that stable
  // person key. The API returns the selected person's stored assertions only.
  async function onSelectPerson(personKey: string) {
    const trimmed = question.trim();
    if (state.kind === "loading" || trimmed === "") return;
    setState({ kind: "loading" });
    try {
      setState({
        kind: "answered",
        answer: await askQuestion(trimmed, personKey),
      });
    } catch (error) {
      setState(failureState(error));
    }
  }

  const answer = state.kind === "answered" ? state.answer : null;
  return (
    <>
      <PageTitle title="Ask the archive" />
      <PageHeader
        title="Ask the archive"
        subtitle="Ask a question. Follow the evidence."
      />

      <section className="card ask-panel" aria-labelledby="ask-panel-title">
        <h2 id="ask-panel-title" className="ask-panel-title">
          What would you like to know?
        </h2>
        <p className="ask-helper" id="ask-helper">
          Ask about a document or an event to get a cited archive answer. Type a
          person&apos;s name to get a person record built from stored
          assertions.
        </p>
        <form className="ask-form" onSubmit={onAsk}>
          <label htmlFor="ask-question" className="ask-label">
            Your question
          </label>
          <div className="ask-field">
            <svg
              className="ask-icon"
              viewBox="0 0 24 24"
              aria-hidden="true"
              fill="none"
              stroke="currentColor"
              strokeWidth="1.8"
              strokeLinecap="round"
              strokeLinejoin="round"
            >
              <path d="M4 5.5A2.5 2.5 0 0 1 6.5 3h11A2.5 2.5 0 0 1 20 5.5v8a2.5 2.5 0 0 1-2.5 2.5H10l-4.5 4v-4h0A1.5 1.5 0 0 1 4 14.5z" />
              <path d="M10 8a2 2 0 1 1 2.8 1.8c-.5.3-.8.7-.8 1.2" />
              <path d="M12 13.2h.01" />
            </svg>
            <input
              id="ask-question"
              ref={input}
              type="text"
              value={question}
              onChange={(event) => setQuestion(event.target.value)}
              placeholder="Ask about your family…"
              aria-describedby="ask-helper"
              data-testid="question-input"
            />
          </div>
          <button
            type="submit"
            className="ask-submit"
            disabled={state.kind === "loading" || question.trim() === ""}
            data-testid="ask-button"
          >
            {state.kind === "loading" ? "Asking…" : "Ask"}
          </button>
        </form>

        {suggestions.kind === "loading" && (
          <p
            className="ask-state ask-state-loading"
            role="status"
            data-testid="ask-suggestions-loading"
          >
            <span className="ask-state-icon" aria-hidden="true">
              <AskIcon name="loading" />
            </span>
            Looking for suggested questions…
          </p>
        )}
        {suggestions.kind === "ready" && (
          <div className="ask-suggestions">
            <h3 className="ask-suggestions-title">
              Suggested from the archive
            </h3>
            <ul className="ask-examples" aria-label="Suggested questions">
              {suggestions.questions.map((suggestion) => (
                <li key={suggestion}>
                  <button
                    type="button"
                    data-testid="ask-example"
                    onClick={() => {
                      setQuestion(suggestion);
                      input.current?.focus();
                    }}
                  >
                    <span className="ask-example-mark" aria-hidden="true">
                      <AskIcon name="search" />
                    </span>
                    <span>{suggestion}</span>
                  </button>
                </li>
              ))}
            </ul>
          </div>
        )}
        {suggestions.kind === "empty" && (
          <div
            className="ask-empty ask-state ask-state-empty"
            role="status"
            data-testid="ask-suggestions-empty"
          >
            <h2>No searchable archive content yet</h2>
            <p>
              An administrator must approve and process a family document before
              the archive can suggest or answer questions.
            </p>
          </div>
        )}
        {suggestions.kind === "error" && (
          <p
            className="ask-state ask-state-warn"
            role="status"
            data-testid="ask-suggestions-error"
          >
            <span className="ask-state-icon" aria-hidden="true">
              <AskIcon name="alert" />
            </span>
            Suggested questions are unavailable right now. You can still ask a
            question.
          </p>
        )}
      </section>

      <div className="ask-results" aria-live="polite">
        {state.kind === "loading" && (
          <p
            className="ask-state ask-state-loading"
            role="status"
            data-testid="ask-loading"
          >
            <span className="ask-state-icon" aria-hidden="true">
              <AskIcon name="loading" />
            </span>
            Searching the archive…
          </p>
        )}
        {state.kind === "expired" && (
          <p
            className="form-status error ask-state ask-state-error"
            role="alert"
            data-testid="ask-expired"
          >
            <span className="ask-state-icon" aria-hidden="true">
              <AskIcon name="alert" />
            </span>
            Your session expired. <a href="/signin">Sign in again</a> to ask the
            archive.
          </p>
        )}
        {state.kind === "denied" && (
          <p
            className="form-status error ask-state ask-state-error"
            role="alert"
            data-testid="ask-denied"
          >
            <span className="ask-state-icon" aria-hidden="true">
              <AskIcon name="alert" />
            </span>
            Your account is not permitted to ask the archive. Ask an
            administrator for access.
          </p>
        )}
        {state.kind === "error" && (
          <p
            className="form-status error ask-state ask-state-error"
            role="alert"
            data-testid="ask-error"
          >
            <span className="ask-state-icon" aria-hidden="true">
              <AskIcon name="alert" />
            </span>
            {state.message}
          </p>
        )}
        {answer?.person ? (
          <PersonAnswerView
            person={answer.person}
            busy={state.kind === "loading"}
            onSelect={onSelectPerson}
          />
        ) : (
          <>
            {answer?.insufficient && (
              <p
                className="insufficient ask-state ask-state-warn"
                role="status"
                data-testid="insufficient"
              >
                <span className="ask-state-icon" aria-hidden="true">
                  <AskIcon name="help" />
                </span>
                The archive does not have enough evidence to answer this
                question.
              </p>
            )}
            {answer && !answer.insufficient && (
              <section className="card ask-answer">
                <p className="ask-answer-eyebrow">Archive answer</p>
                <h2 className="ask-answer-heading">Answer</h2>
                <p className="ask-answer-body" data-testid="answer">
                  {answer.answer}
                </p>
                {answer.sources.length > 0 && (
                  <div className="ask-citations">
                    <h3 className="ask-citations-heading">Sources</h3>
                    <ol className="sources" data-testid="sources">
                      {answer.sources.map((source, index) => (
                        <li
                          key={`${source.source}-${source.section}-${index}`}
                          className="ask-citation"
                        >
                          <div className="ask-citation-head">
                            <span
                              className="ask-citation-number"
                              aria-hidden="true"
                            >
                              {index + 1}
                            </span>
                            <div className="ask-citation-meta">
                              <span className="cite">
                                [{index + 1}] {source.source} — {source.section}
                              </span>
                              <span className="page">
                                {source.page === null
                                  ? " (no page recorded)"
                                  : ` (page ${source.page})`}
                              </span>
                            </div>
                          </div>
                          <blockquote className="source-quote">
                            {source.text}
                          </blockquote>
                        </li>
                      ))}
                    </ol>
                  </div>
                )}
                <p className="ask-answer-note">
                  Answers never change family records or appoint a Head of
                  Family.
                </p>
              </section>
            )}
          </>
        )}
      </div>
    </>
  );
}
