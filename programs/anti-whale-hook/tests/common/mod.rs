//! Shared helpers: PDA derivation, instruction builders and raw Token-2022
//! account layouts.
#![allow(dead_code)]

use anti_whale_hook::{
    CONFIG_SEED, EXECUTE_DISCRIMINATOR, EXTRA_ACCOUNT_METAS_SEED,
    INITIALIZE_EXTRA_ACCOUNT_METAS_DISCRIMINATOR, TOKEN_2022_PROGRAM_ID,
};
use solana_instruction::{AccountMeta, Instruction};
use solana_pubkey::Pubkey;

pub const PROGRAM_ID: Pubkey = anti_whale_hook::ID;
pub const TOKEN_2022: Pubkey = TOKEN_2022_PROGRAM_ID;
pub const SYSTEM_PROGRAM: Pubkey = Pubkey::new_from_array([0; 32]);
pub const DECIMALS: u8 = 6;

/// Path of the program binary built by `cargo build-sbf`.
pub const PROGRAM_SO: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/target/deploy/anti_whale_hook.so");

/// Mint with the `TransferHook` extension.
pub const MINT_WITH_HOOK_LEN: usize = 165 + 1 + 4 + 64;
/// Token account with the `TransferHookAccount` extension.
pub const TOKEN_ACCOUNT_WITH_HOOK_LEN: usize = 165 + 1 + 4 + 1;

pub fn extra_metas_pda(mint: &Pubkey) -> Pubkey {
    Pubkey::find_program_address(&[EXTRA_ACCOUNT_METAS_SEED, mint.as_ref()], &PROGRAM_ID).0
}

