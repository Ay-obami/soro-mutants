#![no_std]
use soroban_sdk::{contract, contractimpl, Env};
#[contract] pub struct Store;
#[contractimpl] impl Store {
 pub fn write(env: Env, key: u32, value: u32) { env.storage().persistent().set(&key,&value); }
 pub fn read_persistent(env: Env, key: u32) -> Option<u32> { env.storage().persistent().get(&key) }
}
#[cfg(test)] mod test { use super::*; #[test] fn requires_persistent_class(){ let env=Env::default(); let id=env.register(Store,()); let c=StoreClient::new(&env,&id); c.write(&7,&42); assert_eq!(c.read_persistent(&7),Some(42)); }}
