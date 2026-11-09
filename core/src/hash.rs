//! Algorithmes de hash rencontrés dans les MHL (v1.1 et ASC MHL v2).
//!
//! Chaque entrée d'un MHL peut porter plusieurs hash : on n'en calcule qu'un,
//! le plus rapide disponible (voir `Algo::cost`).

use md5::{Digest as _, Md5};
use serde::Serialize;
use sha1::Sha1;
use sha2::{Sha256, Sha512};
use xxhash_rust::xxh3::Xxh3;
use xxhash_rust::xxh32::Xxh32;
use xxhash_rust::xxh64::Xxh64;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, PartialOrd, Ord)]
#[serde(rename_all = "lowercase")]
pub enum Algo {
    Xxh3,
    Xxh64,
    Xxh128,
    Xxh32,
    Md5,
    Sha1,
    Sha256,
    C4,
}

impl Algo {
    pub fn label(self) -> &'static str {
        match self {
            Algo::Xxh3 => "XXH3-64",
            Algo::Xxh64 => "xxHash64",
            Algo::Xxh128 => "XXH128",
            Algo::Xxh32 => "xxHash32",
            Algo::Md5 => "MD5",
            Algo::Sha1 => "SHA-1",
            Algo::Sha256 => "SHA-256",
            Algo::C4 => "C4",
        }
    }
    /// Plus petit = plus rapide à calculer.
    pub fn cost(self) -> u8 {
        match self {
            Algo::Xxh3 => 0,
            Algo::Xxh128 => 1,
            Algo::Xxh64 => 2,
            Algo::Xxh32 => 3,
            Algo::Md5 => 4,
            Algo::Sha1 => 5,
            Algo::Sha256 => 6,
            Algo::C4 => 7,
        }
    }
    /// Balise XML → algorithme, et « tolérant » si la notation varie selon les
    /// logiciels (MHL 1.1 : `xxhash` et `xxhash64` ont été écrits en hexadécimal
    /// dans les deux ordres d'octets, ou en décimal).
    pub fn from_tag(tag: &str) -> Option<(Algo, bool)> {
        Some(match tag.to_ascii_lowercase().as_str() {
            "xxhash64be" | "xxh64" => (Algo::Xxh64, false),
            "xxhash64" => (Algo::Xxh64, true),
            "xxhash" | "xxh32" => (Algo::Xxh32, true),
            "xxh3" | "xxh3_64" | "xxhash3" => (Algo::Xxh3, false),
            "xxh128" | "xxh3_128" => (Algo::Xxh128, false),
            "md5" => (Algo::Md5, false),
            "sha1" => (Algo::Sha1, false),
            "sha256" => (Algo::Sha256, false),
            "c4" => (Algo::C4, false),
            _ => return None,
        })
    }
}

/// Hash attendu, tel qu'écrit dans le MHL.
#[derive(Debug, Clone, Serialize)]
pub struct Expected {
    pub algo: Algo,
    pub value: String,
    #[serde(skip)]
    pub loose: bool,
}

pub enum Hasher {
    Xxh3(Box<Xxh3>),
    Xxh128(Box<Xxh3>),
    Xxh64(Box<Xxh64>),
    Xxh32(Box<Xxh32>),
    Md5(Md5),
    Sha1(Sha1),
    Sha256(Sha256),
    C4(Sha512),
}

/// Résultat brut d'un calcul, pour pouvoir le comparer sous plusieurs notations.
pub enum Digest {
    Int(u128, usize),
    Bytes(Vec<u8>),
    Text(String),
}

impl Hasher {
    pub fn new(algo: Algo) -> Self {
        match algo {
            Algo::Xxh3 => Hasher::Xxh3(Box::new(Xxh3::new())),
            Algo::Xxh128 => Hasher::Xxh128(Box::new(Xxh3::new())),
            Algo::Xxh64 => Hasher::Xxh64(Box::new(Xxh64::new(0))),
            Algo::Xxh32 => Hasher::Xxh32(Box::new(Xxh32::new(0))),
            Algo::Md5 => Hasher::Md5(Md5::new()),
            Algo::Sha1 => Hasher::Sha1(Sha1::new()),
            Algo::Sha256 => Hasher::Sha256(Sha256::new()),
            Algo::C4 => Hasher::C4(Sha512::new()),
        }
    }
    pub fn update(&mut self, data: &[u8]) {
        match self {
            Hasher::Xxh3(h) | Hasher::Xxh128(h) => h.update(data),
            Hasher::Xxh64(h) => h.update(data),
            Hasher::Xxh32(h) => h.update(data),
            Hasher::Md5(h) => h.update(data),
            Hasher::Sha1(h) => h.update(data),
            Hasher::Sha256(h) => h.update(data),
            Hasher::C4(h) => h.update(data),
        }
    }
    pub fn finish(self) -> Digest {
        match self {
            Hasher::Xxh3(h) => Digest::Int(h.digest() as u128, 8),
            Hasher::Xxh128(h) => Digest::Int(h.digest128(), 16),
            Hasher::Xxh64(h) => Digest::Int(h.digest() as u128, 8),
            Hasher::Xxh32(h) => Digest::Int(h.digest() as u128, 4),
            Hasher::Md5(h) => Digest::Bytes(h.finalize().to_vec()),
            Hasher::Sha1(h) => Digest::Bytes(h.finalize().to_vec()),
            Hasher::Sha256(h) => Digest::Bytes(h.finalize().to_vec()),
            Hasher::C4(h) => Digest::Text(c4_from_sha512(&h.finalize())),
        }
    }
}