/// Prefixes listed in `hook-init.json`, in order. The file must be exactly
/// `{"initAccounts": ["p1", ...]}` (whitespace aside), as the server expects.
pub fn init_account_prefixes() -> Vec<String> {
    let manifest = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/hook-init.json"))
        .expect("hook-init.json");
    let compact: String = manifest.chars().filter(|c| !c.is_whitespace()).collect();
    let list = compact
        .strip_prefix(r#"{"initAccounts":["#)
        .and_then(|rest| rest.strip_suffix("]}"))
        .unwrap_or_else(|| panic!("hook-init.json must be {{\"initAccounts\": [..]}}, got {manifest}"));
    list.split(',')
        .map(|item| {
            let prefix = item
                .strip_prefix('"')
                .and_then(|p| p.strip_suffix('"'))
                .unwrap_or_else(|| panic!("prefix {item} is not a JSON string"));
            assert!(!prefix.is_empty() && !prefix.contains(['"', '\\']), "bad prefix {item}");
            prefix.to_string()
        })
        .collect()
}

/// Derives the state accounts listed in `hook-init.json`, in order, the way
/// the server does: PDAs of this program with seeds `[prefix, mint]`.
pub fn init_state_accounts(mint: &Pubkey) -> Vec<Pubkey> {
    init_account_prefixes()
        .iter()
        .map(|prefix| Pubkey::find_program_address(&[prefix.as_bytes(), mint.as_ref()], &PROGRAM_ID).0)
        .collect()
}

/// Resolves the extra accounts of `Execute` from the on-chain extra account
/// metas data the way a client does (`spl-tlv-account-resolution`), given the
/// keys of the base Execute accounts `[source, mint, destination, owner,
/// extra metas]` and the instruction data. Only fixed addresses and PDAs of
/// the hook program built from literals, instruction data and account keys
/// are supported: anything else is not derivable from the transfer alone.
pub fn resolve_execute_extra_accounts(metas: &[u8], base: &[Pubkey; 5], ix_data: &[u8]) -> Vec<AccountMeta> {
    assert_eq!(metas[0..8], EXECUTE_DISCRIMINATOR, "TLV entry is not for Execute");
    let value_len = u32::from_le_bytes(metas[8..12].try_into().unwrap()) as usize;
    let value = &metas[12..12 + value_len];
    let count = u32::from_le_bytes(value[0..4].try_into().unwrap()) as usize;
    assert_eq!(value_len, 4 + 35 * count, "PodSlice length");

    let mut keys: Vec<Pubkey> = base.to_vec();
    let mut resolved = Vec::new();
    for entry in value[4..].chunks_exact(35) {
        let config = &entry[1..33];
        let key = match entry[0] {
            0 => Pubkey::try_from(config).unwrap(),
            1 => {
                let mut seeds: Vec<Vec<u8>> = Vec::new();
                let mut i = 0;
                while i < config.len() && config[i] != 0 {
                    match config[i] {
                        1 => {
                            let len = config[i + 1] as usize;
                            seeds.push(config[i + 2..i + 2 + len].to_vec());
                            i += 2 + len;
                        }
                        2 => {
                            let (index, len) = (config[i + 1] as usize, config[i + 2] as usize);
                            seeds.push(ix_data[index..index + len].to_vec());
                            i += 3;
                        }
                        3 => {
                            seeds.push(keys[config[i + 1] as usize].to_bytes().to_vec());
                            i += 2;
                        }
                        other => panic!("seed type {other} is not derivable from the transfer"),
                    }
                }
                let seeds: Vec<&[u8]> = seeds.iter().map(|s| s.as_slice()).collect();
                Pubkey::find_program_address(&seeds, &PROGRAM_ID).0
            }
            other => panic!("extra account meta type {other} is not supported"),
        };
        keys.push(key);
        resolved.push(AccountMeta { pubkey: key, is_signer: entry[33] != 0, is_writable: entry[34] != 0 });
    }
    resolved
}

pub fn config_pda(mint: &Pubkey) -> (Pubkey, u8) {
    Pubkey::find_program_address(&[CONFIG_SEED, mint.as_ref()], &PROGRAM_ID)
}

/// `InitializeExtraAccountMetaList` exactly as the server sends it:
/// discriminator + empty list (u32 0); accounts
/// `[extra metas PDA, mint, mint authority, system program, state PDAs..]`.
pub fn initialize_ix(mint: &Pubkey, authority: &Pubkey) -> Instruction {
    let mut data = INITIALIZE_EXTRA_ACCOUNT_METAS_DISCRIMINATOR.to_vec();
    data.extend_from_slice(&0u32.to_le_bytes());
    let mut accounts = vec![
        AccountMeta::new(extra_metas_pda(mint), false),
        AccountMeta::new_readonly(*mint, false),
        AccountMeta::new(*authority, true),
        AccountMeta::new_readonly(SYSTEM_PROGRAM, false),
    ];
    accounts.extend(init_state_accounts(mint).into_iter().map(|k| AccountMeta::new(k, false)));
    Instruction { program_id: PROGRAM_ID, accounts, data }
}

/// Direct `Execute` call with the accounts Token-2022 would pass.
pub fn execute_ix(source: &Pubkey, mint: &Pubkey, destination: &Pubkey, owner: &Pubkey, amount: u64) -> Instruction {
    let mut data = EXECUTE_DISCRIMINATOR.to_vec();
    data.extend_from_slice(&amount.to_le_bytes());
    Instruction {
        program_id: PROGRAM_ID,
        accounts: vec![
            AccountMeta::new_readonly(*source, false),
            AccountMeta::new_readonly(*mint, false),
            AccountMeta::new_readonly(*destination, false),
            AccountMeta::new_readonly(*owner, false),
            AccountMeta::new_readonly(extra_metas_pda(mint), false),
            AccountMeta::new_readonly(config_pda(mint).0, false),
        ],
        data,
    }
}

// ---- Token-2022 instructions (raw encoding) ----

/// `TransferHookInstruction::Initialize` (mint extension).
pub fn init_transfer_hook_ix(mint: &Pubkey, authority: &Pubkey, hook_program: &Pubkey) -> Instruction {
    let mut data = vec![36, 0];
    data.extend_from_slice(authority.as_ref());
    data.extend_from_slice(hook_program.as_ref());
    Instruction {
        program_id: TOKEN_2022,
        accounts: vec![AccountMeta::new(*mint, false)],
        data,
    }
}

/// `InitializeMint2` without a freeze authority.
pub fn init_mint_ix(mint: &Pubkey, mint_authority: &Pubkey, decimals: u8) -> Instruction {
    let mut data = vec![20, decimals];
    data.extend_from_slice(mint_authority.as_ref());
    data.push(0);
    Instruction {
        program_id: TOKEN_2022,
        accounts: vec![AccountMeta::new(*mint, false)],
        data,
    }
}

/// `InitializeAccount3`.
pub fn init_account_ix(account: &Pubkey, mint: &Pubkey, owner: &Pubkey) -> Instruction {
    let mut data = vec![18];
    data.extend_from_slice(owner.as_ref());
    Instruction {
        program_id: TOKEN_2022,
        accounts: vec![AccountMeta::new(*account, false), AccountMeta::new_readonly(*mint, false)],
        data,
    }
}

/// `MintTo` (does not invoke the transfer hook).
pub fn mint_to_ix(mint: &Pubkey, destination: &Pubkey, authority: &Pubkey, amount: u64) -> Instruction {
    let mut data = vec![7];
    data.extend_from_slice(&amount.to_le_bytes());
    Instruction {
        program_id: TOKEN_2022,
        accounts: vec![
            AccountMeta::new(*mint, false),
            AccountMeta::new(*destination, false),
            AccountMeta::new_readonly(*authority, true),
        ],
        data,
    }
}

/// `Approve`: lets `delegate` move up to `amount` from `source`.
pub fn approve_ix(source: &Pubkey, delegate: &Pubkey, owner: &Pubkey, amount: u64) -> Instruction {
    let mut data = vec![4];
    data.extend_from_slice(&amount.to_le_bytes());
    Instruction {
        program_id: TOKEN_2022,
        accounts: vec![
            AccountMeta::new(*source, false),
            AccountMeta::new_readonly(*delegate, false),
            AccountMeta::new_readonly(*owner, true),
        ],
        data,
    }
}

/// `TransferChecked` with the transfer hook accounts appended, as resolved
/// by clients from the extra account metas: config PDA, hook program,
/// validation account.
pub fn transfer_checked_ix(
    source: &Pubkey,
    mint: &Pubkey,
    destination: &Pubkey,
    authority: &Pubkey,
    amount: u64,
) -> Instruction {
    let mut data = vec![12];
    data.extend_from_slice(&amount.to_le_bytes());
    data.push(DECIMALS);
    Instruction {
        program_id: TOKEN_2022,
        accounts: vec![
            AccountMeta::new(*source, false),
            AccountMeta::new_readonly(*mint, false),
            AccountMeta::new(*destination, false),
            AccountMeta::new_readonly(*authority, true),
            AccountMeta::new_readonly(config_pda(mint).0, false),
            AccountMeta::new_readonly(PROGRAM_ID, false),
            AccountMeta::new_readonly(extra_metas_pda(mint), false),
        ],
        data,
    }
}

// ---- Raw Token-2022 account data (for single-instruction tests) ----

pub fn mint_data(mint_authority: Option<&Pubkey>, hook_program: Option<&Pubkey>, supply: u64) -> Vec<u8> {
    let mut data = vec![0u8; 165];
    if let Some(authority) = mint_authority {
        data[0..4].copy_from_slice(&1u32.to_le_bytes());
        data[4..36].copy_from_slice(authority.as_ref());
    }
    data[36..44].copy_from_slice(&supply.to_le_bytes());
    data[44] = DECIMALS;
    data[45] = 1;
    data.push(1); // AccountType::Mint
    if let Some(program) = hook_program {
        data.extend_from_slice(&14u16.to_le_bytes());
        data.extend_from_slice(&64u16.to_le_bytes());
        data.extend_from_slice(&[0; 32]);
        data.extend_from_slice(program.as_ref());
    }
    data
}

pub fn token_account_data(mint: &Pubkey, owner: &Pubkey, amount: u64, transferring: bool) -> Vec<u8> {
    let mut data = vec![0u8; 165];
    data[0..32].copy_from_slice(mint.as_ref());
    data[32..64].copy_from_slice(owner.as_ref());
    data[64..72].copy_from_slice(&amount.to_le_bytes());
    data[108] = 1; // AccountState::Initialized
    data.push(2); // AccountType::Account
    data.extend_from_slice(&15u16.to_le_bytes());
    data.extend_from_slice(&1u16.to_le_bytes());
    data.push(transferring as u8);
    data
}
