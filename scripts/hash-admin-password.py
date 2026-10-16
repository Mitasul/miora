#!/usr/bin/env python3
"""Generate MIORA_ADMIN_PASSWORD_HASH without putting a password on the command line."""
import getpass
import hashlib
import secrets
import sys


def encode_password(password: str) -> str:
    raw = password.encode("utf-8")
    if not 12 <= len(raw) <= 1024:
        raise ValueError("Password must contain 12-1024 UTF-8 bytes")
    salt = secrets.token_bytes(32)
    rounds = 600_000
    digest = hashlib.pbkdf2_hmac("sha256", raw, salt, rounds)
    return f"pbkdf2-sha256${rounds}${salt.hex()}${digest.hex()}"


def main() -> int:
    first = getpass.getpass("Administrator password: ")
    second = getpass.getpass("Confirm password: ")
    if first != second:
        print("Passwords do not match", file=sys.stderr)
        return 1
    try:
        print(encode_password(first))
    except ValueError as error:
        print(error, file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
