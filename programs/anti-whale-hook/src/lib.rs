//! Anti-whale Token-2022 transfer hook.
//!
//! Every transfer of the hooked mint (wallet-to-wallet as well as buys and
//! sells through the pool vaults) may move at most
//! [`MAX_TRANSFER_AMOUNT`] base units: 2% of the 1,000,000,000 token supply,
//! i.e. 20,000,000 tokens with 6 decimals. Larger transfers fail with
//! [`HookError::TransferAmountExceedsLimit`].
//!
//! Instructions (SPL transfer hook interface):
//! - `InitializeExtraAccountMetaList`: creates the extra account metas PDA and
//!   the per-mint config PDA (`["config", mint]`) holding the limit.
//! - `Execute`: invoked by Token-2022 on every transfer; enforces the limit.
#![no_std]

pub mod error;
pub mod processor;
pub mod state;
pub mod token;

pub use error::HookError;

use pinocchio::{address::declare_id, error::ProgramError, AccountView, Address, ProgramResult};

// Placeholder: the platform sets the real program id before deployment.
declare_id!("4ap8qPKiDmp4HKevVpxcbihdiJypzt1jJSNjK78nzaNG");

/// Token-2022 program id.
pub const TOKEN_2022_PROGRAM_ID: Address =
    Address::from_str_const("TokenzQdBNbLqP5VEhdkAS6EPFLC1PHnBqCXEpPxuEb");

/// Total token supply in base units (1,000,000,000 tokens, 6 decimals).
pub const TOTAL_SUPPLY: u64 = 1_000_000_000_000_000;

/// Largest amount a single transfer may move: 2% of [`TOTAL_SUPPLY`]
/// (20,000,000 tokens = 20,000,000,000,000 base units).
pub const MAX_TRANSFER_AMOUNT: u64 = TOTAL_SUPPLY / 50;

/// First 8 bytes of `sha256("spl-transfer-hook-interface:execute")`.
pub const EXECUTE_DISCRIMINATOR: [u8; 8] = [105, 37, 101, 197, 75, 251, 102, 26];

/// First 8 bytes of
/// `sha256("spl-transfer-hook-interface:initialize-extra-account-metas")`.
pub const INITIALIZE_EXTRA_ACCOUNT_METAS_DISCRIMINATOR: [u8; 8] =
    [43, 34, 13, 49, 167, 88, 235, 235];

/// Seed prefix of the extra account metas PDA (`["extra-account-metas", mint]`).
pub const EXTRA_ACCOUNT_METAS_SEED: &[u8] = b"extra-account-metas";

/// Seed prefix of the config PDA (`["config", mint]`).
pub const CONFIG_SEED: &[u8] = b"config";

#[cfg(not(feature = "no-entrypoint"))]
mod entrypoint {
    use pinocchio::{no_allocator, nostd_panic_handler, program_entrypoint};

    program_entrypoint!(super::process_instruction);
    no_allocator!();
    nostd_panic_handler!();
}

pub fn process_instruction(
    program_id: &Address,
    accounts: &mut [AccountView],
    instruction_data: &[u8],
) -> ProgramResult {
    let (discriminator, data) = instruction_data
        .split_first_chunk::<8>()
        .ok_or(ProgramError::InvalidInstructionData)?;

    match *discriminator {
        EXECUTE_DISCRIMINATOR => processor::execute::process(program_id, accounts, data),
        INITIALIZE_EXTRA_ACCOUNT_METAS_DISCRIMINATOR => {
            processor::initialize::process(program_id, accounts, data)
        }
        _ => Err(ProgramError::InvalidInstructionData),
    }
}

/// Logs a message on-chain; no-op on the host.
#[inline(always)]
pub(crate) fn log(message: &str) {
    #[cfg(target_os = "solana")]
    unsafe {
        pinocchio::syscalls::sol_log_(message.as_ptr(), message.len() as u64);
    }
    #[cfg(not(target_os = "solana"))]
    let _ = message;
}

#[cfg(test)]
mod tests {
    extern crate std;

    use super::*;
    use sha2::{Digest, Sha256};

    fn discriminator(name: &str) -> [u8; 8] {
        let hash = Sha256::digest(name.as_bytes());
        hash[..8].try_into().unwrap()
    }

    #[test]
    fn discriminators_match_interface() {
        assert_eq!(
            EXECUTE_DISCRIMINATOR,
            discriminator("spl-transfer-hook-interface:execute")
        );
        assert_eq!(
            INITIALIZE_EXTRA_ACCOUNT_METAS_DISCRIMINATOR,
            discriminator("spl-transfer-hook-interface:initialize-extra-account-metas")
        );
    }

    #[test]
    fn limit_is_two_percent_of_supply() {
        assert_eq!(MAX_TRANSFER_AMOUNT, 20_000_000_000_000);
        assert_eq!(MAX_TRANSFER_AMOUNT, 20_000_000 * 10u64.pow(6));
    }
}
