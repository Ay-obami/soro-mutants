# Benchmark Notes

These results are evidence for the product hypothesis, not security audit findings.

## Controlled fixtures

### Weak auth fixture

Contract behavior:

```rust
current_admin.require_auth();
```

Test behavior:

- uses `mock_all_auths()`;
- checks only the functional result.

Observed:

| Operator | Result |
| --- | --- |
| AUTH-001 remove `current_admin.require_auth()` | SURVIVED |
| AUTH-002 authenticate `new_admin` instead | SURVIVED |

Semantic mutation score: **0%**.

### Strong auth fixture

Same contract behavior, but the test uses explicit `MockAuth` and asserts the resulting `env.auths()` tree.

Observed:

| Operator | Result |
| --- | --- |
| AUTH-001 | KILLED |
| AUTH-002 | KILLED |

Semantic mutation score: **100%**.

This establishes that the engine can discriminate between a functional test and a test that actually enforces authorization semantics.

## Phoenix Protocol

Repository:

`Phoenix-Protocol-Group/phoenix-contracts`

Benchmark commit:

`aa9bfc0`

### Pool: `propose_admin`

Source:

`contracts/pool/src/contract.rs`

Configured test command:

```bash
cargo test -q -p phoenix-pool admin_change
```

Observed:

| Operator | Mutation | Result |
| --- | --- | --- |
| AUTH-001 | remove current-admin authorization | SURVIVED |
| AUTH-002 | authenticate the proposed new admin instead | SURVIVED |
| EVENT-001 | remove old-admin event | SURVIVED |
| EVENT-001 | remove new-admin event | SURVIVED |

### Pool: `accept_admin`

| Operator | Mutation | Result |
| --- | --- | --- |
| AUTH-001 | remove pending-admin authorization | SURVIVED |
| EVENT-001 | remove accepted-admin event | SURVIVED |

### Pool: `revoke_admin_change`

| Operator | Mutation | Result |
| --- | --- | --- |
| AUTH-001 | remove current-admin authorization | SURVIVED |
| EVENT-001 | remove revoke event | SURVIVED |

Interpretation:

The existing admin-change tests exercise successful and failing state transitions under `mock_all_auths()`, but these selected tests do not enforce the authorization/event semantics represented by the mutants.

This is a mutation-testing observation. It is **not** a claim that the deployed contract is exploitable.

### Token: `mint`

Source:

`contracts/token/src/contract.rs`

Configured test command:

```bash
cargo test -q -p soroban-token-contract test
```

Observed:

| Operator | Mutation | Result |
| --- | --- | --- |
| AUTH-001 | remove administrator authorization | KILLED |

Interpretation:

The token test explicitly checks `env.auths()`, so removing the authorization requirement is detected.

### Factory: `query_pools`

Source:

`contracts/factory/src/contract.rs`

Configured test command:

```bash
cargo test -q -p phoenix-factory test_ttl_extensions_with_multiple_pool_queries
```

Observed:

| Operator | Mutation | Result |
| --- | --- | --- |
| TTL-001 | remove instance `extend_ttl()` | KILLED |

Interpretation:

The factory test advances ledger sequence and explicitly checks TTL values. The semantic mutation therefore causes a real test failure.

## Soroswap Core strong control

Repository: `soroswap/core`
Benchmark commit: `6eade00`

The historical checkout's **full token suite** aborts under the current local host/toolchain because two legacy `#[should_panic]` tests trigger non-unwinding panics. The full suite is therefore not used as a benchmark baseline.

A focused existing test, `test::test`, is green and explicitly asserts the exact `env.auths()` tree for token operations, so it provides a valid strong-control baseline for the `transfer()` authorization semantics.

Source:

```text
contracts/token/src/contract.rs
```

Configured test command:

```bash
cargo test -q test::test -- --exact
```

Observed for `transfer()`:

| Operator | Mutation | Result |
| --- | --- | --- |
| AUTH-001 | remove `from.require_auth()` | KILLED |
| AUTH-002 | authenticate `to` instead of `from` | KILLED |

