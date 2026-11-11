#![no_std]
use soroban_sdk::{contract, contractimpl, token, Address, Env};
#[contract] pub struct AllowanceContract;
#[contractimpl] impl AllowanceContract {
 pub fn set_allowance(env: Env, token_address: Address, owner: Address, spender: Address, amount: i128, expiration: u32) -> i128 {
  owner.require_auth();
  let client=token::Client::new(&env,&token_address);
  client.approve(&owner,&spender,&amount,&expiration);
  amount
 }
}
#[cfg(test)] mod test { extern crate std; use super::*; use soroban_sdk::testutils::Address as _;
#[test] fn checks_exact_allowance_roles(){ let env=Env::default(); env.mock_all_auths(); let admin=Address::generate(&env); let token_address=env.register_stellar_asset_contract_v2(admin).address(); let owner=Address::generate(&env); let spender=Address::generate(&env); let id=env.register(AllowanceContract,()); let c=AllowanceContractClient::new(&env,&id); c.set_allowance(&token_address,&owner,&spender,&10,&1000); let token=token::Client::new(&env,&token_address); assert_eq!(token.allowance(&owner,&spender),10); assert_eq!(token.allowance(&spender,&owner),0); }}
