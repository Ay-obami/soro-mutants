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
    fn asserts_allowance_state_and_spendable_balance() {
        let env = Env::default();
        // The approval is a nested token call, so allow non-root authorization.
        env.mock_all_auths_allowing_non_root_auth();
        let issuer = Address::generate(&env);
        let token_id = env.register_stellar_asset_contract_v2(issuer).address();
        let owner = Address::generate(&env);
        let spender = Address::generate(&env);
        let recipient = Address::generate(&env);
        token::StellarAssetClient::new(&env, &token_id).mint(&owner, &100);

        let contract_id = env.register(AllowanceManager, ());
        let client = AllowanceManagerClient::new(&env, &contract_id);
        assert_eq!(client.grant(&token_id, &owner, &spender, &50, &1_000), 50);

        // Strong: the allowance must be owner -> spender, never the reverse.
        let token = token::Client::new(&env, &token_id);
        assert_eq!(token.allowance(&owner, &spender), 50);
        assert_eq!(token.allowance(&spender, &owner), 0);

        // And the spender can actually move the owner's funds.
        token.transfer_from(&spender, &owner, &recipient, &30);
        assert_eq!(token.balance(&owner), 70);
        assert_eq!(token.balance(&recipient), 30);
        assert_eq!(token.allowance(&owner, &spender), 20);
    }
}
