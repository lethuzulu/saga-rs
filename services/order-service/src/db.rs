use shared::{
    ids::{CustomerId, OrderId, ReservationId, SagaId},
    money::Money,
    saga::{SagaData, SagaEvent, SagaState, apply},
};
use sqlx::{PgPool, Postgres, Transaction, postgres::PgPoolOptions};
use std::future::Future;

use crate::{
    error::OrderError,
    types::{Order, OrderStatus},
};

#[derive(Clone)]
pub struct OrderDb {
    pub pool: PgPool,
}

impl OrderDb {
    pub async fn new(url: &str) -> Result<Self, OrderError> {
        let pool = PgPoolOptions::new()
            .max_connections(10)
            .connect(url)
            .await?;
        Ok(Self { pool })
    }

    pub async fn with_transaction<'a, F, Fut, T>(&self, f: F) -> Result<T, OrderError>
    where
        F: FnOnce(Transaction<'a, Postgres>) -> Fut,
        Fut: Future<Output = Result<(T, Transaction<'a, Postgres>), OrderError>>,
    {
        let tx = self.pool.begin().await?;
        let (result, tx) = f(tx).await?;
        tx.commit().await?;
        Ok(result)
    }
}

impl OrderDb {
    pub async fn create_saga(
        &self,
        customer_id: CustomerId,
        amount: Money,
    ) -> Result<(OrderId, SagaId), OrderError> {
        let order_id = OrderId::new();
        let saga_id = SagaId::new();
        let state = SagaState::Started;

        self.with_transaction(|mut tx| async move {
            sqlx::query!(
                r#"
                INSERT INTO orders (id, customer_id, amount_minor, currency, status)
                VALUES ($1, $2, $3, $4, 'pending')
                "#,
                order_id.as_uuid(),
                customer_id.as_uuid(),
                amount.minor_units,
                amount.currency,
            )
            .execute(tx.as_mut())
            .await?;

            sqlx::query!(
                r#"
                INSERT INTO sagas (id, order_id, customer_id, amount_minor, currency, state)
                VALUES ($1, $2, $3, $4, $5, $6)
                "#,
                saga_id.as_uuid(),
                order_id.as_uuid(),
                customer_id.as_uuid(),
                amount.minor_units,
                amount.currency,
                serde_json::to_value(&state)?,
            )
            .execute(tx.as_mut())
            .await?;

            Ok(((order_id, saga_id), tx))
        })
        .await
    }

    pub async fn advance_saga(
        &self,
        saga_id: SagaId,
        event: SagaEvent,
    ) -> Result<SagaState, OrderError> {
        self.with_transaction(|mut tx| async move {
            // load with row-level lock
            let row = sqlx::query!(
                r#"
                SELECT state, reservation_id
                FROM   sagas
                WHERE  id = $1
                FOR UPDATE
                "#,
                saga_id.as_uuid(),
            )
            .fetch_optional(tx.as_mut())
            .await?
            .ok_or(OrderError::NotFound)?;

            let current: SagaState = serde_json::from_value(row.state)?;

            // pure state transition
            let next = apply(current.clone(), event.clone())?;

            // persist new state
            sqlx::query!(
                "UPDATE sagas SET state = $1, updated_at = now() WHERE id = $2",
                serde_json::to_value(&next)?,
                saga_id.as_uuid(),
            )
            .execute(tx.as_mut())
            .await?;

            // append to history — full audit trail
            sqlx::query!(
                r#"
                INSERT INTO saga_history (saga_id, from_state, event, to_state)
                VALUES ($1, $2, $3, $4)
                "#,
                saga_id.as_uuid(),
                serde_json::to_value(&current)?,
                serde_json::to_value(&event)?,
                serde_json::to_value(&next)?,
            )
            .execute(tx.as_mut())
            .await?;

            Ok((next, tx))
        })
        .await
    }

    // Used by the orchestrator when it receives CreditReserved reply.
    pub async fn set_reservation_id(
        &self,
        saga_id: SagaId,
        reservation_id: ReservationId,
    ) -> Result<(), OrderError> {
        sqlx::query!(
            "UPDATE sagas SET reservation_id = $1 WHERE id = $2",
            reservation_id.as_uuid(),
            saga_id.as_uuid(),
        )
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    // load a saga record for the orchestrator.
    pub async fn load_saga(&self, saga_id: SagaId) -> Result<SagaData, OrderError> {
        let row = sqlx::query!(
            r#"
            SELECT id, order_id, customer_id, amount_minor, currency,
                   state, reservation_id
            FROM   sagas
            WHERE  id = $1
            "#,
            saga_id.as_uuid(),
        )
        .fetch_optional(&self.pool)
        .await?
        .ok_or(OrderError::NotFound)?;

        Ok(SagaData {
            id: SagaId::from_uuid(row.id),
            order_id: OrderId::from_uuid(row.order_id),
            customer_id: CustomerId::from_uuid(row.customer_id),
            amount: Money {
                minor_units: row.amount_minor,
                currency: row.currency,
            },
            state: serde_json::from_value(row.state)?,
            reservation_id: row.reservation_id.map(ReservationId::from_uuid),
        })
    }

    //load all non-terminal sagas for crash recovery.
    pub async fn load_inflight_sagas(&self) -> Result<Vec<SagaData>, OrderError> {
        let rows = sqlx::query!(
            r#"
            SELECT id, order_id, customer_id, amount_minor, currency,
                   state, reservation_id
            FROM   sagas
            WHERE  (state->>'state') NOT IN ('completed', 'failed', 'credit_failed')
            "#,
        )
        .fetch_all(&self.pool)
        .await?;

        rows.into_iter()
            .map(|row| {
                Ok(SagaData {
                    id: SagaId::from_uuid(row.id),
                    order_id: OrderId::from_uuid(row.order_id),
                    customer_id: CustomerId::from_uuid(row.customer_id),
                    amount: Money {
                        minor_units: row.amount_minor,
                        currency: row.currency,
                    },
                    state: serde_json::from_value(row.state)?,
                    reservation_id: row.reservation_id.map(ReservationId::from_uuid),
                })
            })
            .collect()
    }

    //update order status when saga reaches a terminal state.
    pub async fn update_order_status(
        &self,
        order_id: OrderId,
        status: OrderStatus,
    ) -> Result<(), OrderError> {
        sqlx::query!(
            "UPDATE orders SET status = $1, updated_at = now() WHERE id = $2",
            status.to_string(),
            order_id.as_uuid(),
        )
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn get_order(
        &self,
        order_id: OrderId,
    ) -> Result<Option<(Order, SagaState)>, OrderError> {
        let row = sqlx::query!(
            r#"
            SELECT o.id, o.customer_id, o.amount_minor, o.currency,
                   o.status, o.created_at, s.state
            FROM   orders o
            JOIN   sagas s ON s.order_id = o.id
            WHERE  o.id = $1
            "#,
            order_id.as_uuid(),
        )
        .fetch_optional(&self.pool)
        .await?;

        row.map(|r| {
            Ok((
                Order {
                    id: OrderId::from_uuid(r.id),
                    customer_id: CustomerId::from_uuid(r.customer_id),
                    amount: Money {
                        minor_units: r.amount_minor,
                        currency: r.currency,
                    },
                    status: OrderStatus::try_from(r.status.as_str())?,
                    created_at: r.created_at,
                },
                serde_json::from_value::<SagaState>(r.state)?,
            ))
        })
        .transpose()
    }

    // Test helpers
    pub async fn count_sagas_in_state(&self, state_name: &str) -> Result<i64, OrderError> {
        Ok(sqlx::query_scalar!(
            "SELECT COUNT(*) FROM sagas WHERE state->>'state' = $1",
            state_name
        )
        .fetch_one(&self.pool)
        .await?
        .unwrap_or(0))
    }

    pub async fn count_history_entries(&self, saga_id: SagaId) -> Result<i64, OrderError> {
        Ok(sqlx::query_scalar!(
            "SELECT COUNT(*) FROM saga_history WHERE saga_id = $1",
            saga_id.as_uuid()
        )
        .fetch_one(&self.pool)
        .await?
        .unwrap_or(0))
    }
}
