
CREATE TABLE customers (
    id           UUID        PRIMARY KEY DEFAULT gen_random_uuid(),
    name         TEXT        NOT NULL,
    email        TEXT        UNIQUE,
    credit_limit BIGINT      NOT NULL,
    currency     TEXT        NOT NULL DEFAULT 'GBP',
    created_at   TIMESTAMPTZ NOT NULL DEFAULT now(),

    CONSTRAINT credit_limit_positive CHECK (credit_limit > 0)
);

CREATE TABLE credit_reservations (
    id           UUID        PRIMARY KEY DEFAULT gen_random_uuid(),
    saga_id      UUID        NOT NULL UNIQUE,  -- idempotency key
    customer_id  UUID        NOT NULL REFERENCES customers(id),
    amount_minor BIGINT      NOT NULL,
    currency     TEXT        NOT NULL DEFAULT 'GBP',
    status       TEXT        NOT NULL DEFAULT 'active',
    created_at   TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at   TIMESTAMPTZ NOT NULL DEFAULT now(),

    CONSTRAINT amount_positive CHECK (amount_minor > 0),
    CONSTRAINT valid_status    CHECK (status IN ('active', 'released'))
);

-- Index for available credit calculation
CREATE INDEX reservations_customer_active_idx
    ON credit_reservations (customer_id)
    WHERE status = 'active';

-- Seed data — test customers
INSERT INTO customers (id, name, email, credit_limit, currency) VALUES
    ('00000000-0000-0000-0000-000000000001', 'Alice',   'alice@example.com', 100000, 'GBP'),
    ('00000000-0000-0000-0000-000000000002', 'Bob',     'bob@example.com',   50000,  'GBP'),
    ('00000000-0000-0000-0000-000000000003', 'Charlie', 'charlie@example.com', 200000, 'GBP');
