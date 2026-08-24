"""
This module contains the user database configuration.
"""

import logging
from typing import TypeVar

from sqlalchemy.exc import SQLAlchemyError

from web_app.db.models import Base, Position, Status, TelegramUser, User

from .base import DBConnector
from .referal import ReferalDBConnector
from web_app.utils.logger import get_logger

logger = get_logger(__name__)
ModelType = TypeVar("ModelType", bound=Base)

referal_db = ReferalDBConnector()


class UserDBConnector(DBConnector):
    """
    Provides database connection and operations management for the User model.
    """

    def get_users_for_notifications(self) -> list[tuple[str, str]]:
        """
        Retrieves the contract_address of users with an OPENED position status and
        the telegram_id of Telegram users with notifications enabled.

        :return: List of tuples (contract_address, telegram_id)
        """
        with self.Session() as db:
            try:
                results = (
                    db.query(User.contract_address, TelegramUser.telegram_id)
                    .join(Position, Position.user_id == User.id)
                    .join(TelegramUser, TelegramUser.wallet_id == User.wallet_id)
                    .filter(
                        Position.status == Status.OPENED.value,
                        TelegramUser.is_allowed_notification == True,
                    )
                    .distinct()
                    .all()
                )
                return results
            except SQLAlchemyError as e:
                logger.error("db_get_notification_users_failed", error=str(e))
                return []

    def get_user_by_wallet_id(self, wallet_id: str) -> User | None:
        """
        Retrieves a user by their wallet ID.
        :param wallet_id: str
        :return: User | None
        """
        return self.get_object_by_field(User, "wallet_id", wallet_id)

    def get_contract_address_by_wallet_id(self, wallet_id: str) -> str:
        """
        Retrieves the contract address of a user by their wallet ID.
        :param wallet_id: str
        :return: str
        """
        user = self.get_user_by_wallet_id(wallet_id)
        return user.contract_address if user else None

    def create_user(self, wallet_id: str, referral_code: str | None = None) -> User:
        """
        Creates a new user in the database.

        When a referral_code is provided and matches an existing referral
        row, the new user is recorded as the referred user on that row so
        the referral relationship is attributed at signup. Invalid codes are
        ignored (the signup still succeeds).

        :param wallet_id: str
        :param referral_code: str | None - optional code used at signup
        :return: User
        """
        user = User(wallet_id=wallet_id)
        self.write_to_db(user)

        if referral_code:
            referal = referal_db.get_referal_by_code(referral_code)
            if referal and referal.user_id != user.id:
                referal_db.record_referral_use(referal, user.id)

        return user

    def update_user_contract(self, user: User, contract_address: str) -> None:
        """
        Updates the contract of a user in the database.
        :param user: User
        :param contract_address: str
        :return: None
        """
        user.is_contract_deployed = not user.is_contract_deployed
        user.contract_address = contract_address
        self.write_to_db(user)

    def get_unique_users_count(self) -> int:
        """
        Retrieves the number of unique users in the database.
        :return: The count of unique users.
        """
        with self.Session() as db:
            try:
                # Query to count distinct users based on wallet ID
                unique_users_count = db.query(User.wallet_id).distinct().count()
                return unique_users_count

            except SQLAlchemyError as e:
                logger.error("db_unique_users_count_failed", error=str(e))
                return 0

    def fetch_user_history(self, user_id: int) -> list[dict]:
        """
        Fetches all positions for a user with the specified fields:
        - status
        - created_at
        - start_price
        - amount
        - multiplier

        ### Parameters:
        - `user_id` (int): Unique identifier of the user.

        ### Returns:
        - A list of dictionaries containing position details.
        """
        with self.Session() as db:
            try:
                # Query positions matching the user_id
                positions = (
                    db.query(
                        Position.status,
                        Position.created_at,
                        Position.start_price,
                        Position.amount,
                        Position.multiplier,
                    )
                    .filter(Position.user_id == user_id)
                    .all()
                )

                # Transform the query result into a list of dictionaries
                return [
                    {
                        "status": position.status,
                        "created_at": position.created_at,
                        "start_price": position.start_price,
                        "amount": position.amount,
                        "multiplier": position.multiplier,
                    }
                    for position in positions
                ]

            except SQLAlchemyError as e:
                logger.error("db_fetch_user_history_failed", user_id=user_id, error=str(e))
                return []

    def delete_user_by_wallet_id(self, wallet_id: str) -> None:
        """
        Deletes a user from the database by their wallet ID.
        Rolls back the transaction if the operation fails.

        :param wallet_id: str
        :return: None
        :raises SQLAlchemyError: If the operation fails
        """
        with self.Session() as session:
            try:
                user = session.query(User).filter(User.wallet_id == wallet_id).first()
                if user:
                    session.delete(user)
                    session.commit()
                    logger.info("db_user_deleted", wallet_id=wallet_id)
                else:
                    logger.warning("db_user_not_found", wallet_id=wallet_id)
            except SQLAlchemyError as e:
                session.rollback()
                logger.error("db_delete_user_failed", wallet_id=wallet_id, error=str(e))
                raise e
