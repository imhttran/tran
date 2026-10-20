import { test } from "node:test";
import assert from "node:assert/strict";

import {
  OIDC_SUPPORTED,
  canApproveOrRevoke,
  canNominate,
  canUpload,
  evidenceStatusLabel,
  isForbiddenField,
  outcomeFromStatus,
  redact,
  unsupportedSignIn,
} from "./experience-contract";

test("maps 401 to an expired result with a sign-in action", () => {
  const result = outcomeFromStatus<{ record_id: string }>(401, null);
  assert.equal(result.kind, "expired");
  if (result.kind === "expired") {
    assert.equal(result.signInUrl, "/signin");
  }
});

test("maps 403 to a denied result", () => {
  assert.equal(outcomeFromStatus(403, { record_id: "r" }).kind, "denied");
});

test("maps 404 to a no-access result", () => {
  assert.equal(outcomeFromStatus(404, null).kind, "no_access");
});

test("maps 2xx to an allowed result", () => {
  const result = outcomeFromStatus(200, { record_id: "r", title: "t" });
  assert.equal(result.kind, "allowed");
});

test("keeps the three denial kinds distinct", () => {
  const kinds = new Set([
    outcomeFromStatus(401, null).kind,
    outcomeFromStatus(403, null).kind,
    outcomeFromStatus(404, null).kind,
  ]);
  assert.equal(kinds.size, 3);
});

test("drops forbidden fields during redaction", () => {
  const redacted = redact({
    record_id: "r",
    email: "a@b.c",
    access_token: "x",
    database_url: "postgres://x",
    oidc_issuer: "https://idp",
  });
  assert.deepEqual(redacted, { record_id: "r" });
});

test("flags forbidden field names", () => {
  assert.equal(isForbiddenField("client_secret"), true);
  assert.equal(isForbiddenField("database_url"), true);
  assert.equal(isForbiddenField("record_id"), false);
});

test("returns an explicit unsupported sign-in without fabricating a session", () => {
  assert.equal(OIDC_SUPPORTED, false);
  const result = unsupportedSignIn();
  assert.equal(result.kind, "no_access");
  assert.equal("data" in result, false);
});

test("gates uploads and nominations to non-viewers", () => {
  assert.equal(canUpload("viewer"), false);
  assert.equal(canUpload("editor"), true);
  assert.equal(canUpload("admin"), true);
  assert.equal(canNominate("viewer"), false);
  assert.equal(canNominate("editor"), true);
});

test("gates approval and revocation to admins", () => {
  assert.equal(canApproveOrRevoke("viewer"), false);
  assert.equal(canApproveOrRevoke("editor"), false);
  assert.equal(canApproveOrRevoke("admin"), true);
});

test("labels evidence states", () => {
  assert.equal(evidenceStatusLabel("unverified"), "Unverified");
  assert.equal(evidenceStatusLabel("needs_review"), "Needs review");
});
