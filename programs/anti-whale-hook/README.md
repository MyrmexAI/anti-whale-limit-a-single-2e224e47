# anti-whale-hook

Token-2022 transfer hook (Pinocchio) that caps every transfer of the hooked
mint at **2% of the total supply**: 20,000,000 tokens with 6 decimals =
`20_000_000_000_000` base units (supply 1,000,000,000 tokens). Transfers at or
below the limit pass unchanged; larger ones fail with
`TransferAmountExceedsLimit` (custom error `6000` / `0x1770`). The rule
applies to every transfer: wallet to wallet, and buys/sells through the pool
vaults.

## Instructions

| Instruction | Discriminator |
|---|---|
| `Execute` | `sha256("spl-transfer-hook-interface:execute")[..8]` = `[105, 37, 101, 197, 75, 251, 102, 26]` |
| `InitializeExtraAccountMetaList` | `sha256("spl-transfer-hook-interface:initialize-extra-account-metas")[..8]` = `[43, 34, 13, 49, 167, 88, 235, 235]` |

### InitializeExtraAccountMetaList

| # | Account | Flags |
|---|---|---|
| 0 | extra account metas PDA `["extra-account-metas", mint]` | writable |
| 1 | mint (Token-2022, transfer hook extension = this program) | |
| 2 | mint authority, pays rent | signer, writable |
| 3 | system program | |
| 4 | config PDA `["config", mint]` | writable |

Data: discriminator followed by any list (the server sends `u32 0`). The
passed list is ignored; the program writes its own fixed list. Both PDAs are
created here (Execute has no payer). Lamports already sitting on a PDA do not
block creation. Running it a second time fails with `AlreadyInitialized`.

`hook-init.json` lists the state account prefixes in the order expected at
accounts `[4..]`: `{"initAccounts": ["config"]}`.

### Execute

Accounts as passed by Token-2022: `[0]` source, `[1]` mint, `[2]` destination,
`[3]` owner/delegate, `[4]` extra account metas PDA, `[5]` config PDA.
The only extra account is the config PDA (`["config", mint]` of this program,
read-only), so clients can derive it with nothing but the mint.

Checks:
1. The source is a Token-2022 account of this mint and its
   `TransferHookAccount.transferring` flag is set. This rejects direct calls
   made outside a Token-2022 transfer (`NotTransferring`).
2. The config account exists, is owned by this program, is initialized and is
   the config PDA of this mint. Otherwise `ConfigNotInitialized` /
   `InvalidConfigAccount`. A missing account gives `NotEnoughAccountKeys`.
3. `amount <= config.max_transfer_amount`.

## Config account (`["config", mint]`, 50 bytes)

`[0..8]` discriminator `"awhconfg"`, `[8..40]` mint, `[40..48]` max transfer
amount (u64 LE), `[48]` bump, `[49]` version. The limit is written once at
initialization and has no update instruction.

## Errors

| Code | Name |
|---|---|
| 6000 | `TransferAmountExceedsLimit` |
| 6001 | `NotTransferring` |
| 6002 | `ConfigNotInitialized` |
| 6003 | `InvalidConfigAccount` |
| 6004 | `InvalidExtraAccountMetasAccount` |
| 6005 | `InvalidMint` |
| 6006 | `MintHookProgramMismatch` |
| 6007 | `MintAuthorityMismatch` |
| 6008 | `InvalidTokenAccount` |
| 6009 | `AlreadyInitialized` |

## Integration notes

- Minting (`MintTo`) does not invoke transfer hooks, so minting the full
  supply straight into the pool vault works. Moving more than 20M tokens with
  one *transfer* (seeding a pool from a wallet, or a pool migration that moves
  the whole remaining reserve in one transfer) is rejected. Such moves have to
  be split into transfers of at most 20M tokens.
- The limit is a fixed number of base units and assumes the mint has 6
  decimals.
- `declare_id!` in `src/lib.rs` holds a placeholder; the platform sets the
  real program id.

## Build and test

```sh
cargo build-sbf          # target/deploy/anti_whale_hook.so
cargo test               # unit + Mollusk + LiteSVM (real Token-2022) tests
```

Tests load `target/deploy/anti_whale_hook.so`, so build first.
