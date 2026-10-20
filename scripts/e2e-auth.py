#!/usr/bin/env python3
"""Test-only HS256 OIDC issuer/JWKS and token minter for the web E2E suite.

This helper is invoked only by ``scripts/web-e2e.sh``. It is not a production
identity provider: the signing secret is an explicit test secret, and the
tokens are valid only for the local E2E issuer/audience the script configures.
"""

from __future__ import annotations

import argparse
import base64
import hashlib
import hmac
import json
import time
from http.server import BaseHTTPRequestHandler, HTTPServer

DEFAULT_SECRET = "e2e-test-secret-not-for-production"
KID = "e2e"


def b64url(data: bytes) -> str:
    return base64.urlsafe_b64encode(data).rstrip(b"=").decode("ascii")


def make_token(
    issuer: str,
    audience: str,
    subject: str,
    roles: list[str],
    corpora: list[str],
    secret: str,
    ttl: int,
) -> str:
    now = int(time.time())
    header = {"alg": "HS256", "kid": KID, "typ": "JWT"}
    payload = {
        "iss": issuer,
        "aud": audience,
        "sub": subject,
        "iat": now,
        "exp": now + ttl,
        "roles": roles,
        "corpus_grants": corpora,
    }
    signing_input = ".".join(
        [
            b64url(json.dumps(header, separators=(",", ":")).encode("utf-8")),
            b64url(json.dumps(payload, separators=(",", ":")).encode("utf-8")),
        ]
    )
    signature = hmac.new(
        secret.encode("utf-8"), signing_input.encode("ascii"), hashlib.sha256
    ).digest()
    return f"{signing_input}.{b64url(signature)}"


class JwksHandler(BaseHTTPRequestHandler):
    def do_GET(self) -> None:  # noqa: N802 - http.server API
        if self.path not in ("/jwks", "/.well-known/jwks.json"):
            self.send_error(404)
            return
        body = json.dumps(
            {"keys": [{"kid": KID, "alg": "HS256", "k": self.server.secret}]},
            separators=(",", ":"),
        ).encode("utf-8")
        self.send_response(200)
        self.send_header("content-type", "application/json")
        self.send_header("content-length", str(len(body)))
        self.end_headers()
        self.wfile.write(body)

    def log_message(self, *_args: object) -> None:
        # Keep the E2E log readable.
        return


def serve(port: int, secret: str) -> None:
    server = HTTPServer(("127.0.0.1", port), JwksHandler)
    server.secret = secret  # type: ignore[attr-defined]
    print(f"e2e-auth: jwks on http://127.0.0.1:{port}/jwks", flush=True)
    server.serve_forever()


def main() -> None:
    parser = argparse.ArgumentParser()
    sub = parser.add_subparsers(dest="command", required=True)

    serve_cmd = sub.add_parser("serve")
    serve_cmd.add_argument("--port", type=int, default=9099)
    serve_cmd.add_argument("--secret", default=DEFAULT_SECRET)

    token_cmd = sub.add_parser("token")
    token_cmd.add_argument("--issuer", required=True)
    token_cmd.add_argument("--audience", required=True)
    token_cmd.add_argument("--subject", required=True)
    token_cmd.add_argument("--role", action="append", default=[])
    token_cmd.add_argument("--corpus", action="append", default=[])
    token_cmd.add_argument("--secret", default=DEFAULT_SECRET)
    token_cmd.add_argument("--ttl", type=int, default=3600)

    args = parser.parse_args()
    if args.command == "serve":
        serve(args.port, args.secret)
    else:
        print(
            make_token(
                args.issuer,
                args.audience,
                args.subject,
                args.role,
                args.corpus,
                args.secret,
                args.ttl,
            )
        )


if __name__ == "__main__":
    main()
