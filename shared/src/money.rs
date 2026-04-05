use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct Money {
    pub minor_units: i64,
    pub currency: String,
}
impl Money {
    pub fn from_minor_units(
        minor_units: i64,
        currency: impl Into<String>,
    ) -> Result<Self, MoneyError> {
        if minor_units <= 0 {
            return Err(MoneyError::NonPositive(minor_units));
        }
        Ok(Self {
            minor_units,
            currency: currency.into(),
        })
    }

    pub fn gbp(pence: i64) -> Result<Self, MoneyError> {
        Self::from_minor_units(pence, "GBP")
    }
}

impl std::fmt::Display for Money {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{} {:.2}",
            self.currency,
            self.minor_units as f64 / 100.0
        )
    }
}

#[derive(Debug, thiserror::Error)]
pub enum MoneyError {
    #[error("amount must be greater than zero, got {0}")]
    NonPositive(i64),
}
