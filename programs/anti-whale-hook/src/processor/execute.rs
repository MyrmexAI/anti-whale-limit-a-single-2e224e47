//! `Execute`, invoked by Token-2022 on every transfer of the mint.
//!
//! Accounts:
//! - `[0]` source token account
//! - `[1]` mint
//! - `[2]` destination token account
//! - `[3]` source owner / delegate
//! - `[4]` extra account metas PDA
//! - `[5]` config PDA `["config", mint]` (extra account)
//!
//! Data (after the discriminator): `amount: u64` (little-endian).

use pinocchio::{error::ProgramError, AccountView, Address, ProgramResult};

use crate::{log, state::Config, token, HookError, CONFIG_SEED, TOKEN_2022_PROGRAM_ID};

pub fn process(program_id: &Address, accounts: &mut [AccountView], data: &[u8]) -> ProgramResult {
    let amount = data
        .first_chunk::<8>()
        .map(|bytes| u64::from_le_bytes(*bytes))
        .ok_or(ProgramError::InvalidInstructionData)?;

    let [source, mint, _destination, _owner, _extra_metas, config, ..] = accounts else {
        log("Missing hook accounts (config PDA required)");
        return Err(ProgramError::NotEnoughAccountKeys);
    };
    let mint_key = mint.address().as_array();

    // Only accept calls made by Token-2022 in the middle of a transfer of
    // this mint: the source account's `transferring` flag is set by
    // Token-2022 before invoking the hook and cleared right after.
    if !source.owned_by(&TOKEN_2022_PROGRAM_ID) {
        return Err(HookError::InvalidTokenAccount.into());
    }
    {
        let source_data = source.try_borrow()?;
        if token::account::mint(&source_data) != Some(mint_key) {
            return Err(HookError::InvalidTokenAccount.into());
        }
        if !token::account::is_transferring(&source_data) {
            log("Execute called outside of a Token-2022 transfer");
            return Err(HookError::NotTransferring.into());
        }
    }

    // Config: must exist, be initialized and belong to this mint.
    if !config.owned_by(program_id) {
        log("Config account is not initialized; run InitializeExtraAccountMetaList");
        return Err(HookError::ConfigNotInitialized.into());
    }
    let max_transfer_amount = {
        let config_data = config.try_borrow()?;
        let (config_mint, max, bump) = Config::read(&config_data).ok_or_else(|| {
            log("Config account is not initialized; run InitializeExtraAccountMetaList");
            ProgramError::from(HookError::ConfigNotInitialized)
        })?;
        if config_mint != mint_key
            || config.address()
                != &Address::derive_address(&[CONFIG_SEED, mint_key], Some(bump), program_id)
        {
            return Err(HookError::InvalidConfigAccount.into());
        }
        max
    };

    if amount > max_transfer_amount {
        log("Anti-whale: transfer amount exceeds the per-transfer limit (2% of supply)");
        return Err(HookError::TransferAmountExceedsLimit.into());
    }

    Ok(())
}
