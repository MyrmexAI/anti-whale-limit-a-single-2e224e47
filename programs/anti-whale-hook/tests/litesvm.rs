//! End-to-end scenarios against the real Token-2022 program (LiteSVM).
//!
//! The pool is modelled by a vault token account owned by a pool authority:
//! buys are vault -> trader transfers, sells are trader -> vault transfers.

mod common;

use anti_whale_hook::{HookError, MAX_TRANSFER_AMOUNT, TOTAL_SUPPLY};
use common::*;
use litesvm::LiteSVM;
use solana_account::Account;
use solana_instruction::{error::InstructionError, Instruction};
use solana_keypair::Keypair;
use solana_message::Message;
use solana_pubkey::Pubkey;
use solana_signer::Signer;
use solana_transaction::Transaction;
use solana_transaction_error::TransactionError;

struct Env {
    svm: LiteSVM,
    payer: Keypair,
    mint_authority: Keypair,
    mint: Pubkey,
}

impl Env {
    /// Creates a Token-2022 mint (6 decimals) whose transfer hook is this program.
    fn new() -> Self {
        let mut svm = LiteSVM::new();
        svm.add_program_from_file(PROGRAM_ID, PROGRAM_SO).expect("run `cargo build-sbf` first");
        let payer = Keypair::new();
        let mint_authority = Keypair::new();
        svm.airdrop(&payer.pubkey(), 100_000_000_000).unwrap();
        svm.airdrop(&mint_authority.pubkey(), 10_000_000_000).unwrap();

        let mint = Keypair::new();
        let mut env = Env { svm, payer, mint_authority, mint: mint.pubkey() };
        env.create_owned_by_token(&mint, MINT_WITH_HOOK_LEN);
        let authority = env.mint_authority.pubkey();
        env.send(
            &[
                init_transfer_hook_ix(&env.mint, &authority, &PROGRAM_ID),
                init_mint_ix(&env.mint, &authority, DECIMALS),
            ],
            &[],
        )
        .unwrap();
        env
    }

    fn create_owned_by_token(&mut self, account: &Keypair, len: usize) {
        let lamports = self.svm.minimum_balance_for_rent_exemption(len);
        let ix = create_account_ix(&self.payer.pubkey(), &account.pubkey(), lamports, len as u64, &TOKEN_2022);
        self.send(&[ix], &[account]).unwrap();
    }

    fn send(&mut self, ixs: &[Instruction], signers: &[&Keypair]) -> Result<u64, TransactionError> {
        self.svm.expire_blockhash();
        let mut all: Vec<&Keypair> = vec![&self.payer];
        all.extend_from_slice(signers);
        let tx = Transaction::new(
            &all,
            Message::new(ixs, Some(&self.payer.pubkey())),
            self.svm.latest_blockhash(),
        );
        self.svm
            .send_transaction(tx)
            .map(|meta| meta.compute_units_consumed)
            .map_err(|failed| {
                println!("{}", failed.meta.logs.join("\n"));
                failed.err
            })
    }

    fn initialize(&mut self) -> Result<u64, TransactionError> {
        let authority = self.mint_authority.pubkey();
        let ix = initialize_ix(&self.mint, &authority);
        let mint_authority = self.mint_authority.insecure_clone();
        self.send(&[ix], &[&mint_authority])
    }

    fn token_account(&mut self, owner: &Pubkey) -> Pubkey {
        let account = Keypair::new();
        self.create_owned_by_token(&account, TOKEN_ACCOUNT_WITH_HOOK_LEN);
        self.send(&[init_account_ix(&account.pubkey(), &self.mint, owner)], &[]).unwrap();
        account.pubkey()
    }

    fn mint_to(&mut self, destination: &Pubkey, amount: u64) {
        let authority = self.mint_authority.insecure_clone();
        self.send(&[mint_to_ix(&self.mint, destination, &authority.pubkey(), amount)], &[&authority])
            .unwrap();
    }

