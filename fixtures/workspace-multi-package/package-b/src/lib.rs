pub struct Storage;

impl Storage {
    pub fn extend_ttl(&self, _threshold: u32, _extend_to: u32) {}
}

pub fn contract_b_action(storage: Storage) {
    storage.extend_ttl(100, 1000);
}
