#![no_std]
use soroban_sdk::{contract, contractimpl, Address, Env};
#[contract] pub struct Oracle;
#[contractimpl] impl Oracle { pub fn value(_env: Env) -> u32 { 7 } }
#[contract] pub struct Verifier;
#[contractimpl] impl Verifier {
 pub fn verify(env: Env, dependency: Address, expected: u32) -> bool {
  let client=OracleClient::new(&env,&dependency);
  if client.value() != expected { panic!("unexpected return"); }
  true
 }
}
#[cfg(test)] mod test { use super::*; #[test] fn accepts_expected_value(){ let env=Env::default(); let dep=env.register(Oracle,()); let verifier=env.register(Verifier,()); let c=VerifierClient::new(&env,&verifier); assert!(c.verify(&dep,&7)); }}
