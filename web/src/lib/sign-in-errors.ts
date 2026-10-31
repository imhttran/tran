// Prose for the safe codes the API returns when Keycloak could not prepare an
// invitee's sign-in. The code names a rule; it never carries a name or address.
const SIGN_IN_ERRORS: Record<string, string> = {
  login_username_taken:
    "That username already belongs to a different Keycloak login. Change the username, or fix the login in Keycloak, then send again.",
  login_ambiguous:
    "More than one Keycloak login uses this email address. Remove the extra login in Keycloak, then send again.",
  login_disabled:
    "This person's Keycloak login is disabled. Enable it in Keycloak, then send again.",
  login_username_unavailable:
    "No username could be reserved for this person. Check their name, then send again.",
  login_redirect_rejected:
    "Keycloak refused the activation link. Add it to the web client's valid redirect URIs.",
  login_email_failed:
    "Keycloak could not send its password email. Check the realm's email settings.",
  login_provider_unauthorized:
    "Keycloak refused the invitation service account. Check its client secret and user-management roles.",
  login_provider_unavailable: "Keycloak could not be reached. Nothing was sent.",
  login_provider_failed: "Keycloak did not accept the request. Nothing was sent.",
  login_provider_invalid_response: "Keycloak sent an unexpected answer. Nothing was sent.",
  login_conflict: "Keycloak reported a conflicting login. Try again.",
};

/** Returns prose for a sign-in error code, or null for any other code. */
export function signInErrorText(code: string | null | undefined): string | null {
  if (!code || !code.startsWith("login_")) return null;
  return SIGN_IN_ERRORS[code] ?? "Keycloak could not prepare this person's sign-in.";
}
