alter table submissions
    drop constraint if exists submissions_review_state_follows_status,
    drop constraint if exists submissions_status_is_known,
    add constraint submissions_status_check check (
        status in ('pending_review', 'accepted', 'rejected')
    ),
    add constraint submissions_check check (
        (
            status = 'pending_review'
            and reviewed_by is null
            and reviewed_at is null
            and review_comment is null
        )
        or (
            status in ('accepted', 'rejected')
            and reviewed_by is not null
            and reviewed_at is not null
            and review_comment is not null
            and btrim(review_comment) <> ''
        )
    ),
    drop column if exists withdrawn_at;
