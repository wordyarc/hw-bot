alter table review_notification_outbox
    drop constraint if exists review_notification_outbox_score_fits_the_scale,
    drop constraint if exists review_notification_outbox_score_is_complete,
    drop column if exists max_score,
    drop column if exists final_score;

alter table submissions
    drop constraint if exists submissions_grade_belongs_to_accepted_submission,
    drop constraint if exists submissions_grade_is_complete,
    drop column if exists final_score,
    drop column if exists penalty,
    drop column if exists late_periods,
    drop column if exists raw_score;

alter table assignments
    drop constraint if exists assignments_late_policy_requires_a_scale,
    drop constraint if exists assignments_late_policy_is_complete,
    drop column if exists late_max_periods,
    drop column if exists late_penalty,
    drop column if exists late_period_days,
    drop column if exists max_score;
