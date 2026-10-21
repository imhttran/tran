// Field definitions for the My Profile screen. The keys, limits, and
// visibility rules mirror the owner-scoped profile API; the browser never
// invents a field that Rust does not accept.

import type { ProfileVisibility } from "@/lib/api";

export type ProfileSection = "about" | "story" | "memories";

export type ProfileFieldDefinition = {
  key: string;
  label: string;
  section: ProfileSection;
  multiline: boolean;
  rows: number;
  placeholder: string;
  helper: string;
  maxLength: number;
  privateByDefault: boolean;
};

export const PROFILE_FIELDS: ProfileFieldDefinition[] = [
  {
    key: "display_name",
    label: "Display name",
    section: "about",
    multiline: false,
    rows: 1,
    placeholder: "How your name appears on your profile",
    helper:
      "This is your profile display label; it does not change your genealogy record.",
    maxLength: 200,
    privateByDefault: false,
  },
  {
    key: "preferred_name",
    label: "Preferred name or nickname",
    section: "about",
    multiline: false,
    rows: 1,
    placeholder: "What family and friends call you",
    helper: "Optional. Keep the name you want future generations to see.",
    maxLength: 200,
    privateByDefault: false,
  },
  {
    key: "biography",
    label: "Biography and introduction",
    section: "about",
    multiline: true,
    rows: 5,
    placeholder: "Introduce yourself to future generations…",
    helper:
      "A short introduction. Personal recollections are narratives, not verified genealogy facts.",
    maxLength: 5_000,
    privateByDefault: false,
  },
  {
    key: "current_location",
    label: "Current location",
    section: "about",
    multiline: false,
    rows: 1,
    placeholder: "City, country, or region (optional)",
    helper:
      "Owner-private by default. Do not add a precise address unless you choose to.",
    maxLength: 200,
    privateByDefault: true,
  },
  {
    key: "contact_preferences",
    label: "Contact preferences",
    section: "about",
    multiline: true,
    rows: 3,
    placeholder: "How would you prefer family to contact you? (optional)",
    helper:
      "Owner-private by default. Reviewers and family do not see this unless you choose family visibility.",
    maxLength: 500,
    privateByDefault: true,
  },
  {
    key: "facebook_url",
    label: "Facebook",
    section: "about",
    multiline: false,
    rows: 1,
    placeholder: "Profile or contact link (optional)",
    helper: "Owner-private by default.",
    maxLength: 500,
    privateByDefault: true,
  },
  {
    key: "linkedin_url",
    label: "LinkedIn",
    section: "about",
    multiline: false,
    rows: 1,
    placeholder: "Profile or contact link (optional)",
    helper: "Owner-private by default.",
    maxLength: 500,
    privateByDefault: true,
  },
  {
    key: "snapchat",
    label: "Snapchat",
    section: "about",
    multiline: false,
    rows: 1,
    placeholder: "Username or contact link (optional)",
    helper: "Owner-private by default.",
    maxLength: 500,
    privateByDefault: true,
  },
  {
    key: "whatsapp",
    label: "WhatsApp",
    section: "about",
    multiline: false,
    rows: 1,
    placeholder: "Contact link or number (optional)",
    helper: "Owner-private by default.",
    maxLength: 500,
    privateByDefault: true,
  },
  {
    key: "my_story",
    label: "My story",
    section: "story",
    multiline: true,
    rows: 12,
    placeholder:
      "Where did you grow up? What shaped you? Share education, career, important life events, traditions, memories, lessons, or advice for future generations…",
    helper:
      "A longer personal narrative. This is not a verified genealogy fact and does not change relationships.",
    maxLength: 20_000,
    privateByDefault: false,
  },
  {
    key: "family_memories",
    label: "Family memories",
    section: "memories",
    multiline: true,
    rows: 12,
    placeholder:
      "Share a memory about your parents, grandparents, family traditions, or a story passed down through generations…",
    helper:
      "Personal recollections remain narrative submissions unless independently reviewed as genealogy evidence.",
    maxLength: 20_000,
    privateByDefault: false,
  },
];

export const PROFILE_SECTION_COPY: Record<
  ProfileSection,
  { title: string; intro: string; invitation: string }
> = {
  about: {
    title: "About me",
    intro:
      "Tell future generations who you are. Contact details and your current location are owner-private by default.",
    invitation: "Tell future generations about yourself.",
  },
  story: {
    title: "My story",
    intro:
      "Preserve the experiences that made you who you are. These are personal narratives, separate from verified genealogy facts.",
    invitation: "Share the experiences that shaped you.",
  },
  memories: {
    title: "Family memories",
    intro:
      "Record recollections about parents, grandparents, traditions, and stories passed down. Personal stories do not automatically create or change genealogy relationships.",
    invitation: "Share a memory about your parents or grandparents.",
  },
};

export function profileField(
  fieldKey: string,
): ProfileFieldDefinition | undefined {
  return PROFILE_FIELDS.find((field) => field.key === fieldKey);
}

// Every new value starts owner-private on the server. Private contact fields
// stay hidden by default; a member can opt in to family visibility explicitly.
export function defaultVisibility(
  _field: ProfileFieldDefinition,
): ProfileVisibility {
  return "owner_private";
}
