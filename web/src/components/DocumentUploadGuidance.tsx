"use client";

import { useEffect, useRef, useState } from "react";

import {
  EXAMPLE_FAMILY_STORY,
  documentUploadLimitLabel,
} from "@/lib/document-upload";

// First-use guidance for the family documents submission form.
//
// Everything here is descriptive: it never changes what the server accepts or
// how a submission is reviewed. The example story lives in a native <details>
// element so keyboard and screen-reader users get the standard expand/collapse
// behaviour. Copying reports success through a polite live region; it never
// submits or replays anything.
export function DocumentUploadGuidance() {
  const [copyState, setCopyState] = useState<"idle" | "copied" | "failed">(
    "idle",
  );
  const timer = useRef<number | null>(null);

  useEffect(
    () => () => {
      if (timer.current !== null) {
        window.clearTimeout(timer.current);
      }
    },
    [],
  );

  // Returns whether the example really reached the clipboard, so the live
  // region never claims success for a copy that did not happen.
  async function writeExample(): Promise<boolean> {
    try {
      await navigator.clipboard.writeText(EXAMPLE_FAMILY_STORY);
      return true;
    } catch {
      // Fallback for browsers that do not expose the async clipboard (for
      // example an insecure context). It copies the same text.
      const area = document.createElement("textarea");
      area.value = EXAMPLE_FAMILY_STORY;
      area.setAttribute("readonly", "");
      area.style.position = "fixed";
      area.style.opacity = "0";
      document.body.appendChild(area);
      area.select();
      try {
        return document.execCommand("copy");
      } catch {
        return false;
      } finally {
        document.body.removeChild(area);
      }
    }
  }

  async function copyExample() {
    const copied = await writeExample();
    setCopyState(copied ? "copied" : "failed");
    if (timer.current !== null) {
      window.clearTimeout(timer.current);
    }
    timer.current = window.setTimeout(() => setCopyState("idle"), 5000);
  }

  return (
    <section
      className="card documents-guidance"
      data-testid="documents-guidance"
      aria-labelledby="documents-guidance-title"
    >
      <h2 id="documents-guidance-title">What can I upload?</h2>
      <ul>
        <li>Family stories and personal memories.</li>
        <li>Notes about parents, grandparents, and other relatives.</li>
        <li>Letters, biographies, and historical records.</li>
        <li>Vietnamese and English are both welcome.</li>
        <li>No special writing format is required.</li>
      </ul>

      <p>
        You can upload Markdown (<code>.md</code>, <code>.markdown</code>), plain
        text (<code>.txt</code>), or PDF (<code>.pdf</code>) files. Each file can
        be up to {documentUploadLimitLabel()}.
      </p>
      <p>
        A scanned PDF must contain readable text. If your PDF was scanned, apply
        OCR (optical character recognition) before uploading; the archive does
        not run OCR automatically, so a PDF without a text layer cannot be
        processed.
      </p>

      <details data-testid="example-story">
        <summary>Example family story (fictional)</summary>
        <p className="hint">
          This example is fictional. Replace the name and details with your own.
          Every field is optional: leave out anything you do not know rather than
          guessing.
        </p>
        <pre data-testid="example-story-text">{EXAMPLE_FAMILY_STORY}</pre>
        <div className="example-actions">
          <button
            type="button"
            onClick={() => void copyExample()}
            data-testid="copy-example"
          >
            Copy example
          </button>
          <p
            className="hint"
            role="status"
            aria-live="polite"
            data-testid="copy-example-feedback"
          >
            {copyState === "copied"
              ? "Example copied to the clipboard."
              : copyState === "failed"
                ? "Your browser blocked the copy. Select the example text above and copy it."
                : ""}
          </p>
        </div>
      </details>

      <h3>Tips</h3>
      <ul>
        <li>Use full names when possible.</li>
        <li>Explain relationships clearly.</li>
        <li>Identify who supplied the information.</li>
        <li>Mark approximate dates and uncertain details.</li>
        <li>Choose visibility carefully before submitting.</li>
      </ul>
      <p>
        <strong>Visibility.</strong> <em>Family</em> documents can appear in Ask
        the archive once an administrator approves them and processing finishes.{" "}
        <em>Admin only</em> documents stay restricted to administrators.
      </p>

      <h3>What happens after you submit</h3>
      <p className="documents-workflow" data-testid="documents-workflow">
        Submit → Administrator review → Processing → Available to Ask the
        archive.
      </p>
      <p className="hint">
        Uploading a document does not create or update family-tree records. To
        add or correct a person or relationship in the tree, use the family tree
        and its review tools.
      </p>
    </section>
  );
}