    fn transfer(&mut self, source: &Pubkey, destination: &Pubkey, owner: &Keypair, amount: u64) -> Result<u64, TransactionError> {
        let ix = transfer_checked_ix(source, &self.mint, destination, &owner.pubkey(), amount);
        self.send(&[ix], &[owner])
    }

    fn balance(&self, account: &Pubkey) -> u64 {
        let data = self.svm.get_account(account).unwrap().data;
        u64::from_le_bytes(data[64..72].try_into().unwrap())
    }
}

fn create_account_ix(from: &Pubkey, to: &Pubkey, lamports: u64, space: u64, owner: &Pubkey) -> Instruction {
    let mut data = 0u32.to_le_bytes().to_vec();
    data.extend_from_slice(&lamports.to_le_bytes());
    data.extend_from_slice(&space.to_le_bytes());
    data.extend_from_slice(owner.as_ref());
    Instruction {
        program_id: SYSTEM_PROGRAM,
        accounts: vec![
            solana_instruction::AccountMeta::new(*from, true),
            solana_instruction::AccountMeta::new(*to, true),
        ],
        data,
    }
}

/// Error of the hook as propagated by Token-2022 (first instruction).
fn hook_error(e: HookError) -> TransactionError {
    TransactionError::InstructionError(0, InstructionError::Custom(e as u32))
}

/// Pool with the whole supply in its vault, hook initialized.
struct Pool {
    env: Env,
    pool_authority: Keypair,
    vault: Pubkey,
}

fn pool() -> Pool {
    let mut env = Env::new();
    env.initialize().unwrap();
    let pool_authority = Keypair::new();
    let vault = env.token_account(&pool_authority.pubkey());
    env.mint_to(&vault, TOTAL_SUPPLY);
    Pool { env, pool_authority, vault }
}

#[test]
fn transfers_fail_before_initialization() {
    let mut env = Env::new();
    let alice = Keypair::new();
    let bob = Keypair::new();
    let a = env.token_account(&alice.pubkey());
    let b = env.token_account(&bob.pubkey());
    env.mint_to(&a, 1_000);
    assert!(env.transfer(&a, &b, &alice, 1).is_err());
    assert_eq!(env.balance(&b), 0);

    // After initialization the same transfer goes through.
    env.initialize().unwrap();
    env.transfer(&a, &b, &alice, 1).unwrap();
    assert_eq!(env.balance(&b), 1);
}

#[test]
fn initialize_creates_state_accounts() {
    let mut env = Env::new();
    let cu = env.initialize().unwrap();
    println!("InitializeExtraAccountMetaList CU (LiteSVM): {cu}");
    let metas: Account = env.svm.get_account(&extra_metas_pda(&env.mint)).unwrap();
    assert_eq!(metas.owner, PROGRAM_ID);
    for state in init_state_accounts(&env.mint) {
        assert_eq!(env.svm.get_account(&state).unwrap().owner, PROGRAM_ID);
    }
    // Second initialization fails.
    assert_eq!(env.initialize(), Err(hook_error(HookError::AlreadyInitialized)));
}

#[test]
fn initialize_requires_mint_authority() {
    let mut env = Env::new();
    let impostor = Keypair::new();
    env.svm.airdrop(&impostor.pubkey(), 1_000_000_000).unwrap();
    let ix = initialize_ix(&env.mint, &impostor.pubkey());
    assert_eq!(env.send(&[ix], &[&impostor]), Err(hook_error(HookError::MintAuthorityMismatch)));
}

