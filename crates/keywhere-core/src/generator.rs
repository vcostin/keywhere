//! Password generator: random, passphrase (EFF short wordlist) and PIN.
//! Unbiased (rejection sampling over the OS CSPRNG); output is `Zeroizing`.

use zeroize::Zeroizing;

use crate::Error;

const UPPER: &str = "ABCDEFGHIJKLMNOPQRSTUVWXYZ";
const LOWER: &str = "abcdefghijklmnopqrstuvwxyz";
const DIGITS: &str = "0123456789";
const SYMBOLS: &str = "!@#$%^&*-_=+?";
const LOOKALIKES: &str = "Il1O0o";

/// EFF short wordlist #1 (CC BY 3.0, https://www.eff.org/dice): 1296 words.
const WORDLIST: &str = include_str!("../data/eff_short_wordlist_1.txt");

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    Random,
    Passphrase,
    Pin,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Options {
    pub mode: Mode,
    /// Characters (Random/Pin) or words (Passphrase).
    pub length: usize,
    pub upper: bool,
    pub lower: bool,
    pub digits: bool,
    pub symbols: bool,
    pub no_lookalikes: bool,
    pub separator: char,
}

impl Default for Options {
    fn default() -> Self {
        Options {
            mode: Mode::Random,
            length: 20,
            upper: true,
            lower: true,
            digits: true,
            symbols: true,
            no_lookalikes: false,
            separator: '-',
        }
    }
}

impl Options {
    pub fn min_len(&self) -> usize {
        match self.mode {
            Mode::Random => 8,
            Mode::Passphrase => 3,
            Mode::Pin => 4,
        }
    }
    pub fn max_len(&self) -> usize {
        match self.mode {
            Mode::Random => 128,
            Mode::Passphrase => 12,
            Mode::Pin => 16,
        }
    }

    fn classes(&self) -> Vec<Vec<char>> {
        let keep = |set: &str| -> Vec<char> {
            set.chars()
                .filter(|c| !(self.no_lookalikes && LOOKALIKES.contains(*c)))
                .collect()
        };
        let mut v = Vec::new();
        if self.upper {
            v.push(keep(UPPER));
        }
        if self.lower {
            v.push(keep(LOWER));
        }
        if self.digits {
            v.push(keep(DIGITS));
        }
        if self.symbols {
            v.push(keep(SYMBOLS));
        }
        if v.is_empty() {
            v.push(keep(LOWER));
        }
        v
    }

    /// Entropy of one generated value, in bits.
    pub fn entropy_bits(&self) -> f64 {
        let len = self.length.clamp(self.min_len(), self.max_len()) as f64;
        match self.mode {
            Mode::Random => {
                len * (self.classes().iter().map(Vec::len).sum::<usize>() as f64).log2()
            }
            Mode::Passphrase => len * (words().count() as f64).log2(),
            Mode::Pin => len * 10f64.log2(),
        }
    }
}

fn words() -> impl Iterator<Item = &'static str> {
    WORDLIST.lines().filter_map(|l| l.split_whitespace().nth(1))
}

/// Uniform integer in 0..n.
fn uniform(n: usize) -> Result<usize, Error> {
    assert!(n > 0 && n <= u32::MAX as usize);
    let n = n as u32;
    let zone = u32::MAX - (u32::MAX % n);
    loop {
        let mut b = [0u8; 4];
        getrandom::getrandom(&mut b).map_err(|_| Error::Rng)?;
        let v = u32::from_le_bytes(b);
        if v < zone {
            return Ok((v % n) as usize);
        }
    }
}

pub fn generate(o: &Options) -> Result<Zeroizing<String>, Error> {
    let len = o.length.clamp(o.min_len(), o.max_len());
    let mut out = Zeroizing::new(String::with_capacity(len * 8));
    match o.mode {
        Mode::Pin => {
            for _ in 0..len {
                out.push(char::from(b'0' + uniform(10)? as u8));
            }
        }
        Mode::Passphrase => {
            let list: Vec<&str> = words().collect();
            for i in 0..len {
                if i > 0 {
                    out.push(o.separator);
                }
                out.push_str(list[uniform(list.len())?]);
            }
        }
        Mode::Random => {
            let classes = o.classes();
            let all: Vec<char> = classes.iter().flatten().copied().collect();
            // Re-draw until every selected class is present (len >= 8 >= #classes).
            loop {
                let mut chars: Zeroizing<Vec<char>> = Zeroizing::new(Vec::with_capacity(len));
                for _ in 0..len {
                    chars.push(all[uniform(all.len())?]);
                }
                if classes
                    .iter()
                    .all(|cl| chars.iter().any(|c| cl.contains(c)))
                {
                    out.extend(chars.iter());
                    break;
                }
            }
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wordlist_loaded() {
        assert_eq!(words().count(), 1296);
    }

    #[test]
    fn random_has_all_classes_and_length() {
        let o = Options::default();
        for _ in 0..50 {
            let p = generate(&o).unwrap();
            assert_eq!(p.chars().count(), 20);
            assert!(p.chars().any(|c| c.is_ascii_uppercase()));
            assert!(p.chars().any(|c| c.is_ascii_lowercase()));
            assert!(p.chars().any(|c| c.is_ascii_digit()));
            assert!(p.chars().any(|c| SYMBOLS.contains(c)));
        }
        assert!(o.entropy_bits() > 120.0);
    }

    #[test]
    fn lookalikes_excluded() {
        let o = Options {
            no_lookalikes: true,
            length: 128,
            ..Default::default()
        };
        for _ in 0..20 {
            assert!(!generate(&o)
                .unwrap()
                .chars()
                .any(|c| LOOKALIKES.contains(c)));
        }
    }

    #[test]
    fn passphrase_and_pin() {
        let p = generate(&Options {
            mode: Mode::Passphrase,
            length: 5,
            ..Default::default()
        })
        .unwrap();
        assert_eq!(p.split('-').count(), 5);
        let pin = generate(&Options {
            mode: Mode::Pin,
            length: 6,
            ..Default::default()
        })
        .unwrap();
        assert!(pin.len() == 6 && pin.chars().all(|c| c.is_ascii_digit()));
    }
}
