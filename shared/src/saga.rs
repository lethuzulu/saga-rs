use serde::{Deserialize, Serialize};

use crate::{
    ids::{CustomerId, OrderId, ReservationId, SagaId},
    money::Money,
};

// the saga state machine
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum SagaState {
    // forward states
    Started,
    ReservingCredit,
    CreditReserved,
    ApprovingOrder,
    Completed,

    // failure states - no compensation needed
    CreditFailed { reason: String },

    // compensation states
    ReleasingCredit { reason: String },

    // terminal failure - reached after compensation
    Failed { reason: String },
}

impl SagaState {
    pub fn is_terminal(&self) -> bool {
        matches!(
            self,
            Self::Completed | SagaState::Failed { .. } | SagaState::CreditFailed { .. }
        )
    }

    pub fn next_command(&self, saga: &SagaData) -> Option<SagaCommand> {
        match self {
            SagaState::ReservingCredit => Some(SagaCommand::ReserveCredit {
                saga_id: saga.id,
                customer_id: saga.customer_id,
                amount: saga.amount.clone(),
            }),
            SagaState::ReleasingCredit { .. } => {
                saga.reservation_id
                    .map(|reservation_id| SagaCommand::ReleaseCredit {
                        saga_id: saga.id,
                        customer_id: saga.customer_id,
                        reservation_id,
                    })
            }
            _ => None,
        }
    }
}

