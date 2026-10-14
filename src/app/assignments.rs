use std::collections::BTreeSet;

use chrono::{DateTime, Utc};
use sqlx::PgPool;

use crate::app::access::{
    ManagedAssignment, load_managed_assignment_on, load_managed_versioned_assignment,
    load_managing_teacher, load_managing_teacher_on, load_student,
};
use crate::app::{Actor, AppError, AppResult, Required};
use crate::db;
use crate::domain::{
    Assignment, AssignmentId, AssignmentStatus, DomainError, DomainResult, Grading, LatePolicy,
    MakeupOffer, MakeupTerms, NewAssignment, StoredFileMetadata, StudentAssignmentProgress,
    StudentGroupId, Subject, Submission, SubmissionHistory, TeachingCourse, TeachingCourseId, User,
    UserId,
};
use crate::pagination::{Page, PageRequest};
use crate::read_models::{AssignmentStudentSummary, MakeupPool, VersionedAssignment};

#[derive(Clone, Debug)]
pub struct CreateAssignment {
    pub teaching_course_id: TeachingCourseId,
    pub actor: Actor,
    pub title: String,
    pub body: String,
    pub student_group_ids: Option<BTreeSet<StudentGroupId>>,
    pub max_attempts: u16,
    pub grading: Grading,
    pub late_policy: Option<LatePolicy>,
    pub makeup: Option<MakeupTerms>,
    pub created_at: DateTime<Utc>,
    pub deadline_at: DateTime<Utc>,
}

#[derive(Clone, Debug)]
pub struct ChangeDeadline {
    pub assignment_id: AssignmentId,
    pub actor: Actor,
    pub deadline_at: DateTime<Utc>,
    pub expected_revision: Option<i64>,
}

#[derive(Clone, Debug)]
pub struct ChangeTerms {
    pub assignment_id: AssignmentId,
    pub actor: Actor,
    pub deadline_at: DateTime<Utc>,
    pub late_policy: Option<LatePolicy>,
    pub makeup: Option<MakeupTerms>,
    pub expected_revision: i64,
}

#[derive(Clone, Debug)]
pub struct ChangeGrading {
    pub assignment_id: AssignmentId,
    pub actor: Actor,
    pub grading: Grading,
    pub late_policy: Option<LatePolicy>,
    pub expected_revision: Option<i64>,
}

#[derive(Clone, Debug)]
pub struct GrantExtraAttempts {
    pub assignment_id: AssignmentId,
    pub student_id: UserId,
    pub actor: Actor,
    pub amount: u16,
}

#[derive(Clone, Debug)]
pub struct StudentAssignment {
    pub assignment: Assignment,
    pub subject: Subject,
    pub history: SubmissionHistory,
    pub makeup: Option<MakeupOffer>,
}

fn makeup_offer(
    assignment: &Assignment,
    history: SubmissionHistory,
    pool: Option<MakeupPool>,
    at: DateTime<Utc>,
) -> Option<MakeupOffer> {
    let pool = pool?;
    history.makeup_offer_at(assignment, at, pool.course, pool.balance)
}

#[derive(Clone, Debug)]
pub struct StudentAssignmentDetails {
    pub assignment: StudentAssignment,
    pub files: Vec<StoredFileMetadata>,
    pub latest_submission: Option<Submission>,
}

#[derive(Clone, Debug)]
pub struct AssignmentStudents {
    pub assignment: Assignment,
    pub students: Page<AssignmentStudentSummary>,
}

#[derive(Clone, Debug)]
pub struct AssignmentSubmissionHistory {
    pub subject_name: String,
    pub assignment_title: String,
    pub submissions: Page<Submission>,
}

#[derive(Clone, Debug)]
pub struct AssignmentService {
    pool: PgPool,
}

impl AssignmentService {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    pub async fn create(&self, command: CreateAssignment) -> AppResult<Assignment> {
        let mut tx = self.pool.begin().await?;

