#![no_std]
use soroban_sdk::{contract, contractevent, contractimpl, Address, Env};
#[contractevent] #[derive(Clone, Debug, Eq, PartialEq)]
pub struct TransferNotice { #[topic] pub actor: Address, pub amount: i128 }
#[contract] pub struct EventContract;
#[contractimpl] impl EventContract {
    pub fn record(env: Env, actor: Address, alternate: Address, amount: i128) -> i128 {
        TransferNotice { actor, amount }.publish(&env); let _=alternate; amount
    }
}
#[cfg(test)] mod test { extern crate std; use super::*; use soroban_sdk::{testutils::{Address as _, Events as _}, Event as _};
#[test] fn asserts_exact_topic(){ let env=Env::default(); let id=env.register(EventContract,()); let c=EventContractClient::new(&env,&id); let a=Address::generate(&env); let b=Address::generate(&env); assert_eq!(c.record(&a,&b,&123),123); assert_eq!(env.events().all(), [TransferNotice{actor:a.clone(),amount:123}.to_xdr(&env,&id)]); }}
