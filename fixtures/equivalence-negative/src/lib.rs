// Distinct addresses must retain their directional transfer mutant.
fn transfer(token: TokenClient, owner: Address, recipient: Address, amount: i128) {
    token.transfer(&owner, &recipient, &amount);
}
