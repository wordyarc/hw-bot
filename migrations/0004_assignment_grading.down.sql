drop table if exists student_course_progress;

drop index if exists submissions_one_accepted_per_student_assignment_kind_idx;

create unique index if not exists submissions_one_accepted_per_student_assignment_idx
    on submissions (assignment_id, student_id)
    where status = 'accepted';

alter table submissions
    drop constraint if exists submissions_makeup_is_not_late,
    drop constraint if exists submissions_grade_follows_penalty_deducted_from,
    drop constraint if exists submissions_penalty_deducted_from_marks_a_late_grade,
    drop constraint if exists submissions_grade_belongs_to_accepted_submission,
    drop constraint if exists submissions_grade_is_complete,
    drop column if exists course_total_penalty,
    drop column if exists work_score,
    drop column if exists penalty_deducted_from,
    drop column if exists penalty,
    drop column if exists late_periods,
    drop column if exists grade_max_score,
    drop column if exists raw_score,
    drop column if exists kind;

alter table assignments
    drop constraint if exists assignments_makeup_credit_follows_the_scale,
    drop constraint if exists assignments_makeup_credit_fits_the_scale,
    drop constraint if exists assignments_late_policy_requires_a_scale,
    drop constraint if exists assignments_late_policy_is_complete,
    drop column if exists makeup_credit,
    drop column if exists makeup_offered,
    drop column if exists late_deducted_from,
    drop column if exists late_max_periods,
    drop column if exists late_penalty_per_period,
    drop column if exists late_period_days,
    drop column if exists max_score;

alter table teaching_courses
    drop constraint if exists teaching_courses_makeup_ends_after_opening,
    drop constraint if exists teaching_courses_makeup_is_complete,
    drop column if exists makeup_opened_at,
    drop column if exists makeup_attempts,
    drop column if exists makeup_until;
