"use client";

import { useRef, useState } from "react";
import type { FormEvent } from "react";

import { ExperienceShell } from "@/components/ExperienceShell";
import { PageHeader } from "@/components/PageHeader";
import { PageTitle } from "@/components/PageTitle";
import { PersonAnswerView } from "@/components/PersonAnswer";
import { ApiRequestError, askQuestion } from "@/lib/api";
import type { QueryResponse } from "@/lib/api";

// Family-facing question page over the existing /api/query service. Rust
// authorizes, retrieves, and refuses; this page only renders its answer and
// citations. It makes no claim about what the archive contains.
const EXAMPLES = [
  "Who are the earliest recorded ancestors?",
  "Where did the family live?",
  "What do the sources say about the family branches?",
  "Which family events are recorded in the register?",
];

type AskState =
  | { kind: "idle" }
  | { kind: "loading" }
  | { kind: "answered"; answer: QueryResponse }
  | { kind: "expired" }
  | { kind: "denied" }
  | { kind: "error"; message: string };

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
  const input = useRef<HTMLInputElement>(null);

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
      <section className="card">
        <form className="ask-form" onSubmit={onAsk}>
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
            ref={input}
            type="text"
            value={question}
            onChange={(event) => setQuestion(event.target.value)}
            placeholder="Ask about your family…"
            aria-label="Ask about your family"
            data-testid="question-input"
          />
          <button
            type="submit"
            disabled={state.kind === "loading" || question.trim() === ""}
            data-testid="ask-button"
          >
            {state.kind === "loading" ? "Asking…" : "Ask"}
          </button>
        </form>
        <ul className="ask-examples" aria-label="Example questions">
          {EXAMPLES.map((example) => (
            <li key={example}>
              <button
                type="button"
                data-testid="ask-example"
                onClick={() => {
                  setQuestion(example);
                  input.current?.focus();
                }}
              >
                {example}
              </button>
            </li>
          ))}
        </ul>
      </section>

      <div aria-live="polite">
        {state.kind === "loading" && (
          <p
            className="form-status info"
            role="status"
            data-testid="ask-loading"
          >
            Searching the archive…
          </p>
        )}
        {state.kind === "expired" && (
          <p
            className="form-status error"
            role="alert"
            data-testid="ask-expired"
          >
            Your session expired. <a href="/signin">Sign in again</a> to ask the
            archive.
          </p>
        )}
        {state.kind === "denied" && (
          <p
            className="form-status error"
            role="alert"
            data-testid="ask-denied"
          >
            Your account is not permitted to ask the archive. Ask an
            administrator for access.
          </p>
        )}
        {state.kind === "error" && (
          <p className="form-status error" role="alert" data-testid="ask-error">
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
              <p className="insufficient" data-testid="insufficient">
                The archive does not have enough evidence to answer this
                question.
              </p>
            )}
            {answer && !answer.insufficient && (
              <section className="card">
                <h2>Answer</h2>
                <p data-testid="answer">{answer.answer}</p>
                {answer.sources.length > 0 && (
                  <>
                    <h3>Sources</h3>
                    <ol className="sources" data-testid="sources">
                      {answer.sources.map((source, index) => (
                        <li key={`${source.source}-${source.section}-${index}`}>
                          <span className="cite">
                            [{index + 1}] {source.source} — {source.section}
                          </span>
                          <span className="page">
                            {source.page === null
                              ? " (no page recorded)"
                              : ` (page ${source.page})`}
                          </span>
                          <p className="source-quote">{source.text}</p>
                        </li>
                      ))}
                    </ol>
                  </>
                )}
                <p className="hint">
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
