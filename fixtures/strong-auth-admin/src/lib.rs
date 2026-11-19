#![no_std]

use soroban_sdk::{contract, contractimpl, contracttype, Address, Env};

#[contracttype]
#[derive(Clone)]
pub enum DataKey {
    Admin,
}

#[contract]
pub struct AdminContract;

#[contractimpl]
impl AdminContract {
    pub fn __constructor(env: Env, admin: Address) {
        env.storage().instance().set(&DataKey::Admin, &admin);
    }

    /// Only the currently stored admin may hand over administration.
    pub fn set_admin(env: Env, new_admin: Address) {
        let admin: Address = env.storage().instance().get(&DataKey::Admin).unwrap();
        admin.require_auth();
        env.storage().instance().set(&DataKey::Admin, &new_admin);
    }

    pub fn admin(env: Env) -> Address {
        env.storage().instance().get(&DataKey::Admin).unwrap()
    }
}

#[cfg(test)]
mod test {
    extern crate std;

    use super::*;
    use soroban_sdk::{
        testutils::{Address as _, AuthorizedFunction, AuthorizedInvocation},
        IntoVal, Symbol,
    };

    #[test]
    fn requires_the_stored_admin_to_authorize_the_handover() {
        let env = Env::default();
        env.mock_all_auths();
        let admin = Address::generate(&env);
        let new_admin = Address::generate(&env);
        let contract_id = env.register(AdminContract, (&admin,));
        let client = AdminContractClient::new(&env, &contract_id);

        client.set_admin(&new_admin);

        // Strong: the authorizing address must be the previously stored admin.
        // `env.auths()` reflects the most recent invocation, so check it first.
        assert_eq!(
            env.auths(),
            std::vec![(
                admin,
                AuthorizedInvocation {
                    function: AuthorizedFunction::Contract((
                        contract_id,
                        Symbol::new(&env, "set_admin"),
                        (new_admin.clone(),).into_val(&env),
                    )),
                    sub_invocations: std::vec![],
                },
            )]
        );
        assert_eq!(client.admin(), new_admin);
    }
}
