export type FormStatusState = { kind: "ok" | "error" | "info"; text: string } | null;

// Inline result for a control. role=status announces changes to assistive tech.
export function FormStatus({ status }: { status: FormStatusState }) {
  if (!status) return null;
  return (
    <p className={`form-status ${status.kind}`} role="status">
      {status.text}
    </p>
  );
}
