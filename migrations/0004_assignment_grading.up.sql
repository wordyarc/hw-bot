-- Шкала оценивания задания, политика опозданий и замороженная арифметика оценки сдачи.

alter table assignments
    add column max_score smallint check (max_score is null or max_score between 1 and 100),
    add column late_period_days smallint check (
        late_period_days is null or late_period_days between 1 and 365
    ),
    add column late_penalty smallint check (late_penalty is null or late_penalty between 1 and 100),
    add column late_max_periods smallint check (
        late_max_periods is null or late_max_periods between 1 and 100
    ),
    add constraint assignments_late_policy_is_complete check (
        num_nonnulls(late_period_days, late_penalty, late_max_periods) in (0, 3)
    ),
    add constraint assignments_late_policy_requires_a_scale check (
        late_period_days is null or max_score is not null
    );

alter table submissions
    add column raw_score smallint,
    add column grade_max_score smallint,
    add column late_periods integer,
    add column penalty smallint,
    add column final_score smallint,
    add constraint submissions_grade_is_complete check (
        num_nonnulls(raw_score, grade_max_score, late_periods, penalty, final_score) in (0, 5)
    ),
    add constraint submissions_grade_belongs_to_accepted_submission check (
        raw_score is null
        or (
            status = 'accepted'
            and grade_max_score between 1 and 100
            and raw_score between 0 and grade_max_score
            and late_periods >= 0
            and penalty between 0 and 100
            and final_score = greatest(raw_score - penalty, 0)
        )
    );

alter table review_notification_outbox
    add column final_score smallint,
    add column max_score smallint,
    add constraint review_notification_outbox_score_is_complete check (
        num_nonnulls(final_score, max_score) in (0, 2)
    ),
    add constraint review_notification_outbox_score_fits_the_scale check (
        final_score is null
        or (max_score between 1 and 100 and final_score between 0 and max_score)
    );
