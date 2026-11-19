#![no_std]

use soroban_sdk::{contract, contractimpl, token, Address, Env};

#[contract]
pub struct AllowanceManager;

#[contractimpl]
impl AllowanceManager {
    pub fn grant(
        env: Env,
        token_id: Address,
        owner: Address,
        spender: Address,
        amount: i128,
        expiration_ledger: u32,
    ) -> i128 {
        owner.require_auth();
        let client = token::Client::new(&env, &token_id);
        client.approve(&owner, &spender, &amount, &expiration_ledger);
        amount
    }
}

#[cfg(test)]
mod test {
    extern crate std;

    use super::*;
    use soroban_sdk::testutils::Address as _;

    #[test]
    fn checks_only_the_returned_amount() {
        let env = Env::default();
        // The approval is a nested token call, so allow non-root authorization.
        env.mock_all_auths_allowing_non_root_auth();
        let issuer = Address::generate(&env);
        let token_id = env.register_stellar_asset_contract_v2(issuer).address();
        let owner = Address::generate(&env);
        let spender = Address::generate(&env);

        let contract_id = env.register(AllowanceManager, ());
        let client = AllowanceManagerClient::new(&env, &contract_id);

        // Weak: never observes which (owner, spender) pair received the allowance.
        assert_eq!(client.grant(&token_id, &owner, &spender, &50, &1_000), 50);
    }
}
