use serde::{Deserialize, Serialize};
use shared::ids::{CustomerId, ReservationId};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Customer {
    pub id: CustomerId,
    pub name: String,
    pub credit_limit: i64,
    pub currency: String,
}

impl Customer {
    // available credit = limit minus all active reservations
    pub fn available_credit_from_reserved(&self, reserved: i64) -> i64 {
        self.credit_limit - reserved
    }
}

#[derive(Debug, Clone)]
pub enum ReserveResult {
    //credit successfully reserved
    Reserved { reservation_id: ReservationId },

    //this saga_id already has a reservation - idempotent replay
    //returns the existing reservation_id unchanged
    AlreadyReserved { reservation_id: ReservationId },

    // customer dos not have enough credit available
    InsufficientCredit { available: i64, requested: i64 },

    // customer record not found
    CustomerNotFound,
}