Semantic mutation score for the two focused auth mutants: **100%**.

This is an important positive control against the Phoenix result: the same semantic mutation families survive a functional-only authorization path in Phoenix but are killed when Soroswap explicitly checks the authorization tree.

During discovery, the prototype initially misclassified:

```rust
TokenUtils::new(&e).events().transfer(from, to, amount)
```

as an asset-transfer target. The recognizer was tightened to exclude receivers containing an `.events()` chain, and a regression test now protects against this false positive.

## Stellar official `soroban-examples`

Repository: `stellar/soroban-examples`
Benchmark commit: `03d42aa`

### `single_offer::create`

Configured test command:

```bash
RUSTUP_TOOLCHAIN=1.95.0 cargo test -q
```

The unmodified SDK-28 example passed its baseline test.

| Operator | Mutation | Result |
| --- | --- | --- |
| AUTH-001 | remove seller authorization from `create()` | KILLED |

The official example explicitly checks the authorization tree after creating an offer, so deleting `seller.require_auth()` is detected. This is an external positive control independent of the Phoenix fixtures.


### `single_offer::trade` token direction

Configured test command:

```bash
RUSTUP_TOOLCHAIN=1.95.0 cargo test -q
```

`trade()` performs three direct token transfers: buyer → contract, contract → buyer, and contract → seller. `TOKEN-001` reversed the sender/recipient arguments independently for each transfer.

| Operator | Mutation | Result |
| --- | --- | --- |
| TOKEN-001 | reverse buyer → contract transfer | KILLED |
| TOKEN-001 | reverse contract → buyer transfer | KILLED |
| TOKEN-001 | reverse contract → seller transfer | KILLED |

Semantic mutation score for the three token-direction mutants: **100%**.

The example asserts the resulting token balances and authorization structure, so every reversed value-flow direction is detected. This provides an external positive control for `TOKEN-001`.

## RWA Toolkit Stellar contracts

Repository: `RWA-ToolKit/stellar-rwa-contracts`
Benchmark commit: `a92ad6a`

The workspace uses Soroban SDK 26 and pins `ed25519-dalek = 2.2`, avoiding the host-testutils incompatibility seen when broad constraints resolve to the 3.x line.

### Asset token: `transfer`

Configured test command:

```bash
RUSTUP_TOOLCHAIN=1.95.0 cargo test -p asset-token test_transfer_requires_only_sender_auth -q
```

The unmodified focused test passed.

| Operator | Mutation | Result |
| --- | --- | --- |
| AUTH-001 | remove `from.require_auth()` | KILLED |
| AUTH-002 | authenticate `to` instead of `from` | KILLED |
| EVENT-001 | remove self-transfer event publication | SURVIVED |
| EVENT-001 | remove normal transfer event publication | SURVIVED |

Semantic mutation score: **50%**.

The focused test proves the sender authorization semantics, so both auth mutants die. It does not assert the documented transfer events, so both event-deletion mutants survive. This is a useful mixed result: strong authorization coverage and weak observable-event coverage in the same path.

## Blend Capital control attempt

A preliminary `blend-capital/blend-contracts` run produced surviving auth mutants in the public `submit()` wrapper when the configured command was only `cargo test -p pool -q`. Those results are **not counted as benchmark evidence** because many package tests call internal `execute_submit()` directly and do not exercise the public wrapper.

The benchmark was tightened to the integration layer, but the **unmodified integration baseline aborts** in the current local environment. Under the project rules, mutation results are invalid when the baseline is not green.

This is an important guardrail: Soro Mutants must not convert a bad test selection or broken baseline into a claimed test-quality finding.

## Current conclusion

The proof of concept has demonstrated:

1. real semantic mutants can survive functional Soroban tests;
2. explicit semantic assertions kill the corresponding mutants;
3. the same real repository can contain both strong and weak semantic coverage;
4. compile/baseline gates prevent invalid benchmark claims;
5. narrow recognizers are necessary to keep semantic mutation useful.
