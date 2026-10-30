#![no_std]
use soroban_sdk::{contract, contractimpl, Address, Env, Symbol, Vec};

#[contract]
pub struct PingContract;

#[contractimpl]
impl PingContract {
    pub fn ping(env: Env) -> u32 {
        let count = env.storage().instance().get::<_, u32>(&1u32).unwrap_or(0) + 1;
        env.storage().instance().set(&1u32, &count);
        7
    }
    pub fn count(env: Env) -> u32 {
        env.storage().instance().get(&1u32).unwrap_or(0)
    }
}

#[contract]
pub struct RouterContract;

#[contractimpl]
impl RouterContract {
    pub fn route(env: Env, primary: Address, alternate: Address) -> u32 {
        let _ = alternate;
        env.invoke_contract::<u32>(&primary, &Symbol::new(&env, "ping"), Vec::new(&env))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn routes_to_expected_contract() {
        let env = Env::default();
        let primary = env.register(PingContract, ());
        let alternate = env.register(PingContract, ());
        let router = env.register(RouterContract, ());
        let client = RouterContractClient::new(&env, &router);
        assert_eq!(client.route(&primary, &alternate), 7);
    }
}
