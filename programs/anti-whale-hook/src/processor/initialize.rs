//! `InitializeExtraAccountMetaList`.
//!
//! Accounts:
//! - `[0]` extra account metas PDA `["extra-account-metas", mint]` (writable)
//! - `[1]` mint
//! - `[2]` mint authority (signer, writable, pays rent)
//! - `[3]` system program
//! - `[4]` config PDA `["config", mint]` (writable)
//!
//! The instruction data after the discriminator (a serialized list of extra
//! accounts) is ignored: the program always writes its own fixed list.

use pinocchio::{
    cpi::{Seed, Signer},
    error::ProgramError,
    AccountView, Address, ProgramResult,
};
use pinocchio_system::create_account_with_minimum_balance_signed;

use crate::{
    log,
    state::{Config, EXTRA_ACCOUNT_METAS, EXTRA_ACCOUNT_METAS_LEN},
    token, HookError, CONFIG_SEED, EXTRA_ACCOUNT_METAS_SEED, MAX_TRANSFER_AMOUNT,
    TOKEN_2022_PROGRAM_ID,
};

pub fn process(program_id: &Address, accounts: &mut [AccountView], _data: &[u8]) -> ProgramResult {
    let [extra_metas, mint, authority, system_program, config, ..] = accounts else {
        return Err(ProgramError::NotEnoughAccountKeys);
    };

    // Mint: an initialized Token-2022 mint whose transfer hook is this program.
    if !mint.owned_by(&TOKEN_2022_PROGRAM_ID) {
        return Err(HookError::InvalidMint.into());
    }
    {
        let mint_data = mint.try_borrow()?;
        let mint_authority = token::mint::authority(&mint_data).ok_or_else(|| {
            log("Mint is not initialized or has no mint authority");
            ProgramError::from(HookError::InvalidMint)
        })?;
        if token::mint::transfer_hook_program_id(&mint_data) != Some(program_id.as_array()) {
            log("Mint transfer hook extension does not point to this program");
            return Err(HookError::MintHookProgramMismatch.into());
        }

        // Mint authority: signs and pays rent.
        if !authority.is_signer() {
            return Err(ProgramError::MissingRequiredSignature);
        }
        if !authority.is_writable() {
            return Err(ProgramError::InvalidAccountData);
        }
        if mint_authority != authority.address().as_array() {
            log("Signer is not the mint authority");
            return Err(HookError::MintAuthorityMismatch.into());
        }
    }

    if system_program.address() != &pinocchio_system::ID {
        return Err(ProgramError::IncorrectProgramId);
    }

    let mint_key = mint.address().as_array();

    let (extra_metas_address, extra_metas_bump) =
        Address::find_program_address(&[EXTRA_ACCOUNT_METAS_SEED, mint_key], program_id);
    if extra_metas.address() != &extra_metas_address {
        return Err(HookError::InvalidExtraAccountMetasAccount.into());
    }
    let (config_address, config_bump) =
        Address::find_program_address(&[CONFIG_SEED, mint_key], program_id);
    if config.address() != &config_address {
        return Err(HookError::InvalidConfigAccount.into());
    }

    if extra_metas.owned_by(program_id) || config.owned_by(program_id) {
        log("Hook accounts are already initialized for this mint");
        return Err(HookError::AlreadyInitialized.into());
    }
    if !extra_metas.is_writable() || !config.is_writable() {
        return Err(ProgramError::InvalidAccountData);
    }

    // Extra account metas PDA.
    let bump = [extra_metas_bump];
    let seeds = [
        Seed::from(EXTRA_ACCOUNT_METAS_SEED),
        Seed::from(mint_key),
        Seed::from(&bump),
    ];
    create_account_with_minimum_balance_signed(
        extra_metas,
        EXTRA_ACCOUNT_METAS_LEN,
        program_id,
        authority,
        None,
        &[Signer::from(&seeds)],
    )?;
    extra_metas
        .try_borrow_mut()?
        .copy_from_slice(&EXTRA_ACCOUNT_METAS);

    // Config PDA holding the anti-whale limit.
    let bump = [config_bump];
    let seeds = [
        Seed::from(CONFIG_SEED),
        Seed::from(mint_key),
        Seed::from(&bump),
    ];
    create_account_with_minimum_balance_signed(
        config,
        Config::LEN,
        program_id,
        authority,
        None,
        &[Signer::from(&seeds)],
    )?;
    Config::write(
        &mut config.try_borrow_mut()?,
        mint_key,
        MAX_TRANSFER_AMOUNT,
        config_bump,
    );

    Ok(())
}
