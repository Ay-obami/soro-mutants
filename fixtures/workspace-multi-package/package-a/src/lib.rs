pub struct Address;

impl Address {
    pub fn require_auth(&self) {}
}

pub fn contract_a_action(user: Address) {
    user.require_auth();
}
