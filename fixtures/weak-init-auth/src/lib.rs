#![no_std]

use soroban_sdk::{contract, contractimpl, symbol_short, Address, Env};

#[contract]
pub struct InitContract;

#[contractimpl]
impl InitContract {
    pub fn initialize(env: Env, admin: Address, proposer: Address) -> Address {
        admin.require_auth();
        let _ = &proposer;
        env.storage().instance().set(&symbol_short!("admin"), &admin);
        admin
    }
}

#[cfg(test)]
mod test {
    extern crate std;

    use super::*;
    use soroban_sdk::testutils::Address as _;

    #[test]
    fn functional_test_only() {
        let env = Env::default();
        env.mock_all_auths();

        let contract_id = env.register(InitContract, ());
        let client = InitContractClient::new(&env, &contract_id);
        let admin = Address::generate(&env);
        let proposer = Address::generate(&env);

        assert_eq!(client.initialize(&admin, &proposer), admin);
    }
}