#[test]
fn buys_through_pool_respect_limit() {
    let Pool { mut env, pool_authority, vault } = pool();
    let trader = Keypair::new();
    let trader_ata = env.token_account(&trader.pubkey());

    // Buy exactly at the limit: passes unchanged.
    let cu = env.transfer(&vault, &trader_ata, &pool_authority, MAX_TRANSFER_AMOUNT).unwrap();
    println!("TransferChecked + hook CU: {cu}");
    assert_eq!(env.balance(&trader_ata), MAX_TRANSFER_AMOUNT);

    // One base unit above the limit: rejected, balances untouched.
    assert_eq!(
        env.transfer(&vault, &trader_ata, &pool_authority, MAX_TRANSFER_AMOUNT + 1),
        Err(hook_error(HookError::TransferAmountExceedsLimit))
    );
    assert_eq!(env.balance(&trader_ata), MAX_TRANSFER_AMOUNT);
    assert_eq!(env.balance(&vault), TOTAL_SUPPLY - MAX_TRANSFER_AMOUNT);
}

#[test]
fn sells_through_pool_respect_limit() {
    let Pool { mut env, pool_authority, vault } = pool();
    let trader = Keypair::new();
    let trader_ata = env.token_account(&trader.pubkey());
    // Two buys at the limit accumulate more than the limit.
    env.transfer(&vault, &trader_ata, &pool_authority, MAX_TRANSFER_AMOUNT).unwrap();
    env.transfer(&vault, &trader_ata, &pool_authority, MAX_TRANSFER_AMOUNT).unwrap();
    assert_eq!(env.balance(&trader_ata), 2 * MAX_TRANSFER_AMOUNT);

    // Selling the whole bag at once is rejected...
    assert_eq!(
        env.transfer(&trader_ata, &vault, &trader, 2 * MAX_TRANSFER_AMOUNT),
        Err(hook_error(HookError::TransferAmountExceedsLimit))
    );
    // ...selling in chunks at the limit passes.
    env.transfer(&trader_ata, &vault, &trader, MAX_TRANSFER_AMOUNT).unwrap();
    env.transfer(&trader_ata, &vault, &trader, MAX_TRANSFER_AMOUNT).unwrap();
    assert_eq!(env.balance(&trader_ata), 0);
    assert_eq!(env.balance(&vault), TOTAL_SUPPLY);
}

#[test]
fn wallet_transfers_respect_limit() {
    let Pool { mut env, pool_authority, vault } = pool();
    let alice = Keypair::new();
    let bob = Keypair::new();
    let a = env.token_account(&alice.pubkey());
    let b = env.token_account(&bob.pubkey());
    env.transfer(&vault, &a, &pool_authority, MAX_TRANSFER_AMOUNT).unwrap();
    env.transfer(&vault, &a, &pool_authority, MAX_TRANSFER_AMOUNT).unwrap();

    env.transfer(&a, &b, &alice, 12_345).unwrap();
    assert_eq!(env.balance(&b), 12_345);
    assert_eq!(
        env.transfer(&a, &b, &alice, MAX_TRANSFER_AMOUNT + 1),
        Err(hook_error(HookError::TransferAmountExceedsLimit))
    );
    env.transfer(&a, &b, &alice, MAX_TRANSFER_AMOUNT).unwrap();
    assert_eq!(env.balance(&b), MAX_TRANSFER_AMOUNT + 12_345);
}

#[test]
fn several_transfers_in_one_transaction_are_checked_individually() {
    let Pool { mut env, pool_authority, vault } = pool();
    let trader = Keypair::new();
    let trader_ata = env.token_account(&trader.pubkey());
    let ix = transfer_checked_ix(&vault, &env.mint, &trader_ata, &pool_authority.pubkey(), MAX_TRANSFER_AMOUNT);
    // Each transfer is at the limit; the per-transfer rule lets both pass.
    env.send(&[ix.clone(), ix], &[&pool_authority]).unwrap();
    assert_eq!(env.balance(&trader_ata), 2 * MAX_TRANSFER_AMOUNT);
}

