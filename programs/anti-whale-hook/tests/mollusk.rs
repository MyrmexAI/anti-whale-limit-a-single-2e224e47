//! Single-instruction tests (Mollusk): account validation and CU usage.

mod common;

use anti_whale_hook::{state::Config, HookError, MAX_TRANSFER_AMOUNT};
use common::*;
use mollusk_svm::{program::keyed_account_for_system_program, result::ProgramResult, Mollusk};
use solana_account::Account;
use solana_program_error::ProgramError;
use solana_pubkey::Pubkey;

fn mollusk() -> Mollusk {
    let mut mollusk = Mollusk::default();
    let elf = std::fs::read(PROGRAM_SO).expect("run `cargo build-sbf` first");
    mollusk.add_program_with_loader_and_elf(
        &PROGRAM_ID,
        &mollusk_svm::program::loader_keys::LOADER_V3,
        &elf,
    );
    mollusk
}

fn custom(e: HookError) -> ProgramResult {
    ProgramResult::Failure(ProgramError::Custom(e as u32))
}

fn token_owned(data: Vec<u8>) -> Account {
    Account { lamports: 1_000_000_000, data, owner: TOKEN_2022, executable: false, rent_epoch: 0 }
}

fn system_account(lamports: u64) -> Account {
    Account::new(lamports, 0, &SYSTEM_PROGRAM)
}

struct InitSetup {
    mint: Pubkey,
    authority: Pubkey,
    accounts: Vec<(Pubkey, Account)>,
}

fn init_setup(mint_authority: Option<&Pubkey>, hook_program: Option<&Pubkey>) -> InitSetup {
    let mint = Pubkey::new_unique();
    let authority = Pubkey::new_unique();
    let authority_for_mint = mint_authority.copied().unwrap_or(authority);
    let mut accounts = vec![
        (extra_metas_pda(&mint), system_account(0)),
        (mint, token_owned(mint_data(Some(&authority_for_mint), hook_program, 0))),
        (authority, system_account(10_000_000_000)),
        keyed_account_for_system_program(),
    ];
    for state in init_state_accounts(&mint) {
        accounts.push((state, system_account(0)));
    }
    InitSetup { mint, authority, accounts }
}

#[test]
fn hook_init_manifest_lists_config() {
    let mint = Pubkey::new_unique();
    assert_eq!(init_state_accounts(&mint), vec![config_pda(&mint).0]);
}

#[test]
fn initialize_with_server_account_layout() {
    let mollusk = mollusk();
    let s = init_setup(None, Some(&PROGRAM_ID));
    let ix = initialize_ix(&s.mint, &s.authority);
    // [0] metas PDA (w), [1] mint, [2] authority (s, w), [3] system, [4] config (w)
    assert_eq!(ix.accounts.len(), 5);
    assert!(ix.accounts[0].is_writable && !ix.accounts[0].is_signer);
    assert!(!ix.accounts[1].is_writable);
    assert!(ix.accounts[2].is_writable && ix.accounts[2].is_signer);
    assert_eq!(ix.accounts[3].pubkey, SYSTEM_PROGRAM);
    assert!(ix.accounts[4].is_writable);
    assert_eq!(ix.data.len(), 12);

    let result = mollusk.process_instruction(&ix, &s.accounts);
    assert_eq!(result.program_result, ProgramResult::Success, "{:?}", result.raw_result);
    println!("InitializeExtraAccountMetaList CU: {}", result.compute_units_consumed);

    let metas = result.get_account(&extra_metas_pda(&s.mint)).unwrap();
    assert_eq!(metas.owner, PROGRAM_ID);
    assert_eq!(metas.data, anti_whale_hook::state::EXTRA_ACCOUNT_METAS);
    assert!(metas.lamports >= mollusk.sysvars.rent.minimum_balance(metas.data.len()));

    let (config_key, bump) = config_pda(&s.mint);
    let config = result.get_account(&config_key).unwrap();
    assert_eq!(config.owner, PROGRAM_ID);
    assert_eq!(
        Config::read(&config.data),
        Some((&s.mint.to_bytes(), MAX_TRANSFER_AMOUNT, bump))
    );
    assert_eq!(MAX_TRANSFER_AMOUNT, 20_000_000_000_000);
}

