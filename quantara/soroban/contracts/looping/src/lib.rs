//! Looping contract - leverage loop engine for the Quantara protocol.
//!
//! This contract allows users to create and manage leveraged positions on
//! the Stellar network by automating the borrow->swap->redeposit loop.
//!
//! Every position is persisted on-chain under a monotonically increasing
//! position id (the `pos_cnt` instance counter). Opening a position records
//! the owner, the collateral and debt assets/amounts and the leverage, so the
//! position can later be identified and closed. Closing is owner-authenticated
//! and idempotent: closing an already-closed position returns a defined
//! [`LoopingError`] instead of trapping the host.
//!
//! The borrow->swap->redeposit mechanics themselves and multi-asset positions
//! are out of scope; this contract only maintains the position record and its
//! lifecycle.

#![no_std]

use soroban_sdk::{
    contract, contracterror, contractimpl, contracttype, panic_with_error, symbol_short, Address,
    Env,
};

use common::auth::assert_caller_auth;

// ---------------------------------------------------------------------------
// Data types
// ---------------------------------------------------------------------------

/// On-chain lifecycle status of a looping position.
///
/// Mirrors the backend `Status` lifecycle (`pending` → `opened` → `closed`):
/// a position is created `Open` and transitions to `Closed` exactly once.
/// `pending` is a backend-only pre-relay state and never exists on-chain.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum PositionStatus {
    /// Position is open and can be closed by its owner.
    Open,
    /// Position has been closed; further closes are rejected.
    Closed,
}

/// On-chain record of a single leveraged position.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Position {
    /// The wallet address that owns the position.
    pub owner: Address,
    /// Collateral token (Stellar Asset Contract id) backing the position.
    pub collateral_asset: Address,
    /// Collateral amount deposited (in base units).
    pub collateral_amount: i128,
    /// Debt token (Stellar Asset Contract id) borrowed for the position.
    pub debt_asset: Address,
    /// Debt amount borrowed (in base units).
    pub debt_amount: i128,
    /// Leverage multiplier (1x-5x, scaled x100).
    pub leverage: u32,
    /// Current lifecycle status of the position.
    pub status: PositionStatus,
}

/// Defined, recoverable errors returned by the looping contract.
///
/// These surface as contract errors (observable through `try_*` invocations)
/// rather than raw host traps, so callers can distinguish failure modes.
#[contracterror]
#[derive(Copy, Clone, Debug, Eq, PartialEq, PartialOrd, Ord)]
#[repr(u32)]
pub enum LoopingError {
    /// Collateral amount must be positive.
    InvalidCollateral = 1,
    /// Debt amount must be non-negative.
    InvalidDebt = 2,
    /// Leverage must be 1x-5x (100-500).
    InvalidLeverage = 3,
    /// No position exists for the given id.
    PositionNotFound = 4,
    /// The position is already closed.
    PositionAlreadyClosed = 5,
    /// The caller is not the position owner.
    NotPositionOwner = 6,
}

// ---------------------------------------------------------------------------
// Contract
// ---------------------------------------------------------------------------

/// Quantara looping contract.
#[contract]
pub struct LoopingContract;

