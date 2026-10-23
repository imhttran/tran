#!/usr/bin/env python3
"""Initial Tran administrator setup through Keycloak.

This is the explicit operator path that resolves a Keycloak *username* to the
provider's immutable user id, shows what it resolved, and then delegates every
write to the existing `admin` CLI commands. It never writes identity rows
directly, never touches genealogy data, and never links an account to a person.

Flow:
  1. Read the effective configuration (the wrapper loads .env with caller-wins).
  2. Obtain a Keycloak admin token (password grant, or client credentials when a
     client secret is configured). Credentials are prompted for, never printed.
  3. Look the username up and require exactly one case-insensitive match.
  4. Reject missing users, ambiguous matches, and placeholder user ids.
  5. Read the application state (read-only) to decide between the first-admin
     bootstrap and the existing provision / grant / grant-corpus commands.
  6. Print the realm, username, resolved id, issuer, and planned actions.
  7. Run only the commands needed, so a repeated run is a no-op.

Required configuration:
  KEYCLOAK_URL                    Keycloak base URL, e.g. https://kc.example.com
  KEYCLOAK_REALM                  Realm name
  AUTH_ISSUER                     Must equal {KEYCLOAK_URL}/realms/{KEYCLOAK_REALM}
  DATABASE_URL                    Application PostgreSQL database
  KEYCLOAK_USERNAME (or --username)  Username to resolve; prompted when absent
  ADMIN_CORPUS (or --corpus)      Corpus to grant; default trusted

Optional:
  KEYCLOAK_ADMIN_CLIENT_ID        Default admin-cli
  KEYCLOAK_ADMIN_CLIENT_SECRET    When set, client_credentials is used
  KEYCLOAK_ADMIN_USERNAME         Admin username; prompted when absent
  KEYCLOAK_ADMIN_PASSWORD         Admin password; prompted when absent
"""

from __future__ import annotations

import argparse
import getpass
import json
import os
import re
import subprocess
import sys
import urllib.error
import urllib.parse
import urllib.request
from dataclasses import dataclass
from typing import Dict, List, Optional, Sequence

DEFAULT_CLIENT_ID = "admin-cli"
DEFAULT_CORPUS = "trusted"

# Values that mean "the operator forgot to fill this in". They are never a
# usable Keycloak user id.
PLACEHOLDER_SUBJECTS = {
    "sub",
    "subject",
    "user",
    "user-id",
    "user_id",
    "your-user-id",
    "changeme",
    "change-me",
    "todo",
    "tbd",
    "placeholder",
    "example",
    "none",
    "null",
}

_SUBJECT_RE = re.compile(r"^[^\s<>{}]+$")


class SetupError(Exception):
    """A setup step failed. The message is safe to print."""


@dataclass(frozen=True)
class KeycloakUser:
    id: str
    username: str
    enabled: bool


@dataclass(frozen=True)
class AppState:
    admin_exists: bool
    user_active: bool
    has_admin_role: bool
    has_corpus: bool


@dataclass(frozen=True)
class Config:
    keycloak_url: str
    realm: str
    issuer: str
    database_url: str
    client_id: str
    client_secret: Optional[str]
    target_username: Optional[str]
    admin_username: Optional[str]
    admin_password: Optional[str]
    corpus: str


@dataclass(frozen=True)
class Credentials:
    username: str
    admin_username: str
    admin_password: str


def env(name: str) -> Optional[str]:
    value = os.environ.get(name)
    if value is None:
        return None
    value = value.strip()
    return value or None


def keycloak_issuer(base_url: str, realm: str) -> str:
    return f"{base_url.rstrip('/')}/realms/{realm}"


def validate_subject(subject: str) -> str:
    """Return the immutable id, or raise when it cannot be a real one."""
    value = (subject or "").strip()
    if not value:
        raise SetupError("the resolved Keycloak user id is empty")
    if value.casefold() in PLACEHOLDER_SUBJECTS:
        raise SetupError(f"refusing the placeholder user id {value!r}")
    if value.startswith("<") and value.endswith(">"):
        raise SetupError(f"refusing the placeholder user id {value!r}")
    if not _SUBJECT_RE.match(value):
        raise SetupError(
            f"refusing user id {value!r}: it contains whitespace or template characters"
        )
    if len(value) < 6:
        raise SetupError(f"refusing user id {value!r}: too short to be an immutable id")
    return value


