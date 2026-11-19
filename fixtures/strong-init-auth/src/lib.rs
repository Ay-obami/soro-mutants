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
    use soroban_sdk::{
        testutils::{Address as _, AuthorizedFunction, AuthorizedInvocation, MockAuth, MockAuthInvoke},
        IntoVal, Symbol,
    };

    #[test]
    fn functional_and_auth_test() {
        let env = Env::default();

        let contract_id = env.register(InitContract, ());
        let client = InitContractClient::new(&env, &contract_id);
        let admin = Address::generate(&env);
        let proposer = Address::generate(&env);

        let result = client
            .mock_auths(&[MockAuth {
                address: &admin,
                invoke: &MockAuthInvoke {
                    contract: &contract_id,
                    fn_name: "initialize",
                    args: (admin.clone(), proposer.clone()).into_val(&env),
                    sub_invokes: &[],
                },
            }])
            .initialize(&admin, &proposer);

        assert_eq!(result, admin);
        assert_eq!(
            env.auths(),
            std::vec![(
                admin.clone(),
                AuthorizedInvocation {
                    function: AuthorizedFunction::Contract((
                        contract_id,
                        Symbol::new(&env, "initialize"),
                        (admin, proposer).into_val(&env),
                    )),
                    sub_invocations: std::vec![],
                },
            )]
        );
    }
}
