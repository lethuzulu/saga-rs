
CREATE TABLE orders (
    id           UUID        PRIMARY KEY,
    customer_id  UUID        NOT NULL,
    amount_minor BIGINT      NOT NULL,
    currency     TEXT        NOT NULL DEFAULT 'GBP',
    status       TEXT        NOT NULL DEFAULT 'pending',
    created_at   TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at   TIMESTAMPTZ NOT NULL DEFAULT now(),

    CONSTRAINT amount_positive CHECK (amount_minor > 0),
    CONSTRAINT valid_status    CHECK (status IN ('pending', 'approved', 'rejected'))
);



CREATE TABLE sagas (
    id              UUID        PRIMARY KEY,
    order_id        UUID        NOT NULL REFERENCES orders(id),
    customer_id     UUID        NOT NULL,
    amount_minor    BIGINT      NOT NULL,
    currency        TEXT        NOT NULL DEFAULT 'GBP',
    state           JSONB       NOT NULL,
    reservation_id  UUID,
    created_at      TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at      TIMESTAMPTZ NOT NULL DEFAULT now()
);

-- Index for recovery query — finds all non-terminal sagas on startup.
CREATE INDEX sagas_state_idx ON sagas ((state->>'state'))
    WHERE (state->>'state') NOT IN ('completed', 'failed', 'credit_failed');

CREATE TABLE saga_history (
    id          BIGSERIAL   PRIMARY KEY,
    saga_id     UUID        NOT NULL REFERENCES sagas(id),
    from_state  JSONB       NOT NULL,
    event       JSONB       NOT NULL,
    to_state    JSONB       NOT NULL,
    occurred_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX saga_history_saga_idx ON saga_history (saga_id, occurred_at);
