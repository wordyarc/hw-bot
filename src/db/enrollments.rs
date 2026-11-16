use sqlx::postgres::PgExecutor;
use uuid::Uuid;

use crate::db::DbResult;
use crate::domain::{Participation, StudentGroupId, Subject, SubjectId, TeachingCourseId, UserId};
use crate::read_models::OfferedCourse;

pub async fn insert(
    executor: impl PgExecutor<'_>,
    student_id: UserId,
    course_ids: &[TeachingCourseId],
) -> DbResult<()> {
    let course_ids: Vec<Uuid> = course_ids.iter().map(|id| id.as_uuid()).collect();

    sqlx::query!(
        r#"
        insert into teaching_course_students (teaching_course_id, student_id)
        select course_id, $1
        from unnest($2::uuid[]) as chosen (course_id)
        on conflict (teaching_course_id, student_id) do nothing
        "#,
        student_id.as_uuid(),
        &course_ids,
    )
    .execute(executor)
    .await?;

    Ok(())
}

pub async fn participation(
    executor: impl PgExecutor<'_>,
    course_id: TeachingCourseId,
    student_id: UserId,
) -> DbResult<Participation> {
    let participates = sqlx::query_scalar!(
        r#"
        select exists (
            select 1
            from course_participants
            where teaching_course_id = $1
              and student_id = $2
        ) as "participates!"
        "#,
        course_id.as_uuid(),
        student_id.as_uuid(),
    )
    .fetch_one(executor)
    .await?;

    Ok(if participates {
        Participation::Participant
    } else {
        Participation::Outsider
    })
}

pub async fn offered_courses(
    executor: impl PgExecutor<'_>,
    group_id: StudentGroupId,
    not_enrolled: Option<UserId>,
) -> DbResult<Vec<OfferedCourse>> {
    sqlx::query!(
        r#"
        select course_groups.teaching_course_id, subjects.id as subject_id, subjects.name
        from teaching_course_groups course_groups
        join subjects
          on subjects.id = course_groups.subject_id
        where course_groups.student_group_id = $1
          and not subjects.sandbox
          and (
              $2::uuid is null
              or not exists (
                  select 1
                  from teaching_course_students enrolled
                  where enrolled.teaching_course_id = course_groups.teaching_course_id
                    and enrolled.student_id = $2
              )
          )
        order by subjects.name, course_groups.teaching_course_id
        "#,
        group_id.as_uuid(),
        not_enrolled.map(UserId::as_uuid),
    )
    .fetch_all(executor)
    .await?
    .into_iter()
    .map(|row| {
        Ok(OfferedCourse {
            course_id: TeachingCourseId::from_uuid(row.teaching_course_id),
            subject: Subject::new(SubjectId::from_uuid(row.subject_id), row.name)?,
        })
    })
    .collect()
}

pub async fn subjects_of_student(
    executor: impl PgExecutor<'_>,
    student_id: UserId,
) -> DbResult<Vec<Subject>> {
    sqlx::query!(
        r#"
        select subjects.id, subjects.name
        from teaching_course_students enrolled
        join users
          on users.id = enrolled.student_id
        join teaching_course_groups course_groups
          on course_groups.teaching_course_id = enrolled.teaching_course_id
         and course_groups.student_group_id = users.student_group_id
        join subjects
          on subjects.id = course_groups.subject_id
        where enrolled.student_id = $1
          and not subjects.sandbox
        order by subjects.name, subjects.id
        "#,
        student_id.as_uuid(),
    )
    .fetch_all(executor)
    .await?
    .into_iter()
    .map(|row| Ok(Subject::new(SubjectId::from_uuid(row.id), row.name)?))
    .collect()
}
