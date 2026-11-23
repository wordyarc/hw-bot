use sqlx::PgExecutor;

use super::submissions::{GradeColumns, parse_kind};
use crate::db::{DbError, DbResult, checked_num_cast};
use crate::domain::{
    AssignmentId, DeductedFrom, Grade, Grading, StudentGroupId, SubmissionKind, TeachingCourseId,
    UserId,
};
use crate::read_models::{AcceptedWork, GradeSheetColumn, GradeSheetRow, SubmissionStates};

const MAX_SCORE: &str = "assignments.max_score";
const LATE_DEDUCTED_FROM: &str = "assignments.late_deducted_from";

pub async fn course_rows(
    executor: impl PgExecutor<'_>,
    course_id: TeachingCourseId,
) -> DbResult<Vec<GradeSheetRow>> {
    sqlx::query!(
        r#"
        with issued as (
            select issued.student_group_id, assignments.id, assignments.title,
                   assignments.deadline_at, assignments.max_score, assignments.late_deducted_from
            from assignment_groups issued
            join assignments on assignments.id = issued.assignment_id
            where issued.teaching_course_id = $1
              and assignments.published_at is not null
        ), course_submissions as (
            select submissions.student_id, submissions.assignment_id, submissions.status,
                   submissions.kind, submissions.raw_score, submissions.grade_max_score,
                   submissions.late_periods, submissions.penalty,
                   submissions.penalty_deducted_from, submissions.work_score,
                   submissions.course_total_penalty
            from submissions
            join assignments on assignments.id = submissions.assignment_id
            where assignments.teaching_course_id = $1
        ), best as (
            select distinct on (student_id, assignment_id)
                student_id, assignment_id, kind, raw_score, grade_max_score, late_periods,
                penalty, penalty_deducted_from, work_score, course_total_penalty
            from course_submissions
            where status = 'accepted'
            order by student_id, assignment_id,
                     work_score - course_total_penalty desc nulls last,
                     kind = 'regular' desc
        ), states as (
            select student_id, assignment_id,
                   bool_or(status = 'pending_review') as pending,
                   bool_or(status = 'rejected') as rejected
            from course_submissions
            group by student_id, assignment_id
        )
        select
            student_groups.id as "group_id!",
            student_groups.code as "group_code!",
            users.id as "student_id!",
            users.display_name as "display_name!",
            issued.id as "assignment_id?",
            issued.title as "title?",
            issued.max_score as "max_score?",
            issued.late_deducted_from as "late_deducted_from?",
            best.kind as "kind?",
            best.raw_score as "raw_score?",
            best.grade_max_score as "grade_max_score?",
            best.late_periods as "late_periods?",
            best.penalty as "penalty?",
            best.penalty_deducted_from as "penalty_deducted_from?",
            best.work_score as "work_score?",
            best.course_total_penalty as "course_total_penalty?",
            coalesce(states.pending, false) as "pending!",
            coalesce(states.rejected, false) as "rejected!"
        from course_participants participant
        join users on users.id = participant.student_id
        join student_groups on student_groups.id = participant.student_group_id
        left join issued on issued.student_group_id = participant.student_group_id
        left join best on best.student_id = users.id and best.assignment_id = issued.id
        left join states on states.student_id = users.id and states.assignment_id = issued.id
        where participant.teaching_course_id = $1
        order by student_groups.code, users.display_name, users.id,
                 issued.max_score is null, issued.deadline_at, issued.id
        "#,
        course_id.as_uuid(),
    )
    .fetch_all(executor)
    .await?
    .into_iter()
    .map(|row| {
        let assignment = match (row.assignment_id, row.title) {
            (Some(id), Some(title)) => {
                let column = GradeSheetColumn {
                    assignment_id: AssignmentId::from_uuid(id),
                    title,
                    grading: grading(row.max_score)?,
                    deducted_from: row
                        .late_deducted_from
                        .map(|value| {
                            DeductedFrom::parse_str(&value)
                                .ok_or_else(|| DbError::invalid_value(LATE_DEDUCTED_FROM, value))
                        })
                        .transpose()?,
                };
                let accepted = row
                    .kind
                    .map(|kind| {
                        let kind = parse_kind(&kind)?;
                        let grade = Grade::restore(
                            GradeColumns {
                                raw_score: row.raw_score,
                                max_score: row.grade_max_score,
                                late_periods: row.late_periods,
                                penalty: row.penalty,
                                penalty_deducted_from: row.penalty_deducted_from,
                                work_score: row.work_score,
                                course_total_penalty: row.course_total_penalty,
                            }
                            .parts()?,
                        )?;
                        Ok::<_, DbError>(AcceptedWork {
                            grade,
                            makeup: kind == SubmissionKind::Makeup,
                        })
                    })
                    .transpose()?;
                Some((
                    column,
                    SubmissionStates {
                        accepted,
                        pending: row.pending,
                        rejected: row.rejected,
                    },
                ))
            }
            _ => None,
        };

        Ok(GradeSheetRow {
            group_id: StudentGroupId::from_uuid(row.group_id),
            group_code: row.group_code,
            student_id: UserId::from_uuid(row.student_id),
            display_name: row.display_name,
            assignment,
        })
    })
    .collect()
}

fn grading(max_score: Option<i16>) -> DbResult<Grading> {
    max_score.map_or(Ok(Grading::PassFail), |value| {
        Ok(Grading::scored(checked_num_cast(MAX_SCORE, value)?)?)
    })
}