        let (author, teaching_course) = load_managing_teacher_on(
            &mut tx,
            command.actor,
            command.teaching_course_id,
            "create assignment",
        )
        .await?;

        let assignment = Assignment::new(
            AssignmentId::new(),
            &teaching_course,
            &author,
            NewAssignment {
                title: command.title,
                body: command.body,
                groups: command.student_group_ids,
                max_attempts: command.max_attempts,
                grading: command.grading,
                late_policy: command.late_policy,
                makeup: command.makeup,
                created_at: command.created_at,
                deadline_at: command.deadline_at,
            },
        )?;

        db::assignments::insert(&mut tx, &assignment).await?;

        tx.commit().await?;

        Ok(assignment)
    }

    pub async fn publish(
        &self,
        assignment_id: AssignmentId,
        actor: Actor,
        published_at: DateTime<Utc>,
    ) -> AppResult<Assignment> {
        self.change(assignment_id, actor, None, |assignment, course, actor| {
            assignment.publish(course, actor, published_at)
        })
        .await
    }

    pub async fn archive(
        &self,
        assignment_id: AssignmentId,
        actor: Actor,
    ) -> AppResult<Assignment> {
        self.change(assignment_id, actor, None, Assignment::archive)
            .await
    }

    pub async fn change_deadline(&self, command: ChangeDeadline) -> AppResult<Assignment> {
        self.change(
            command.assignment_id,
            command.actor,
            command.expected_revision,
            |assignment, course, actor| {
                assignment.change_deadline(course, actor, command.deadline_at)
            },
        )
        .await
    }

    pub async fn change_grading(&self, command: ChangeGrading) -> AppResult<Assignment> {
        self.change(
            command.assignment_id,
            command.actor,
            command.expected_revision,
            |assignment, course, actor| {
                assignment.change_grading(course, actor, command.grading, command.late_policy)
            },
        )
        .await
    }

    pub async fn change_terms(&self, command: ChangeTerms) -> AppResult<Assignment> {
        self.change(
            command.assignment_id,
            command.actor,
            Some(command.expected_revision),
            |assignment, course, actor| {
                assignment.change_terms(
                    course,
                    actor,
                    command.deadline_at,
                    command.late_policy,
                    command.makeup,
                )
            },
        )
        .await
    }

    pub async fn grant_extra_attempts(
        &self,
        command: GrantExtraAttempts,
    ) -> AppResult<StudentAssignmentProgress> {
        let mut tx = self.pool.begin().await?;

        let ManagedAssignment {
            actor,
            course,
            stored: VersionedAssignment { assignment, .. },
        } = load_managed_assignment_on(
            &mut tx,
            command.actor,
            command.assignment_id,
            "grant extra attempts",
        )
        .await?;

        let student = db::users::find_by_id(&mut *tx, command.student_id)
            .await?
            .required("student")?;

        let extra_attempts =
            db::attempts::lock_or_create(&mut tx, assignment.id(), student.id()).await?;
        let mut progress = StudentAssignmentProgress::new(&assignment, &student, extra_attempts)?;

        progress.grant_extra_attempts(&assignment, &course, &actor, command.amount)?;

        db::attempts::update(&mut tx, &progress).await?;
        tx.commit().await?;

        Ok(progress)
    }

    pub async fn list_for_student(
        &self,
        actor: Actor,
        page: PageRequest,
        now: DateTime<Utc>,
    ) -> AppResult<Page<StudentAssignment>> {
        self.list_for_group(actor, page, now, |group_id| {
            db::assignments::list_published_for_group(&self.pool, group_id, page)
        })
        .await
    }

    pub async fn list_archived_for_student(
        &self,
        actor: Actor,
        page: PageRequest,
        now: DateTime<Utc>,
    ) -> AppResult<Page<StudentAssignment>> {
        self.list_for_group(actor, page, now, |group_id| {
            db::assignments::list_archived_for_group(&self.pool, group_id, page)
        })
        .await
    }

