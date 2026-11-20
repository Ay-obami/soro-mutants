#![no_std]

use soroban_sdk::{contract, contractimpl, token, Address, Env};

#[contract]
pub struct TokenCompileViabilityContract;

#[contractimpl]
impl TokenCompileViabilityContract {
    pub fn do_transfer(env: Env, token: Address, from: Address, to: Address, amount: i128) {
        let client = token::Client::new(&env, &token);
        client.transfer(&from, &to, &amount);
    }

    pub fn do_allowance(env: Env, token: Address, owner: Address, spender: Address) -> i128 {
        let client = token::Client::new(&env, &token);
        client.allowance(&owner, &spender)
    }

    pub fn do_multi_client(
        env: Env,
        token_a: Address,
        token_b: Address,
        from: Address,
        to: Address,
        amount: i128,
    ) {
        let client_a = token::Client::new(&env, &token_a);
        let client_b = token::Client::new(&env, &token_b);
        client_a.transfer(&from, &to, &amount);
        client_b.transfer(&to, &from, &amount);
    }
}
