#!/usr/bin/env python3
"""Focused checks for scripts/admin_setup.py.

Unit cases need nothing. The integration cases use a fake Keycloak admin API
and a disposable PostgreSQL database derived from DATABASE_URL; they are
skipped when DATABASE_URL is unset or not disposable.

    python3 scripts/test_admin_setup.py -v
"""

from __future__ import annotations

import importlib.util
import json
import os
import shutil
import subprocess
import sys
import threading
import unittest
import urllib.parse
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
SCRIPT = ROOT / "scripts" / "admin-setup.sh"
USER_ID = "8f14e45f-ceea-467f-a7c1-1b2c3d4e5f60"
SECOND_ID = "0b7c9e1a-2d3f-4a5b-8c6d-7e8f9a0b1c2d"

_spec = importlib.util.spec_from_file_location(
    "admin_setup", ROOT / "scripts" / "admin_setup.py"
)
admin_setup = importlib.util.module_from_spec(_spec)
assert _spec and _spec.loader
sys.modules[_spec.name] = admin_setup
_spec.loader.exec_module(admin_setup)


class UnitTests(unittest.TestCase):
    def test_placeholder_ids_are_rejected(self):
        for value in [
            "",
            "   ",
            "<sub>",
            "{{user}}",
            "TODO",
            "changeme",
            "your-user-id",
        ]:
            with self.subTest(value=value), self.assertRaises(admin_setup.SetupError):
                admin_setup.validate_subject(value)

    def test_real_ids_are_accepted(self):
        self.assertEqual(admin_setup.validate_subject(USER_ID), USER_ID)
        self.assertEqual(
            admin_setup.validate_subject("custom-immutable-id"), "custom-immutable-id"
        )

    def test_username_matching_is_exact_and_case_insensitive(self):
        users = [
            admin_setup.KeycloakUser(USER_ID, "Alice", True),
            admin_setup.KeycloakUser(SECOND_ID, "alice2", True),
        ]
        matches = admin_setup.match_username(users, " alice ")
        self.assertEqual([user.id for user in matches], [USER_ID])

    def test_username_matching_detects_ambiguity(self):
        users = [
            admin_setup.KeycloakUser(USER_ID, "Alice", True),
            admin_setup.KeycloakUser(SECOND_ID, "ALICE", True),
        ]
        self.assertEqual(len(admin_setup.match_username(users, "alice")), 2)

    def test_state_and_plan(self):
        state = admin_setup.parse_state("0|0|0|0")
        self.assertFalse(state.admin_exists)
        self.assertEqual(admin_setup.plan_actions(state), ["bootstrap"])

        existing = admin_setup.parse_state("1|1|1|1")
        self.assertEqual(admin_setup.plan_actions(existing), [])

        reactivate = admin_setup.parse_state("1|0|0|0")
        self.assertEqual(
            admin_setup.plan_actions(reactivate),
            ["provision", "grant-admin", "grant-corpus"],
        )

        partial = admin_setup.parse_state("1|1|1|0")
        self.assertEqual(admin_setup.plan_actions(partial), ["grant-corpus"])

        with self.assertRaises(admin_setup.SetupError):
            admin_setup.parse_state("not-a-state")

    def test_issuer_is_derived_from_realm(self):
        self.assertEqual(
            admin_setup.keycloak_issuer("https://kc.example.com/", "tran"),
            "https://kc.example.com/realms/tran",
        )


# --- fake Keycloak -----------------------------------------------------------


class FakeKeycloak(BaseHTTPRequestHandler):
    users = []
    token_grants = []

    def log_message(self, *_args):  # silence the test output
        pass

    def _json(self, status, body):
        payload = json.dumps(body).encode("utf-8")
        self.send_response(status)
        self.send_header("Content-Type", "application/json")
        self.send_header("Content-Length", str(len(payload)))
        self.end_headers()
        self.wfile.write(payload)

    def do_POST(self):
        if not self.path.endswith("/protocol/openid-connect/token"):
            self._json(404, {"error": "not_found"})
            return
        length = int(self.headers.get("Content-Length", "0"))
        form = urllib.parse.parse_qs(self.rfile.read(length).decode("utf-8"))
        FakeKeycloak.token_grants.append(form.get("grant_type", [""])[0])
        self._json(
            200,
            {"access_token": "fake-token", "token_type": "Bearer", "expires_in": 300},
        )

    def do_GET(self):
        parsed = urllib.parse.urlparse(self.path)
        if not parsed.path.endswith("/admin/realms/test/users"):
            self._json(404, {"error": "not_found"})
            return
        if self.headers.get("Authorization") != "Bearer fake-token":
            self._json(401, {"error": "unauthorized"})
            return
        self._json(200, FakeKeycloak.users)