#[test]
fn initialize_ignores_passed_extra_account_list() {
    let mollusk = mollusk();
    let s = init_setup(None, Some(&PROGRAM_ID));
    let mut ix = initialize_ix(&s.mint, &s.authority);
    // A non-empty client list (one fixed-address meta) is ignored.
    ix.data.truncate(8);
    ix.data.extend_from_slice(&1u32.to_le_bytes());
    ix.data.push(0);
    ix.data.extend_from_slice(Pubkey::new_unique().as_ref());
    ix.data.extend_from_slice(&[0, 1]);
    let result = mollusk.process_instruction(&ix, &s.accounts);
    assert_eq!(result.program_result, ProgramResult::Success);
    let metas = result.get_account(&extra_metas_pda(&s.mint)).unwrap();
    assert_eq!(metas.data, anti_whale_hook::state::EXTRA_ACCOUNT_METAS);
}

#[test]
fn initialize_handles_prefunded_pdas() {
    let mollusk = mollusk();
    let mut s = init_setup(None, Some(&PROGRAM_ID));
    // Someone sent lamports to the PDAs beforehand (griefing attempt).
    s.accounts[0].1.lamports = 1;
    s.accounts[4].1.lamports = 1;
    let result = mollusk.process_instruction(&initialize_ix(&s.mint, &s.authority), &s.accounts);
    assert_eq!(result.program_result, ProgramResult::Success, "{:?}", result.raw_result);
    let config = result.get_account(&config_pda(&s.mint).0).unwrap();
    assert_eq!(config.owner, PROGRAM_ID);
    assert!(config.lamports >= mollusk.sysvars.rent.minimum_balance(Config::LEN));
}

#[test]
fn initialize_rejects_non_mint_authority() {
    let mollusk = mollusk();
    let real_authority = Pubkey::new_unique();
    let s = init_setup(Some(&real_authority), Some(&PROGRAM_ID));
    let result = mollusk.process_instruction(&initialize_ix(&s.mint, &s.authority), &s.accounts);
    assert_eq!(result.program_result, custom(HookError::MintAuthorityMismatch));
}

#[test]
fn initialize_requires_signer() {
    let mollusk = mollusk();
    let s = init_setup(None, Some(&PROGRAM_ID));
    let mut ix = initialize_ix(&s.mint, &s.authority);
    ix.accounts[2].is_signer = false;
    let result = mollusk.process_instruction(&ix, &s.accounts);
    assert_eq!(result.program_result, ProgramResult::Failure(ProgramError::MissingRequiredSignature));
}

#[test]
fn initialize_rejects_mint_with_other_hook_program() {
    let mollusk = mollusk();
    let other = Pubkey::new_unique();
    let s = init_setup(None, Some(&other));
    let result = mollusk.process_instruction(&initialize_ix(&s.mint, &s.authority), &s.accounts);
    assert_eq!(result.program_result, custom(HookError::MintHookProgramMismatch));

    let s = init_setup(None, None);
    let result = mollusk.process_instruction(&initialize_ix(&s.mint, &s.authority), &s.accounts);
    assert_eq!(result.program_result, custom(HookError::MintHookProgramMismatch));
}

#[test]
fn initialize_rejects_mint_not_owned_by_token_2022() {
    let mollusk = mollusk();
    let mut s = init_setup(None, Some(&PROGRAM_ID));
    s.accounts[1].1.owner = Pubkey::new_unique();
    let result = mollusk.process_instruction(&initialize_ix(&s.mint, &s.authority), &s.accounts);
    assert_eq!(result.program_result, custom(HookError::InvalidMint));
}

