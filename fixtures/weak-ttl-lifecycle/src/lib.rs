#![no_std]
use soroban_sdk::{contract, contractimpl, Env};

#[contract]
pub struct TtlContract;

#[contractimpl]
impl TtlContract {
    pub fn save(env: Env, value: u32) -> bool {
        env.storage().persistent().set(&1u32, &value);
        env.storage().persistent().extend_ttl(&1u32, 100, 1000);
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use soroban_sdk::testutils::Ledger as _;

    #[test]
    fn ledger_lifecycle_policy() {
        let env = Env::default();
        env.ledger().set_min_persistent_entry_ttl(10);
        env.ledger().set_sequence_number(100);
        let id = env.register(TtlContract, ());
        let client = TtlContractClient::new(&env, &id);
        assert!(client.save(&12));
    }
}
