import { expect, test } from "@playwright/test";
import type { Page, Route } from "@playwright/test";

// PIR-005: the invitation panel on the family profile.
//
// Every request is stubbed, so the suite proves only what this change owns: the
// panel's states, the explicit send action, and the privacy boundary. Synthetic
// names and example.test addresses are used; no real record is read.

const PERSON_KEY = "synthetic-person";
const UNAUTHORIZED = "synthetic.contact@example.test";
const MASKED = "s****@example.test";

const SESSION = {
  state: "signed_in",
  role: "viewer",
  display_name: "Synthetic viewer",
};

const PROFILE = {
  ok: true,
  state: "published",
  person: { person_key: PERSON_KEY, display_name: "Synthetic Person" },
  fields: [],
  media: [],
};

type InvitationState = {
  ok: true;
  person_key: string;
  authorization: string;
  eligibility: string;
  invitation: {
    pending: boolean;
    invitation_id: number | null;
    expires_at: string | null;
    delivery_state: string;
  };
  linked_account: boolean;
  proposed_username: string | null;
  masked_contact: string | null;
  blockers: { code: string; message: string }[];
};

type InvitationStatus = {
  ok: true;
  person_key: string;
  invitation_id: number | null;
  invitation_status: string | null;
  delivery_state: string;
  delivery_provider: string | null;
  last_delivery_error: string | null;
  delivery_attempts: number;
  expires_at: string | null;
  accepted_at: string | null;
};

function invitationState(
  overrides: Partial<InvitationState> = {},
): InvitationState {
  return {
    ok: true,
    person_key: PERSON_KEY,
    authorization: "system_admin",
    eligibility: "eligible_for_invite",
    invitation: {
      pending: false,
      invitation_id: null,
      expires_at: null,
      delivery_state: "not_sent",
    },
    linked_account: false,
    proposed_username: "synthetic.person",
    masked_contact: null,
    blockers: [
      {
        code: "missing_contact",
        message: "a draft contact is required before preparation",
      },
    ],
    ...overrides,
  };
}

function invitationStatus(
  overrides: Partial<InvitationStatus> = {},
): InvitationStatus {
  return {
    ok: true,
    person_key: PERSON_KEY,
    invitation_id: null,
    invitation_status: null,
    delivery_state: "not_sent",
    delivery_provider: null,
    last_delivery_error: null,
    delivery_attempts: 0,
    expires_at: null,
    accepted_at: null,
    ...overrides,
  };
}

function json(route: Route, status: number, body: unknown) {
  return route.fulfill({
    status,
    contentType: "application/json",
    body: JSON.stringify(body),
  });
}

async function stubSession(page: Page) {
  await page.route("**/api/auth/session", (route) => json(route, 200, SESSION));
}

type Handlers = {
  profile?: (route: Route) => Promise<void> | void;
  invitation?: (route: Route) => Promise<void> | void;
  "invitation/status"?: (route: Route) => Promise<void> | void;
  "invitation/prepare"?: (route: Route) => Promise<void> | void;
  "invitation/send"?: (route: Route) => Promise<void> | void;
  "invitation/resend"?: (route: Route) => Promise<void> | void;
  "invitation/revoke"?: (route: Route) => Promise<void> | void;
};

// Routes are matched by the most specific suffix first: `/invitation/status`
// must not be served by the bare `/invitation` handler.
async function stubPerson(page: Page, handlers: Handlers) {
  await page.route("**/api/family/persons/**", async (route) => {
    const path = new URL(route.request().url()).pathname;
    const suffix = [
      "invitation/status",
      "invitation/prepare",
      "invitation/send",
      "invitation/resend",
      "invitation/revoke",
      "invitation",
      "profile",
    ].find((candidate) => path.endsWith(candidate));
    const handler = suffix ? handlers[suffix as keyof Handlers] : undefined;
    if (!handler) {
      await json(route, 404, {
        ok: false,
        error: "not_found",
        message: "no such endpoint",
      });
      return;
    }
    await handler(route);
  });
}

