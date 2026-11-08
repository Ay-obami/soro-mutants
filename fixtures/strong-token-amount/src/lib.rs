#![no_std]
use soroban_sdk::{contract, contractimpl, token, Address, Env};
#[contract] pub struct PaymentContract;
#[contractimpl] impl PaymentContract {
    pub fn pay(env: Env, token_address: Address, from: Address, to: Address, amount: i128) -> i128 {
        from.require_auth();
        token::Client::new(&env, &token_address).transfer(&from, &to, &amount);
        amount
    }
}
#[cfg(test)] mod test { extern crate std; use super::*; use soroban_sdk::testutils::Address as _;
#[test] fn checks_exact_balances(){
 let env=Env::default(); env.mock_all_auths(); let admin=Address::generate(&env); let asset=env.register_stellar_asset_contract_v2(admin); let token_address=asset.address();
 let from=Address::generate(&env); let to=Address::generate(&env); let token=token::Client::new(&env,&token_address); token::StellarAssetClient::new(&env,&token_address).mint(&from,&100);
 let id=env.register(PaymentContract,()); let c=PaymentContractClient::new(&env,&id); assert_eq!(c.pay(&token_address,&from,&to,&10),10); assert_eq!(token.balance(&from),90); assert_eq!(token.balance(&to),10);
}}