#[test]
fn initialize_rejects_wrong_pdas() {
    let mollusk = mollusk();

    let mut s = init_setup(None, Some(&PROGRAM_ID));
    let mut ix = initialize_ix(&s.mint, &s.authority);
    let fake = Pubkey::new_unique();
    ix.accounts[0].pubkey = fake;
    s.accounts[0].0 = fake;
    let result = mollusk.process_instruction(&ix, &s.accounts);
    assert_eq!(result.program_result, custom(HookError::InvalidExtraAccountMetasAccount));

    let mut s = init_setup(None, Some(&PROGRAM_ID));
    let mut ix = initialize_ix(&s.mint, &s.authority);
    ix.accounts[4].pubkey = fake;
    s.accounts[4].0 = fake;
    let result = mollusk.process_instruction(&ix, &s.accounts);
    assert_eq!(result.program_result, custom(HookError::InvalidConfigAccount));
}

#[test]
fn initialize_requires_state_accounts() {
    let mollusk = mollusk();
    let mut s = init_setup(None, Some(&PROGRAM_ID));
    let mut ix = initialize_ix(&s.mint, &s.authority);
    ix.accounts.truncate(4);
    s.accounts.truncate(4);
    let result = mollusk.process_instruction(&ix, &s.accounts);
    assert_eq!(result.program_result, ProgramResult::Failure(ProgramError::NotEnoughAccountKeys));
}

#[test]
fn initialize_twice_fails() {
    let mollusk = mollusk();
    let s = init_setup(None, Some(&PROGRAM_ID));
    let ix = initialize_ix(&s.mint, &s.authority);
    let first = mollusk.process_instruction(&ix, &s.accounts);
    assert_eq!(first.program_result, ProgramResult::Success);
    let second = mollusk.process_instruction(&ix, &first.resulting_accounts);
    assert_eq!(second.program_result, custom(HookError::AlreadyInitialized));
}

struct ExecSetup {
    mint: Pubkey,
    source: Pubkey,
    destination: Pubkey,
    owner: Pubkey,
    accounts: Vec<(Pubkey, Account)>,
}

/// Accounts as seen by Execute after a successful initialization.
fn exec_setup(transferring: bool) -> ExecSetup {
    let mollusk = mollusk();
    let s = init_setup(None, Some(&PROGRAM_ID));
    let init = mollusk.process_instruction(&initialize_ix(&s.mint, &s.authority), &s.accounts);
    assert_eq!(init.program_result, ProgramResult::Success);

    let owner = Pubkey::new_unique();
    let source = Pubkey::new_unique();
    let destination = Pubkey::new_unique();
    let mint_account = init.get_account(&s.mint).unwrap().clone();
    let metas = init.get_account(&extra_metas_pda(&s.mint)).unwrap().clone();
    let config = init.get_account(&config_pda(&s.mint).0).unwrap().clone();
    let accounts = vec![
        (source, token_owned(token_account_data(&s.mint, &owner, u64::MAX, transferring))),
        (s.mint, mint_account),
        (destination, token_owned(token_account_data(&s.mint, &Pubkey::new_unique(), 0, transferring))),
        (owner, system_account(1_000_000)),
        (extra_metas_pda(&s.mint), metas),
        (config_pda(&s.mint).0, config),
    ];
    ExecSetup { mint: s.mint, source, destination, owner, accounts }
}

fn exec(s: &ExecSetup, amount: u64) -> mollusk_svm::result::InstructionResult {
    mollusk().process_instruction(&execute_ix(&s.source, &s.mint, &s.destination, &s.owner, amount), &s.accounts)
}

#[test]
fn execute_allows_up_to_limit() {
    let s = exec_setup(true);
    for amount in [0, 1, MAX_TRANSFER_AMOUNT - 1, MAX_TRANSFER_AMOUNT] {
        let result = exec(&s, amount);
        assert_eq!(result.program_result, ProgramResult::Success, "amount {amount}");
        println!("Execute CU (amount {amount}): {}", result.compute_units_consumed);
        assert!(result.compute_units_consumed < 2_000);
    }
}

