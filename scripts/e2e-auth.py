#!/usr/bin/env python3
"""Test-only OIDC provider and token minter for the web E2E suite.

The provider issues RS256 tokens from an in-memory RSA key and exposes the public
JWKS. It supports authorization-code + PKCE so the Next.js sign-in flow is tested
end to end. It is not a production identity provider: keys are generated per run
and the seeded accounts are synthetic.
"""

from __future__ import annotations

import argparse
import base64
import hashlib
import json
import secrets
import time
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from urllib.parse import parse_qs, urlencode, urlparse

from cryptography.hazmat.primitives import hashes, serialization
from cryptography.hazmat.primitives.asymmetric import padding, rsa

KID = "e2e"
CLIENT_ID = "tran-e2e"
SUBJECT = "e2e-admin"


class ProviderUnavailable(Exception):
    """Test-only transient identity-provider failure."""


def b64url(data: bytes) -> str:
    return base64.urlsafe_b64encode(data).rstrip(b"=").decode("ascii")


def int_to_bytes(value: int) -> bytes:
    return value.to_bytes((value.bit_length() + 7) // 8, "big")


def generate_key() -> rsa.RSAPrivateKey:
    return rsa.generate_private_key(public_exponent=65537, key_size=2048)


def public_jwk(key: rsa.RSAPrivateKey) -> dict:
    numbers = key.public_key().public_numbers()
    return {
        "kty": "RSA",
        "kid": KID,
        "alg": "RS256",
        "use": "sig",
        "n": b64url(int_to_bytes(numbers.n)),
        "e": b64url(int_to_bytes(numbers.e)),
    }


def make_token(
    key: rsa.RSAPrivateKey,
    issuer: str,
    audience: str,
    subject: str,
    roles: list[str],
    corpora: list[str],
    ttl: int,
    nonce: str | None = None,
) -> str:
    now = int(time.time())
    header = {"alg": "RS256", "kid": KID, "typ": "JWT"}
    payload = {
        "iss": issuer,
        "aud": audience,
        "sub": subject,
        "iat": now,
        "exp": now + ttl,
        "roles": roles,
        "corpus_grants": corpora,
    }
    if nonce is not None:
        payload["nonce"] = nonce
    signing_input = ".".join(
        [
            b64url(json.dumps(header, separators=(",", ":")).encode("utf-8")),
            b64url(json.dumps(payload, separators=(",", ":")).encode("utf-8")),
        ]
    )
    signature = key.sign(
        signing_input.encode("ascii"),
        padding.PKCS1v15(),
        hashes.SHA256(),
    )
    return f"{signing_input}.{b64url(signature)}"


class Provider:
    def __init__(self, port: int, key: rsa.RSAPrivateKey | None = None) -> None:
        self.port = port
        self.key = key or generate_key()
        self.issuer = f"http://127.0.0.1:{port}"
        # code -> (challenge, redirect_uri, nonce, subject)
        self.codes: dict[str, tuple[str, str, str, str]] = {}
        # refresh token -> claims reissued on renewal
        self.refresh_tokens: dict[str, dict] = {}
        self.refresh_calls = 0
        self.fail_refreshes = False
        self.ttl = 3600

    def token(self, code: str, verifier: str, redirect_uri: str) -> tuple[str, str] | None:
        record = self.codes.pop(code, None)
        if record is None:
            return None
        challenge, stored_redirect, nonce, subject = record
        if redirect_uri != stored_redirect:
            return None
        computed = b64url(hashlib.sha256(verifier.encode("ascii")).digest())
        if computed != challenge:
            return None
        id_token = make_token(
            self.key,
            self.issuer,
            CLIENT_ID,
            subject,
            ["admin"],
            [],
            self.ttl,
            nonce=nonce,
        )
        refresh_token = secrets.token_urlsafe(32)
        self.refresh_tokens[refresh_token] = {
            "subject": subject,
            "roles": ["admin"],
            "corpora": [],
        }
        return id_token, refresh_token

    def refresh(self, token: str) -> tuple[str, str] | None:
        """Rotates a refresh token. The old token is invalidated on success."""
        record = self.refresh_tokens.get(token)
        if record is None:
            return None
        self.refresh_calls += 1
        if self.fail_refreshes:
            # A transient failure must not consume the refresh token.
            raise ProviderUnavailable()
        self.refresh_tokens.pop(token, None)
        id_token = make_token(
            self.key,
            self.issuer,
            CLIENT_ID,
            record["subject"],
            record["roles"],
            record["corpora"],
            self.ttl,
        )
        new_refresh = secrets.token_urlsafe(32)
        self.refresh_tokens[new_refresh] = record
        return id_token, new_refresh

    def revoke(self, token: str) -> None:
        self.refresh_tokens.pop(token, None)


class Handler(BaseHTTPRequestHandler):
    server: "AuthServer"

    def log_message(self, *_args: object) -> None:
        return

    def send_json(self, status: int, payload: dict) -> None:
        body = json.dumps(payload, separators=(",", ":")).encode("utf-8")
        self.send_response(status)
        self.send_header("content-type", "application/json")
        self.send_header("content-length", str(len(body)))
        self.end_headers()
        self.wfile.write(body)

    def do_GET(self) -> None:  # noqa: N802 - http.server API
        parsed = urlparse(self.path)
        if parsed.path in ("/jwks", "/.well-known/jwks.json"):
            self.send_json(200, {"keys": [public_jwk(self.server.provider.key)]})
            return
        if parsed.path == "/authorize":
            query = parse_qs(parsed.query)
            if query.get("response_type", [""])[0] != "code":
                self.send_json(400, {"error": "unsupported_response_type"})
                return
            if query.get("client_id", [""])[0] != CLIENT_ID:
                self.send_json(400, {"error": "invalid_client"})
                return
            redirect_uri = query.get("redirect_uri", [""])[0]
            state = query.get("state", [""])[0]
            nonce = query.get("nonce", [""])[0]
            challenge = query.get("code_challenge", [""])[0]
            if not redirect_uri or not state or not nonce or not challenge:
                self.send_json(400, {"error": "invalid_request"})
                return
            code = secrets.token_urlsafe(32)
            self.server.provider.codes[code] = (
                challenge,
                redirect_uri,
                nonce,
                SUBJECT,
            )
            location = f"{redirect_uri}?{urlencode({'code': code, 'state': state})}"
            self.send_response(303)
            self.send_header("location", location)
            self.end_headers()
            return
        if parsed.path == "/logout":
            self.send_json(200, {"ok": True})
            return
        if parsed.path == "/test/stats":
            self.send_json(
                200,
                {"refresh_calls": self.server.provider.refresh_calls},
            )
            return
        self.send_json(404, {"error": "not_found"})

    def do_POST(self) -> None:  # noqa: N802 - http.server API
        parsed = urlparse(self.path)
        length = int(self.headers.get("content-length", "0"))
        raw = self.rfile.read(length).decode("utf-8") if length else ""

        if parsed.path == "/revoke":
            form = parse_qs(raw)
            token = form.get("token", [""])[0]
            if token:
                self.server.provider.revoke(token)
            self.send_json(200, {"ok": True})
            return

        if parsed.path == "/test/mint":
            payload = json.loads(raw or "{}")
            subject = payload.get("subject", SUBJECT)
            ttl = int(payload.get("ttl", 30))
            roles = payload.get("roles", ["admin"])
            corpora = payload.get("corpora", [])
            id_token = make_token(
                self.server.provider.key,
                self.server.provider.issuer,
                CLIENT_ID,
                subject,
                roles,
                corpora,
                ttl,
            )
            refresh = secrets.token_urlsafe(32)
            self.server.provider.refresh_tokens[refresh] = {
                "subject": subject,
                "roles": roles,
                "corpora": corpora,
            }
            self.send_json(
                200,
                {
                    "id_token": id_token,
                    "refresh_token": refresh,
                    "expires_in": ttl,
                },
            )
            return

        if parsed.path == "/test/refresh-failure":
            payload = json.loads(raw or "{}")
            self.server.provider.fail_refreshes = bool(payload.get("enabled", False))
            self.send_json(
                200,
                {
                    "ok": True,
                    "enabled": self.server.provider.fail_refreshes,
                },
            )
            return

        if parsed.path != "/token":
            self.send_json(404, {"error": "not_found"})
            return
        form = parse_qs(raw)
        client_id = form.get("client_id", [""])[0]
        if client_id != CLIENT_ID:
            self.send_json(400, {"error": "invalid_client"})
            return
        grant_type = form.get("grant_type", ["authorization_code"])[0]
        if grant_type == "refresh_token":
            refresh_token = form.get("refresh_token", [""])[0]
            try:
                result = self.server.provider.refresh(refresh_token)
            except ProviderUnavailable:
                self.send_json(503, {"error": "temporarily_unavailable"})
                return
            if result is None:
                self.send_json(400, {"error": "invalid_grant"})
                return
            id_token, rotated = result
            self.send_json(
                200,
                {
                    "id_token": id_token,
                    "access_token": id_token,
                    "refresh_token": rotated,
                    "token_type": "Bearer",
                    "expires_in": self.server.provider.ttl,
                },
            )
            return
        code = form.get("code", [""])[0]
        verifier = form.get("code_verifier", [""])[0]
        redirect_uri = form.get("redirect_uri", [""])[0]
        result = self.server.provider.token(code, verifier, redirect_uri)
        if result is None:
            self.send_json(400, {"error": "invalid_grant"})
            return
        id_token, refresh_token = result
        self.send_json(
            200,
            {
                "id_token": id_token,
                "access_token": id_token,
                "refresh_token": refresh_token,
                "token_type": "Bearer",
                "expires_in": self.server.provider.ttl,
            },
        )


class AuthServer(ThreadingHTTPServer):
    def __init__(self, address: tuple[str, int], provider: Provider) -> None:
        super().__init__(address, Handler)
        self.provider = provider


def serve(port: int, key_file: str | None) -> None:
    provider = Provider(port)
    if key_file:
        pem = provider.key.private_bytes(
            serialization.Encoding.PEM,
            serialization.PrivateFormat.PKCS8,
            serialization.NoEncryption(),
        )
        with open(key_file, "wb") as handle:
            handle.write(pem)
        import os

        os.chmod(key_file, 0o600)
    server = AuthServer(("127.0.0.1", port), provider)
    print(f"e2e-auth: issuer {provider.issuer}", flush=True)
    server.serve_forever()


def main() -> None:
    parser = argparse.ArgumentParser()
    sub = parser.add_subparsers(dest="command", required=True)

    serve_cmd = sub.add_parser("serve")
    serve_cmd.add_argument("--port", type=int, default=9099)
    serve_cmd.add_argument("--key-file")

    token_cmd = sub.add_parser("token")
    token_cmd.add_argument("--issuer", required=True)
    token_cmd.add_argument("--audience", required=True)
    token_cmd.add_argument("--subject", required=True)
    token_cmd.add_argument("--role", action="append", default=[])
    token_cmd.add_argument("--corpus", action="append", default=[])
    token_cmd.add_argument("--ttl", type=int, default=3600)
    token_cmd.add_argument("--key-file")

    args = parser.parse_args()
    if args.command == "serve":
        serve(args.port, args.key_file)
    else:
        if args.key_file:
            with open(args.key_file, "rb") as handle:
                key = serialization.load_pem_private_key(handle.read(), password=None)
        else:
            key = generate_key()
        print(
            make_token(
                key,
                args.issuer,
                args.audience,
                args.subject,
                args.role,
                args.corpus,
                args.ttl,
            )
        )


if __name__ == "__main__":
    main()
