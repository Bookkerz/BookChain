"""
FastAPI app for generating and consuming referral codes.

Endpoint:
- POST /api/create_referal_link: Returns the stable referral code for a
  user, creating and persisting one the first time it is called. Repeating
  the call returns the same code (one code per user).

Consumption:
- A referral code is recorded at signup: `UserDBConnector.create_user`
  accepts an optional referral_code, and GET /api/check-user forwards it
  when a new user is created, linking the new account to the referrer's
  Referal row.

Dependencies:
- SQLAlchemy: For user lookup and referral persistence in the database.
- FastAPI: For handling API requests.
- secrets and string: For generating referral codes.

Errors:
- 404: If the user with the provided wallet ID does not exist.
- 400: If the wallet ID is empty.
"""

import secrets
import string

from fastapi import APIRouter, FastAPI, HTTPException, Request
from pydantic import BaseModel

from web_app.api.rate_limiter import WRITE_LIMIT, limiter
from web_app.db.crud import ReferalDBConnector, UserDBConnector

router = APIRouter(
    prefix="/api",
    tags=["referral"],
    responses={404: {"description": "Not found"}},
)


class ReferralCreateRequest(BaseModel):
    """
    Request model for creating or retrieving a referral link.
    """

    wallet_id: str


class ReferralResponse(BaseModel):
    """
    Response model.
    """

    wallet_id: str
    referral_code: str


user_db = UserDBConnector()
referal_db = ReferalDBConnector()


def generate_random_string(length=16):
    """
    Generate a cryptographically secure random string of letters and digits.

    Uses the ``secrets`` module (rather than ``random``) so codes cannot be
    predicted by an observer, and the default length matches the
    ``referal_id`` column (String(16)).

    Args:
        length (int): Length of the string (default is 16).

    Returns:
        str: Randomly generated string.
    """

    alphabet = string.ascii_letters + string.digits
    return "".join(secrets.choice(alphabet) for _ in range(length))


@router.post(
    "/create_referal_link",
    response_model=ReferralResponse,
    summary="Create or retrieve a user's referral code",
    description=(
        "Returns the stable referral code for the given wallet. On the first "
        "call the code is generated with a cryptographically secure source "
        "and persisted; subsequent calls return the same code so each user "
        "has exactly one referral code."
    ),
)
@limiter.limit(WRITE_LIMIT)
async def create_referal_link(
    request: Request,
    data: ReferralCreateRequest,
):
    """
    Create a referral link for a user (idempotent).

    Args:
        request: The incoming request (used by the rate limiter).
        data: Request body containing the wallet ID of the user.

    Returns:
        dict: Wallet ID and the user's stable referral code

    Raises:
        HTTPException: If the user is not found in the database
    """
    wallet_id = data.wallet_id

    if not wallet_id or not wallet_id.strip():
        raise HTTPException(status_code=400, detail="Wallet ID cannot be empty")

    user = user_db.get_user_by_wallet_id(wallet_id)
    if not user:
        raise HTTPException(
            status_code=404, detail="User with the provided wallet_id does not exist"
        )

    existing = referal_db.get_referal_by_user_id(user.id)
    if existing:
        return ReferralResponse(wallet_id=wallet_id, referral_code=existing.referal_id)

    referal = referal_db.create_referal_with_retry(user.id, generate_random_string)
    return ReferralResponse(wallet_id=wallet_id, referral_code=referal.referal_id)


# Standalone FastAPI app exposing the referral endpoints. This allows
# the test suite to mount the router in isolation (see
# tests/test_create_referal_link.py) without depending on the full
# application defined in web_app.api.main.
app = FastAPI()
app.include_router(router)
