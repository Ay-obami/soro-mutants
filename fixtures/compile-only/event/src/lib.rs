#![no_std]

use soroban_sdk::{contract, contractevent, contractimpl, symbol_short, Address, Env};

#[contractevent]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TransferEvent {
    pub from: Address,
    pub to: Address,
    pub amount: i128,
}

#[contract]
pub struct EventCompileViabilityContract;

#[contractimpl]
impl EventCompileViabilityContract {
    pub fn emit_events(env: Env, from: Address, to: Address, amount: i128) {
        env.events()
            .publish((symbol_short!("transfer"), from.clone()), amount);
        TransferEvent { from, to, amount }.publish(&env);
    }
}
