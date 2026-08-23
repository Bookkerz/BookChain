"""add referred_user_id and one-code-per-user constraint to referal

Revision ID: add_referred_user_to_referal_rev
Revises: add_claimed_at_outbox_rev, b2c3d4e5f6a7
Create Date: 2026-08-23

This revision is a merge point: the two previous heads
(``add_claimed_at_outbox_rev`` and ``b2c3d4e5f6a7``) branched off the same
parent, leaving the migration graph with two heads. Chaining this revision
onto both resolves the branch so ``alembic upgrade head`` applies a single,
deterministic target again.

Schema changes:
- Adds ``referred_user_id`` (nullable FK to ``user.id``) so a referral code
  can record which new user consumed it at signup.
- Enforces one referral code per user with a unique constraint on
  ``user_id``. Existing duplicates (from the era before codes were
  persisted) are deduplicated first, keeping the most recent row per user.

"""
from alembic import op
import sqlalchemy as sa


# revision identifiers, used by Alembic.
revision = "add_referred_user_to_referal_rev"
down_revision = ("add_claimed_at_outbox_rev", "b2c3d4e5f6a7")
branch_labels = None
depends_on = None


def upgrade() -> None:
    conn = op.get_bind()

    # Deduplicate: for each user keep only the row with the latest created_at,
    # then delete the rest so the unique constraint on user_id can be added.
    conn.execute(
        sa.text(
            """
            DELETE FROM referal
            WHERE id IN (
                SELECT id FROM (
                    SELECT id,
                           ROW_NUMBER() OVER (
                               PARTITION BY user_id ORDER BY created_at DESC, id
                           ) AS rn
                    FROM referal
                ) ranked
                WHERE rn > 1
            )
            """
        )
    )

    op.add_column(
        "referal",
        sa.Column("referred_user_id", sa.UUID(), nullable=True),
    )
    op.create_foreign_key(
        None, "referal", "user", ["referred_user_id"], ["id"]
    )
    op.create_index(
        op.f("ix_referal_referred_user_id"),
        "referal",
        ["referred_user_id"],
        unique=False,
    )
    op.create_unique_constraint("uq_referal_user_id", "referal", ["user_id"])


def downgrade() -> None:
    op.drop_constraint("uq_referal_user_id", "referal", type_="unique")
    op.drop_index(op.f("ix_referal_referred_user_id"), table_name="referal")
    # Dropping the column removes its foreign key constraint as well.
    op.drop_column("referal", "referred_user_id")
