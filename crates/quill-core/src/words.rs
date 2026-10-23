//! Short codes people compare or type: 6 words from the BIP-39 English list
//! (`bip39-english.txt`, 2048 words, SHA-256 `2f5eed53…24dbda`), carrying
//! the first 66 bits of a hash, 11 bits per word, big-endian (protocol
//! draft, section 11).

use sha2::{Digest, Sha256};

const LIST: &str = include_str!("bip39-english.txt");

fn list() -> Vec<&'static str> {
    LIST.lines().collect()
}

/// The 6 words for the first 66 bits of `hash` (at least 9 bytes).
pub fn words(hash: &[u8]) -> Vec<&'static str> {
    let list = list();
    let mut bits: u128 = 0;
    for b in hash.iter().take(9) {
        bits = (bits << 8) | u128::from(*b);
    }
    // 72 bits read; keep the top 66.
    let bits = bits >> 6;
    (0..6)
        .rev()
        .map(|i| {
            let index = usize::try_from((bits >> (11 * i)) & 0x7FF).unwrap_or(0);
            list.get(index).copied().unwrap_or("")
        })
        .collect()
}

/// Whether `typed` (any case, any spacing) are the words for `hash`.
pub fn matches(hash: &[u8], typed: &str) -> bool {
    let typed: Vec<String> = typed.split_whitespace().map(str::to_lowercase).collect();
    typed.len() == 6 && typed.iter().zip(words(hash)).all(|(t, w)| t == w)
}

/// SHA-256 of `parts`, labelled, for codes derived from other values.
pub fn labelled_hash(label: &str, parts: &[&[u8]]) -> [u8; 32] {
    let mut h = Sha256::new();
    h.update(label.as_bytes());
    for p in parts {
        h.update(p);
    }
    h.finalize().into()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_list_is_the_bip39_english_list() {
        // As published, with LF line endings (a checkout could add CRs).
        let lf = LIST.replace("\r\n", "\n");
        assert_eq!(
            hex(&Sha256::digest(lf.as_bytes())),
            "2f5eed53a4727b4bf8880d8f3f199efc90e58503646d9ff8eff3a2ed3b24dbda"
        );
        let l = list();
        assert_eq!((l.len(), l[0], l[2047]), (2048, "abandon", "zoo"));
    }

    fn hex(b: &[u8]) -> String {
        b.iter().map(|x| format!("{x:02x}")).collect()
    }

    #[test]
    fn words_take_the_first_66_bits_big_endian() {
        assert_eq!(words(&[0; 9]), vec!["abandon"; 6]);
        assert_eq!(words(&[0xFF; 9]), vec!["zoo"; 6]);
        // 0b00000000_001... : the first word's index is 1, the rest 0.
        let mut h = [0u8; 9];
        h[1] = 0b0010_0000;
        assert_eq!(
            words(&h),
            vec![
                "ability", "abandon", "abandon", "abandon", "abandon", "abandon"
            ]
        );
        // Bits past the 66th don't matter.
        let mut late = [0u8; 9];
        late[8] = 0b0011_1111;
        assert_eq!(words(&late), vec!["abandon"; 6]);
        assert!(matches(
            &h,
            "  ABILITY abandon abandon\tabandon abandon abandon "
        ));
        assert!(!matches(
            &h,
            "abandon abandon abandon abandon abandon abandon"
        ));
        assert!(!matches(&h, "ability abandon"));
    }
}
