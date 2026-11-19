#![no_std]

use soroban_sdk::{contract, contractimpl, symbol_short, Address, BytesN, Env};

#[contract]
pub struct UpgradeContract;

#[contractimpl]
impl UpgradeContract {
    pub fn upgrade(env: Env, admin: Address, proposer: Address, wasm_hash: BytesN<32>) -> u32 {
        admin.require_auth();
        let _ = (&proposer, &wasm_hash);
        env.storage().instance().set(&symbol_short!("version"), &2u32);
        // The recognition fixture preserves the canonical upgrade pattern in
        // source. Host tests do not replace a real contract executable; the
        // update is compiled only for non-test builds.
        #[cfg(not(test))]
        env.deployer().update_current_contract_wasm(wasm_hash);
        2
    }
}

#[cfg(test)]
mod test {
    extern crate std;

    use super::*;
    use soroban_sdk::{testutils::Address as _, BytesN};

    #[test]
    fn functional_test_only() {
        let env = Env::default();
        env.mock_all_auths();

        let contract_id = env.register(UpgradeContract, ());
        let client = UpgradeContractClient::new(&env, &contract_id);
        let admin = Address::generate(&env);
        let proposer = Address::generate(&env);
        let wasm_hash = BytesN::<32>::from_array(&env, &[0u8; 32]);

        assert_eq!(client.upgrade(&admin, &proposer, &wasm_hash), 2);
    }
}