def match_username(users: Sequence[KeycloakUser], username: str) -> List[KeycloakUser]:
    """Case-insensitive exact match. Substring matches from Keycloak are ignored."""
    wanted = (username or "").strip().casefold()
    if not wanted:
        return []
    return [user for user in users if user.username.strip().casefold() == wanted]


def parse_state(output: str) -> AppState:
    fields = output.strip().split("|")
    if len(fields) != 4:
        raise SetupError(f"unexpected application state output: {output!r}")
    try:
        admin_count, user_active, has_role, has_corpus = (int(field) for field in fields)
    except ValueError as error:
        raise SetupError(f"unexpected application state output: {output!r}") from error
    return AppState(
        admin_exists=admin_count > 0,
        user_active=user_active == 1,
        has_admin_role=has_role == 1,
        has_corpus=has_corpus == 1,
    )


def plan_actions(state: AppState) -> List[str]:
    """The minimum audited commands needed to make this user an active admin."""
    if not state.admin_exists:
        # The bootstrap command itself creates (or reuses) the user, grants
        # admin, and optionally grants the corpus, all in one transaction.
        return ["bootstrap"]
    actions: List[str] = []
    if not state.user_active:
        actions.append("provision")
    if not state.has_admin_role:
        actions.append("grant-admin")
    if not state.has_corpus:
        actions.append("grant-corpus")
    return actions


# --- configuration ----------------------------------------------------------


def load_config(args: argparse.Namespace) -> Config:
    keycloak_url = env("KEYCLOAK_URL")
    realm = env("KEYCLOAK_REALM")
    issuer = env("AUTH_ISSUER")
    database_url = env("DATABASE_URL")

    missing = [
        name
        for name, value in (
            ("KEYCLOAK_URL", keycloak_url),
            ("KEYCLOAK_REALM", realm),
            ("AUTH_ISSUER", issuer),
            ("DATABASE_URL", database_url),
        )
        if not value
    ]
    if missing:
        raise SetupError(
            "missing required configuration: " + ", ".join(missing) + " (set it or add it to .env)"
        )

    base = keycloak_url.rstrip("/")
    if base.endswith(f"/realms/{realm}"):
        raise SetupError(
            "KEYCLOAK_URL must be the Keycloak base URL, not the realm URL; "
            f"use {base[:-len('/realms/' + realm)]}"
        )
    expected = keycloak_issuer(keycloak_url, realm)
    if issuer.rstrip("/") != expected:
        raise SetupError(
            "AUTH_ISSUER does not match KEYCLOAK_URL/KEYCLOAK_REALM; "
            f"expected {expected}, refusing to provision a different issuer"
        )

    corpus = (args.corpus or env("ADMIN_CORPUS") or DEFAULT_CORPUS).strip()
    if not corpus:
        raise SetupError("a corpus is required (ADMIN_CORPUS or --corpus)")

    return Config(
        keycloak_url=keycloak_url,
        realm=realm,
        issuer=issuer.rstrip("/"),
        database_url=database_url,
        client_id=env("KEYCLOAK_ADMIN_CLIENT_ID") or DEFAULT_CLIENT_ID,
        client_secret=env("KEYCLOAK_ADMIN_CLIENT_SECRET"),
        target_username=(args.username or env("KEYCLOAK_USERNAME")),
        admin_username=env("KEYCLOAK_ADMIN_USERNAME"),
        admin_password=env("KEYCLOAK_ADMIN_PASSWORD"),
        corpus=corpus,
    )


# --- Keycloak ---------------------------------------------------------------


def _post_form(url: str, form: Dict[str, str]) -> Dict[str, object]:
    data = urllib.parse.urlencode(form).encode("utf-8")
    request = urllib.request.Request(
        url,
        data=data,
        headers={"Content-Type": "application/x-www-form-urlencoded"},
        method="POST",
    )
    try:
        with urllib.request.urlopen(request, timeout=15) as response:
            return json.loads(response.read().decode("utf-8"))
    except urllib.error.HTTPError as error:
        raise SetupError(
            f"Keycloak token request failed with HTTP {error.code}: {_kc_reason(error)}"
        ) from error
    except urllib.error.URLError as error:
        raise SetupError(f"cannot reach Keycloak at {url}: {error.reason}") from error
    except json.JSONDecodeError as error:
        raise SetupError("Keycloak returned a token response that is not JSON") from error


