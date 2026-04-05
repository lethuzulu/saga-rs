#[derive(Debug, thiserror::Error)]
pub enum CustomerError {
    #[error("customer not found")]
    CustomerNotFound,

    #[error("reservation not found")]
    ReservationNotFound,

    #[error("database error: {0}")]
    Database(#[from] sqlx::Error),

    #[error("serialization error: {0}")]
    Serialization(#[from] serde_json::Error),

    #[error("kafka error: {0}")]
    Kafka(String),

    #[error("internal error: {0}")]
    Internal(String),
}
