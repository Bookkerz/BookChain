"""
This module contains the database configuration for referral codes.
"""

import uuid
from typing import TypeVar

from sqlalchemy.exc import IntegrityError, SQLAlchemyError

from web_app.db.models import Base, Referal
from web_app.utils.logger import get_logger

from .base import DBConnector

logger = get_logger(__name__)
ModelType = TypeVar("ModelType", bound=Base)

# Number of attempts when minting a code that collides with an existing one.
# The code space (62^16) makes collisions negligible; this is a safety net.
MAX_CODE_GENERATION_ATTEMPTS = 3


class ReferalDBConnector(DBConnector):
    """
    Provides database connection and operations management for the Referal model.
    """

    def get_referal_by_user_id(self, user_id: uuid.UUID) -> Referal | None:
        """
        Retrieves the referral row for a user (the referrer).

        :param user_id: uuid.UUID
        :return: Referal | None
        """
        return self.get_object_by_field(Referal, "user_id", user_id)

    def get_referal_by_code(self, code: str) -> Referal | None:
        """
        Retrieves the referral row that owns the given code.

        :param code: str
        :return: Referal | None
        """
        return self.get_object_by_field(Referal, "referal_id", code)

    def create_referal(self, user_id: uuid.UUID, code: str) -> Referal:
        """
        Persists a new referral row for a user.

        :param user_id: uuid.UUID
        :param code: str
        :return: Referal - the persisted row
        :raises SQLAlchemyError: If the row cannot be written
        """
        referal = Referal(user_id=user_id, referal_id=code)
        return self.write_to_db(referal)

    def record_referral_use(
        self, referal: Referal, referred_user_id: uuid.UUID
    ) -> Referal:
        """
        Records which user signed up using the given referral code.

        The referred user is only set on the first consumption; a code that
        has already been used is left untouched.

        :param referal: Referal - the row that owns the consumed code
        :param referred_user_id: uuid.UUID - the user who signed up with it
        :return: Referal - the updated row
        """
        if referal.referred_user_id is None:
            referal.referred_user_id = referred_user_id
            self.write_to_db(referal)
        return referal

    def create_referal_with_retry(
        self, user_id: uuid.UUID, code_factory
    ) -> Referal:
        """
        Persists a referral row, retrying with a fresh code if the minted
        code collides with an existing one (unique constraint on referal_id).

        :param user_id: uuid.UUID
        :param code_factory: callable returning a new code string
        :return: Referal - the persisted row
        :raises SQLAlchemyError: If the row cannot be written after retries
        """
        last_error = None
        for _ in range(MAX_CODE_GENERATION_ATTEMPTS):
            try:
                return self.create_referal(user_id, code_factory())
            except IntegrityError as e:
                # A code collision is expected and retried with a new code.
                last_error = e
                logger.warning("referal_code_collision_retry", user_id=user_id)
        raise last_error
