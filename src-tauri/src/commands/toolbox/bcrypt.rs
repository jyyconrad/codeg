use bcrypt::{hash, verify, DEFAULT_COST};

use crate::app_error::AppCommandError;

pub const BCRYPT_COST_MIN: u32 = 4;
pub const BCRYPT_COST_MAX: u32 = 14;
pub const BCRYPT_COST_DEFAULT: u32 = DEFAULT_COST; // 12 in some versions; we pin 10 in the command.

pub fn bcrypt_hash_core(password: &str, cost: u32) -> Result<String, AppCommandError> {
    if !(BCRYPT_COST_MIN..=BCRYPT_COST_MAX).contains(&cost) {
        return Err(AppCommandError::invalid_input(format!(
            "bcrypt cost must be {BCRYPT_COST_MIN}–{BCRYPT_COST_MAX}."
        )));
    }
    if password.is_empty() {
        return Err(AppCommandError::invalid_input("Password is empty."));
    }
    hash(password, cost).map_err(|e| AppCommandError::invalid_input(e.to_string()))
}

pub fn bcrypt_verify_core(password: &str, hashed: &str) -> Result<bool, AppCommandError> {
    if hashed.is_empty() {
        return Err(AppCommandError::invalid_input("Hash is empty."));
    }
    verify(password, hashed).map_err(|e| AppCommandError::invalid_input(e.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hash_and_verify() {
        let hashed = bcrypt_hash_core("secret-pass", 4).unwrap();
        assert!(hashed.starts_with("$2"));
        assert!(bcrypt_verify_core("secret-pass", &hashed).unwrap());
        assert!(!bcrypt_verify_core("other", &hashed).unwrap());
    }

    #[test]
    fn rejects_cost_out_of_range() {
        assert!(bcrypt_hash_core("x", 3).is_err());
        assert!(bcrypt_hash_core("x", 15).is_err());
    }
}
