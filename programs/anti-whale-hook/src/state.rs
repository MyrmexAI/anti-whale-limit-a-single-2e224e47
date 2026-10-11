//! On-chain account layouts owned by this program.

use crate::{CONFIG_SEED, EXECUTE_DISCRIMINATOR};

/// Per-mint configuration, stored at the PDA `["config", mint]`.
///
/// Layout (little-endian):
/// - `[0..8]`   account discriminator [`Config::DISCRIMINATOR`]
/// - `[8..40]`  mint
/// - `[40..48]` maximum amount per transfer (base units)
/// - `[48]`     PDA bump
/// - `[49]`     layout version
pub struct Config;

impl Config {
    pub const LEN: usize = 50;
    pub const DISCRIMINATOR: [u8; 8] = *b"awhconfg";
    pub const VERSION: u8 = 1;

    pub fn write(data: &mut [u8], mint: &[u8; 32], max_transfer_amount: u64, bump: u8) {
        data[0..8].copy_from_slice(&Self::DISCRIMINATOR);
        data[8..40].copy_from_slice(mint);
        data[40..48].copy_from_slice(&max_transfer_amount.to_le_bytes());
        data[48] = bump;
        data[49] = Self::VERSION;
    }

    /// Returns `(mint, max_transfer_amount, bump)` for initialized config data.
    pub fn read(data: &[u8]) -> Option<(&[u8; 32], u64, u8)> {
        if data.len() != Self::LEN || data[0..8] != Self::DISCRIMINATOR {
            return None;
        }
        let mint = data[8..40].try_into().ok()?;
        let max = u64::from_le_bytes(data[40..48].try_into().ok()?);
        Some((mint, max, data[48]))
    }
}

/// Size of one serialized `ExtraAccountMeta`.
const EXTRA_ACCOUNT_META_LEN: usize = 35;
/// Number of extra accounts Execute requires.
const EXTRA_ACCOUNT_COUNT: usize = 1;

/// Size of the extra account metas account: TLV header (8-byte
/// discriminator + u32 length) + `PodSlice` (u32 count + entries).
pub const EXTRA_ACCOUNT_METAS_LEN: usize = 8 + 4 + 4 + EXTRA_ACCOUNT_META_LEN * EXTRA_ACCOUNT_COUNT;

/// `ExtraAccountMeta` discriminator: PDA derived from this hook program.
const META_PDA_OF_HOOK_PROGRAM: u8 = 1;
/// Seed config: literal bytes `[1, len, bytes..]`.
const SEED_LITERAL: u8 = 1;
/// Seed config: key of an Execute account `[3, index]`.
const SEED_ACCOUNT_KEY: u8 = 3;
/// Index of the mint in the Execute accounts.
const EXECUTE_MINT_INDEX: u8 = 1;

/// Serialized `ExtraAccountMetaList` for Execute (the format read by
/// `spl-tlv-account-resolution`). The single extra account is the config PDA
/// `["config", mint]` of this program, read-only and not a signer.
pub const EXTRA_ACCOUNT_METAS: [u8; EXTRA_ACCOUNT_METAS_LEN] = {
    let mut data = [0u8; EXTRA_ACCOUNT_METAS_LEN];
    let mut i = 0;
    while i < 8 {
        data[i] = EXECUTE_DISCRIMINATOR[i];
        i += 1;
    }
    let value_len = ((4 + EXTRA_ACCOUNT_META_LEN * EXTRA_ACCOUNT_COUNT) as u32).to_le_bytes();
    let count = (EXTRA_ACCOUNT_COUNT as u32).to_le_bytes();
    let mut i = 0;
    while i < 4 {
        data[8 + i] = value_len[i];
        data[12 + i] = count[i];
        i += 1;
    }

    // Entry 0: discriminator, 32-byte address config, is_signer, is_writable.
    let entry = 16;
    data[entry] = META_PDA_OF_HOOK_PROGRAM;
    let seeds = entry + 1;
    data[seeds] = SEED_LITERAL;
    data[seeds + 1] = CONFIG_SEED.len() as u8;
    let mut i = 0;
    while i < CONFIG_SEED.len() {
        data[seeds + 2 + i] = CONFIG_SEED[i];
        i += 1;
    }
    data[seeds + 2 + CONFIG_SEED.len()] = SEED_ACCOUNT_KEY;
    data[seeds + 3 + CONFIG_SEED.len()] = EXECUTE_MINT_INDEX;
    // is_signer = false, is_writable = false (already zero).
    data
};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extra_account_metas_layout() {
        let d = &EXTRA_ACCOUNT_METAS;
        assert_eq!(d.len(), 51);
        assert_eq!(d[0..8], EXECUTE_DISCRIMINATOR);
        assert_eq!(u32::from_le_bytes(d[8..12].try_into().unwrap()), 39);
        assert_eq!(u32::from_le_bytes(d[12..16].try_into().unwrap()), 1);
        assert_eq!(d[16], 1);
        assert_eq!(d[17..27], [1, 6, b'c', b'o', b'n', b'f', b'i', b'g', 3, 1]);
        assert!(d[27..49].iter().all(|b| *b == 0));
        assert_eq!(d[49..51], [0, 0]);
    }

    #[test]
    fn config_roundtrip() {
        let mut data = [0u8; Config::LEN];
        assert!(Config::read(&data).is_none());
        Config::write(&mut data, &[5; 32], 42, 254);
        assert_eq!(Config::read(&data), Some((&[5; 32], 42, 254)));
        assert!(Config::read(&data[..Config::LEN - 1]).is_none());
    }
}
