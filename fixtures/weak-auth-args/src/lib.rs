#![no_std]

use soroban_sdk::{contract, contractimpl, Address, Env, IntoVal};

#[contract]
pub struct ArgsContract;

#[contractimpl]
impl ArgsContract {
    pub fn authorize(env: Env, admin: Address, recipient: Address) -> Address {
        admin.require_auth_for_args((recipient.clone(),).into_val(&env));
        recipient
    }
}

#[cfg(test)]
mod test {
    extern crate std;
    use super::*;
    use soroban_sdk::testutils::Address as _;

    #[test]
    fn checks_only_the_functional_result() {
        let env = Env::default();
        env.mock_all_auths();
        let contract_id = env.register(ArgsContract, ());
        let client = ArgsContractClient::new(&env, &contract_id);
        let admin = Address::generate(&env);
        let recipient = Address::generate(&env);
        assert_eq!(client.authorize(&admin, &recipient), recipient);
    }
}
