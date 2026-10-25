#![no_std]
use soroban_sdk::{contract, contractevent, contractimpl, Address, Env};

#[contractevent]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TransferNotice {
    #[topic]
    pub recipient: Address,
    pub amount: i128,
}

#[contract]
pub struct EventContract;

#[contractimpl]
impl EventContract {
    pub fn record(env: Env, recipient: Address, amount: i128) -> i128 {
        TransferNotice { recipient, amount }.publish(&env);
        amount
    }
}

#[cfg(test)]
mod test {
    extern crate std;
    use super::*;
    use soroban_sdk::testutils::Address as _;

    #[test]
    fn checks_only_the_recorded_return_value() {
        let env = Env::default();
        let contract_id = env.register(EventContract, ());
        let client = EventContractClient::new(&env, &contract_id);
        let recipient = Address::generate(&env);
        assert_eq!(client.record(&recipient, &123), 123);
    }
}
