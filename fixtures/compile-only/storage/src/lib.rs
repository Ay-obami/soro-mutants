#![no_std]

use soroban_sdk::{contract, contractimpl, symbol_short, Address, Env};

#[contract]
pub struct StorageCompileViabilityContract;

#[contractimpl]
impl StorageCompileViabilityContract {
    pub fn update_storage(env: Env, _user: Address, val: u32) {
        env.storage().instance().set(&symbol_short!("val"), &val);
        env.storage().temporary().set(&symbol_short!("val"), &val);
        env.storage().persistent().set(&symbol_short!("val"), &val);
    }

    pub fn extend_lifetimes(env: Env) {
        env.storage().instance().extend_ttl(100, 1000);
        env.storage()
            .persistent()
            .extend_ttl(&symbol_short!("val"), 100, 1000);
    }
}
