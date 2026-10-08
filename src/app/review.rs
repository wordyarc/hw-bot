use std::collections::HashMap;
use std::collections::hash_map::Entry;

use chrono::{DateTime, Utc};
use sqlx::postgres::PgExecutor;
use sqlx::{PgConnection, PgPool};

use crate::app::courses::active_makeup_pool;
use crate::app::users::load_actor;
use crate::app::{Actor, AppResult, Required};
use crate::db;
use crate::domain::{
    Assignment, AssignmentId, DomainError, ReviewDecision, Subject, Submission, SubmissionId,
    SubmissionKind, TeachingCourse, TeachingCourseId, User,
};
use crate::pagination::{Page, PageRequest};
use crate::read_models::{MakeupPool, SandboxSubmission};

async fn load_reviewer<'e, E>(executor: E, actor: Actor) -> AppResult<User>
where
    E: PgExecutor<'e>,
{
    let reviewer = load_actor(executor, actor, "reviewer").await?;

    if !reviewer.role().can_review_homework() {
        return Err(DomainError::PermissionDenied {
            action: "review homework",
        }
        .into());
    }

    Ok(reviewer)
}

#[derive(Clone, Debug)]
pub struct ReviewSubmission {
    pub submission_id: SubmissionId,
    pub actor: Actor,
    pub decision: ReviewDecision,
    pub score: Option<u16>,
    pub comment: String,
    pub reviewed_at: DateTime<Utc>,
}

#[derive(Clone, Debug)]
pub struct ReviewSandbox {
    pub actor: Actor,
    pub decision: ReviewDecision,
    pub cutoff: DateTime<Utc>,
    pub comment: String,
    pub reviewed_at: DateTime<Utc>,
}

#[derive(Clone, Debug)]
pub struct ReviewedSubmission {
    pub submission: Submission,
    pub course_id: TeachingCourseId,
    pub sandbox: bool,
}

#[derive(Clone, Debug)]
pub struct ReviewQueueItem {
    pub submission: Submission,
    pub assignment: Assignment,
    pub subject: Subject,
    pub student: User,
    pub previous_attempts: Vec<Submission>,
    pub makeup: Option<MakeupPool>,
}

#[derive(Clone, Debug)]
pub struct ReviewService {
    pool: PgPool,
}

impl ReviewService {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    /// Работа не резервируется за ревьюером: к моменту его решения блокировки уже нет.
    /// При одновременной проверке `save_review_result` условным update запишет первое решение,
    /// а последующие отклонит с `SubmissionIsNotPendingReview`.
    pub async fn take_next_from_queue(&self, actor: Actor) -> AppResult<Option<ReviewQueueItem>> {
        let mut tx = self.pool.begin().await?;

        let reviewer = load_reviewer(&mut *tx, actor).await?;

        let Some(submission) =
            db::submissions::lock_next_pending_for_reviewer(&mut tx, &reviewer).await?
        else {
            return Ok(None);
        };

        let item = load_queue_item(&mut tx, submission).await?;
        tx.commit().await?;
        Ok(Some(item))
    }

    pub async fn take_next_makeup(&self, actor: Actor) -> AppResult<Option<ReviewQueueItem>> {
        let mut tx = self.pool.begin().await?;

        let reviewer = load_reviewer(&mut *tx, actor).await?;

        let Some(submission) =
            db::submissions::lock_next_makeup_for_reviewer(&mut tx, &reviewer).await?
        else {
            return Ok(None);
        };

        let item = load_queue_item(&mut tx, submission).await?;
        tx.commit().await?;
        Ok(Some(item))
    }

    /// Между подсчётом и `take_next_from_queue` работы могут разобрать; ненулевого счётчика недостаточно.
    pub async fn queue_size(&self, actor: Actor) -> AppResult<u32> {
        let reviewer = load_reviewer(&self.pool, actor).await?;

        Ok(db::submissions::count_pending_for_reviewer(&self.pool, &reviewer).await?)
    }

    pub async fn makeup_size(&self, actor: Actor) -> AppResult<u32> {
        let reviewer = load_reviewer(&self.pool, actor).await?;

        Ok(db::submissions::count_makeup_for_reviewer(&self.pool, &reviewer).await?)
    }

    pub async fn sandbox_size(
        &self,
        actor: Actor,
        cutoff: Option<DateTime<Utc>>,
    ) -> AppResult<u32> {
        let reviewer = load_reviewer(&self.pool, actor).await?;

        Ok(db::submissions::count_sandbox_for_reviewer(&self.pool, &reviewer, cutoff).await?)
    }

    pub async fn sandbox_page(
        &self,
        actor: Actor,
        page: PageRequest,
    ) -> AppResult<Page<SandboxSubmission>> {
        let reviewer = load_reviewer(&self.pool, actor).await?;

        Ok(Page::from_rows(
            db::submissions::sandbox_page_for_reviewer(&self.pool, &reviewer, page).await?,
            page,
        ))
    }

