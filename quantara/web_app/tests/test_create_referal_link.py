"""
Unit tests for the FastAPI referral link endpoint.

Tests include:
- First creation persists a Referal row and returns its code.
- Repeating the call returns the same stable code (idempotent).
- Unknown wallet returns 404, empty wallet returns 400.
- Codes are generated with the cryptographically secure ``secrets`` module.

Uses pytest, unittest.mock for mocking, and FastAPI's TestClient for
testing the API.
"""

import secrets
import uuid

import pytest
from fastapi.testclient import TestClient

from web_app.api.referal import app, generate_random_string
from web_app.db.crud import ReferalDBConnector, UserDBConnector
from web_app.db.models import Referal, User


@pytest.fixture
def client():
    """
    Returns a TestClient for the FastAPI app.
    """
    return TestClient(app)


@pytest.fixture
def mock_user_db(mocker):
    """
    Mocks the UserDBConnector used by the endpoint.
    """
    return mocker.patch.object(UserDBConnector, "get_user_by_wallet_id")


@pytest.fixture
def mock_referal_db(mocker):
    """
    Mocks the ReferalDBConnector used by the endpoint.
    """
    return {
        "get": mocker.patch.object(ReferalDBConnector, "get_referal_by_user_id"),
        "create": mocker.patch.object(
            ReferalDBConnector, "create_referal_with_retry"
        ),
    }


def _existing_user(wallet_id: str = "valid_wallet_id") -> User:
    """Return a User instance with a stable id."""
    return User(id=uuid.uuid4(), wallet_id=wallet_id)


def test_create_referral_link_persists_code_on_first_call(
    client, mock_user_db, mock_referal_db
):
    """Positive Test Case: first call persists a new Referal row."""
    user = _existing_user()
    mock_user_db.return_value = user
    mock_referal_db["get"].return_value = None
    persisted = Referal(id=uuid.uuid4(), user_id=user.id, referal_id="AbCdEf1234567890")
    mock_referal_db["create"].return_value = persisted

    response = client.post("/api/create_referal_link", json={"wallet_id": user.wallet_id})

    assert response.status_code == 200
    body = response.json()
    assert body["wallet_id"] == user.wallet_id
    assert body["referral_code"] == "AbCdEf1234567890"
    assert len(body["referral_code"]) == 16
    mock_referal_db["create"].assert_called_once()
    code = mock_referal_db["create"].call_args.args[1]
    assert isinstance(code(), str)


def test_create_referral_link_is_idempotent(client, mock_user_db, mock_referal_db):
    """Positive Test Case: repeat calls return the same persisted code."""
    user = _existing_user()
    mock_user_db.return_value = user
    existing = Referal(id=uuid.uuid4(), user_id=user.id, referal_id="StableCode123456")
    mock_referal_db["get"].return_value = existing

    response1 = client.post("/api/create_referal_link", json={"wallet_id": user.wallet_id})
    response2 = client.post("/api/create_referal_link", json={"wallet_id": user.wallet_id})

    assert response1.status_code == 200
    assert response2.status_code == 200
    assert response1.json()["referral_code"] == "StableCode123456"
    assert response2.json()["referral_code"] == "StableCode123456"
    # The existing code is returned; no new row is minted.
    mock_referal_db["create"].assert_not_called()


def test_create_referral_link_for_non_existent_user(client, mock_user_db, mock_referal_db):
    """Negative Test Case: unknown wallet returns 404."""
    mock_user_db.return_value = None
    response = client.post(
        "/api/create_referal_link", json={"wallet_id": "non_existent_wallet_id"}
    )

    assert response.status_code == 404
    assert response.json() == {
        "detail": "User with the provided wallet_id does not exist"
    }
    mock_referal_db["get"].assert_not_called()
    mock_referal_db["create"].assert_not_called()


def test_create_referral_link_with_empty_wallet_id(client, mock_referal_db):
    """Negative Test Case: empty wallet ID returns 400."""
    response = client.post("/api/create_referal_link", json={"wallet_id": ""})

    assert response.status_code == 400
    assert response.json() == {"detail": "Wallet ID cannot be empty"}
    mock_referal_db["get"].assert_not_called()


def test_create_referral_link_with_blank_wallet_id(client, mock_referal_db):
    """Negative Test Case: whitespace-only wallet ID returns 400."""
    response = client.post("/api/create_referal_link", json={"wallet_id": "   "})

    assert response.status_code == 400
    assert response.json() == {"detail": "Wallet ID cannot be empty"}


def test_create_referral_link_with_malformed_wallet_id(
    client, mock_user_db, mock_referal_db
):
    """Negative Test Case: unknown malformed wallet ID returns 404."""
    mock_user_db.return_value = None
    response = client.post(
        "/api/create_referal_link", json={"wallet_id": "@@!invalidwallet"}
    )

    assert response.status_code == 404
    assert response.json() == {
        "detail": "User with the provided wallet_id does not exist"
    }


def test_generate_random_string_uses_secrets(mocker):
    """The code generator must use the cryptographically secure secrets module."""
    spy = mocker.spy(secrets, "choice")
    code = generate_random_string()
    assert len(code) == 16
    assert code.isalnum()
    assert spy.call_count == 16
