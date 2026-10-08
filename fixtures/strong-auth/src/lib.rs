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
    use soroban_sdk::{
        testutils::{Address as _, AuthorizedFunction, AuthorizedInvocation, MockAuth, MockAuthInvoke},
        IntoVal, Symbol,
    };

    #[test]
    fn functional_and_auth_test() {
        let env = Env::default();

        let contract_id = env.register(AdminContract, ());
        let client = AdminContractClient::new(&env, &contract_id);
        let current_admin = Address::generate(&env);
        let new_admin = Address::generate(&env);

        let result = client
            .mock_auths(&[MockAuth {
                address: &current_admin,
                invoke: &MockAuthInvoke {
                    contract: &contract_id,
                    fn_name: "change_admin",
                    args: (current_admin.clone(), new_admin.clone()).into_val(&env),
                    sub_invokes: &[],
                },
            }])
            .change_admin(&current_admin, &new_admin);

        assert_eq!(result, new_admin);

        assert_eq!(
            env.auths(),
            std::vec![(
                current_admin.clone(),
                AuthorizedInvocation {
                    function: AuthorizedFunction::Contract((
                        contract_id,
                        Symbol::new(&env, "change_admin"),
                        (current_admin, new_admin).into_val(&env),
                    )),
                    sub_invocations: std::vec![],
                },
            )]
        );
    }
}
