use crate::pagination::{Page, PageRequest};
use sqlx::PgPool;

use crate::app::access::load_managing_teacher;
use crate::app::users::load_actor;
use crate::app::{Actor, AppError, AppResult, Required};
use crate::db;
use crate::db::academic::TeachingCourseCard;
use crate::domain::{
    AssignmentId, DomainError, InviteGeneration, StudentGroup, Subject, TeachingCourse,
    TeachingCourseId, UserId,
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

#[derive(Clone, Debug)]
pub struct CourseGradebook {
    pub students: Page<StudentGrades>,
}

#[derive(Clone, Debug)]
pub struct CourseStudent {
    pub student_id: UserId,
    pub display_name: String,
}

#[derive(Clone, Debug)]
pub struct StudentGrades {
    pub student_id: UserId,
    pub display_name: String,
    pub grades: Vec<AssignmentGrade>,
    pub grade_page: u32,
    pub next_grade_page: Option<u32>,
    total: u32,
    max_total: u32,
}

#[derive(Clone, Debug)]
pub struct AssignmentGrade {
    pub assignment_id: AssignmentId,
    pub title: String,
    pub max_score: u16,
    pub final_score: Option<u16>,
}

impl From<db::submissions::CourseAssignmentGrade> for AssignmentGrade {
    fn from(grade: db::submissions::CourseAssignmentGrade) -> Self {
        Self {
            assignment_id: grade.assignment_id,
            title: grade.title,
            max_score: grade.max_score,
            final_score: grade.final_score,
        }
    }
}

impl StudentGrades {
    /// Непроверенная сдача в сумму не входит: у неё ещё нет балла, и это не ноль.
    pub fn total(&self) -> u32 {
        self.total
    }

    pub fn max_total(&self) -> u32 {
        self.max_total
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

    pub async fn list_for_teacher(
        &self,
        actor: Actor,
        page: PageRequest,
    ) -> AppResult<Page<TeachingCourseSummary>> {
        let actor = load_actor(&self.pool, actor, "teacher").await?;

        if !actor.role().can_manage_assignments() {
            return Err(DomainError::PermissionDenied {
                action: "list teaching courses",
            }
            .into());
        }

        Ok(Page::from_rows(
            db::academic::list_teaching_courses_for_teacher(&self.pool, &actor, page)
                .await?
                .into_iter()
                .map(TeachingCourseSummary::from)
                .collect(),
            page,
        ))
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
        page: PageRequest,
    ) -> AppResult<CourseGradebook> {
        let (_, course) =
            load_managing_teacher(&self.pool, actor, course_id, "read course gradebook").await?;

        let rows = db::submissions::course_grades(
            &self.pool,
            course_id,
            course.student_group_id(),
            page,
            None,
            PageRequest::default(),
        )
        .await?;
        Ok(CourseGradebook {
            students: Page::from_rows(group_grades(rows, PageRequest::default()), page),
        })
    }

    pub async fn list_students(
        &self,
        actor: Actor,
        course_id: TeachingCourseId,
        page: PageRequest,
    ) -> AppResult<Page<CourseStudent>> {
        let (_, course) =
            load_managing_teacher(&self.pool, actor, course_id, "read course students").await?;
        Ok(Page::from_rows(
            db::users::list_student_names(&self.pool, course.student_group_id(), page).await?,
            page,
        )
        .map(|student| CourseStudent {
            student_id: student.student_id,
            display_name: student.display_name,
        }))
    }

    pub async fn student_grades(
        &self,
        actor: Actor,
        course_id: TeachingCourseId,
        student_id: UserId,
        page: PageRequest,
    ) -> AppResult<StudentGrades> {
        let (_, course) =
            load_managing_teacher(&self.pool, actor, course_id, "read student grades").await?;
        let rows = db::submissions::course_grades(
            &self.pool,
            course_id,
            course.student_group_id(),
            PageRequest::default(),
            Some(student_id),
            page,
        )
        .await?;
        group_grades(rows, page)
            .into_iter()
            .next()
            .required("student")
    }

    async fn managed_group(
        &self,
        actor: Actor,
        course_id: TeachingCourseId,
        action: &'static str,
    ) -> AppResult<StudentGroup> {
        let (_, course) = load_managing_teacher(&self.pool, actor, course_id, action).await?;

        db::academic::find_student_group_by_id(&self.pool, course.student_group_id())
            .await?
            .required("student group")
    }
}

fn group_grades(
    rows: Vec<db::submissions::CourseGradeRow>,
    page: PageRequest,
) -> Vec<StudentGrades> {
    let mut students: Vec<StudentGrades> = Vec::new();
    for row in rows {
        match students.last_mut() {
            Some(last) if last.student_id == row.student_id => last
                .grades
                .extend(row.assignment.map(AssignmentGrade::from)),
            _ => students.push(StudentGrades {
                student_id: row.student_id,
                display_name: row.display_name,
                grades: row
                    .assignment
                    .into_iter()
                    .map(AssignmentGrade::from)
                    .collect(),
                total: row.total,
                max_total: row.max_total,
                grade_page: page.page,
                next_grade_page: None,
            }),
        }
    }
    for student in &mut students {
        let grades = Page::from_rows(std::mem::take(&mut student.grades), page);
        student.grades = grades.items;
        student.next_grade_page = grades.next_page;
    }
    students
}