#[contractimpl]
impl LoopingContract {
    /// Open a leveraged position.
    ///
    /// Persists the full position — owner, collateral asset/amount, debt
    /// asset/amount and leverage — under a monotonically increasing position
    /// id, and returns that id. The caller supplies the debt amount it
    /// intends to borrow; the actual borrow->swap->redeposit loop is out of
    /// scope for this contract.
    ///
    /// # Arguments
    /// * `env`               - The Soroban environment.
    /// * `user`              - The wallet address opening the position.
    /// * `collateral_asset`  - The Stellar Asset Contract id of the collateral token.
    /// * `collateral_amount` - The amount of collateral to deposit (in base units, must be > 0).
    /// * `debt_asset`        - The Stellar Asset Contract id of the debt token.
    /// * `debt_amount`       - The amount of debt to borrow (in base units, must be >= 0).
    /// * `leverage`          - The desired leverage multiplier (1x-5x, scaled x100).
    ///
    /// # Returns
    /// The position ID assigned to this new position.
    pub fn open_position(
        env: Env,
        user: Address,
        collateral_asset: Address,
        collateral_amount: i128,
        debt_asset: Address,
        debt_amount: i128,
        leverage: u32,
    ) -> u64 {
        assert_caller_auth(
            &env,
            &user,
            symbol_short!("open_pos"),
            &(
                collateral_asset.clone(),
                collateral_amount,
                debt_asset.clone(),
                debt_amount,
                leverage,
            ),
        );

        if collateral_amount <= 0 {
            panic_with_error!(env, LoopingError::InvalidCollateral);
        }
        if debt_amount < 0 {
            panic_with_error!(env, LoopingError::InvalidDebt);
        }
        if !(100..=500).contains(&leverage) {
            panic_with_error!(env, LoopingError::InvalidLeverage);
        }

        // Monotonically increasing position id. The `pos_cnt` instance
        // counter is preserved from the original contract so ids never
        // repeat and existing deployments keep their numbering.
        let key = symbol_short!("pos_cnt");
        let count: u64 = env.storage().instance().get(&key).unwrap_or(0u64);
        let position_id = count + 1;
        env.storage().instance().set(&key, &position_id);

        let position = Position {
            owner: user,
            collateral_asset,
            collateral_amount,
            debt_asset,
            debt_amount,
            leverage,
            status: PositionStatus::Open,
        };
        env.storage().persistent().set(&position_id, &position);

        position_id
    }

    /// Close an existing leveraged position.
    ///
    /// The caller must be the position owner. The position must exist and be
    /// open; closing an already-closed position is a defined
    /// `PositionAlreadyClosed` error (idempotent), and a non-owner caller is
    /// rejected with `NotPositionOwner` — neither panics the host.
    ///
    /// # Arguments
    /// * `env`         - The Soroban environment.
    /// * `user`        - The wallet address that owns the position.
    /// * `position_id` - The ID of the position to close.
    ///
    /// # Returns
    /// The closed position record (status `Closed`).
    pub fn close_position(env: Env, user: Address, position_id: u64) -> Position {
        assert_caller_auth(&env, &user, symbol_short!("close_pos"), &(position_id,));

        let mut position = Self::load_position(&env, position_id);

        if position.owner != user {
            panic_with_error!(env, LoopingError::NotPositionOwner);
        }
        if position.status == PositionStatus::Closed {
            panic_with_error!(env, LoopingError::PositionAlreadyClosed);
        }

        position.status = PositionStatus::Closed;
        env.storage().persistent().set(&position_id, &position);

        position
    }

    /// Query the on-chain state of a position, or `None` if it does not exist.
    pub fn get_position(env: Env, position_id: u64) -> Option<Position> {
        env.storage().persistent().get(&position_id)
    }

    // ------------------------------------------------------------------
    // Private helpers
    // ------------------------------------------------------------------

