// The TOKEN-001 sender/recipient swap is a no-op: both arguments are identical.
fn self_transfer(token: TokenClient, owner: Address, amount: i128) {
    token.transfer(&owner, &owner, &amount);
}
