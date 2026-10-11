use chrono::{DateTime, Utc};
use sqlx::{PgConnection, PgExecutor, PgPool};

use crate::app::access::{load_course_student, load_managing_teacher, load_managing_teacher_on};
use crate::app::users::load_actor;
use crate::app::{Actor, AppError, AppResult, Required};
use crate::db;
use crate::domain::{
    AssignmentId, AssignmentStatus, CourseMakeup, DomainError, Grading, InviteGeneration,
    MakeupTerms, StudentGroup, StudentGroupId, Subject, TeachingCourse, TeachingCourseId,
    TelegramUserId, User, UserId,
};
use crate::pagination::{Page, PageRequest};
use crate::read_models::{
    AssignmentGrade, CourseStudent, MakeupPool, TeachingCourseSummary, UndecidedAssignment,
    VersionedAssignment,
};

#[derive(Clone, Debug)]
pub struct CourseInviteState {
    pub subject: Subject,
    pub groups: Vec<StudentGroup>,
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
pub struct StudentGradesCard {
    pub grades: StudentGrades,
    pub group_id: StudentGroupId,
    pub makeup: Option<MakeupPool>,
}

#[derive(Clone, Debug)]
pub struct CourseMakeupState {
    pub saved: Option<CourseMakeup>,
    pub undecided: Vec<UndecidedAssignment>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct MakeupSettings {
    pub until: DateTime<Utc>,
    pub attempts: u16,
}

#[derive(Clone, Debug)]
pub struct MakeupDecision {
    pub grading: Grading,
    pub assignment_ids: Vec<AssignmentId>,
    pub terms: MakeupTerms,
}

#[derive(Clone, Debug)]
pub struct ConfigureMakeup {
    pub course_id: TeachingCourseId,
    pub actor: Actor,
    pub expected: Option<CourseMakeup>,
    pub settings: Option<MakeupSettings>,
    pub decisions: Vec<MakeupDecision>,
    pub now: DateTime<Utc>,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct MakeupQuestion {
    pub enabled: bool,
    pub hint: Option<u16>,
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

    pub async fn find_for_teacher(
        &self,
        actor: Actor,
        course_id: TeachingCourseId,
    ) -> AppResult<TeachingCourseSummary> {
        self.managed_summary(actor, course_id, "read teaching course")
            .await
    }

    pub async fn get_course_invite(
        &self,
        actor: Actor,
        course_id: TeachingCourseId,
    ) -> AppResult<CourseInviteState> {
        let summary = self
            .managed_summary(actor, course_id, "read course invite")
            .await?;
        let generation = db::course_invites::find_generation(&self.pool, course_id)
            .await?
            .required("course invite")?;

        Ok(CourseInviteState {
            subject: summary.subject,
            groups: summary.groups,
            generation,
        })
    }

    pub async fn rotate_course_invite(
        &self,
        actor: Actor,
        course_id: TeachingCourseId,
        expected: InviteGeneration,
    ) -> AppResult<CourseInviteState> {
        let summary = self
            .managed_summary(actor, course_id, "rotate course invite")
            .await?;
        let generation = super::on_conflict(
            db::course_invites::rotate(&self.pool, course_id, expected).await,
            AppError::CourseInviteChangedConcurrently,
        )?;

        Ok(CourseInviteState {
            subject: summary.subject,
            groups: summary.groups,
            generation,
        })
    }

    pub async fn gradebook(
        &self,
        actor: Actor,
        course_id: TeachingCourseId,
        group_id: StudentGroupId,
        page: PageRequest,
    ) -> AppResult<Page<StudentGrades>> {
        self.ensure_course_group(actor, course_id, group_id, "read course gradebook")
            .await?;

        let rows =
            db::submissions::course_gradebook_page(&self.pool, course_id, group_id, page).await?;
        Ok(Page::from_rows(
            group_grades(rows, PageRequest::default()),
            page,
        ))
    }

    pub async fn list_students(
        &self,
        actor: Actor,
        course_id: TeachingCourseId,
        group_id: StudentGroupId,
        page: PageRequest,
    ) -> AppResult<Page<CourseStudent>> {
        self.ensure_course_group(actor, course_id, group_id, "read course students")
            .await?;
        Ok(Page::from_rows(
            db::users::list_student_names(&self.pool, group_id, page).await?,
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
        let (student, group_id) = load_course_student(&self.pool, &course, student_id).await?;

        load_student_grades(&self.pool, &course, group_id, student.id(), page).await
    }

    pub async fn student_grades_card(
        &self,
        actor: Actor,
        course_id: TeachingCourseId,
        student_id: UserId,
        page: PageRequest,
    ) -> AppResult<StudentGradesCard> {
        let (_, course) =
            load_managing_teacher(&self.pool, actor, course_id, "read student grades").await?;
        let (student, group_id) = load_course_student(&self.pool, &course, student_id).await?;
        let grades = load_student_grades(&self.pool, &course, group_id, student.id(), page).await?;
        let makeup = active_makeup_pool(&self.pool, course_id, student.id(), Utc::now()).await?;

        Ok(StudentGradesCard {
            grades,
            group_id,
            makeup,
        })
    }

    pub async fn student_contact(
        &self,
        actor: Actor,
        course_id: TeachingCourseId,
        student_id: UserId,
    ) -> AppResult<StudentContact> {
        let (_, course) =
            load_managing_teacher(&self.pool, actor, course_id, "read student contact").await?;
        let (student, group_id) = load_course_student(&self.pool, &course, student_id).await?;
        let group = db::academic::find_student_group_by_id(&self.pool, group_id)
            .await?
            .required("student group")?;

        Ok(StudentContact {
            student_id: student.id(),
            telegram_user_id: student.telegram_user_id(),
            display_name: student.display_name().as_str().to_owned(),
            group_code: group.code().as_str().to_owned(),
        })
    }

    pub async fn active_makeup(
        &self,
        actor: Actor,
        course_id: TeachingCourseId,
    ) -> AppResult<Option<CourseMakeup>> {
        load_managing_teacher(&self.pool, actor, course_id, "read course makeup").await?;

        let now = Utc::now();
        Ok(db::makeup::find_course(&self.pool, course_id)
            .await?
            .filter(|makeup| makeup.is_active_at(now)))
    }

    pub async fn makeup_state(
        &self,
        actor: Actor,
        course_id: TeachingCourseId,
    ) -> AppResult<CourseMakeupState> {
        load_managing_teacher(&self.pool, actor, course_id, "read course makeup").await?;

        Ok(CourseMakeupState {
            saved: db::makeup::find_course(&self.pool, course_id).await?,
            undecided: db::makeup::undecided_assignments(&self.pool, course_id).await?,
        })
    }

    pub async fn makeup_question(
        &self,
        actor: Actor,
        course_id: TeachingCourseId,
        grading: Grading,
        except: Option<AssignmentId>,
    ) -> AppResult<MakeupQuestion> {
        load_managing_teacher(&self.pool, actor, course_id, "read course makeup").await?;

        let now = Utc::now();
        let enabled = db::makeup::find_course(&self.pool, course_id)
            .await?
            .is_some_and(|makeup| makeup.is_active_at(now));
        let hint = match grading {
            Grading::Score { max_score } if enabled => {
                db::makeup::common_credit(&self.pool, course_id, max_score, except).await?
            }
            Grading::Score { .. } | Grading::PassFail => None,
        };

        Ok(MakeupQuestion { enabled, hint })
    }

    pub async fn configure_makeup(
        &self,
        command: ConfigureMakeup,
    ) -> AppResult<Option<CourseMakeup>> {
        let mut tx = self.pool.begin().await?;

        let (actor, course) = load_managing_teacher_on(
            &mut tx,
            command.actor,
            command.course_id,
            "configure course makeup",
        )
        .await?;
        let active = command
            .expected
            .filter(|current| current.is_active_at(command.now));
        let settings = match (command.settings, active) {
            (None, _) => None,
            (Some(settings), None) => Some(CourseMakeup::open(
                settings.until,
                settings.attempts,
                command.now,
            )?),
            (Some(settings), Some(current)) => {
                Some(current.change(settings.until, settings.attempts, command.now)?)
            }
        };

        let settings = super::on_conflict(
            db::makeup::update_course(&mut *tx, course.id(), command.expected, settings).await,
            AppError::CourseMakeupChangedConcurrently,
        )?;
        if active.is_none() && settings.is_some() {
            db::makeup::clear_pool(&mut *tx, course.id()).await?;
        }
        for decision in &command.decisions {
            for &assignment_id in &decision.assignment_ids {
                decide_makeup(&mut tx, &course, &actor, assignment_id, decision).await?;
            }
        }

        tx.commit().await?;

        Ok(settings)
    }

    pub async fn student_makeup(
        &self,
        actor: Actor,
        course_id: TeachingCourseId,
        student_id: UserId,
    ) -> AppResult<Option<MakeupPool>> {
        let (_, course) =
            load_managing_teacher(&self.pool, actor, course_id, "read student makeup").await?;
        let (student, _) = load_course_student(&self.pool, &course, student_id).await?;

        active_makeup_pool(&self.pool, course_id, student.id(), Utc::now()).await
    }

    pub async fn grant_makeup_attempt(
        &self,
        actor: Actor,
        course_id: TeachingCourseId,
        student_id: UserId,
    ) -> AppResult<MakeupPool> {
        let mut tx = self.pool.begin().await?;

        let (_, course) =
            load_managing_teacher_on(&mut tx, actor, course_id, "grant makeup attempt").await?;
        let (student, _) = load_course_student(&mut *tx, &course, student_id).await?;
        let makeup = db::makeup::find_course(&mut *tx, course_id)
            .await?
            .ok_or(DomainError::MakeupNotOffered)?;

        let extra = db::makeup::lock_pool(&mut tx, course_id, student.id()).await?;
        db::makeup::set_extra_attempts(
            &mut tx,
            course_id,
            student.id(),
            makeup.grant_attempt(extra, Utc::now())?,
        )
        .await?;
        let pool = db::makeup::pool_for_student(&mut *tx, course_id, student.id())
            .await?
            .ok_or(DomainError::MakeupNotOffered)?;

        tx.commit().await?;

        Ok(pool)
    }

    async fn managed_summary(
        &self,
        actor: Actor,
        course_id: TeachingCourseId,
        action: &'static str,
    ) -> AppResult<TeachingCourseSummary> {
        load_managing_teacher(&self.pool, actor, course_id, action).await?;

        db::academic::find_teaching_course_summary(&self.pool, course_id)
            .await?
            .required("teaching course")
    }

    async fn ensure_course_group(
        &self,
        actor: Actor,
        course_id: TeachingCourseId,
        group_id: StudentGroupId,
        action: &'static str,
    ) -> AppResult<()> {
        let (_, course) = load_managing_teacher(&self.pool, actor, course_id, action).await?;
        if !course.includes_group(group_id) {
            return Err(AppError::NotFound {
                resource: "student group",
            });
        }

        Ok(())
    }
}

pub(super) async fn active_makeup_pool<'e, E>(
    executor: E,
    course_id: TeachingCourseId,
    student_id: UserId,
    now: DateTime<Utc>,
) -> AppResult<Option<MakeupPool>>
where
    E: PgExecutor<'e>,
{
    Ok(
        db::makeup::pool_for_student(executor, course_id, student_id)
            .await?
            .filter(|pool| pool.course.is_active_at(now)),
    )
}

async fn load_student_grades(
    pool: &PgPool,
    course: &TeachingCourse,
    group_id: StudentGroupId,
    student_id: UserId,
    page: PageRequest,
) -> AppResult<StudentGrades> {
    let rows =
        db::submissions::one_student_grades(pool, course.id(), group_id, student_id, page).await?;
    group_grades(rows, page)
        .into_iter()
        .next()
        .required("student")
}

async fn decide_makeup(
    connection: &mut PgConnection,
    course: &TeachingCourse,
    actor: &User,
    assignment_id: AssignmentId,
    decision: &MakeupDecision,
) -> AppResult<()> {
    let Some(VersionedAssignment {
        mut assignment,
        revision,
    }) = db::assignments::find_versioned_by_id(&mut *connection, assignment_id).await?
    else {
        return Ok(());
    };
    if assignment.teaching_course_id() != course.id()
        || assignment.status() == AssignmentStatus::Archived
        || assignment.makeup().is_some()
        || assignment.grading() != decision.grading
    {
        return Ok(());
    }
    assignment.decide_makeup(course, actor, decision.terms)?;

    match db::assignments::update(&mut *connection, &assignment, revision).await {
        Err(db::DbError::Conflict { .. }) => Ok(()),
        result => Ok(result?),
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
