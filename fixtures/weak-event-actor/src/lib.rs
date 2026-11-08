#![no_std]
use soroban_sdk::{contract, contractevent, contractimpl, Address, Env};

#[contractevent]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TransferNotice {
    #[topic]
    pub actor: Address,
    pub amount: i128,
}

#[contract]
pub struct EventContract;

#[contractimpl]
impl EventContract {
    pub fn record(env: Env, actor: Address, alternate: Address, amount: i128) -> i128 {
        TransferNotice { actor, amount }.publish(&env);
        let _ = alternate;
        amount
    }
}

#[cfg(test)]
mod test {
    extern crate std;
    use super::*;
    use soroban_sdk::testutils::Address as _;

    #[test]
    fn checks_only_return_value() {
        let env = Env::default();
        let id = env.register(EventContract, ());
        let client = EventContractClient::new(&env, &id);
        let actor = Address::generate(&env);
        let alternate = Address::generate(&env);
        assert_eq!(client.record(&actor, &alternate, &123), 123);
    }
}