#[test]
fn self_transfer_within_limit_passes() {
    let Pool { mut env, pool_authority, vault } = pool();
    let trader = Keypair::new();
    let trader_ata = env.token_account(&trader.pubkey());
    env.transfer(&vault, &trader_ata, &pool_authority, 1_000).unwrap();
    env.transfer(&trader_ata, &trader_ata, &trader, 1_000).unwrap();
    assert_eq!(env.balance(&trader_ata), 1_000);
}

#[test]
fn direct_execute_outside_transfer_is_rejected() {
    let Pool { mut env, pool_authority, vault } = pool();
    let trader = Keypair::new();
    let trader_ata = env.token_account(&trader.pubkey());
    let ix = execute_ix(&vault, &env.mint, &trader_ata, &pool_authority.pubkey(), 1);
    assert_eq!(env.send(&[ix], &[]), Err(hook_error(HookError::NotTransferring)));
}

#[test]
fn client_resolves_extra_accounts_from_chain() {
    let Pool { mut env, pool_authority, vault } = pool();
    let trader = Keypair::new();
    let trader_ata = env.token_account(&trader.pubkey());
    let metas_key = extra_metas_pda(&env.mint);
    let metas = env.svm.get_account(&metas_key).unwrap().data;

    let resolve = |env: &Env, amount: u64| {
        let mut data = anti_whale_hook::EXECUTE_DISCRIMINATOR.to_vec();
        data.extend_from_slice(&amount.to_le_bytes());
        let base = [vault, env.mint, trader_ata, pool_authority.pubkey(), metas_key];
        resolve_execute_extra_accounts(&metas, &base, &data)
    };
    // One read-only, non-signer extra account: the config PDA of the mint.
    let extra = resolve(&env, MAX_TRANSFER_AMOUNT);
    assert_eq!(extra.len(), 1);
    assert_eq!(extra[0].pubkey, config_pda(&env.mint).0);
    assert!(!extra[0].is_signer && !extra[0].is_writable);

    // A transfer built purely from the resolved list works like the
    // hard-coded helper: at the limit passes, above is rejected.
    let build = |env: &Env, amount: u64| {
        let mut ix = transfer_checked_ix(&vault, &env.mint, &trader_ata, &pool_authority.pubkey(), amount);
        ix.accounts.truncate(4);
        ix.accounts.extend(resolve(env, amount));
        ix.accounts.push(solana_instruction::AccountMeta::new_readonly(PROGRAM_ID, false));
        ix.accounts.push(solana_instruction::AccountMeta::new_readonly(metas_key, false));
        ix
    };
    let ix = build(&env, MAX_TRANSFER_AMOUNT);
    env.send(&[ix], &[&pool_authority]).unwrap();
    let ix = build(&env, MAX_TRANSFER_AMOUNT + 1);
    assert_eq!(
        env.send(&[ix], &[&pool_authority]),
        Err(hook_error(HookError::TransferAmountExceedsLimit))
    );
    assert_eq!(env.balance(&trader_ata), MAX_TRANSFER_AMOUNT);
}

#[test]
fn delegate_transfers_respect_limit() {
    let Pool { mut env, pool_authority, vault } = pool();
    let alice = Keypair::new();
    let delegate = Keypair::new();
    let a = env.token_account(&alice.pubkey());
    let b = env.token_account(&Pubkey::new_unique());
    env.transfer(&vault, &a, &pool_authority, MAX_TRANSFER_AMOUNT).unwrap();
    env.transfer(&vault, &a, &pool_authority, MAX_TRANSFER_AMOUNT).unwrap();
    env.send(&[approve_ix(&a, &delegate.pubkey(), &alice.pubkey(), 2 * MAX_TRANSFER_AMOUNT)], &[&alice])
        .unwrap();

    assert_eq!(
        env.transfer(&a, &b, &delegate, MAX_TRANSFER_AMOUNT + 1),
        Err(hook_error(HookError::TransferAmountExceedsLimit))
    );
    env.transfer(&a, &b, &delegate, MAX_TRANSFER_AMOUNT).unwrap();
    assert_eq!(env.balance(&b), MAX_TRANSFER_AMOUNT);
}

