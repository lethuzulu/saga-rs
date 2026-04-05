use shared::{
    ids::{CustomerId, ReservationId, SagaId},
    money::Money,
};
use sqlx::{PgPool, Postgres, Transaction, postgres::PgPoolOptions};

use crate::{error::CustomerError, types::ReserveResult};

pub struct CustomerDb {
    pub pool: PgPool,
}

impl CustomerDb {
    pub async fn new(url: &str) -> Result<Self, CustomerError> {
        let pool = PgPoolOptions::new()
            .max_connections(10)
            .connect(url)
            .await?;
        Ok(Self { pool })
    }

    pub async fn with_transaction<'a, F, Fut, T>(&self, f: F) -> Result<T, CustomerError>
    where
        F: FnOnce(Transaction<'a, Postgres>) -> Fut,
        Fut: Future<Output = Result<(T, Transaction<'a, Postgres>), CustomerError>>,
    {
        let tx = self.pool.begin().await?;
        let (result, tx) = f(tx).await?;
        tx.commit().await?;
        Ok(result)
    }
}

impl CustomerDb {
    pub async fn reserve_credit(
        &self,
        saga_id: SagaId,
        customer_id: CustomerId,
        amount: Money,
    ) -> Result<ReserveResult, CustomerError> {
        self.with_transaction(|mut tx| async move {
            let existing = sqlx::query!(
                r#"
                SELECT id FROM credit_reservations
                WHERE  saga_id = $1
                "#,
                saga_id.as_uuid(),
            )
            .fetch_optional(tx.as_mut())
            .await?;

            if let Some(row) = existing {
                return Ok((
                    ReserveResult::AlreadyReserved {
                        reservation_id: ReservationId::from_uuid(row.id),
                    },
                    tx,
                ));
            }

            // Lock the customer row first 
            let customer = sqlx::query!(
                r#"
                SELECT id, credit_limit, currency
                FROM   customers
                WHERE  id = $1
                FOR UPDATE
                "#,
                customer_id.as_uuid(),
            )
            .fetch_optional(tx.as_mut())
            .await?;

            let Some(customer) = customer else {
                return Ok((ReserveResult::CustomerNotFound, tx));
            };

             // aggregate reservations
            let reserved = sqlx::query_scalar!(
                r#"
                SELECT COALESCE(SUM(amount_minor) FILTER (WHERE status = 'active'), 0)::BIGINT AS "reserved_minor!"
                FROM   credit_reservations
                WHERE  customer_id = $1
                "#,
                customer_id.as_uuid(),
            )
            .fetch_one(tx.as_mut())
            .await?;

            let available = customer.credit_limit - reserved;

            if amount.minor_units > available {
                return Ok((
                    ReserveResult::InsufficientCredit {
                        available,
                        requested: amount.minor_units,
                    },
                    tx,
                ));
            }

            let reservation_id = ReservationId::new();

            sqlx::query!(
                r#"
                INSERT INTO credit_reservations
                    (id, saga_id, customer_id, amount_minor, currency, status)
                VALUES ($1, $2, $3, $4, $5, 'active')
                "#,
                reservation_id.as_uuid(),
                saga_id.as_uuid(),
                customer_id.as_uuid(),
                amount.minor_units,
                amount.currency,
            )
            .execute(tx.as_mut())
            .await?;

            tracing::info!(
                saga_id        = %saga_id,
                customer_id    = %customer_id,
                amount         = %amount,
                reservation_id = %reservation_id,
                "credit reserved"
            );

            Ok((ReserveResult::Reserved { reservation_id }, tx))
        })
        .await
    }

    pub async fn release_credit(
        &self,
        saga_id: SagaId,
        reservation_id: ReservationId,
    ) -> Result<(), CustomerError> {
        let rows = sqlx::query!(
            r#"
            UPDATE credit_reservations
            SET    status     = 'released',
                   updated_at = now()
            WHERE  id      = $1
            AND    saga_id  = $2
            AND    status   = 'active'
            "#,
            reservation_id.as_uuid(),
            saga_id.as_uuid(),
        )
        .execute(&self.pool)
        .await?
        .rows_affected();

        if rows == 0 {
            tracing::warn!(
                saga_id        = %saga_id,
                reservation_id = %reservation_id,
                "release_credit: no active reservation found — may be already released"
            );
        } else {
            tracing::info!(
                saga_id        = %saga_id,
                reservation_id = %reservation_id,
                "credit reservation released"
            );
        }

        Ok(())
    }
}
