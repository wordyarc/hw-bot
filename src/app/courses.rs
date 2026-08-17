use sqlx::PgPool;

use crate::app::users::load_actor;
use crate::app::{Actor, AppError, AppResult, Required};
use crate::db;
use crate::db::academic::TeachingCourseCard;
use crate::domain::{
    DomainError, InviteGeneration, StudentGroup, Subject, TeachingCourse, TeachingCourseId, UserId,
};

#[derive(Clone, Debug)]
pub struct TeachingCourseSummary {
    pub course: TeachingCourse,
    pub subject: Subject,
    pub group: StudentGroup,
}

#[derive(Clone, Debug)]
pub struct GroupInviteState {
    pub group: StudentGroup,
    pub generation: InviteGeneration,
}

impl From<TeachingCourseCard> for TeachingCourseSummary {
    fn from(card: TeachingCourseCard) -> Self {
        Self {
            course: card.course,
            subject: card.subject,
            group: card.group,
        }
    }
}

/// Ведомость курса
#[derive(Clone, Debug)]
pub struct CourseGradebook {
    pub students: Vec<StudentGrades>,
}

#[derive(Clone, Debug)]
pub struct StudentGrades {
    pub student_id: UserId,
    pub display_name: String,
    pub grades: Vec<db::submissions::CourseAssignmentGrade>,
}

impl StudentGrades {
    /// Непроверенная сдача в сумму не входит: у неё ещё нет балла, и это не ноль.
    pub fn total(&self) -> u32 {
        self.grades
            .iter()
            .filter_map(|grade| grade.final_score)
            .map(u32::from)
            .sum()
    }

    pub fn max_total(&self) -> u32 {
        self.grades
            .iter()
            .map(|grade| u32::from(grade.max_score))
            .sum()
    }
}

#[derive(Clone, Debug)]
pub struct CourseService {
    pool: PgPool,
}

impl CourseService {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    pub async fn list_for_teacher(&self, actor: Actor) -> AppResult<Vec<TeachingCourseSummary>> {
        let actor = load_actor(&self.pool, actor, "teacher").await?;

        if !actor.role().can_manage_assignments() {
            return Err(DomainError::PermissionDenied {
                action: "list teaching courses",
            }
            .into());
        }

        Ok(
            db::academic::list_teaching_courses_for_teacher(&self.pool, &actor)
                .await?
                .into_iter()
                .map(TeachingCourseSummary::from)
                .collect(),
        )
    }

    pub async fn get_group_invite(
        &self,
        actor: Actor,
        course_id: TeachingCourseId,
    ) -> AppResult<GroupInviteState> {
        let group = self
            .managed_group(actor, course_id, "read group invite")
            .await?;
        let generation = db::group_invites::find_generation(&self.pool, group.id())
            .await?
            .required("group invite")?;

        Ok(GroupInviteState { group, generation })
    }

    pub async fn rotate_group_invite(
        &self,
        actor: Actor,
        course_id: TeachingCourseId,
        expected: InviteGeneration,
    ) -> AppResult<GroupInviteState> {
        let group = self
            .managed_group(actor, course_id, "rotate group invite")
            .await?;
        let generation = match db::group_invites::rotate(&self.pool, group.id(), expected).await {
            Err(db::DbError::Conflict { .. }) => Err(AppError::GroupInviteChangedConcurrently),
            other => Ok(other?),
        }?;

        Ok(GroupInviteState { group, generation })
    }

    pub async fn gradebook(
        &self,
        actor: Actor,
        course_id: TeachingCourseId,
    ) -> AppResult<CourseGradebook> {
        let actor = load_actor(&self.pool, actor, "teacher").await?;
        let course = db::academic::find_teaching_course_by_id(&self.pool, course_id)
            .await?
            .required("teaching course")?;

        if !course.can_be_managed_by(&actor) {
            return Err(DomainError::PermissionDenied {
                action: "read course gradebook",
            }
            .into());
        }

        let rows = db::submissions::course_grades(&self.pool, course_id, course.student_group_id())
            .await?;

        let mut students: Vec<StudentGrades> = Vec::new();
        for row in rows {
            match students.last_mut() {
                Some(last) if last.student_id == row.student_id => {
                    last.grades.extend(row.assignment);
                }
                _ => students.push(StudentGrades {
                    student_id: row.student_id,
                    display_name: row.display_name,
                    grades: row.assignment.into_iter().collect(),
                }),
            }
        }

        Ok(CourseGradebook { students })
    }

    async fn managed_group(
        &self,
        actor: Actor,
        course_id: TeachingCourseId,
        action: &'static str,
    ) -> AppResult<StudentGroup> {
        let actor = load_actor(&self.pool, actor, "teacher").await?;
        let course = db::academic::find_teaching_course_by_id(&self.pool, course_id)
            .await?
            .required("teaching course")?;

        if !course.can_be_managed_by(&actor) {
            return Err(DomainError::PermissionDenied { action }.into());
        }

        db::academic::find_student_group_by_id(&self.pool, course.student_group_id())
            .await?
            .required("student group")
    }
}