    async fn list_for_group<F>(
        &self,
        actor: Actor,
        page: PageRequest,
        now: DateTime<Utc>,
        fetch: impl FnOnce(StudentGroupId) -> F,
    ) -> AppResult<Page<StudentAssignment>>
    where
        F: Future<Output = db::DbResult<Vec<db::assignments::StudentAssignmentCard>>>,
    {
        let student = self.student(actor).await?;
        let group_id = student.student_group_id().required("student group")?;

        let assignments = fetch(group_id).await?;
        self.with_student_histories(assignments, student.id(), now)
            .await
            .map(|items| Page::from_rows(items, page))
    }

    async fn with_student_histories(
        &self,
        cards: Vec<db::assignments::StudentAssignmentCard>,
        student_id: UserId,
        now: DateTime<Utc>,
    ) -> AppResult<Vec<StudentAssignment>> {
        let ids: Vec<_> = cards
            .iter()
            .map(|card| card.assignment.id().as_uuid())
            .collect();
        let mut courses: Vec<_> = cards
            .iter()
            .map(|card| card.assignment.teaching_course_id().as_uuid())
            .collect();
        courses.sort_unstable();
        courses.dedup();
        let mut histories = db::submissions::load_histories(&self.pool, &ids, student_id).await?;
        let pools = db::makeup::pools_for_student(&self.pool, &courses, student_id).await?;

        Ok(cards
            .into_iter()
            .map(|card| {
                let history = histories.remove(&card.assignment.id()).unwrap_or_default();
                let pool = pools.get(&card.assignment.teaching_course_id()).copied();
                StudentAssignment {
                    makeup: makeup_offer(&card.assignment, history, pool, now),
                    history,
                    assignment: card.assignment,
                    subject: card.subject,
                }
            })
            .collect())
    }

    pub async fn find_for_student(
        &self,
        actor: Actor,
        assignment_id: AssignmentId,
        now: DateTime<Utc>,
    ) -> AppResult<StudentAssignmentDetails> {
        let (student, assignment) =
            load_visible_assignment_for_student(&self.pool, actor, assignment_id).await?;

        let history =
            db::submissions::load_histories(&self.pool, &[assignment.id().as_uuid()], student.id())
                .await?
                .remove(&assignment.id())
                .unwrap_or_default();
        let latest_submission =
            db::submissions::latest_for_student(&self.pool, assignment.id(), student.id()).await?;
        let files = db::files::list_for_assignment(&self.pool, assignment.id()).await?;
        let (subject, _) =
            db::academic::find_subject_with_sandbox(&self.pool, assignment.subject_id())
                .await?
                .required("subject")?;
        let pool =
            db::makeup::pool_for_student(&self.pool, assignment.teaching_course_id(), student.id())
                .await?;

        Ok(StudentAssignmentDetails {
            assignment: StudentAssignment {
                makeup: makeup_offer(&assignment, history, pool, now),
                assignment,
                subject,
                history,
            },
            files,
            latest_submission,
        })
    }

    pub async fn find_for_teacher(
        &self,
        actor: Actor,
        assignment_id: AssignmentId,
    ) -> AppResult<Assignment> {
        self.load_managed_assignment(actor, assignment_id, "view assignment")
            .await
    }

    pub async fn submission_history(
        &self,
        actor: Actor,
        assignment_id: AssignmentId,
        page: PageRequest,
    ) -> AppResult<AssignmentSubmissionHistory> {
        let (student, assignment) =
            load_visible_assignment_for_student(&self.pool, actor, assignment_id).await?;
        let (subject, _) =
            db::academic::find_subject_with_sandbox(&self.pool, assignment.subject_id())
                .await?
                .required("subject")?;
        let submissions = Page::from_rows(
            db::submissions::list_for_student(&self.pool, assignment.id(), student.id(), page)
                .await?,
            page,
        );
        Ok(AssignmentSubmissionHistory {
            subject_name: subject.name().as_str().to_owned(),
            assignment_title: assignment.title().as_str().to_owned(),
            submissions,
        })
    }

