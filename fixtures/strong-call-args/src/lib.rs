#![no_std]
use soroban_sdk::{contract, contractimpl, Address, Env};
#[contract] pub struct Recorder;
#[contractimpl] impl Recorder {
 pub fn record(env: Env, actor: Address, amount: i128) { env.storage().instance().set(&1u32,&actor); env.storage().instance().set(&2u32,&amount); }
 pub fn actor(env: Env) -> Address { env.storage().instance().get(&1u32).unwrap() }
 pub fn amount(env: Env) -> i128 { env.storage().instance().get(&2u32).unwrap() }
}
#[contract] pub struct Router;
#[contractimpl] impl Router {
 pub fn route(env: Env, dependency: Address, actor: Address, alternate: Address, amount: i128, other: i128) -> i128 {
  let client=RecorderClient::new(&env,&dependency); client.record(&actor,&amount); let _=(alternate,other); amount
 }
}
#[cfg(test)] mod test { use super::*; use soroban_sdk::testutils::Address as _;
#[test] fn checks_stored_arguments(){ let env=Env::default(); let dep=env.register(Recorder,()); let router=env.register(Router,()); let actor=Address::generate(&env); let alt=Address::generate(&env); let c=RouterClient::new(&env,&router); c.route(&dep,&actor,&alt,&7,&9); let r=RecorderClient::new(&env,&dep); assert_eq!(r.actor(),actor); assert_eq!(r.amount(),7); }}
