use crate::error::OrderError;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use shared::{
    ids::{CustomerId, OrderId, SagaId},
    money::Money,
    saga::SagaState,
};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum OrderStatus {
    Pending,
    Approved,
    Rejected,
}

impl std::fmt::Display for OrderStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            OrderStatus::Pending => write!(f, "pending"),
            OrderStatus::Approved => write!(f, "approved"),
            OrderStatus::Rejected => write!(f, "rejected"),
        }
    }
}

impl TryFrom<&str> for OrderStatus {
    type Error = OrderError;
    fn try_from(s: &str) -> Result<Self, Self::Error> {
        match s {
            "pending" => Ok(Self::Pending),
            "approved" => Ok(Self::Approved),
            "rejected" => Ok(Self::Rejected),
            _ => Err(OrderError::Internal(format!("unknown status: {s}"))),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Order {
    pub id: OrderId,
    pub customer_id: CustomerId,
    pub amount: Money,
    pub status: OrderStatus,
    pub created_at: DateTime<Utc>,
}

// HTTP types

#[derive(Debug, Deserialize)]
pub struct CreateOrderRequest {
    pub customer_id: uuid::Uuid,
    pub amount_minor: i64,
    pub currency: String,
}

#[derive(Debug, Serialize)]
pub struct CreateOrderResponse {
    pub order_id: uuid::Uuid,
    pub saga_id: uuid::Uuid,
    pub status: OrderStatus,
    pub message: &'static str,
}

#[derive(Debug, Serialize)]
pub struct OrderResponse {
    pub order_id: uuid::Uuid,
    pub customer_id: uuid::Uuid,
    pub amount: Money,
    pub status: OrderStatus,
    pub saga_state: SagaState,
    pub created_at: DateTime<Utc>,
}
