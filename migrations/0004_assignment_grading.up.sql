-- Шкала оценивания задания, политика опозданий и замороженная арифметика оценки сдачи.

alter table teaching_courses
    add column makeup_until timestamptz,
    add column makeup_attempts smallint check (
        makeup_attempts is null or makeup_attempts between 1 and 100
    ),
    add column makeup_opened_at timestamptz,
    add constraint teaching_courses_makeup_is_complete check (
        num_nonnulls(makeup_until, makeup_attempts, makeup_opened_at) in (0, 3)
    ),
    add constraint teaching_courses_makeup_ends_after_opening check (
        makeup_until > makeup_opened_at
    );

alter table assignments
    add column max_score smallint check (max_score is null or max_score between 1 and 100),
    add column late_period_days smallint check (
        late_period_days is null or late_period_days between 1 and 365
    ),
    add column late_penalty_per_period smallint check (
        late_penalty_per_period is null or late_penalty_per_period between 1 and 100
    ),
    add column late_max_periods smallint check (
        late_max_periods is null or late_max_periods between 1 and 100
    ),
    add column late_deducted_from text check (
        late_deducted_from is null or late_deducted_from in ('work_score', 'course_total')
    ),
    add column makeup_offered boolean,
    add column makeup_credit smallint,
    add constraint assignments_late_policy_is_complete check (
        num_nonnulls(
            late_period_days, late_penalty_per_period, late_max_periods, late_deducted_from
        ) in (0, 4)
    ),
    add constraint assignments_late_policy_requires_a_scale check (
        late_period_days is null or max_score is not null
    ),
    add constraint assignments_makeup_credit_fits_the_scale check (
        makeup_credit is null
        or (
            makeup_offered is true
            and max_score is not null
            and makeup_credit between 1 and max_score
        )
    ),
    add constraint assignments_makeup_credit_follows_the_scale check (
        makeup_offered is not true or (max_score is null) = (makeup_credit is null)
    );

alter table submissions
    add column kind text not null check (kind in ('regular', 'makeup')),
    add column raw_score smallint,
    add column grade_max_score smallint,
    add column late_periods integer,
    add column penalty smallint,
    add column penalty_deducted_from text check (
        penalty_deducted_from is null or penalty_deducted_from in ('work_score', 'course_total')
    ),
    add column work_score smallint,
    add column course_total_penalty smallint,
    add constraint submissions_grade_is_complete check (
        num_nonnulls(
            raw_score, grade_max_score, late_periods, penalty, work_score, course_total_penalty
        ) in (0, 6)
    ),
    add constraint submissions_grade_belongs_to_accepted_submission check (
        raw_score is null
        or (
            status = 'accepted'
            and grade_max_score between 1 and 100
            and raw_score between 0 and grade_max_score
            and late_periods >= 0
            and penalty between 0 and 100
            and (late_periods > 0) = (penalty > 0)
        )
    ),
    add constraint submissions_penalty_deducted_from_marks_a_late_grade check (
        (penalty_deducted_from is not null) = (raw_score is not null and late_periods > 0)
    ),
    add constraint submissions_grade_follows_penalty_deducted_from check (
        raw_score is null
        or (
            work_score = case penalty_deducted_from
                when 'work_score' then greatest(raw_score - penalty, 0)
                else raw_score
            end
            and course_total_penalty = case penalty_deducted_from
                when 'course_total' then penalty
                else 0
            end
        )
    ),
    add constraint submissions_makeup_is_not_late check (
        kind = 'regular' or late_periods is null or (late_periods = 0 and penalty = 0)
    );

drop index submissions_one_accepted_per_student_assignment_idx;

create unique index submissions_one_accepted_per_student_assignment_kind_idx
    on submissions (assignment_id, student_id, kind)
    where status = 'accepted';

create table student_course_progress (
    teaching_course_id uuid not null references teaching_courses (id) on delete restrict,
    student_id uuid not null references users (id) on delete restrict,
    extra_makeup_attempts smallint not null default 0 check (
        extra_makeup_attempts between 0 and 100
    ),
    primary key (teaching_course_id, student_id)
);