#[test]
fn zero_amount_transfer_passes() {
    let Pool { mut env, pool_authority, vault } = pool();
    let trader_ata = env.token_account(&Pubkey::new_unique());
    env.transfer(&vault, &trader_ata, &pool_authority, 0).unwrap();
    assert_eq!(env.balance(&vault), TOTAL_SUPPLY);
}

#[test]
fn initialize_succeeds_with_prefunded_config_pda() {
    let mut env = Env::new();
    // Griefing: lamports sent to the config PDA before initialization.
    let griefer = Keypair::new();
    env.svm.airdrop(&griefer.pubkey(), 1_000_000_000).unwrap();
    let mut data = 2u32.to_le_bytes().to_vec();
    data.extend_from_slice(&env.svm.minimum_balance_for_rent_exemption(0).to_le_bytes());
    let ix = Instruction {
        program_id: SYSTEM_PROGRAM,
        accounts: vec![
            solana_instruction::AccountMeta::new(griefer.pubkey(), true),
            solana_instruction::AccountMeta::new(config_pda(&env.mint).0, false),
        ],
        data,
    };
    env.send(&[ix], &[&griefer]).unwrap();

    env.initialize().unwrap();
    let config = env.svm.get_account(&config_pda(&env.mint).0).unwrap();
    assert_eq!(config.owner, PROGRAM_ID);
    assert!(config.lamports >= env.svm.minimum_balance_for_rent_exemption(config.data.len()));
}

#[test]
fn transfer_fails_if_config_disappears() {
    let Pool { mut env, pool_authority, vault } = pool();
    let trader_ata = env.token_account(&Pubkey::new_unique());
    // Config PDA wiped (not something the program allows; simulates a
    // missing state account): the hook must fail, not skip the check.
    env.svm.set_account(config_pda(&env.mint).0, Account::default()).unwrap();
    assert_eq!(
        env.transfer(&vault, &trader_ata, &pool_authority, 1),
        Err(hook_error(HookError::ConfigNotInitialized))
    );
    assert_eq!(env.balance(&trader_ata), 0);
}

#[test]
fn transfer_with_another_mints_config_fails() {
    let Pool { mut env, pool_authority, vault } = pool();
    let trader_ata = env.token_account(&Pubkey::new_unique());
    // Second hooked mint with its own (initialized) config in the same SVM.
    let other_mint = Keypair::new();
    env.create_owned_by_token(&other_mint, MINT_WITH_HOOK_LEN);
    let authority = env.mint_authority.insecure_clone();
    env.send(
        &[
            init_transfer_hook_ix(&other_mint.pubkey(), &authority.pubkey(), &PROGRAM_ID),
            init_mint_ix(&other_mint.pubkey(), &authority.pubkey(), DECIMALS),
            initialize_ix(&other_mint.pubkey(), &authority.pubkey()),
        ],
        &[&authority],
    )
    .unwrap();

    let mut ix = transfer_checked_ix(&vault, &env.mint, &trader_ata, &pool_authority.pubkey(), 1);
    ix.accounts[4].pubkey = config_pda(&other_mint.pubkey()).0;
    assert!(env.send(&[ix], &[&pool_authority]).is_err());
    assert_eq!(env.balance(&trader_ata), 0);
}

#[test]
fn transfer_without_hook_accounts_fails() {
    let Pool { mut env, pool_authority, vault } = pool();
    let trader = Keypair::new();
    let trader_ata = env.token_account(&trader.pubkey());
    let mut ix = transfer_checked_ix(&vault, &env.mint, &trader_ata, &pool_authority.pubkey(), 1);
    // Drop the config PDA: Token-2022 cannot resolve the extra account.
    ix.accounts.remove(4);
    assert!(env.send(&[ix], &[&pool_authority]).is_err());
    assert_eq!(env.balance(&trader_ata), 0);
}
