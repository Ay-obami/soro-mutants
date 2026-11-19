#![no_std]

use soroban_sdk::{contract, contractimpl, Address, Env};

/// Simple local authorization helper: the supported AUTH-004 shape.
fn require_owner(owner: &Address) {
    owner.require_auth();
}

#[contract]
pub struct Vault;

#[contractimpl]
impl Vault {
    pub fn transfer(env: Env, from: Address, to: Address, amount: i128) -> i128 {
        require_owner(&from);
        let _ = (env, to);
        amount
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
    fn requires_the_sender_to_authorize_through_the_helper() {
        let env = Env::default();
        env.mock_all_auths();
        let contract_id = env.register(Vault, ());
        let client = VaultClient::new(&env, &contract_id);
        let from = Address::generate(&env);
        let to = Address::generate(&env);

        assert_eq!(client.transfer(&from, &to, &25), 25);

        // Strong: exactly `from` must authorize this invocation.
        assert_eq!(
            env.auths(),
            std::vec![(
                from.clone(),
                AuthorizedInvocation {
                    function: AuthorizedFunction::Contract((
                        contract_id,
                        Symbol::new(&env, "transfer"),
                        (from, to, 25_i128).into_val(&env),
                    )),
                    sub_invocations: std::vec![],
                },
            )]
        );
    }
}