#[test]
fn execute_rejects_above_limit() {
    let s = exec_setup(true);
    for amount in [MAX_TRANSFER_AMOUNT + 1, u64::MAX] {
        assert_eq!(exec(&s, amount).program_result, custom(HookError::TransferAmountExceedsLimit));
    }
}

#[test]
fn execute_rejects_when_not_transferring() {
    let s = exec_setup(false);
    assert_eq!(exec(&s, 1).program_result, custom(HookError::NotTransferring));
}

#[test]
fn execute_rejects_source_without_hook_extension() {
    let mut s = exec_setup(true);
    s.accounts[0].1.data.truncate(165);
    assert_eq!(exec(&s, 1).program_result, custom(HookError::NotTransferring));
}

#[test]
fn execute_rejects_foreign_source_account() {
    let mut s = exec_setup(true);
    s.accounts[0].1.owner = Pubkey::new_unique();
    assert_eq!(exec(&s, 1).program_result, custom(HookError::InvalidTokenAccount));

    let mut s = exec_setup(true);
    s.accounts[0].1.data[0..32].copy_from_slice(Pubkey::new_unique().as_ref());
    assert_eq!(exec(&s, 1).program_result, custom(HookError::InvalidTokenAccount));
}

#[test]
fn execute_fails_without_config_account() {
    let mut s = exec_setup(true);
    s.accounts.pop();
    let mut ix = execute_ix(&s.source, &s.mint, &s.destination, &s.owner, 1);
    ix.accounts.pop();
    let result = mollusk().process_instruction(&ix, &s.accounts);
    assert_eq!(result.program_result, ProgramResult::Failure(ProgramError::NotEnoughAccountKeys));
}

#[test]
fn execute_fails_with_uninitialized_config() {
    let mut s = exec_setup(true);
    // Config PDA never created (system-owned, empty).
    s.accounts[5].1 = system_account(0);
    assert_eq!(exec(&s, 1).program_result, custom(HookError::ConfigNotInitialized));

    // Owned by the program but with garbage data.
    let mut s = exec_setup(true);
    s.accounts[5].1.data = vec![0; Config::LEN];
    assert_eq!(exec(&s, 1).program_result, custom(HookError::ConfigNotInitialized));
}

#[test]
fn execute_rejects_config_of_another_mint() {
    let s = exec_setup(true);
    let other = exec_setup(true);
    let mut accounts = s.accounts.clone();
    // Pass the other mint's config at the config position.
    let other_config = other.accounts[5].clone();
    accounts[5] = other_config.clone();
    let mut ix = execute_ix(&s.source, &s.mint, &s.destination, &s.owner, 1);
    ix.accounts[5].pubkey = other_config.0;
    let result = mollusk().process_instruction(&ix, &accounts);
    assert_eq!(result.program_result, custom(HookError::InvalidConfigAccount));
}

#[test]
fn rejects_unknown_instruction() {
    let s = exec_setup(true);
    let mut ix = execute_ix(&s.source, &s.mint, &s.destination, &s.owner, 1);
    ix.data[0] ^= 0xff;
    let result = mollusk().process_instruction(&ix, &s.accounts);
    assert_eq!(result.program_result, ProgramResult::Failure(ProgramError::InvalidInstructionData));

    ix.data.truncate(4);
    let result = mollusk().process_instruction(&ix, &s.accounts);
    assert_eq!(result.program_result, ProgramResult::Failure(ProgramError::InvalidInstructionData));

    // Execute without the amount.
    let mut ix = execute_ix(&s.source, &s.mint, &s.destination, &s.owner, 1);
    ix.data.truncate(8);
    let result = mollusk().process_instruction(&ix, &s.accounts);
    assert_eq!(result.program_result, ProgramResult::Failure(ProgramError::InvalidInstructionData));
}
