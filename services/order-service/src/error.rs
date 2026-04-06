use axum::{http::StatusCode, response::{IntoResponse, Response}, Json};
use shared::saga::SagaError;

#[derive(Debug, thiserror::Error)]
pub enum OrderError {
    #[error("order not found")]
    NotFound,

    #[error("invalid amount: {0}")]
    InvalidAmount(String),

    #[error("invalid transition: {0}")]
    InvalidTransition(#[from] SagaError),

    #[error("database error: {0}")]
    Database(#[from] sqlx::Error),

    #[error("kafka error: {0}")]
    Kafka(String),

    #[error("serialization error: {0}")]
    Serialization(#[from] serde_json::Error),

    #[error("internal error: {0}")]
    Internal(String),
}

impl IntoResponse for OrderError {
    fn into_response(self) -> Response {
        let (status, message) = match &self {
            OrderError::NotFound          => (StatusCode::NOT_FOUND, self.to_string()),
            OrderError::InvalidAmount(_)  => (StatusCode::UNPROCESSABLE_ENTITY, self.to_string()),
            _ => {
                tracing::error!(error = %self, "internal error");
                (StatusCode::INTERNAL_SERVER_ERROR, "internal error".into())
            }
        };
        (status, Json(serde_json::json!({ "error": message }))).into_response()
    }
}