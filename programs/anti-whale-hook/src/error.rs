use pinocchio::error::ProgramError;

/// Program-specific errors, surfaced as `ProgramError::Custom(code)`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u32)]
pub enum HookError {
    /// The transfer moves more than the per-transfer anti-whale limit.
    TransferAmountExceedsLimit = 6000,
    /// Execute was not invoked by Token-2022 during a transfer (the source
    /// account's `transferring` flag is not set).
    NotTransferring = 6001,
    /// The config account does not exist or was never initialized for this
    /// mint (InitializeExtraAccountMetaList must run first).
    ConfigNotInitialized = 6002,
    /// The config account is not the config PDA of this mint.
    InvalidConfigAccount = 6003,
    /// The extra account metas account is not the expected PDA.
    InvalidExtraAccountMetasAccount = 6004,
    /// The mint is not a valid, initialized Token-2022 mint.
    InvalidMint = 6005,
    /// The mint's transfer hook extension does not point to this program.
    MintHookProgramMismatch = 6006,
    /// The signer is not the mint authority of the mint.
    MintAuthorityMismatch = 6007,
    /// The token account is not a Token-2022 account of this mint.
    InvalidTokenAccount = 6008,
    /// The accounts were already initialized for this mint.
    AlreadyInitialized = 6009,
}

impl From<HookError> for ProgramError {
    fn from(e: HookError) -> Self {
        ProgramError::Custom(e as u32)
    }
}
