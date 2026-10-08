#![no_std]

use soroban_sdk::{contract, contractimpl, symbol_short, Address, Env};

#[contract]
pub struct AdminContract;

#[contractimpl]
impl AdminContract {
    pub fn change_admin(env: Env, current_admin: Address, new_admin: Address) -> Address {
        current_admin.require_auth();
        env.storage().instance().set(&symbol_short!("admin"), &new_admin);
        new_admin
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

        let contract_id = env.register(AdminContract, ());
        let client = AdminContractClient::new(&env, &contract_id);
        let current_admin = Address::generate(&env);
        let new_admin = Address::generate(&env);

        assert_eq!(
            client.change_admin(&current_admin, &new_admin),
            new_admin
        );
    }
}
