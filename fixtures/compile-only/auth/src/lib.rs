#![no_std]

use soroban_sdk::{contract, contractimpl, symbol_short, Address, Env, Val, Vec};

#[contract]
pub struct AuthCompileViabilityContract;

#[contractimpl]
impl AuthCompileViabilityContract {
    pub fn verify_auth_001_and_002(env: Env, from: Address, to: Address) {
        from.require_auth();
        env.storage().instance().set(&symbol_short!("dest"), &to);
    }

    pub fn verify_auth_003(caller: Address, args: Vec<Val>) {
        caller.require_auth_for_args(args);
    }

    pub fn verify_auth_004(caller: Address) {
        Self::local_auth_helper(&caller);
    }

    fn local_auth_helper(caller: &Address) {
        caller.require_auth();
    }

    pub fn verify_auth_005(env: Env, admin: Address, new_admin: Address) {
        let stored_admin: Address = env.storage().instance().get(&symbol_short!("admin")).unwrap();
        stored_admin.require_auth();
        env.storage().instance().set(&symbol_short!("admin"), &new_admin);
    }
}
