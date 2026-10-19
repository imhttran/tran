"use client";

// Status panel. Reports the real API health probe rather than a simulated
// availability. "not checked" is shown until the first probe resolves.
export type Availability = "not-checked" | "up" | "down";

export function SystemStatus({
  api,
  database,
  model,
}: {
  api: Availability;
  database: Availability;
  model: Availability;
}) {
  const label: Record<Availability, string> = {
    "not-checked": "not checked",
    up: "up",
    down: "down",
  };
  return (
    <section className="status-panel" aria-label="System status">
      <h2>System status</h2>
      <dl>
        <div><dt>API</dt><dd className={`avail ${api}`}>{label[api]}</dd></div>
        <div><dt>Database</dt><dd className={`avail ${database}`}>{label[database]}</dd></div>
        <div><dt>Model</dt><dd className={`avail ${model}`}>{label[model]}</dd></div>
      </dl>
      <p className="hint">
        Probed through the same-origin `/api/health` endpoint; no credentials are
        sent to the browser.
      </p>
    </section>
  );
}
