create table review_notification_outbox (
    id uuid primary key default uuidv7(),
    submission_id uuid not null unique references submissions (id) on delete restrict,
    recipient_telegram_user_id bigint not null references users (telegram_user_id) on delete restrict,
    assignment_title text not null check (
        btrim(assignment_title) <> '' and char_length(assignment_title) <= 200
    ),
    outcome text not null check (outcome in ('accepted', 'rejected')),
    review_comment text not null check (
        btrim(review_comment) <> '' and char_length(review_comment) <= 4096
    ),
    created_at timestamptz not null default now(),
    available_at timestamptz not null default now(),
    attempt_count integer not null default 0 check (attempt_count >= 0),
    lease_token uuid,
    lease_expires_at timestamptz,
    delivered_at timestamptz,
    failed_at timestamptz,
    last_error text,
    check ((lease_token is null) = (lease_expires_at is null)),
    check (delivered_at is null or failed_at is null)
);

create index review_notification_outbox_pending_idx
    on review_notification_outbox (available_at, created_at, id)
    where delivered_at is null and failed_at is null;