    fn load_position(env: &Env, position_id: u64) -> Position {
        env.storage()
            .persistent()
            .get(&position_id)
            .unwrap_or_else(|| panic_with_error!(env, LoopingError::PositionNotFound))
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    extern crate std;

    use super::*;
    use proptest::prelude::*;
    use soroban_sdk::testutils::Address as _;
    use soroban_sdk::testutils::{MockAuth, MockAuthInvoke};
    use soroban_sdk::{symbol_short, vec, Env, IntoVal};

    const COLLATERAL_AMOUNT: i128 = 1_000;
    const DEBT_AMOUNT: i128 = 2_000;
    const LEVERAGE: u32 = 200;

    struct Fixture {
        env: Env,
        contract_id: Address,
        user: Address,
        other: Address,
        collateral_asset: Address,
        debt_asset: Address,
    }

    fn setup() -> Fixture {
        let env = Env::default();
        env.mock_all_auths();

        let user = Address::generate(&env);
        let other = Address::generate(&env);
        let contract_id = env.register(LoopingContract, ());
        // Distinct Stellar Asset Contracts: one collateral, one debt.
        let admin = Address::generate(&env);
        let collateral_asset = env
            .register_stellar_asset_contract_v2(admin.clone())
            .address();
        let debt_asset = env.register_stellar_asset_contract_v2(admin).address();

        Fixture {
            env,
            contract_id,
            user,
            other,
            collateral_asset,
            debt_asset,
        }
    }

    fn open(fx: &Fixture, user: &Address) -> u64 {
        let client = LoopingContractClient::new(&fx.env, &fx.contract_id);
        client.open_position(
            user,
            &fx.collateral_asset,
            &COLLATERAL_AMOUNT,
            &fx.debt_asset,
            &DEBT_AMOUNT,
            &LEVERAGE,
        )
    }

    // ------------------------------------------------------------------
    // Open position
    // ------------------------------------------------------------------

    #[test]
    fn test_open_position_returns_first_id() {
        let fx = setup();
        assert_eq!(open(&fx, &fx.user), 1);
    }

    #[test]
    fn test_open_position_returns_incrementing_ids() {
        let fx = setup();
        assert_eq!(open(&fx, &fx.user), 1);
        assert_eq!(open(&fx, &fx.user), 2);
        assert_eq!(open(&fx, &fx.other), 3);
    }

    /// Opening a position persists the full record — owner, collateral and
    /// debt assets/amounts, leverage and an `Open` status — under the
    /// returned id.
    #[test]
    fn test_open_position_persists_full_position() {
        let fx = setup();
        let id = open(&fx, &fx.user);

        let client = LoopingContractClient::new(&fx.env, &fx.contract_id);
        let position = client.get_position(&id).expect("position must exist");

        assert_eq!(position.owner, fx.user);
        assert_eq!(position.collateral_asset, fx.collateral_asset);
        assert_eq!(position.collateral_amount, COLLATERAL_AMOUNT);
        assert_eq!(position.debt_asset, fx.debt_asset);
        assert_eq!(position.debt_amount, DEBT_AMOUNT);
        assert_eq!(position.leverage, LEVERAGE);
        assert_eq!(position.status, PositionStatus::Open);
    }

    #[test]
    fn test_get_position_returns_none_for_missing_id() {
        let fx = setup();
        let client = LoopingContractClient::new(&fx.env, &fx.contract_id);
        assert_eq!(client.get_position(&99u64), None);
    }

    #[test]
    fn test_open_position_rejects_zero_collateral() {
        let fx = setup();
        let client = LoopingContractClient::new(&fx.env, &fx.contract_id);

        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            client.open_position(
                &fx.user,
                &fx.collateral_asset,
                &0,
                &fx.debt_asset,
                &DEBT_AMOUNT,
                &LEVERAGE,
            );
        }));
        assert!(result.is_err());
    }

    #[test]
    fn test_open_position_rejects_negative_collateral() {
        let fx = setup();
        let client = LoopingContractClient::new(&fx.env, &fx.contract_id);

        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            client.open_position(
                &fx.user,
                &fx.collateral_asset,
                &-1,
                &fx.debt_asset,
                &DEBT_AMOUNT,
                &LEVERAGE,
            );
        }));
        assert!(result.is_err());
    }

    #[test]
    fn test_open_position_rejects_negative_debt() {
        let fx = setup();
        let client = LoopingContractClient::new(&fx.env, &fx.contract_id);

        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            client.open_position(
                &fx.user,
                &fx.collateral_asset,
                &COLLATERAL_AMOUNT,
                &fx.debt_asset,
                &-1,
                &LEVERAGE,
            );
        }));
        assert!(result.is_err());
    }

    #[test]
    fn test_open_position_rejects_leverage_below_100() {
        let fx = setup();
        let client = LoopingContractClient::new(&fx.env, &fx.contract_id);

        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            client.open_position(
                &fx.user,
                &fx.collateral_asset,
                &COLLATERAL_AMOUNT,
                &fx.debt_asset,
                &DEBT_AMOUNT,
                &99,
            );
        }));
        assert!(result.is_err());
    }

    #[test]
    fn test_open_position_rejects_leverage_above_500() {
        let fx = setup();
        let client = LoopingContractClient::new(&fx.env, &fx.contract_id);

        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            client.open_position(
                &fx.user,
                &fx.collateral_asset,
                &COLLATERAL_AMOUNT,
                &fx.debt_asset,
                &DEBT_AMOUNT,
                &501,
            );
        }));
        assert!(result.is_err());
    }

    #[test]
    fn test_open_position_accepts_leverage_boundaries() {
        let fx = setup();
        let client = LoopingContractClient::new(&fx.env, &fx.contract_id);

        assert_eq!(
            client.open_position(
                &fx.user,
                &fx.collateral_asset,
                &COLLATERAL_AMOUNT,
                &fx.debt_asset,
                &DEBT_AMOUNT,
                &100
            ),
            1
        );
        assert_eq!(
            client.open_position(
                &fx.user,
                &fx.collateral_asset,
                &COLLATERAL_AMOUNT,
                &fx.debt_asset,
                &DEBT_AMOUNT,
                &500
            ),
            2
        );
    }

    /// A 1x position borrows nothing (debt of 0) and is still persisted.
    #[test]
    fn test_open_position_accepts_zero_debt() {
        let fx = setup();
        let client = LoopingContractClient::new(&fx.env, &fx.contract_id);

        let id = client.open_position(
            &fx.user,
            &fx.collateral_asset,
            &COLLATERAL_AMOUNT,
            &fx.debt_asset,
            &0,
            &100,
        );

        let position = client.get_position(&id).unwrap();
        assert_eq!(position.debt_amount, 0);
        assert_eq!(position.status, PositionStatus::Open);
    }

    // ------------------------------------------------------------------
    // Close position
    // ------------------------------------------------------------------

    /// Open → close round-trip: the position flips to `Closed` and the closed
    /// record is returned to the caller.
    #[test]
    fn test_close_position_round_trip() {
        let fx = setup();
        let id = open(&fx, &fx.user);

        let client = LoopingContractClient::new(&fx.env, &fx.contract_id);
        let closed = client.close_position(&fx.user, &id);

        assert_eq!(closed.status, PositionStatus::Closed);
        assert_eq!(closed.owner, fx.user);
        assert_eq!(closed.collateral_amount, COLLATERAL_AMOUNT);

        let stored = client.get_position(&id).unwrap();
        assert_eq!(stored.status, PositionStatus::Closed);
        // Closing must not mutate the recorded amounts.
        assert_eq!(stored.collateral_amount, COLLATERAL_AMOUNT);
        assert_eq!(stored.debt_amount, DEBT_AMOUNT);
        assert_eq!(stored.leverage, LEVERAGE);
    }

    /// Double-close is a defined `PositionAlreadyClosed` contract error, not a
    /// panic, and leaves the record untouched.
    #[test]
    fn test_close_position_rejects_double_close() {
        let fx = setup();
        let id = open(&fx, &fx.user);

        let client = LoopingContractClient::new(&fx.env, &fx.contract_id);
        client.close_position(&fx.user, &id);

        let res = client.try_close_position(&fx.user, &id);
        assert_contract_error(res, LoopingError::PositionAlreadyClosed);

        // The stored record is unchanged: still closed, same amounts.
        let stored = client.get_position(&id).unwrap();
        assert_eq!(stored.status, PositionStatus::Closed);
        assert_eq!(stored.collateral_amount, COLLATERAL_AMOUNT);
        assert_eq!(stored.debt_amount, DEBT_AMOUNT);
    }

    /// A non-owner cannot close a position: defined `NotPositionOwner` error
    /// and the position stays open.
    #[test]
    fn test_close_position_rejects_non_owner() {
        let fx = setup();
        let id = open(&fx, &fx.user);

        let client = LoopingContractClient::new(&fx.env, &fx.contract_id);
        let res = client.try_close_position(&fx.other, &id);
        assert_contract_error(res, LoopingError::NotPositionOwner);

        let stored = client.get_position(&id).unwrap();
        assert_eq!(stored.status, PositionStatus::Open);
    }

    /// Closing a position that was never opened is a defined
    /// `PositionNotFound` error.
    #[test]
    fn test_close_position_rejects_missing_position() {
        let fx = setup();
        let client = LoopingContractClient::new(&fx.env, &fx.contract_id);

        let res = client.try_close_position(&fx.user, &42u64);
        assert_contract_error(res, LoopingError::PositionNotFound);
    }

    /// The owner must authorise the close: without the owner's signature the
    /// call fails at the auth layer regardless of who invokes it.
    #[test]
    fn test_close_position_requires_owner_auth() {
        let env = Env::default();
        let contract_id = env.register(LoopingContract, ());
        let user = Address::generate(&env);
        let collateral_asset = Address::generate(&env);
        let debt_asset = Address::generate(&env);

        // Authorise only the `open_position` call; nothing authorises the
        // subsequent `close_position`.
        // `assert_caller_auth` folds the operation symbol into the authorised
        // args; the caller (`user`) is the principal and is not part of the
        // signed payload.
        let open_invoke = MockAuthInvoke {
            contract: &contract_id,
            fn_name: "open_position",
            args: vec![
                &env,
                symbol_short!("open_pos").to_val(),
                collateral_asset.clone().to_val(),
                COLLATERAL_AMOUNT.into_val(&env),
                debt_asset.clone().to_val(),
                DEBT_AMOUNT.into_val(&env),
                LEVERAGE.into_val(&env),
            ],
            sub_invokes: &[],
        };
        env.mock_auths(&[MockAuth {
            address: &user,
            invoke: &open_invoke,
        }]);

        let client = LoopingContractClient::new(&env, &contract_id);
        let id = client.open_position(
            &user,
            &collateral_asset,
            &COLLATERAL_AMOUNT,
            &debt_asset,
            &DEBT_AMOUNT,
            &LEVERAGE,
        );

        let res = client.try_close_position(&user, &id);
        assert!(
            res.is_err(),
            "close without owner authorisation must be rejected"
        );
        assert_eq!(
            client.get_position(&id).unwrap().status,
            PositionStatus::Open
        );
    }

    // ------------------------------------------------------------------
    // Property-based validation
    // ------------------------------------------------------------------

    // Property: `open_position` succeeds exactly when the inputs are valid
    // (collateral > 0, debt >= 0, leverage in 100..=500) and is rejected with
    // a defined error otherwise.
    proptest! {
        #![proptest_config(ProptestConfig::with_cases(1_000))]

        #[test]
        fn test_open_position_input_validation(
            collateral in any::<i128>(),
            debt in any::<i128>(),
            leverage in any::<u32>(),
        ) {
            let env = Env::default();
            env.mock_all_auths();
            let contract_id = env.register(LoopingContract, ());
            let user = Address::generate(&env);
            let collateral_asset = Address::generate(&env);
            let debt_asset = Address::generate(&env);
            let client = LoopingContractClient::new(&env, &contract_id);

            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                client.open_position(
                    &user,
                    &collateral_asset,
                    &collateral,
                    &debt_asset,
                    &debt,
                    &leverage,
                );
            }));

            let valid = collateral > 0 && debt >= 0 && (100..=500).contains(&leverage);
            assert_eq!(
                result.is_ok(),
                valid,
                "collateral={collateral}, debt={debt}, leverage={leverage}"
            );
        }
    }

    /// Assert that a `try_*` invocation failed with a specific defined
    /// contract error (not a host trap / abort).
    fn assert_contract_error(
        res: Result<
            Result<Position, soroban_sdk::ConversionError>,
            Result<soroban_sdk::Error, soroban_sdk::InvokeError>,
        >,
        expected: LoopingError,
    ) {
        let code = match res {
            Err(Ok(err)) => {
                assert!(
                    err.is_type(soroban_sdk::xdr::ScErrorType::Contract),
                    "expected a defined contract error, got {err:?}"
                );
                err.get_code()
            }
            Err(Err(soroban_sdk::InvokeError::Contract(code))) => code,
            other => panic!("expected defined contract error, got {other:?}"),
        };
        assert_eq!(code, expected as u32, "unexpected contract error code");
    }
}
