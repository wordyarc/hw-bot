alter table submissions
    add column withdrawn_at timestamptz,
    drop constraint submissions_status_check,
    add constraint submissions_status_is_known check (
        status in ('pending_review', 'accepted', 'rejected', 'withdrawn')
    ),
    drop constraint submissions_check,
    add constraint submissions_review_state_follows_status check (
        (
            status = 'pending_review'
            and reviewed_by is null
            and reviewed_at is null
            and review_comment is null
            and withdrawn_at is null
        )
        or (
            status = 'withdrawn'
            and reviewed_by is null
            and reviewed_at is null
            and review_comment is null
            and withdrawn_at is not null
        )
        or (
            status in ('accepted', 'rejected')
            and reviewed_by is not null
            and reviewed_at is not null
            and review_comment is not null
            and btrim(review_comment) <> ''
            and withdrawn_at is null
        )
    );