    pub async fn list_students_for_assignment(
        &self,
        actor: Actor,
        assignment_id: AssignmentId,
        page: PageRequest,
    ) -> AppResult<AssignmentStudents> {
        let assignment = self
            .load_managed_assignment(actor, assignment_id, "list assignment students")
            .await?;
        let students = Page::from_rows(
            db::users::list_students_with_assignment_history(&self.pool, assignment.id(), page)
                .await?,
            page,
        );

        Ok(AssignmentStudents {
            assignment,
            students,
        })
    }

    /// Преподавателю нужны и черновики, и архив; ограничения студенческого списка здесь не действуют.
    pub async fn list_for_course(
        &self,
        actor: Actor,
        teaching_course_id: TeachingCourseId,
        page: PageRequest,
    ) -> AppResult<Page<Assignment>> {
        load_managing_teacher(
            &self.pool,
            actor,
            teaching_course_id,
            "list course assignments",
        )
        .await?;

        Ok(Page::from_rows(
            db::assignments::list_for_course(&self.pool, teaching_course_id, page).await?,
            page,
        ))
    }

    async fn student(&self, actor: Actor) -> AppResult<User> {
        load_student(&self.pool, actor, "list student assignments").await
    }

    async fn load_managed_assignment(
        &self,
        actor: Actor,
        assignment_id: AssignmentId,
        action: &'static str,
    ) -> AppResult<Assignment> {
        load_managed_versioned_assignment(&self.pool, actor, assignment_id, action)
            .await
            .map(|stored| stored.assignment)
    }

    pub async fn find_versioned_for_teacher(
        &self,
        actor: Actor,
        assignment_id: AssignmentId,
    ) -> AppResult<VersionedAssignment> {
        load_managed_versioned_assignment(&self.pool, actor, assignment_id, "read assignment").await
    }

    async fn change<F>(
        &self,
        assignment_id: AssignmentId,
        actor: Actor,
        expected_revision: Option<i64>,
        apply: F,
    ) -> AppResult<Assignment>
    where
        F: FnOnce(&mut Assignment, &TeachingCourse, &User) -> DomainResult<()>,
    {
        let mut tx = self.pool.begin().await?;

        let ManagedAssignment {
            actor,
            course: teaching_course,
            stored:
                VersionedAssignment {
                    mut assignment,
                    revision,
                },
        } = load_managed_assignment_on(&mut tx, actor, assignment_id, "change assignment").await?;

        if expected_revision.is_some_and(|expected| expected != revision) {
            return Err(AppError::AssignmentChangedConcurrently);
        }
        apply(&mut assignment, &teaching_course, &actor)?;

        super::on_conflict(
            db::assignments::update(&mut *tx, &assignment, revision).await,
            AppError::AssignmentChangedConcurrently,
        )?;

        tx.commit().await?;

        Ok(assignment)
    }
}

pub(crate) async fn load_visible_assignment_for_student(
    pool: &PgPool,
    actor: Actor,
    assignment_id: AssignmentId,
) -> AppResult<(User, Assignment)> {
    let student = load_student(pool, actor, "view student assignment").await?;

    let assignment = db::assignments::find_by_id(pool, assignment_id)
        .await?
        .required("assignment")?;

    // Возвращаем 404, чтобы не раскрывать студенту задание, которое никогда не публиковалось.
    if assignment.status() == AssignmentStatus::Draft
        || (assignment.status() == AssignmentStatus::Archived
            && assignment.published_at().is_none())
    {
        return Err(AppError::NotFound {
            resource: "assignment",
        });
    }
    if !assignment.is_issued_to(&student) {
        return Err(DomainError::StudentDoesNotBelongToAssignmentGroup.into());
    }

    Ok((student, assignment))
}
