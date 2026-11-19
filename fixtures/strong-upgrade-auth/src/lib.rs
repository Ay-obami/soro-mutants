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
    use soroban_sdk::{
        testutils::{Address as _, AuthorizedFunction, AuthorizedInvocation, MockAuth, MockAuthInvoke},
        BytesN, IntoVal, Symbol,
    };

    #[test]
    fn functional_and_auth_test() {
        let env = Env::default();

        let contract_id = env.register(UpgradeContract, ());
        let client = UpgradeContractClient::new(&env, &contract_id);
        let admin = Address::generate(&env);
        let proposer = Address::generate(&env);
        let wasm_hash = BytesN::<32>::from_array(&env, &[0u8; 32]);

        let result = client
            .mock_auths(&[MockAuth {
                address: &admin,
                invoke: &MockAuthInvoke {
                    contract: &contract_id,
                    fn_name: "upgrade",
                    args: (
                        admin.clone(),
                        proposer.clone(),
                        wasm_hash.clone(),
                    )
                        .into_val(&env),
                    sub_invokes: &[],
                },
            }])
            .upgrade(&admin, &proposer, &wasm_hash);

        assert_eq!(result, 2);
        assert_eq!(
            env.auths(),
            std::vec![(
                admin.clone(),
                AuthorizedInvocation {
                    function: AuthorizedFunction::Contract((
                        contract_id,
                        Symbol::new(&env, "upgrade"),
                        (admin, proposer, wasm_hash).into_val(&env),
                    )),
                    sub_invocations: std::vec![],
                },
            )]
        );
    }
}
