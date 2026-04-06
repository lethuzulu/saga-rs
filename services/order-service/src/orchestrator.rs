use rdkafka::{ClientConfig, producer::FutureProducer};

use crate::{db::OrderDb, error::OrderError, types::OrderStatus};
use rdkafka::{
    consumer::{Consumer, StreamConsumer},
    message::Message,
    producer::FutureRecord,
};
use shared::{
    ids::{OrderId, ReservationId, SagaId},
    saga::{SagaCommand, SagaEvent, SagaReply, SagaState, topics},
};
use std::{sync::Arc, time::Duration};
use tokio::sync::watch;

pub struct Orchestrator {
    db: OrderDb,
    producer: FutureProducer,
}

impl Orchestrator {
    pub fn new(db: OrderDb, brokers: &str) -> Result<Self, OrderError> {
        let producer: FutureProducer = ClientConfig::new()
            .set("bootstrap.servers", brokers)
            .set("message.timeout.ms", "5000")
            .create()
            .map_err(|e| OrderError::Kafka(e.to_string()))?;

        Ok(Self { db, producer })
    }

    pub async fn publish_command(&self, command: SagaCommand) -> Result<(), OrderError> {
        let saga_id = command.saga_id();
        let payload = serde_json::to_string(&command)?;
        let key = saga_id.to_string();

        self.producer
            .send(
                FutureRecord::to(topics::COMMANDS)
                    .payload(payload.as_bytes())
                    .key(key.as_bytes()),
                Duration::from_secs(5),
            )
            .await
            .map_err(|(e, _)| OrderError::Kafka(e.to_string()))?;

        tracing::info!(
            saga_id   = %saga_id,
            command   = ?command,
            "command published"
        );

        Ok(())
    }

    pub async fn step(&self, saga_id: SagaId, event: SagaEvent) -> Result<SagaState, OrderError> {
        let new_state = self.db.advance_saga(saga_id, event).await?;

        tracing::info!(saga_id = %saga_id, state = %new_state, "saga advanced");

        match &new_state {
            SagaState::Completed => {
                let saga = self.db.load_saga(saga_id).await?;
                self.db
                    .update_order_status(saga.order_id, OrderStatus::Approved)
                    .await?;
            }
            SagaState::CreditFailed { .. } | SagaState::Failed { .. } => {
                let saga = self.db.load_saga(saga_id).await?;
                self.db
                    .update_order_status(saga.order_id, OrderStatus::Rejected)
                    .await?;
            }
            _ => {}
        }

        let saga = self.db.load_saga(saga_id).await?;
        if let Some(command) = new_state.next_command(&saga) {
            self.publish_command(command).await?;
        }

        Ok(new_state)
    }

    
}
