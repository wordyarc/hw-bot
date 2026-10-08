use sqlx::PgPool;

use crate::app::access::load_managing_teacher;
use crate::app::users::load_actor;
use crate::app::{Actor, AppError, AppResult, Required};
use crate::db;
use crate::domain::{
    DomainError, InviteGeneration, StudentGroup, TeachingCourseId, TelegramUserId, UserId,
};
use crate::pagination::{Page, PageRequest};
use crate::read_models::{AssignmentGrade, CourseStudent, TeachingCourseSummary};

#[derive(Clone, Debug)]
pub struct GroupInviteState {
    pub group: StudentGroup,
    pub generation: InviteGeneration,
}

#[derive(Clone, Debug)]
pub struct StudentContact {
    pub student_id: UserId,
    pub telegram_user_id: TelegramUserId,
    pub display_name: String,
    pub group_code: String,
}

#[derive(Clone, Debug)]
pub struct StudentGrades {
    pub student_id: UserId,
    pub display_name: String,
    pub grades: Vec<AssignmentGrade>,
    pub grade_page: u32,
    pub next_grade_page: Option<u32>,
    work_total: u32,
    penalty_total: u32,
    max_total: u32,
}

impl StudentGrades {
    /// До проверки балл отсутствует, а не равен нулю. Такая сдача не участвует в сумме.
    pub fn work_total(&self) -> u32 {
        self.work_total
    }

    pub fn penalty_total(&self) -> u32 {
        self.penalty_total
    }

    pub fn total(&self) -> i64 {
        i64::from(self.work_total) - i64::from(self.penalty_total)
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
            db::academic::list_teaching_courses_for_teacher(&self.pool, &actor, page).await?,
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
        let generation = super::on_conflict(
            db::group_invites::rotate(&self.pool, group.id(), expected).await,
            AppError::GroupInviteChangedConcurrently,
        )?;

        Ok(GroupInviteState { group, generation })
    }

    pub async fn gradebook(
        &self,
        actor: Actor,
        course_id: TeachingCourseId,
        page: PageRequest,
    ) -> AppResult<Page<StudentGrades>> {
        let (_, course) =
            load_managing_teacher(&self.pool, actor, course_id, "read course gradebook").await?;

        let rows = db::submissions::course_gradebook_page(
            &self.pool,
            course_id,
            course.student_group_id(),
            page,
        )
        .await?;
        Ok(Page::from_rows(
            group_grades(rows, PageRequest::default()),
            page,
        ))
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
        ))
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
        let rows = db::submissions::one_student_grades(
            &self.pool,
            course_id,
            course.student_group_id(),
            student_id,
            page,
        )
        .await?;
        group_grades(rows, page)
            .into_iter()
            .next()
            .required("student")
    }

    pub async fn student_contact(
        &self,
        actor: Actor,
        course_id: TeachingCourseId,
        student_id: UserId,
    ) -> AppResult<StudentContact> {
        let group = self
            .managed_group(actor, course_id, "read student contact")
            .await?;
        let student = db::users::find_by_id(&self.pool, student_id)
            .await?
            .filter(|user| user.role().student_group_id() == Some(group.id()))
            .required("student")?;

        Ok(StudentContact {
            student_id: student.id(),
            telegram_user_id: student.telegram_user_id(),
            display_name: student.display_name().as_str().to_owned(),
            group_code: group.code().as_str().to_owned(),
        })
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
            Some(last) if last.student_id == row.student_id => last.grades.extend(row.assignment),
            _ => students.push(StudentGrades {
                student_id: row.student_id,
                display_name: row.display_name,
                grades: row.assignment.into_iter().collect(),
                work_total: row.work_total,
                penalty_total: row.penalty_total,
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
