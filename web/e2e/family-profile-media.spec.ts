import { expect, test } from "@playwright/test";
import type { APIRequestContext, Page } from "@playwright/test";

// USR-006-C: approved-only family media presentation.
//
// Drives the real Next.js proxy, the real Axum API, and disposable PostgreSQL
// fixtures seeded by scripts/web-e2e.sh. The owner uploads through the product
// API, an independent approver decides, and a *different* linked family member
// reads the profile and the asset. Nothing is seeded directly into the media
// tables, so approval, visibility, and restriction all take their real paths.
//
// The suite is repeatable: each run withdraws a leftover pending submission,
// supersedes the previous approved photo with a new one, and finishes with no
// active restriction.

const API_URL = process.env.E2E_API_URL || "http://127.0.0.1:8080";
const ORIGIN = new URL(process.env.E2E_BASE_URL || "http://127.0.0.1:3000")
  .origin;
const ISSUER = process.env.E2E_ISSUER || "";
const APPROVER = process.env.E2E_APPROVER_TOKEN || "";
const OWNER = process.env.E2E_RESTRICT_OWNER_TOKEN || "";
const MEMBER = process.env.E2E_FAMILY_MEMBER_TOKEN || "";
const VIEWER = process.env.E2E_VIEWER_TOKEN || "";
const PERSON = "e2e-branch-restrictable";
const READY = Boolean(ISSUER && APPROVER && OWNER && MEMBER && VIEWER);

type MediaItem = {
  media_id: number;
  revision_id: number;
  kind: string;
  mime_type: string;
  status: string;
  visibility: string;
};

type FamilyMedia = {
  media_id: number;
  kind: string;
  mime_type: string;
  approved_revision_number: number;
};

// A 1x1 PNG. The server re-encodes it, so only the dimensions and MIME type are
// asserted on the response, never the original bytes.
const PNG = Buffer.from(
  "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mP8z8BQDwAEhQGAhKmMIQAAAABJRU5ErkJggg==",
  "base64",
);

function auth(token: string) {
  return { authorization: `Bearer ${token}` };
}

async function api(
  request: APIRequestContext,
  token: string,
  method: "GET" | "POST" | "DELETE",
  path: string,
  data?: unknown,
) {
  const response = await request.fetch(`${API_URL}${path}`, {
    method,
    headers: auth(token),
    data,
  });
  const body = await response.json().catch(() => null);
  return { status: response.status(), body };
}

async function uploadPhoto(
  request: APIRequestContext,
  visibility: "family" | "owner_private",
  name: string,
) {
  const response = await request.post(`${API_URL}/api/profile/media/photo`, {
    headers: auth(OWNER),
    multipart: {
      visibility,
      file: { name, mimeType: "image/png", buffer: PNG },
    },
  });
  const body = await response.json().catch(() => null);
  return { status: response.status(), body };
}

async function approve(request: APIRequestContext, revisionId: number) {
  return api(
    request,
    APPROVER,
    "POST",
    `/api/profile/revisions/${revisionId}/review`,
    { decision: "approve", note: "Approved for the family archive." },
  );
}

async function familyProfile(request: APIRequestContext) {
  return api(request, MEMBER, "GET", `/api/family/persons/${PERSON}/profile`);
}

function familyMedia(body: unknown): FamilyMedia[] {
  const media = (body as { media?: FamilyMedia[] } | null)?.media;
  return Array.isArray(media) ? media : [];
}

/// Leaves the fixture in a state a fresh submission can start from: a pending
/// photo from an interrupted run is withdrawn through the product API.
async function withdrawPendingPhoto(request: APIRequestContext) {
  const owner = await api(request, OWNER, "GET", "/api/profile");
  const media = ((owner.body?.media ?? []) as MediaItem[]).filter(
    (item) => item.kind === "profile_photo",
  );
  for (const item of media) {
    if (item.status === "pending_review" || item.status === "draft") {
      await api(
        request,
        OWNER,
        "DELETE",
        `/api/profile/media/${item.media_id}`,
      );
    }
  }
}

async function useToken(page: Page, token: string) {
  await page.context().addCookies([
    {
      name: "rag_session",
      value: token,
      url: ORIGIN,
      httpOnly: true,
      sameSite: "Lax",
    },
    {
      name: "rag_csrf",
      value: "e2e-csrf-token",
      url: ORIGIN,
      sameSite: "Lax",
    },
  ]);
}

test.beforeEach(() => {
  test.skip(!READY, "run this suite through scripts/web-e2e.sh");
});

