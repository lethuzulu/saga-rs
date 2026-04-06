use crate::{db::CustomerDb, error::CustomerError, types::ReserveResult};
use rdkafka::{
    config::ClientConfig,
    consumer::{Consumer, StreamConsumer},
    message::Message,
    producer::{FutureProducer, FutureRecord},
};
use shared::saga::{SagaCommand, SagaReply, topics};
use std::time::Duration;
use tokio::sync::watch;

pub struct CommandConsumer {
    db: CustomerDb,
    consumer: StreamConsumer,
    producer: FutureProducer,
}

impl CommandConsumer {
    pub fn new(db: CustomerDb, brokers: &str, group_id: &str) -> Result<Self, CustomerError> {
        let consumer: StreamConsumer = ClientConfig::new()
            .set("bootstrap.servers", brokers)
            .set("group.id", group_id)
            .set("enable.auto.commit", "false")
            .set("auto.offset.reset", "earliest")
            .create()
            .map_err(|e| CustomerError::Kafka(e.to_string()))?;

        consumer
            .subscribe(&[topics::COMMANDS])
            .map_err(|e| CustomerError::Kafka(e.to_string()))?;

        let producer: FutureProducer = ClientConfig::new()
            .set("bootstrap.servers", brokers)
            .set("message.timeout.ms", "5000")
            .create()
            .map_err(|e| CustomerError::Kafka(e.to_string()))?;

        Ok(Self {
            db,
            consumer,
            producer,
        })
    }

    pub async fn run(self, mut shutdown: watch::Receiver<bool>) -> Result<(), CustomerError> {
        tracing::info!("command consumer started");

        loop {
            tokio::select! {
                _ = shutdown.changed() => {
                    if *shutdown.borrow() {
                        tracing::info!("command consumer shutting down");
                        return Ok(());
                    }
                }

                result = self.consumer.recv() => {
                    match result {
                        Err(e) => {
                            tracing::error!(error = %e, "kafka recv error");
                            continue;
                        }
                        Ok(msg) => {
                            let Some(payload) = msg.payload() else { continue; };

                            let command: SagaCommand = match serde_json::from_slice(payload) {
                                Ok(c)  => c,
                                Err(e) => {
                                    tracing::error!(error = %e, "failed to deserialize command");
                                    // Commit offset — bad message, cannot process
                                    let _ = self.consumer.commit_message(
                                        &msg, rdkafka::consumer::CommitMode::Async,
                                    );
                                    continue;
                                }
                            };

                            tracing::info!(
                                saga_id = %command.saga_id(),
                                command = ?command,
                                "received command"
                            );


                            let reply = self.handle_command(command).await;

                            match reply {
                                Ok(reply)  => {
                                    if let Err(e) = self.publish_reply(reply).await {
                                        tracing::error!(error = %e, "failed to publish reply");
                                        // Don't commit — orchestrator needs to retry
                                        continue;
                                    }
                                }
                                Err(e) => {
                                    tracing::error!(error = %e, "command handler error");
                                    continue;
                                }
                            }

                            if let Err(e) = self.consumer.commit_message(
                                &msg, rdkafka::consumer::CommitMode::Async,
                            ) {
                                tracing::error!(error = %e, "commit failed");
                            }
                        }
                    }
                }
            }
        }
    }

    async fn handle_command(&self, command: SagaCommand) -> Result<SagaReply, CustomerError> {
        match command {
            SagaCommand::ReserveCredit {
                saga_id,
                customer_id,
                amount,
            } => match self.db.reserve_credit(saga_id, customer_id, amount).await? {
                ReserveResult::Reserved { reservation_id } => Ok(SagaReply::CreditReserved {
                    saga_id,
                    reservation_id,
                }),

                ReserveResult::AlreadyReserved { reservation_id } => {
                    tracing::info!(
                        saga_id = %saga_id,
                        "idempotent replay — returning existing reservation"
                    );
                    Ok(SagaReply::CreditReserved {
                        saga_id,
                        reservation_id,
                    })
                }

                ReserveResult::InsufficientCredit {
                    available,
                    requested,
                } => Ok(SagaReply::CreditReservationFailed {
                    saga_id,
                    reason: format!(
                        "insufficient credit: requested {requested}, available {available}"
                    ),
                }),

                ReserveResult::CustomerNotFound => Ok(SagaReply::CreditReservationFailed {
                    saga_id,
                    reason: "customer not found".into(),
                }),
            },

            SagaCommand::ReleaseCredit {
                saga_id,
                reservation_id,
                ..
            } => match self.db.release_credit(saga_id, reservation_id).await {
                Ok(()) => Ok(SagaReply::CreditReleased { saga_id }),
                Err(e) => Ok(SagaReply::CreditReleaseFailed {
                    saga_id,
                    reason: e.to_string(),
                }),
            },
        }
    }

    async fn publish_reply(&self, reply: SagaReply) -> Result<(), CustomerError> {
        let saga_id = reply.saga_id();
        let payload = serde_json::to_string(&reply)?;
        let key = saga_id.to_string();

        self.producer
            .send(
                FutureRecord::to(topics::REPLIES)
                    .payload(payload.as_bytes())
                    .key(key.as_bytes()),
                Duration::from_secs(5),
            )
            .await
            .map_err(|(e, _)| CustomerError::Kafka(e.to_string()))?;

        tracing::info!(saga_id = %saga_id, reply = ?reply, "reply published");

        Ok(())
    }
}