    pub async fn review_sandbox(&self, command: ReviewSandbox) -> AppResult<usize> {
        let mut tx = self.pool.begin().await?;

        let reviewer = load_reviewer(&mut *tx, command.actor).await?;
        let submissions =
            db::submissions::lock_sandbox_for_reviewer(&mut tx, &reviewer, command.cutoff).await?;
        let mut reviewed = 0;

        let mut contexts = HashMap::new();
        for mut submission in submissions {
            let context = match contexts.entry(submission.assignment_id()) {
                Entry::Occupied(entry) => entry.into_mut(),
                Entry::Vacant(entry) => {
                    entry.insert(load_review_context(&mut tx, submission.assignment_id()).await?)
                }
            };

            let score = match submission.kind() {
                SubmissionKind::Regular => context.assignment.grading().max_score(),
                SubmissionKind::Makeup => None,
            };
            let decided = match command.decision {
                ReviewDecision::Accept => submission.accept(
                    &context.assignment,
                    &context.course,
                    &reviewer,
                    score,
                    command.comment.as_str(),
                    command.reviewed_at,
                ),
                ReviewDecision::Reject => submission.reject(
                    &context.assignment,
                    &context.course,
                    &reviewer,
                    command.comment.as_str(),
                    command.reviewed_at,
                ),
            };
            match decided {
                Err(DomainError::MakeupNotOffered) => continue,
                decided => decided?,
            }
            reviewed += 1;

            db::submissions::save_review_result(&mut *tx, &submission).await?;

            let student = db::users::find_by_id(&mut *tx, submission.student_id())
                .await?
                .required("student")?;
            db::notifications::enqueue_review_result(
                &mut *tx,
                &submission,
                &context.assignment,
                &context.subject,
                &student,
            )
            .await?;
        }

        tx.commit().await?;

        Ok(reviewed)
    }

    pub async fn review_submission(
        &self,
        command: ReviewSubmission,
    ) -> AppResult<ReviewedSubmission> {
        let mut tx = self.pool.begin().await?;

        let reviewer = load_reviewer(&mut *tx, command.actor).await?;

        let mut submission = db::submissions::find_by_id(&mut *tx, command.submission_id)
            .await?
            .required("submission")?;

        let assignment = db::assignments::find_by_id(&mut *tx, submission.assignment_id())
            .await?
            .required("assignment")?;

        let teaching_course =
            db::academic::find_teaching_course_by_id(&mut *tx, assignment.teaching_course_id())
                .await?
                .required("teaching course")?;

        match command.decision {
            ReviewDecision::Accept => submission.accept(
                &assignment,
                &teaching_course,
                &reviewer,
                command.score,
                command.comment,
                command.reviewed_at,
            )?,
            ReviewDecision::Reject if command.score.is_some() => {
                return Err(DomainError::ScoreNotAllowed.into());
            }
            ReviewDecision::Reject => submission.reject(
                &assignment,
                &teaching_course,
                &reviewer,
                command.comment,
                command.reviewed_at,
            )?,
        }

        db::submissions::save_review_result(&mut *tx, &submission).await?;

        let student = db::users::find_by_id(&mut *tx, submission.student_id())
            .await?
            .required("student")?;
        let subject = db::academic::find_subject_by_id(&mut *tx, teaching_course.subject_id())
            .await?
            .required("subject")?;
        let sandbox = db::academic::find_subject_sandbox(&mut *tx, subject.id())
            .await?
            .required("subject")?;
        db::notifications::enqueue_review_result(
            &mut *tx,
            &submission,
            &assignment,
            &subject,
            &student,
        )
        .await?;

        tx.commit().await?;

        Ok(ReviewedSubmission {
            submission,
            course_id: teaching_course.id(),
            sandbox,
        })
    }

    pub async fn find_for_review(
        &self,
        actor: Actor,
        submission_id: SubmissionId,
    ) -> AppResult<ReviewQueueItem> {
        let mut connection = self.pool.acquire().await?;
        let reviewer = load_reviewer(&mut *connection, actor).await?;
        let submission = db::submissions::find_by_id(&mut *connection, submission_id)
            .await?
            .required("submission")?;
        let item = load_queue_item(&mut connection, submission).await?;
        let course = db::academic::find_teaching_course_by_id(
            &mut *connection,
            item.assignment.teaching_course_id(),
        )
        .await?
        .required("teaching course")?;
        item.assignment
            .ensure_managed_by(&course, &reviewer, "review homework")?;
        if !item.submission.is_pending_review() {
            return Err(DomainError::SubmissionIsNotPendingReview.into());
        }
        Ok(item)
    }
}

struct ReviewContext {
    assignment: Assignment,
    course: TeachingCourse,
    subject: Subject,
}

async fn load_review_context(
    connection: &mut PgConnection,
    assignment_id: AssignmentId,
) -> AppResult<ReviewContext> {
    let assignment = db::assignments::find_by_id(&mut *connection, assignment_id)
        .await?
        .required("assignment")?;
    let course =
        db::academic::find_teaching_course_by_id(&mut *connection, assignment.teaching_course_id())
            .await?
            .required("teaching course")?;
    let subject = db::academic::find_subject_by_id(&mut *connection, assignment.subject_id())
        .await?
        .required("subject")?;

    Ok(ReviewContext {
        assignment,
        course,
        subject,
    })
}

async fn load_queue_item(
    connection: &mut PgConnection,
    submission: Submission,
) -> AppResult<ReviewQueueItem> {
    let assignment = db::assignments::find_by_id(&mut *connection, submission.assignment_id())
        .await?
        .required("assignment")?;
    let subject = db::academic::find_subject_by_id(&mut *connection, assignment.subject_id())
        .await?
        .required("subject")?;
    let student = db::users::find_by_id(&mut *connection, submission.student_id())
        .await?
        .required("student")?;
    let mut previous_attempts: Vec<_> = db::submissions::list_for_student(
        &mut *connection,
        assignment.id(),
        student.id(),
        PageRequest::default(),
    )
    .await?
    .into_iter()
    .filter(|attempt| attempt.attempt_number() < submission.attempt_number())
    .collect();
    previous_attempts.reverse();
    let makeup = match submission.kind() {
        SubmissionKind::Makeup => {
            active_makeup_pool(
                &mut *connection,
                assignment.teaching_course_id(),
                student.id(),
                Utc::now(),
            )
            .await?
        }
        SubmissionKind::Regular => None,
    };
    Ok(ReviewQueueItem {
        submission,
        assignment,
        subject,
        student,
        previous_attempts,
        makeup,
    })
}
