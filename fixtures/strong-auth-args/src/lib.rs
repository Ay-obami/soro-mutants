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
    use soroban_sdk::{
        testutils::{Address as _, AuthorizedFunction, AuthorizedInvocation},
        Symbol,
    };

    #[test]
    fn verifies_the_exact_authorized_argument_set() {
        let env = Env::default();
        env.mock_all_auths();
        let contract_id = env.register(ArgsContract, ());
        let client = ArgsContractClient::new(&env, &contract_id);
        let admin = Address::generate(&env);
        let recipient = Address::generate(&env);
        assert_eq!(client.authorize(&admin, &recipient), recipient);

        assert_eq!(
            env.auths(),
            std::vec![(
                admin,
                AuthorizedInvocation {
                    function: AuthorizedFunction::Contract((
                        contract_id,
                        Symbol::new(&env, "authorize"),
                        (recipient,).into_val(&env),
                    )),
                    sub_invocations: std::vec![],
                },
            )]
        );
    }
}