def _kc_reason(error: urllib.error.HTTPError) -> str:
    try:
        body = json.loads(error.read().decode("utf-8"))
    except Exception:  # noqa: BLE001 - never echo an unknown body
        return "no further detail"
    if isinstance(body, dict):
        reason = body.get("error_description") or body.get("error")
        if isinstance(reason, str) and reason:
            return reason
    return "no further detail"


def admin_token(config: Config, username: str, password: str) -> str:
    token_url = (
        f"{config.keycloak_url.rstrip('/')}/realms/"
        f"{urllib.parse.quote(config.realm, safe='')}/protocol/openid-connect/token"
    )
    if config.client_secret:
        response = _post_form(
            token_url,
            {
                "grant_type": "client_credentials",
                "client_id": config.client_id,
                "client_secret": config.client_secret,
            },
        )
    else:
        response = _post_form(
            token_url,
            {
                "grant_type": "password",
                "client_id": config.client_id,
                "username": username,
                "password": password,
            },
        )
    token = response.get("access_token")
    if not isinstance(token, str) or not token:
        raise SetupError("Keycloak did not return an access token")
    return token


def lookup_users(config: Config, token: str, username: str) -> List[KeycloakUser]:
    base = (
        f"{config.keycloak_url.rstrip('/')}/admin/realms/"
        f"{urllib.parse.quote(config.realm, safe='')}/users"
    )
    # `exact=false` avoids case-sensitivity surprises; the result is filtered by
    # an exact case-insensitive match below, so substring hits never count.
    query = urllib.parse.urlencode({"username": username, "exact": "false", "max": "100"})
    request = urllib.request.Request(
        f"{base}?{query}", headers={"Authorization": f"Bearer {token}"}
    )
    try:
        with urllib.request.urlopen(request, timeout=15) as response:
            body = json.loads(response.read().decode("utf-8"))
    except urllib.error.HTTPError as error:
        raise SetupError(
            f"Keycloak user lookup failed with HTTP {error.code}: {_kc_reason(error)}"
        ) from error
    except urllib.error.URLError as error:
        raise SetupError(f"cannot reach Keycloak admin API: {error.reason}") from error
    except json.JSONDecodeError as error:
        raise SetupError("Keycloak returned a user list that is not JSON") from error
    if not isinstance(body, list):
        raise SetupError("Keycloak returned an unexpected user list")
    users: List[KeycloakUser] = []
    for entry in body:
        if not isinstance(entry, dict):
            continue
        user_id = entry.get("id")
        name = entry.get("username")
        if isinstance(user_id, str) and isinstance(name, str):
            users.append(KeycloakUser(id=user_id, username=name, enabled=bool(entry.get("enabled", True))))
    return users


def prompt_credentials(config: Config) -> Credentials:
    """Fill missing usernames/passwords interactively. Never echo a secret."""
    target = (config.target_username or "").strip()
    if not target:
        try:
            target = input("Tran administrator Keycloak username: ").strip()
        except EOFError as error:
            raise SetupError("the Keycloak username to resolve is required") from error
        if not target:
            raise SetupError("the Keycloak username to resolve is required")

    if config.client_secret:
        # A service account needs no interactive administrator credentials.
        return Credentials(username=target, admin_username="", admin_password="")

    admin = (config.admin_username or "").strip()
    if not admin:
        try:
            admin = input("Keycloak administrator username: ").strip()
        except EOFError as error:
            raise SetupError("a Keycloak administrator username is required") from error
        if not admin:
            raise SetupError("a Keycloak administrator username is required")

    password = config.admin_password or ""
    if not password:
        try:
            password = getpass.getpass("Keycloak administrator password: ")
        except EOFError as error:
            raise SetupError("a Keycloak administrator password is required") from error
        if not password:
            raise SetupError("a Keycloak administrator password is required")
    return Credentials(username=target, admin_username=admin, admin_password=password)


# --- application state ------------------------------------------------------


STATE_SQL = """
SELECT
  (SELECT count(*) FROM role_assignments
    WHERE role = 'admin' AND revoked_at IS NULL),
  COALESCE((SELECT 1 FROM users
    WHERE subject = :'subject' AND issuer = :'issuer' AND deactivated_at IS NULL), 0),
  COALESCE((SELECT 1 FROM role_assignments r JOIN users u ON u.id = r.user_id
    WHERE u.subject = :'subject' AND u.issuer = :'issuer'
      AND r.role = 'admin' AND r.revoked_at IS NULL), 0),
  COALESCE((SELECT 1 FROM corpus_grants c JOIN users u ON u.id = c.user_id
    WHERE u.subject = :'subject' AND u.issuer = :'issuer'
      AND c.corpus = :'corpus' AND c.revoked_at IS NULL), 0);
"""


