import os
import sys
import time
import logging
import uuid

import jwt
from fastapi import HTTPException, Request

LOGGER = logging.getLogger(__name__)

JWT_SECRET = os.getenv("TM_JWT_SECRET", "")
if not JWT_SECRET:
    LOGGER.error("The environment variable TM_JWT_SECRET is not set.")
    sys.exit(-1)
JWT_ALGORITHM = "HS256"
ACCESS_TOKEN_EXPIRATION_SECONDS = 2 * 3600  # 2시간
REFRESH_TOKEN_EXPIRATION_SECONDS = 7 * 24 * 3600  # 7일
ACCESS_COOKIE_NAME = "tm_access_token"
REFRESH_COOKIE_NAME = "tm_refresh_token"

TM_ADMIN_EMAIL = os.getenv("TM_ADMIN_EMAIL", "")
_allowed_raw = os.getenv("TM_ALLOWED_EMAILS", "")
TM_ALLOWED_EMAILS = [e.strip() for e in _allowed_raw.split(",") if e.strip()]


def _get_admin_email() -> str:
    return os.getenv("TM_ADMIN_EMAIL", "")


def _get_allowed_emails() -> list[str]:
    allowed_raw = os.getenv("TM_ALLOWED_EMAILS", "")
    return [email.strip() for email in allowed_raw.split(",") if email.strip()]


def determine_role(email: str) -> str | None:
    if email == _get_admin_email():
        return "admin"
    if email in _get_allowed_emails():
        return "viewer"
    return None


def create_jwt_token(email: str, role: str, name: str = "", picture: str = "") -> str:
    now = int(time.time())
    payload = {"type": "access", "email": email, "role": role, "name": name, "picture": picture, "exp": now + ACCESS_TOKEN_EXPIRATION_SECONDS, "iat": now}
    return jwt.encode(payload, JWT_SECRET, algorithm=JWT_ALGORITHM)


def create_refresh_token(email: str, role: str, name: str = "", picture: str = "", family_id: str | None = None, token_id: str | None = None) -> str:
    now = int(time.time())
    payload = {"type": "refresh", "email": email, "role": role, "name": name, "picture": picture, "fid": family_id or uuid.uuid4().hex, "jti": token_id or uuid.uuid4().hex, "exp": now + REFRESH_TOKEN_EXPIRATION_SECONDS, "iat": now}
    return jwt.encode(payload, JWT_SECRET, algorithm=JWT_ALGORITHM)


def decode_refresh_token(token: str) -> dict:
    try:
        payload = jwt.decode(token, JWT_SECRET, algorithms=[JWT_ALGORITHM])
    except jwt.ExpiredSignatureError as err:
        raise HTTPException(status_code=401, detail="Refresh token expired") from err
    except jwt.InvalidTokenError as err:
        raise HTTPException(status_code=401, detail="Invalid refresh token") from err
    if payload.get("type") != "refresh":
        raise HTTPException(status_code=401, detail="Invalid token type")
    if not payload.get("fid") or not payload.get("jti"):
        raise HTTPException(status_code=401, detail="Refresh token missing required claims. Please log in again.")
    return payload


def _extract_payload(request: Request) -> dict:
    auth_header = request.headers.get("Authorization", "")
    token = auth_header[7:] if auth_header.startswith("Bearer ") else ""
    if not token:
        token = request.cookies.get(ACCESS_COOKIE_NAME, "")
    if not token:
        raise HTTPException(status_code=401, detail="Missing authentication token")
    try:
        payload = jwt.decode(token, JWT_SECRET, algorithms=[JWT_ALGORITHM])
    except jwt.ExpiredSignatureError as err:
        raise HTTPException(status_code=401, detail="Token expired") from err
    except jwt.InvalidTokenError as err:
        raise HTTPException(status_code=401, detail="Invalid token") from err
    if payload.get("type") != "access":
        raise HTTPException(status_code=401, detail="Invalid token type")
    return payload


def _resolve_current_role(payload: dict) -> str:
    email = payload.get("email", "")
    current_role = determine_role(email)
    token_role = payload.get("role")
    if current_role is None or token_role != current_role:
        raise HTTPException(status_code=403, detail="Access denied")
    return current_role


async def require_auth(request: Request) -> dict:
    payload = _extract_payload(request)
    current_role = _resolve_current_role(payload)
    if current_role not in ("admin", "viewer"):
        raise HTTPException(status_code=403, detail="Insufficient permissions")
    payload["role"] = current_role
    return payload


async def require_admin(request: Request) -> dict:
    payload = _extract_payload(request)
    current_role = _resolve_current_role(payload)
    if current_role != "admin":
        raise HTTPException(status_code=403, detail="Admin access required")
    payload["role"] = current_role
    return payload