test.describe.serial("approved-only family media", () => {
  test("an approved family photo is listed, served, and rendered", async ({
    page,
    request,
  }) => {
    await withdrawPendingPhoto(request);

    const upload = await uploadPhoto(request, "family", "family-portrait.png");
    expect(upload.status, JSON.stringify(upload.body)).toBe(200);
    const media = (upload.body as { media: MediaItem }).media;
    expect(media.status).toBe("pending_review");

    // Pending media is invisible to the family: no listing entry, no bytes.
    const before = await familyProfile(request);
    expect(before.status).toBe(200);
    expect(familyMedia(before.body)).toHaveLength(0);
    const pendingAsset = await request.get(
      `${API_URL}/api/family/persons/${PERSON}/media/${media.media_id}`,
      { headers: auth(MEMBER) },
    );
    expect(pendingAsset.status()).toBe(404);

    const approved = await approve(request, media.revision_id);
    expect(approved.status, JSON.stringify(approved.body)).toBe(200);

    // The listing carries metadata and provenance, never the storage key.
    const after = await familyProfile(request);
    expect(after.status).toBe(200);
    const listed = familyMedia(after.body);
    expect(listed).toHaveLength(1);
    expect(listed[0].media_id).toBe(media.media_id);
    expect(listed[0].kind).toBe("profile_photo");
    expect(listed[0].mime_type).toBe("image/png");
    expect(listed[0].approved_revision_number).toBeGreaterThan(0);
    expect(JSON.stringify(after.body)).not.toContain("storage_key");

    // The asset route serves the approved bytes to a linked family member.
    const asset = await request.get(
      `${API_URL}/api/family/persons/${PERSON}/media/${media.media_id}`,
      { headers: auth(MEMBER) },
    );
    expect(asset.status()).toBe(200);
    expect(asset.headers()["content-type"]).toContain("image/png");
    expect((await asset.body()).length).toBeGreaterThan(0);

    // An unlinked account is refused by the same route.
    const stranger = await request.get(
      `${API_URL}/api/family/persons/${PERSON}/media/${media.media_id}`,
      { headers: auth(VIEWER) },
    );
    expect(stranger.status()).toBe(403);

    // The page renders the photograph through the same-origin proxy.
    await useToken(page, MEMBER);
    await page.goto(`/family/${PERSON}`);
    const photo = page.getByTestId(`family-profile-photo-${media.media_id}`);
    await expect(photo).toBeVisible();
    const image = photo.locator("img");
    await expect(image).toBeVisible();
    await expect
      .poll(async () =>
        image.evaluate((node) => (node as HTMLImageElement).naturalWidth),
      )
      .toBeGreaterThan(0);
  });

  test("owner-private media is withheld from the family", async ({
    request,
  }) => {
    const upload = await uploadPhoto(request, "owner_private", "private.png");
    expect(upload.status, JSON.stringify(upload.body)).toBe(200);
    const media = (upload.body as { media: MediaItem }).media;
    const approved = await approve(request, media.revision_id);
    expect(approved.status, JSON.stringify(approved.body)).toBe(200);

    const profile = await familyProfile(request);
    expect(profile.status).toBe(200);
    expect(familyMedia(profile.body)).toHaveLength(0);

    const asset = await request.get(
      `${API_URL}/api/family/persons/${PERSON}/media/${media.media_id}`,
      { headers: auth(MEMBER) },
    );
    expect(asset.status()).toBe(404);

    // The owner still reads their own approved asset.
    const ownerAsset = await request.get(
      `${API_URL}/api/profile/media/${media.media_id}`,
      { headers: auth(OWNER) },
    );
    expect(ownerAsset.status()).toBe(200);
  });

  test("a restricted photo is withheld and a new approved one restores it", async ({
    page,
    request,
  }) => {
    // Make a family-visible photo current again.
    const upload = await uploadPhoto(request, "family", "shared.png");
    expect(upload.status, JSON.stringify(upload.body)).toBe(200);
    const media = (upload.body as { media: MediaItem }).media;
    expect((await approve(request, media.revision_id)).status).toBe(200);

    const visible = familyMedia((await familyProfile(request)).body);
    expect(visible).toHaveLength(1);
    expect(visible[0].media_id).toBe(media.media_id);

    // Decision A2: the owner stops sharing the photograph immediately.
    const restrict = await api(
      request,
      OWNER,
      "POST",
      "/api/profile/fields/profile_photo/restrict",
    );
    expect(restrict.status, JSON.stringify(restrict.body)).toBe(200);

    expect(familyMedia((await familyProfile(request)).body)).toHaveLength(0);
    const blocked = await request.get(
      `${API_URL}/api/family/persons/${PERSON}/media/${media.media_id}`,
      { headers: auth(MEMBER) },
    );
    expect(blocked.status()).toBe(404);

    // The generic asset path honours the restriction too.
    const generic = await request.get(
      `${API_URL}/api/profile/media/${media.media_id}`,
      { headers: auth(MEMBER) },
    );
    expect(generic.status()).toBe(404);

    // The page no longer renders the photograph.
    await useToken(page, MEMBER);
    await page.goto(`/family/${PERSON}`);
    await expect(page.getByTestId("family-profile-photos")).toHaveCount(0);
    await expect(
      page.getByTestId(`family-profile-photo-${media.media_id}`),
    ).toHaveCount(0);

    // The owner keeps private access to the approved asset.
    const ownerAsset = await request.get(
      `${API_URL}/api/profile/media/${media.media_id}`,
      { headers: auth(OWNER) },
    );
    expect(ownerAsset.status()).toBe(200);

    // A new family-visible submission created under the restriction, approved
    // independently, lifts it — the only way back.
    const restored = await uploadPhoto(request, "family", "restored.png");
    expect(restored.status, JSON.stringify(restored.body)).toBe(200);
    const restoredMedia = (restored.body as { media: MediaItem }).media;
    expect((await approve(request, restoredMedia.revision_id)).status).toBe(
      200,
    );
    const after = familyMedia((await familyProfile(request)).body);
    expect(after).toHaveLength(1);
    expect(after[0].media_id).toBe(restoredMedia.media_id);
  });

  // The person is shared with the profile suites, which assert an empty family
  // page. Leave no family-visible media behind: an active restriction is the
  // product's own "stop sharing" state, and the next run lifts it by approving a
  // new submission created under it.
  test.afterAll(async ({ request }) => {
    const profile = await familyProfile(request);
    if (familyMedia(profile.body).length === 0) return;
    const response = await api(
      request,
      OWNER,
      "POST",
      "/api/profile/fields/profile_photo/restrict",
    );
    expect(response.status, JSON.stringify(response.body)).toBe(200);
  });
});