test.describe("profile invitation panel", () => {
  test("a system admin saves a contact, reviews, and sends explicitly", async ({
    page,
  }) => {
    await stubSession(page);
    let state = invitationState();
    let status = invitationStatus();
    let sent = 0;

    await stubPerson(page, {
      profile: (route) => json(route, 200, PROFILE),
      invitation: async (route) => {
        if (route.request().method() === "PUT") {
          state = invitationState({ masked_contact: MASKED, blockers: [] });
          await json(route, 200, {
            ok: true,
            person_key: PERSON_KEY,
            masked_contact: MASKED,
            sent: false,
          });
          return;
        }
        await json(route, 200, state);
      },
      "invitation/status": (route) => json(route, 200, status),
      "invitation/prepare": (route) =>
        json(route, 200, {
          ok: true,
          person_key: PERSON_KEY,
          masked_contact: MASKED,
          proposed_username: "synthetic.person",
          access: "viewer",
          invitation_id: 7,
          invitation_reused: false,
          invitation_created: true,
        }),
      "invitation/send": async (route) => {
        sent += 1;
        state = invitationState({
          masked_contact: MASKED,
          blockers: [],
          invitation: {
            pending: true,
            invitation_id: 7,
            expires_at: "2030-01-01 00:00:00+00",
            delivery_state: "sent",
          },
        });
        status = invitationStatus({
          invitation_id: 7,
          invitation_status: "pending",
          delivery_state: "sent",
          delivery_provider: "fake",
          delivery_attempts: 1,
        });
        await json(route, 200, {
          ok: true,
          person_key: PERSON_KEY,
          invitation_id: 7,
          delivery_state: "sent",
          provider: "fake",
          delivered: true,
          masked_contact: MASKED,
        });
      },
    });

    await page.goto(`/family/${PERSON_KEY}`);

    const panel = page.getByTestId("profile-invitation");
    await expect(panel).toBeVisible();
    await expect(
      page.getByTestId("profile-invitation-contact-state"),
    ).toHaveText("Needs contact");
    await expect(page.getByTestId("profile-invitation-access")).toHaveText(
      "Member",
    );
    await expect(page.getByTestId("profile-invitation-username")).toContainText(
      "synthetic.person",
    );
    // The plain-language blocker appears; its raw code does not.
    await expect(page.getByTestId("profile-invitation-blockers")).toContainText(
      "a draft contact is required before preparation",
    );
    await expect(
      page.getByText("missing_contact", { exact: true }),
    ).toHaveCount(0);
    // Nothing can be reviewed or sent before a contact exists.
    await expect(page.getByTestId("profile-invitation-prepare")).toBeDisabled();
    await expect(page.getByTestId("profile-invitation-send")).toHaveCount(0);
    expect(sent).toBe(0);

    await page.getByTestId("profile-invitation-contact").fill(UNAUTHORIZED);
    await page.getByRole("button", { name: "Save contact" }).click();

    await expect(page.getByTestId("profile-invitation-masked")).toContainText(
      MASKED,
    );
    await expect(
      page.getByTestId("profile-invitation-contact-state"),
    ).toHaveText("Contact saved");
    await expect(page.getByTestId("profile-invitation-notice")).toContainText(
      "Nothing has been sent",
    );
    // Saving a draft never sends, and the full address is never rendered.
    expect(sent).toBe(0);
    await expect(page.getByText(UNAUTHORIZED)).toHaveCount(0);

    await page.getByTestId("profile-invitation-prepare").click();
    const review = page.getByTestId("profile-invitation-review");
    await expect(review).toBeVisible();
    await expect(review).toContainText("Synthetic Person");
    await expect(review).toContainText(MASKED);
    await expect(review).toContainText("Member");
    expect(sent).toBe(0);

    await page.getByTestId("profile-invitation-send").click();

    await expect(page.getByTestId("profile-invitation-notice")).toContainText(
      "Invitation sent",
    );
    await expect(page.getByTestId("profile-invitation-delivery")).toHaveText(
      "Sent",
    );
    await expect(page.getByTestId("profile-invitation-resend")).toBeVisible();
    await expect(page.getByTestId("profile-invitation-revoke")).toBeVisible();
    expect(sent).toBe(1);
    await expect(page.getByText(UNAUTHORIZED)).toHaveCount(0);
  });

  test("an ordinary member never sees the panel", async ({ page }) => {
    await stubSession(page);
    await stubPerson(page, {
      profile: (route) => json(route, 200, PROFILE),
      invitation: (route) =>
        json(route, 403, {
          ok: false,
          error: "family_admin_refused",
          message: "the authenticated account may not invite this person",
        }),
      "invitation/status": (route) =>
        json(route, 403, {
          ok: false,
          error: "family_admin_refused",
          message: "the authenticated account may not invite this person",
        }),
    });

    await page.goto(`/family/${PERSON_KEY}`);
    await expect(page.getByTestId("family-profile")).toBeVisible();
    await expect(page.getByTestId("profile-invitation")).toHaveCount(0);
  });

  test("an authorized family head sees the panel", async ({ page }) => {
    await stubSession(page);
    await stubPerson(page, {
      profile: (route) => json(route, 200, PROFILE),
      invitation: (route) =>
        json(
          route,
          200,
          invitationState({
            authorization: "family_head_branch",
            eligibility: "needs_review",
            blockers: [
              {
                code: "disputed",
                message: "a reviewer disputed this person's evidence",
              },
            ],
          }),
        ),
      "invitation/status": (route) => json(route, 200, invitationStatus()),
    });

    await page.goto(`/family/${PERSON_KEY}`);
    await expect(page.getByTestId("profile-invitation")).toBeVisible();
    await expect(page.getByTestId("profile-invitation-blockers")).toContainText(
      "a reviewer disputed this person's evidence",
    );
  });

  test("a linked account shows Account linked instead of an invite action", async ({
    page,
  }) => {
    await stubSession(page);
    await stubPerson(page, {
      profile: (route) => json(route, 200, PROFILE),
      invitation: (route) =>
        json(
          route,
          200,
          invitationState({
            linked_account: true,
            eligibility: "already_linked",
            masked_contact: MASKED,
          }),
        ),
      "invitation/status": (route) =>
        json(route, 200, invitationStatus({ delivery_state: "accepted" })),
    });

    await page.goto(`/family/${PERSON_KEY}`);
    await expect(page.getByTestId("profile-invitation-linked")).toContainText(
      "Account linked",
    );
    await expect(page.getByTestId("profile-invitation-contact")).toHaveCount(0);
    await expect(page.getByTestId("profile-invitation-prepare")).toHaveCount(0);
    await expect(page.getByTestId("profile-invitation-send")).toHaveCount(0);
  });

  test("a delivery failure is reported as prose, not a raw code", async ({
    page,
  }) => {
    await stubSession(page);
    await stubPerson(page, {
      profile: (route) => json(route, 200, PROFILE),
      invitation: (route) =>
        json(
          route,
          200,
          invitationState({ masked_contact: MASKED, blockers: [] }),
        ),
      "invitation/status": (route) =>
        json(
          route,
          200,
          invitationStatus({
            invitation_id: 7,
            invitation_status: "pending",
            delivery_state: "failed",
            last_delivery_error: "provider_failed",
            delivery_attempts: 1,
          }),
        ),
      "invitation/prepare": (route) =>
        json(route, 200, {
          ok: true,
          person_key: PERSON_KEY,
          masked_contact: MASKED,
          proposed_username: "synthetic.person",
          access: "viewer",
          invitation_id: 7,
          invitation_reused: true,
          invitation_created: false,
        }),
      "invitation/send": (route) =>
        json(route, 502, {
          ok: false,
          error: "delivery_failed",
          message:
            "the mail provider did not accept the message; nothing was sent",
        }),
    });

    await page.goto(`/family/${PERSON_KEY}`);
    await expect(page.getByTestId("profile-invitation-delivery")).toHaveText(
      "Delivery failed",
    );
    await expect(
      page.getByTestId("profile-invitation-delivery-error"),
    ).toContainText("The email provider did not accept the message.");
    await expect(
      page.getByText("provider_failed", { exact: true }),
    ).toHaveCount(0);

    await page.getByTestId("profile-invitation-prepare").click();
    await page.getByTestId("profile-invitation-send").click();
    await expect(page.getByTestId("profile-invitation-notice")).toContainText(
      "the mail provider did not accept the message",
    );
    await expect(
      page.getByText("delivery_failed", { exact: true }),
    ).toHaveCount(0);
  });
});
