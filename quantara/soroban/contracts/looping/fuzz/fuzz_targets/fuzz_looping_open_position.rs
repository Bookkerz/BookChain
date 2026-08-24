//! cargo-fuzz harness for LoopingContract::open_position entry-point.
//!
//! Invariants checked:
//! - Valid inputs (collateral_amount > 0, debt_amount >= 0, leverage 100–500)
//!   must succeed.
//! - Returned position IDs are >= 1.
//!
//! Run:
//! ```bash
//! cargo +nightly fuzz run fuzz_looping_open_position -- -max_total_time=60
//! ```

#![no_main]

use libfuzzer_sys::fuzz_target;
use soroban_sdk::{testutils::Address as _, Address, Env, IntoVal, Symbol};
use looping::LoopingContract;

fuzz_target!(|data: &[u8]| {
    if data.len() < 20 {
        return;
    }
    let collateral_amount = i64::from_le_bytes(data[..8].try_into().unwrap()) as i128;
    let debt_amount = i64::from_le_bytes(data[8..16].try_into().unwrap()) as i128;
    let leverage = u32::from_le_bytes(data[16..20].try_into().unwrap());

    let env = Env::default();
    env.mock_all_auths();
    let contract_id = env.register(LoopingContract, ());
    let user = Address::generate(&env);
    let collateral_asset = Address::generate(&env);
    let debt_asset = Address::generate(&env);

    // try_invoke_contract returns `Result<Result<u64, ContractError>, HostError>`;
    // explicitly specify the host error type so type inference succeeds.
    let result = env.try_invoke_contract::<u64, soroban_sdk::Error>(
        &contract_id,
        &Symbol::new(&env, "open_position"),
        soroban_sdk::vec![
            &env,
            user.to_val(),
            collateral_asset.to_val(),
            collateral_amount.into_val(&env),
            debt_asset.to_val(),
            debt_amount.into_val(&env),
            leverage.into_val(&env),
        ],
    );

    if collateral_amount > 0 && debt_amount >= 0 && (100..=500).contains(&leverage) {
        // Unwrap the outer (host) Err first, then the inner (contract) Err.
        let position_id = result
            .expect("open_position returned a host error")
            .expect("open_position with valid args returned a contract error");
        assert!(position_id >= 1, "position_id must be >= 1, got {position_id}");
    }
    // Invalid inputs may error; no further assertion needed.
});