impl std::fmt::Display for SagaState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SagaState::Started => write!(f, "started"),
            SagaState::ReservingCredit => write!(f, "reserving_credit"),
            SagaState::CreditReserved => write!(f, "credit_reserved"),
            SagaState::ApprovingOrder => write!(f, "approving_order"),
            SagaState::Completed => write!(f, "completed"),
            SagaState::CreditFailed { .. } => write!(f, "credit_failed"),
            SagaState::ReleasingCredit { .. } => write!(f, "releasing_credit"),
            SagaState::Failed { .. } => write!(f, "failed"),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum SagaEvent {
    SagaCreated,
    CreditReserved,
    CreditReservationFailed { reason: String },
    OrderApproved,
    OrderApprovalFailed { reason: String },
    CreditReleased,
    CreditReleaseFailed { reason: String },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum SagaCommand {
    ReserveCredit {
        saga_id: SagaId,
        customer_id: CustomerId,
        amount: Money,
    },
    ReleaseCredit {
        saga_id: SagaId,
        customer_id: CustomerId,
        reservation_id: ReservationId,
    },
}

impl SagaCommand {
    pub fn saga_id(&self) -> SagaId {
        match self {
            SagaCommand::ReserveCredit { saga_id, .. } => *saga_id,
            SagaCommand::ReleaseCredit { saga_id, .. } => *saga_id,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum SagaReply {
    CreditReserved {
        saga_id: SagaId,
        reservation_id: ReservationId,
    },
    CreditReservationFailed {
        saga_id: SagaId,
        reason: String,
    },
    CreditReleased {
        saga_id: SagaId,
    },
    CreditReleaseFailed {
        saga_id: SagaId,
        reason: String,
    },
}

impl SagaReply {
    pub fn saga_id(&self) -> SagaId {
        match self {
            SagaReply::CreditReserved { saga_id, .. } => *saga_id,
            SagaReply::CreditReservationFailed { saga_id, .. } => *saga_id,
            SagaReply::CreditReleased { saga_id, .. } => *saga_id,
            SagaReply::CreditReleaseFailed { saga_id, .. } => *saga_id,
        }
    }

    // convert a SagaReply to the SagaEvent it triggers in the orchestrator.
    pub fn to_event(self) -> SagaEvent {
        match self {
            SagaReply::CreditReserved { .. } => SagaEvent::CreditReserved,
            SagaReply::CreditReservationFailed { reason, .. } => {
                SagaEvent::CreditReservationFailed { reason }
            }
            SagaReply::CreditReleased { .. } => SagaEvent::CreditReleased,
            SagaReply::CreditReleaseFailed { reason, .. } => {
                SagaEvent::CreditReleaseFailed { reason }
            }
        }
    }
}

// the full saga record loaded from Postgres
#[derive(Debug, Clone)]
pub struct SagaData {
    pub id: SagaId,
    pub order_id: OrderId,
    pub customer_id: CustomerId,
    pub amount: Money,
    pub state: SagaState,
    pub reservation_id: Option<ReservationId>,
}

// pure state transition function
// takes the current state and an event and returns the next state
pub fn apply(state: SagaState, event: SagaEvent) -> Result<SagaState, SagaError> {
    match (state, event) {
        // Started => ReservingCredit
        (SagaState::Started, SagaEvent::SagaCreated) => Ok(SagaState::ReservingCredit),

        // ReservingCredit => CreditReserved (happy path)
        (SagaState::ReservingCredit, SagaEvent::CreditReserved) => Ok(SagaState::CreditReserved),

        // ReservingCredit => CreditFailed (insufficient credit)
        (SagaState::ReservingCredit, SagaEvent::CreditReservationFailed { reason }) => {
            Ok(SagaState::CreditFailed { reason })
        }

        // CreditReserved => ApprovingOrder
        (SagaState::CreditReserved, SagaEvent::OrderApproved) => Ok(SagaState::ApprovingOrder),

        // ApprovingOrder => Completed (happy path)
        (SagaState::ApprovingOrder, SagaEvent::OrderApproved) => Ok(SagaState::Completed),

        // ApprovingOrder => ReleasingCredit (approval failed — must compensate)
        (SagaState::ApprovingOrder, SagaEvent::OrderApprovalFailed { reason }) => {
            Ok(SagaState::ReleasingCredit { reason })
        }

        // ReleasingCredit => Failed (compensation complete)
        (SagaState::ReleasingCredit { .. }, SagaEvent::CreditReleased) => Ok(SagaState::Failed {
            reason: "order approval failed — credit released".into(),
        }),

        // Any terminal state => error (no transitions from terminal)
        (state, event) if state.is_terminal() => Err(SagaError::InvalidTransition {
            from: format!("{state:?}"),
            event: format!("{event:?}"),
        }),

        // Any other combination is invalid
        (state, event) => Err(SagaError::InvalidTransition {
            from: format!("{state:?}"),
            event: format!("{event:?}"),
        }),
    }
}

#[derive(Debug, thiserror::Error)]
pub enum SagaError {
    #[error("invalid transition from {from} with event {event}")]
    InvalidTransition { from: String, event: String },

    #[error("serialization error: {0}")]
    Serialization(#[from] serde_json::Error),
}




#[cfg(test)]
mod tests {
    use crate::{
        ids::{CustomerId, OrderId, ReservationId, SagaId},
        money::Money,
        saga::{apply, SagaError, SagaEvent, SagaState},
    };

    // Valid transitions

    #[test]
    fn started_saga_created_transitions_to_reserving_credit() {
        let next = apply(SagaState::Started, SagaEvent::SagaCreated).unwrap();
        assert_eq!(next, SagaState::ReservingCredit);
    }

    #[test]
    fn reserving_credit_credit_reserved_transitions_to_credit_reserved() {
        let next = apply(SagaState::ReservingCredit, SagaEvent::CreditReserved).unwrap();
        assert_eq!(next, SagaState::CreditReserved);
    }

    #[test]
    fn reserving_credit_reservation_failed_transitions_to_credit_failed() {
        let reason = "insufficient funds".to_string();
        let next = apply(
            SagaState::ReservingCredit,
            SagaEvent::CreditReservationFailed {
                reason: reason.clone(),
            },
        )
        .unwrap();
        assert_eq!(next, SagaState::CreditFailed { reason });
    }

    #[test]
    fn credit_reserved_order_approved_transitions_to_approving_order() {
        let next = apply(SagaState::CreditReserved, SagaEvent::OrderApproved).unwrap();
        assert_eq!(next, SagaState::ApprovingOrder);
    }

    #[test]
    fn approving_order_order_approved_transitions_to_completed() {
        let next = apply(SagaState::ApprovingOrder, SagaEvent::OrderApproved).unwrap();
        assert_eq!(next, SagaState::Completed);
    }

    #[test]
    fn approving_order_order_approval_failed_transitions_to_releasing_credit() {
        let reason = "order rejected".to_string();
        let next = apply(
            SagaState::ApprovingOrder,
            SagaEvent::OrderApprovalFailed {
                reason: reason.clone(),
            },
        )
        .unwrap();
        assert_eq!(next, SagaState::ReleasingCredit { reason });
    }

    #[test]
    fn releasing_credit_credit_released_transitions_to_failed() {
        let next = apply(
            SagaState::ReleasingCredit {
                reason: "order rejected".into(),
            },
            SagaEvent::CreditReleased,
        )
        .unwrap();
        assert_eq!(
            next,
            SagaState::Failed {
                reason: "order approval failed — credit released".into(),
            }
        );
    }

    // Invalid transitions — wrong event for state

    #[test]
    fn started_with_wrong_event_is_invalid() {
        let result = apply(SagaState::Started, SagaEvent::CreditReserved);
        assert!(matches!(result, Err(SagaError::InvalidTransition { .. })));
    }

    #[test]
    fn reserving_credit_with_wrong_event_is_invalid() {
        let result = apply(SagaState::ReservingCredit, SagaEvent::OrderApproved);
        assert!(matches!(result, Err(SagaError::InvalidTransition { .. })));
    }

    #[test]
    fn credit_reserved_with_wrong_event_is_invalid() {
        let result = apply(SagaState::CreditReserved, SagaEvent::CreditReserved);
        assert!(matches!(result, Err(SagaError::InvalidTransition { .. })));
    }

    #[test]
    fn approving_order_with_wrong_event_is_invalid() {
        let result = apply(SagaState::ApprovingOrder, SagaEvent::CreditReserved);
        assert!(matches!(result, Err(SagaError::InvalidTransition { .. })));
    }

    #[test]
    fn releasing_credit_with_wrong_event_is_invalid() {
        let result = apply(
            SagaState::ReleasingCredit {
                reason: "some reason".into(),
            },
            SagaEvent::CreditReservationFailed {
                reason: "irrelevant".into(),
            },
        );
        assert!(matches!(result, Err(SagaError::InvalidTransition { .. })));
    }

    // Invalid transitions — terminal states reject all events

    #[test]
    fn completed_rejects_any_event() {
        let result = apply(SagaState::Completed, SagaEvent::OrderApproved);
        assert!(matches!(result, Err(SagaError::InvalidTransition { .. })));
    }

    #[test]
    fn credit_failed_rejects_any_event() {
        let result = apply(
            SagaState::CreditFailed {
                reason: "no funds".into(),
            },
            SagaEvent::CreditReserved,
        );
        assert!(matches!(result, Err(SagaError::InvalidTransition { .. })));
    }

    #[test]
    fn failed_rejects_any_event() {
        let result = apply(
            SagaState::Failed {
                reason: "done".into(),
            },
            SagaEvent::CreditReleased,
        );
        assert!(matches!(result, Err(SagaError::InvalidTransition { .. })));
    }
}