// Deliberately exercises overlapping recognizers, aliases and multi-byte text.
pub use soroban_sdk::{Address as Identity, Env as Host};

fn authenticate(address: &Identity) {
    address.require_auth();
}

pub fn initialize(_env: Host, admin: Identity, backup: Identity) {
    // 🦀 Unicode before recognized expressions.
    let café = "日本語 🧪";
    admin.require_auth();
    authenticate(&admin);
    let _ = (café, backup);
}

mod local_types {
    pub use soroban_sdk::Address as Party;
}

pub fn nested(user: crate::local_types::Party) {
    user.require_auth(); // 🍀 Unicode after recognized expressions.
}

pub use unrelated_library::Address as Unrelated;
pub fn unrelated(user: Unrelated) {
    user.require_auth(); // This must not create a semantic mutant.
}