def read_state(config: Config, subject: str) -> AppState:
    command = [
        "psql",
        config.database_url,
        "-X",
        "-A",
        "-t",
        "-F",
        "|",
        "-v",
        f"subject={subject}",
        "-v",
        f"issuer={config.issuer}",
        "-v",
        f"corpus={config.corpus}",
        # psql does not interpolate :variables in a `-c` string, so the query
        # arrives on stdin and is processed as a script.
        "-f",
        "-",
    ]
    try:
        result = subprocess.run(
            command, input=STATE_SQL, capture_output=True, text=True, check=False
        )
    except FileNotFoundError as error:
        raise SetupError("psql is required to read the application administrator state") from error
    if result.returncode != 0:
        # psql's stderr can contain the DSN; print only a generic message.
        raise SetupError(
            "cannot read the application administrator state from DATABASE_URL; "
            "check that the database is reachable and migrated"
        )
    return parse_state(result.stdout)


# --- administrator commands -------------------------------------------------


def admin_command(arguments: Sequence[str]) -> None:
    command = ["cargo", "run", "--locked", "--bin", "admin", "--", *arguments]
    result = subprocess.run(command, check=False)
    if result.returncode != 0:
        raise SetupError(f"administrator command failed: admin {arguments[0]}")


def apply_actions(config: Config, subject: str, actions: Sequence[str]) -> None:
    for action in actions:
        common = ["--subject", subject, "--issuer", config.issuer]
        if action == "bootstrap":
            admin_command(["bootstrap", *common, "--corpus", config.corpus])
        elif action == "provision":
            admin_command(["provision", *common])
        elif action == "grant-admin":
            admin_command(["grant", *common, "--role", "admin"])
        elif action == "grant-corpus":
            admin_command(["grant-corpus", *common, "--corpus", config.corpus])
        else:  # pragma: no cover - plan_actions only emits the values above
            raise SetupError(f"unknown administrator action {action}")


def main(argv: Optional[Sequence[str]] = None) -> int:
    parser = argparse.ArgumentParser(
        description="Resolve a Keycloak username and configure a Tran administrator."
    )
    parser.add_argument("--username", default="", help="Keycloak username to resolve")
    parser.add_argument("--corpus", default="", help="Corpus to grant (default trusted)")
    parser.add_argument(
        "--dry-run",
        action="store_true",
        help="resolve, print the plan, and exit without writing",
    )
    args = parser.parse_args(argv)

    try:
        config = load_config(args)
        credentials = prompt_credentials(config)
        token = admin_token(config, credentials.admin_username, credentials.admin_password)
        users = lookup_users(config, token, credentials.username)
        matches = match_username(users, credentials.username)
        if not matches:
            raise SetupError(
                f"no Keycloak user named {credentials.username!r} in realm {config.realm!r}"
            )
        if len(matches) > 1:
            found = ", ".join(f"{user.username} ({user.id})" for user in matches)
            raise SetupError(
                f"ambiguous Keycloak username {credentials.username!r} in realm "
                f"{config.realm!r}: {found}"
            )
        user = matches[0]
        subject = validate_subject(user.id)

        state = read_state(config, subject)
        actions = plan_actions(state)

        print(f"keycloak url: {config.keycloak_url.rstrip('/')}")
        print(f"realm: {config.realm}")
        print(f"username: {user.username}")
        print(f"resolved user id: {subject}")
        print(f"issuer: {config.issuer}")
        print(f"keycloak enabled: {'yes' if user.enabled else 'no'}")
        print(f"application database: configured (credentials hidden)")
        print(f"corpus: {config.corpus}")
        if actions:
            mode = "first administrator" if actions == ["bootstrap"] else "existing administrator"
            print(f"mode: {mode}")
            print(f"planned actions: {', '.join(actions)}")
        else:
            print("mode: already configured")
            print("planned actions: none")

        if args.dry_run:
            print("dry run: no changes written")
            return 0

        if not actions:
            print("administrator already configured; nothing to do")
            return 0

        apply_actions(config, subject, actions)
        print("administrator setup complete")
        return 0
    except SetupError as error:
        print(f"admin-setup: {error}", file=sys.stderr)
        return 1


if __name__ == "__main__":  # pragma: no cover
    sys.exit(main())