# --- integration -------------------------------------------------------------


def with_database(dsn, name):
    query = ""
    if "?" in dsn:
        dsn, query = dsn.split("?", 1)
        query = "?" + query
    return dsn.rsplit("/", 1)[0] + "/" + name + query


def database_name(dsn):
    return dsn.split("?", 1)[0].rsplit("/", 1)[-1]


def psql(dsn, *args, **kwargs):
    return subprocess.run(
        ["psql", dsn, "-X", *args], capture_output=True, text=True, **kwargs
    )


@unittest.skipUnless(shutil.which("psql"), "psql is required for the integration cases")
class IntegrationTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.base = os.environ.get("DATABASE_URL", "").strip()
        if not cls.base or not database_name(cls.base).endswith(
            ("_test", "_disposable")
        ):
            raise unittest.SkipTest(
                "DATABASE_URL must name a disposable *_test or *_disposable base database"
            )
        cls.database = f"admin_setup_{os.getpid()}_test"
        created = psql(
            cls.base, "-v", "ON_ERROR_STOP=1", "-c", f'CREATE DATABASE "{cls.database}"'
        )
        if created.returncode != 0:
            raise unittest.SkipTest(
                f"cannot create a disposable database: {created.stderr.strip()}"
            )
        cls.dsn = with_database(cls.base, cls.database)
        for migration in (
            "migrations/006_identity.sql",
            "migrations/007_user_lifecycle.sql",
            "migrations/008_identity_issuer_subject.sql",
        ):
            applied = psql(
                cls.dsn, "-v", "ON_ERROR_STOP=1", "-f", str(ROOT / migration)
            )
            if applied.returncode != 0:
                raise RuntimeError(f"apply {migration}: {applied.stderr.strip()}")

        cls.server = ThreadingHTTPServer(("127.0.0.1", 0), FakeKeycloak)
        cls.port = cls.server.server_address[1]
        cls.thread = threading.Thread(target=cls.server.serve_forever, daemon=True)
        cls.thread.start()

    @classmethod
    def tearDownClass(cls):
        cls.server.shutdown()
        cls.server.server_close()
        psql(
            cls.base,
            "-v",
            "ON_ERROR_STOP=1",
            "-c",
            f'DROP DATABASE IF EXISTS "{cls.database}"',
        )

    def setUp(self):
        FakeKeycloak.users = []
        FakeKeycloak.token_grants = []
        psql(
            self.dsn,
            "-v",
            "ON_ERROR_STOP=1",
            "-c",
            "TRUNCATE users, role_assignments, corpus_grants, audit_events RESTART IDENTITY CASCADE",
        )

    # helpers -------------------------------------------------------------

    def run_setup(self, *args, extra_env=None):
        env = os.environ.copy()
        env.update(
            {
                "KEYCLOAK_URL": f"http://127.0.0.1:{self.port}",
                "KEYCLOAK_REALM": "test",
                "KEYCLOAK_ADMIN_USERNAME": "fake-admin",
                "KEYCLOAK_ADMIN_PASSWORD": "fake-password",
                "AUTH_ISSUER": f"http://127.0.0.1:{self.port}/realms/test",
                "DATABASE_URL": self.dsn,
                "ADMIN_CORPUS": "trusted",
            }
        )
        env.pop("KEYCLOAK_ADMIN_CLIENT_SECRET", None)
        env.pop("KEYCLOAK_USERNAME", None)
        if extra_env:
            env.update(extra_env)
        return subprocess.run(
            [str(SCRIPT), *args],
            capture_output=True,
            text=True,
            env=env,
            cwd=ROOT,
            timeout=300,
        )

    def scalar(self, sql):
        result = psql(self.dsn, "-Atc", sql)
        self.assertEqual(result.returncode, 0, result.stderr)
        return int(result.stdout.strip())

    def actions(self):
        result = psql(self.dsn, "-Atc", "SELECT action FROM audit_events ORDER BY id")
        self.assertEqual(result.returncode, 0, result.stderr)
        return result.stdout.split()

    # cases ---------------------------------------------------------------

    def test_first_admin_bootstrap_and_repeated_run(self):
        FakeKeycloak.users = [{"id": USER_ID, "username": "alice", "enabled": True}]
        first = self.run_setup("--username", "alice")
        self.assertEqual(first.returncode, 0, first.stderr)
        self.assertIn("realm: test", first.stdout)
        self.assertIn("username: alice", first.stdout)
        self.assertIn(f"resolved user id: {USER_ID}", first.stdout)
        self.assertNotIn("fake-password", first.stdout + first.stderr)
        self.assertNotIn(self.dsn, first.stdout + first.stderr)
        self.assertEqual(self.scalar("SELECT count(*) FROM users"), 1)
        self.assertEqual(
            self.scalar("SELECT count(*) FROM role_assignments WHERE role = 'admin'"),
            1,
        )
        self.assertEqual(self.scalar("SELECT count(*) FROM corpus_grants"), 1)
        self.assertEqual(self.actions(), ["bootstrap.initial_admin", "corpus.grant"])

        repeated = self.run_setup("--username", "alice")
        self.assertEqual(repeated.returncode, 0, repeated.stderr)
        self.assertIn("already configured", repeated.stdout)
        self.assertEqual(self.actions(), ["bootstrap.initial_admin", "corpus.grant"])

    def test_existing_admin_uses_provision_role_and_corpus_commands(self):
        FakeKeycloak.users = [{"id": USER_ID, "username": "alice", "enabled": True}]
        self.assertEqual(self.run_setup("--username", "alice").returncode, 0)

        FakeKeycloak.users.append({"id": SECOND_ID, "username": "bob", "enabled": True})
        added = self.run_setup("--username", "bob")
        self.assertEqual(added.returncode, 0, added.stderr)
        self.assertEqual(self.scalar("SELECT count(*) FROM users"), 2)
        self.assertIn("grant-corpus", added.stdout)
        self.assertEqual(
            self.scalar(
                "SELECT count(*) FROM role_assignments r JOIN users u ON u.id = r.user_id "
                "WHERE u.subject = '0b7c9e1a-2d3f-4a5b-8c6d-7e8f9a0b1c2d' "
                "AND r.role = 'admin' AND r.revoked_at IS NULL"
            ),
            1,
        )
        self.assertIn("user.provision", self.actions())
        self.assertIn("role.grant", self.actions())
        self.assertIn("corpus.grant", self.actions())

        again = self.run_setup("--username", "bob")
        self.assertEqual(again.returncode, 0, again.stderr)
        self.assertIn("already configured", again.stdout)

    def test_missing_user_is_rejected_without_writes(self):
        FakeKeycloak.users = [{"id": USER_ID, "username": "alice2", "enabled": True}]
        result = self.run_setup("--username", "alice")
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("no Keycloak user", result.stderr)
        self.assertEqual(self.scalar("SELECT count(*) FROM users"), 0)

    def test_ambiguous_user_is_rejected(self):
        FakeKeycloak.users = [
            {"id": USER_ID, "username": "Alice", "enabled": True},
            {"id": SECOND_ID, "username": "alice", "enabled": True},
        ]
        result = self.run_setup("--username", "alice")
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("ambiguous", result.stderr)
        self.assertEqual(self.scalar("SELECT count(*) FROM users"), 0)

    def test_placeholder_id_is_rejected(self):
        FakeKeycloak.users = [{"id": "<sub>", "username": "alice", "enabled": True}]
        result = self.run_setup("--username", "alice")
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("placeholder", result.stderr)
        self.assertEqual(self.scalar("SELECT count(*) FROM users"), 0)

    def test_issuer_mismatch_is_rejected(self):
        FakeKeycloak.users = [{"id": USER_ID, "username": "alice", "enabled": True}]
        result = self.run_setup(
            "--username",
            "alice",
            extra_env={"AUTH_ISSUER": "https://elsewhere/realms/test"},
        )
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("AUTH_ISSUER does not match", result.stderr)

    def test_dry_run_writes_nothing(self):
        FakeKeycloak.users = [{"id": USER_ID, "username": "alice", "enabled": True}]
        result = self.run_setup("--username", "alice", "--dry-run")
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn("dry run", result.stdout)
        self.assertEqual(self.scalar("SELECT count(*) FROM users"), 0)

    def test_client_credentials_flow_never_prints_the_secret(self):
        FakeKeycloak.users = [{"id": USER_ID, "username": "alice", "enabled": True}]
        result = self.run_setup(
            "--username",
            "alice",
            extra_env={
                "KEYCLOAK_ADMIN_CLIENT_SECRET": "fake-client-secret",
                "KEYCLOAK_ADMIN_USERNAME": "",
                "KEYCLOAK_ADMIN_PASSWORD": "",
            },
        )
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn("client_credentials", FakeKeycloak.token_grants)
        self.assertNotIn("fake-client-secret", result.stdout + result.stderr)
        self.assertEqual(self.scalar("SELECT count(*) FROM users"), 1)


if __name__ == "__main__":
    unittest.main()
