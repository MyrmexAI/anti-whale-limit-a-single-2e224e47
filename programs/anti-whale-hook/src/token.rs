//! Minimal zero-copy readers for Token-2022 mint and token account data.
//!
//! Layout: the base state (82-byte mint padded to 165 bytes, or the 165-byte
//! token account) is followed by a one-byte account type and a TLV list of
//! extensions (`u16` type, `u16` length, value).

/// Size of a token account base state; mints are padded to this length when
/// they carry extensions.
pub const BASE_ACCOUNT_LEN: usize = 165;
/// Size of a mint base state.
pub const MINT_LEN: usize = 82;

const ACCOUNT_TYPE_OFFSET: usize = BASE_ACCOUNT_LEN;
const TLV_START: usize = BASE_ACCOUNT_LEN + 1;

const ACCOUNT_TYPE_MINT: u8 = 1;
const ACCOUNT_TYPE_ACCOUNT: u8 = 2;

/// `ExtensionType::TransferHook` (mint extension).
pub const EXTENSION_TRANSFER_HOOK: u16 = 14;
/// `ExtensionType::TransferHookAccount` (token account extension).
pub const EXTENSION_TRANSFER_HOOK_ACCOUNT: u16 = 15;

/// Returns the value of the first extension of type `extension_type`.
///
/// `data` must be the full account data; `None` is returned when the account
/// has no extensions, is not of `account_type`, or the TLV list is malformed.
fn find_extension(data: &[u8], account_type: u8, extension_type: u16) -> Option<&[u8]> {
    if *data.get(ACCOUNT_TYPE_OFFSET)? != account_type {
        return None;
    }
    let mut tlv = data.get(TLV_START..)?;
    while let Some((header, rest)) = tlv.split_first_chunk::<4>() {
        let ty = u16::from_le_bytes([header[0], header[1]]);
        let len = u16::from_le_bytes([header[2], header[3]]) as usize;
        // `Uninitialized` marks the end of the written extensions.
        if ty == 0 {
            return None;
        }
        let value = rest.get(..len)?;
        if ty == extension_type {
            return Some(value);
        }
        tlv = &rest[len..];
    }
    None
}

/// Mint field readers.
pub mod mint {
    use super::*;

    /// Returns the mint authority, or `None` if the data is not an
    /// initialized mint or the authority is unset.
    pub fn authority(data: &[u8]) -> Option<&[u8; 32]> {
        if data.len() < MINT_LEN || data[45] != 1 {
            return None;
        }
        // `COption<Pubkey>`: u32 tag (1 = Some) followed by the key.
        if data[0..4] != [1, 0, 0, 0] {
            return None;
        }
        data[4..36].try_into().ok()
    }

    /// Returns the program id configured in the `TransferHook` extension.
    pub fn transfer_hook_program_id(data: &[u8]) -> Option<&[u8; 32]> {
        // Value: authority (32 bytes) followed by program id (32 bytes).
        find_extension(data, ACCOUNT_TYPE_MINT, EXTENSION_TRANSFER_HOOK)?
            .get(32..64)?
            .try_into()
            .ok()
    }
}

/// Token account field readers.
pub mod account {
    use super::*;

    /// Returns the mint of the token account.
    pub fn mint(data: &[u8]) -> Option<&[u8; 32]> {
        if data.len() < BASE_ACCOUNT_LEN {
            return None;
        }
        data[0..32].try_into().ok()
    }

    /// Returns `true` if the `TransferHookAccount.transferring` flag is set,
    /// which Token-2022 only does for the duration of a transfer.
    pub fn is_transferring(data: &[u8]) -> bool {
        matches!(
            find_extension(data, ACCOUNT_TYPE_ACCOUNT, EXTENSION_TRANSFER_HOOK_ACCOUNT),
            Some([1])
        )
    }
}

#[cfg(test)]
mod tests {
    extern crate std;

    use super::*;
    use std::vec::Vec;

    fn token_account(extensions: &[(u16, &[u8])]) -> Vec<u8> {
        let mut data = std::vec![0u8; BASE_ACCOUNT_LEN];
        data[0..32].copy_from_slice(&[7; 32]);
        data[108] = 1; // initialized
        data.push(ACCOUNT_TYPE_ACCOUNT);
        for (ty, value) in extensions {
            data.extend_from_slice(&ty.to_le_bytes());
            data.extend_from_slice(&(value.len() as u16).to_le_bytes());
            data.extend_from_slice(value);
        }
        data
    }

    #[test]
    fn reads_transferring_flag() {
        assert!(account::is_transferring(&token_account(&[(15, &[1])])));
        assert!(!account::is_transferring(&token_account(&[(15, &[0])])));
        // Found after another extension (ImmutableOwner, zero length).
        assert!(account::is_transferring(&token_account(&[(7, &[]), (15, &[1])])));
        // Missing extension / no extensions at all.
        assert!(!account::is_transferring(&token_account(&[(7, &[])])));
        assert!(!account::is_transferring(&token_account(&[])[..BASE_ACCOUNT_LEN]));
    }

    #[test]
    fn rejects_malformed_tlv() {
        let mut data = token_account(&[(15, &[1])]);
        // Declared length runs past the end of the data.
        data[TLV_START + 2] = 9;
        assert!(!account::is_transferring(&data));
        // Wrong account type (mint instead of account).
        let mut data = token_account(&[(15, &[1])]);
        data[ACCOUNT_TYPE_OFFSET] = ACCOUNT_TYPE_MINT;
        assert!(!account::is_transferring(&data));
    }

    #[test]
    fn reads_mint_fields() {
        let mut data = std::vec![0u8; BASE_ACCOUNT_LEN];
        data[0..4].copy_from_slice(&[1, 0, 0, 0]);
        data[4..36].copy_from_slice(&[3; 32]);
        data[44] = 6;
        data[45] = 1;
        data.push(ACCOUNT_TYPE_MINT);
        data.extend_from_slice(&14u16.to_le_bytes());
        data.extend_from_slice(&64u16.to_le_bytes());
        data.extend_from_slice(&[0; 32]);
        data.extend_from_slice(&[9; 32]);
        assert_eq!(mint::authority(&data), Some(&[3; 32]));
        assert_eq!(mint::transfer_hook_program_id(&data), Some(&[9; 32]));

        data[45] = 0;
        assert_eq!(mint::authority(&data), None);
        data[45] = 1;
        data[0] = 0;
        assert_eq!(mint::authority(&data), None);
        assert_eq!(mint::transfer_hook_program_id(&data[..MINT_LEN]), None);
    }
}
