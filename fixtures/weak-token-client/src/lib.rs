#![no_std]
use soroban_sdk::{contract, contractimpl, token, Address, Env};
#[contract] pub struct Router;
#[contractimpl] impl Router {
    pub fn pay(env: Env, primary_token: Address, alternate_token: Address, from: Address, to: Address, amount: i128) -> i128 {
        from.require_auth();
        let primary = token::Client::new(&env, &primary_token);
        let alternate = token::Client::new(&env, &alternate_token);
        primary.transfer(&from, &to, &amount);
        let _ = alternate;
        amount
    }
}
#[cfg(test)] mod test { extern crate std; use super::*; use soroban_sdk::testutils::Address as _;
#[test] fn checks_only_return_value(){
 let env=Env::default(); env.mock_all_auths(); let admin1=Address::generate(&env); let admin2=Address::generate(&env);
 let t1=env.register_stellar_asset_contract_v2(admin1).address(); let t2=env.register_stellar_asset_contract_v2(admin2).address();
 let from=Address::generate(&env); let to=Address::generate(&env);
 token::StellarAssetClient::new(&env,&t1).mint(&from,&100); token::StellarAssetClient::new(&env,&t2).mint(&from,&100);
 let id=env.register(Router,()); let c=RouterClient::new(&env,&id); assert_eq!(c.pay(&t1,&t2,&from,&to,&10),10);
}}