impl Digest {
    /// Notation canonique (hexadécimal minuscule, big-endian ; C4 en base 58).
    pub fn canonical(&self) -> String {
        match self {
            Digest::Int(v, n) => format!("{v:0w$x}", w = n * 2),
            Digest::Bytes(b) => hex(b),
            Digest::Text(t) => t.clone(),
        }
    }
    /// Compare au hash attendu. En mode tolérant, accepte aussi l'ordre
    /// d'octets inverse et la notation décimale.
    pub fn matches(&self, exp: &Expected) -> bool {
        let want = exp.value.trim();
        if let Digest::Text(t) = self {
            return t == want;
        }
        let want = want.to_ascii_lowercase();
        let want = want.strip_prefix("0x").unwrap_or(&want);
        if self.canonical() == want {
            return true;
        }
        if exp.loose {
            if let Digest::Int(v, n) = self {
                let be = v.to_be_bytes();
                let mut le = be[16 - n..].to_vec();
                le.reverse();
                if hex(&le) == want || v.to_string() == want {
                    return true;
                }
                // hexadécimal sans zéros de tête
                if format!("{v:x}") == want {
                    return true;
                }
            }
        }
        false
    }
}

pub fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

const C4_ALPHABET: &[u8; 58] = b"123456789ABCDEFGHJKLMNPQRSTUVWXYZabcdefghijkmnopqrstuvwxyz";

/// Identifiant C4 (SMPTE ST 2114) : « c4 » + SHA-512 en base 58, sur 90 caractères.
pub fn c4_from_sha512(digest: &[u8]) -> String {
    // division longue du nombre big-endian par 58
    let mut num = digest.to_vec();
    let mut out = Vec::with_capacity(88);
    while num.iter().any(|&b| b != 0) {
        let mut rem = 0u32;
        for b in num.iter_mut() {
            let cur = (rem << 8) | *b as u32;
            *b = (cur / 58) as u8;
            rem = cur % 58;
        }
        out.push(C4_ALPHABET[rem as usize]);
    }
    while out.len() < 88 {
        out.push(b'1');
    }
    out.reverse();
    format!("c4{}", String::from_utf8(out).unwrap())
}

pub fn c4_of_bytes(data: &[u8]) -> String {
    let mut h = Sha512::new();
    h.update(data);
    c4_from_sha512(&h.finalize())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn digest(algo: Algo, data: &[u8]) -> Digest {
        let mut h = Hasher::new(algo);
        h.update(data);
        h.finish()
    }

    #[test]
    fn known_vectors() {
        assert_eq!(digest(Algo::Xxh64, b"").canonical(), "ef46db3751d8e999");
        assert_eq!(digest(Algo::Xxh3, b"").canonical(), "2d06800538d394c2");
        assert_eq!(digest(Algo::Xxh128, b"").canonical(), "99aa06d3014798d86001c324468d497f");
        assert_eq!(digest(Algo::Xxh32, b"").canonical(), "02cc5d05");
        assert_eq!(digest(Algo::Md5, b"abc").canonical(), "900150983cd24fb0d6963f7d28e17f72");
        assert_eq!(digest(Algo::Sha1, b"abc").canonical(), "a9993e364706816aba3e25717850c26c9cd0d89d");
    }

    #[test]
    fn c4_vectors() {
        // valeurs de référence de la spécification C4
        assert_eq!(
            c4_of_bytes(b""),
            "c459dsjfscH38cYeXXYogktxf4Cd9ibshE3BHUo6a58hBXmRQdZrAkZzsWcbWtDg5oQstpDuni4Hirj75GEmTc1sFT"
        );
        assert_eq!(c4_of_bytes(b"foo").len(), 90);
    }

    #[test]
    fn loose_notations() {
        let d = digest(Algo::Xxh64, b"");
        let be = Expected { algo: Algo::Xxh64, value: "EF46DB3751D8E999".into(), loose: false };
        assert!(d.matches(&be));
        let le = Expected { algo: Algo::Xxh64, value: "99e9d85137db46ef".into(), loose: true };
        assert!(d.matches(&le));
        let strict_le = Expected { loose: false, ..le.clone() };
        assert!(!d.matches(&strict_le));
        let dec = Expected { algo: Algo::Xxh64, value: u64::from_str_radix("ef46db3751d8e999", 16).unwrap().to_string(), loose: true };
        assert!(d.matches(&dec));
        let d32 = digest(Algo::Xxh32, b"");
        assert!(d32.matches(&Expected { algo: Algo::Xxh32, value: "46947589".into(), loose: true }));
    }
}
