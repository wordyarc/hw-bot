use std::collections::HashMap;
use std::collections::hash_map::Entry;

use chrono::{DateTime, Utc};
use sqlx::postgres::PgExecutor;
use sqlx::{PgConnection, PgPool};

use crate::app::courses::active_makeup_pool;
use crate::app::users::load_actor;
use crate::app::{Actor, AppResult, Required};
use crate::db;
use crate::db::submissions::ReviewStream;
use crate::domain::{
    Assignment, AssignmentId, DomainError, Review, ReviewDecision, ReviewScore, Subject,
    Submission, SubmissionId, SubmissionKind, TeachingCourse, TeachingCourseId, User,
};
use crate::pagination::{Page, PageRequest};
use crate::read_models::{MakeupPool, SandboxSubmission};

const PREVIOUS_ATTEMPTS_SHOWN: u16 = 10;

async fn load_reviewer(executor: impl PgExecutor<'_>, actor: Actor) -> AppResult<User> {
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
    pub score: Option<ReviewScore>,
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
    pub group_code: String,
    pub previous_attempts: Vec<Submission>,
    pub earlier_attempts_hidden: bool,
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
        self.take_next(actor, ReviewStream::Main).await
    }

    pub async fn take_next_makeup(&self, actor: Actor) -> AppResult<Option<ReviewQueueItem>> {
        self.take_next(actor, ReviewStream::Makeup).await
    }

    /// Между подсчётом и `take_next_from_queue` работы могут разобрать; ненулевого счётчика недостаточно.
    pub async fn queue_size(&self, actor: Actor) -> AppResult<u32> {
        self.count(actor, ReviewStream::Main, None).await
    }

    pub async fn makeup_size(&self, actor: Actor) -> AppResult<u32> {
        self.count(actor, ReviewStream::Makeup, None).await
    }

    pub async fn sandbox_size(
        &self,
        actor: Actor,
        cutoff: Option<DateTime<Utc>>,
    ) -> AppResult<u32> {
        self.count(actor, ReviewStream::Sandbox, cutoff).await
    }

    async fn take_next(
        &self,
        actor: Actor,
        stream: ReviewStream,
    ) -> AppResult<Option<ReviewQueueItem>> {
        let mut tx = self.pool.begin().await?;

        let reviewer = load_reviewer(&mut *tx, actor).await?;

        let Some(submission) =
            db::submissions::lock_next_for_reviewer(&mut tx, &reviewer, stream).await?
        else {
            return Ok(None);
        };

        let context = load_review_context(&mut tx, submission.assignment_id()).await?;
        let item = load_queue_item(&mut tx, submission, context).await?;
        tx.commit().await?;
        Ok(Some(item))
    }

    async fn count(
        &self,
        actor: Actor,
        stream: ReviewStream,
        cutoff: Option<DateTime<Utc>>,
    ) -> AppResult<u32> {
        let reviewer = load_reviewer(&self.pool, actor).await?;

        Ok(db::submissions::count_for_reviewer(&self.pool, &reviewer, stream, cutoff).await?)
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

            let score = match (command.decision, submission.kind()) {
                (ReviewDecision::Accept, SubmissionKind::Regular) => context
                    .assignment
                    .grading()
                    .max_score()
                    .map(|max_score| ReviewScore {
                        value: max_score,
                        max_score,
                    }),
                (ReviewDecision::Accept, SubmissionKind::Makeup) | (ReviewDecision::Reject, _) => {
                    None
                }
            };
            let decided = submission.review(
                &context.assignment,
                &context.course,
                &reviewer,
                Review {
                    decision: command.decision,
                    score,
                    comment: command.comment.clone(),
                    reviewed_at: command.reviewed_at,
                },
            );
            match decided {
                Err(DomainError::MakeupNotOffered) => continue,
                decided => decided?,
            }
            reviewed += 1;

            record_review(&mut tx, &submission, context).await?;
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
        let context = load_review_context(&mut tx, submission.assignment_id()).await?;

        submission.review(
            &context.assignment,
            &context.course,
            &reviewer,
            Review {
                decision: command.decision,
                score: command.score,
                comment: command.comment,
                reviewed_at: command.reviewed_at,
            },
        )?;

        record_review(&mut tx, &submission, &context).await?;

        tx.commit().await?;

        Ok(ReviewedSubmission {
            submission,
            course_id: context.course.id(),
            sandbox: context.sandbox,
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
        let context = load_review_context(&mut connection, submission.assignment_id()).await?;
        context
            .assignment
            .ensure_managed_by(&context.course, &reviewer, "review homework")?;
        if !submission.is_pending_review() {
            return Err(DomainError::SubmissionIsNotPendingReview.into());
        }
        load_queue_item(&mut connection, submission, context).await
    }
}

struct ReviewContext {
    assignment: Assignment,
    course: TeachingCourse,
    subject: Subject,
    sandbox: bool,
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
    let (subject, sandbox) =
        db::academic::find_subject_with_sandbox(&mut *connection, assignment.subject_id())
            .await?
            .required("subject")?;

    Ok(ReviewContext {
        assignment,
        course,
        subject,
        sandbox,
    })
}

async fn record_review(
    connection: &mut PgConnection,
    submission: &Submission,
    context: &ReviewContext,
) -> AppResult<()> {
    db::submissions::save_review_result(&mut *connection, submission).await?;

    let student = db::users::find_by_id(&mut *connection, submission.student_id())
        .await?
        .required("student")?;
    db::notifications::enqueue_review_result(
        &mut *connection,
        submission,
        &context.assignment,
        &context.subject,
        &student,
    )
    .await?;

    Ok(())
}

async fn load_queue_item(
    connection: &mut PgConnection,
    submission: Submission,
    context: ReviewContext,
) -> AppResult<ReviewQueueItem> {
    let student = db::users::find_by_id(&mut *connection, submission.student_id())
        .await?
        .required("student")?;
    let group_id = student.student_group_id().required("student group")?;
    let group = db::academic::find_student_group_by_id(&mut *connection, group_id)
        .await?
        .required("student group")?;
    let mut previous_attempts = db::submissions::list_previous_attempts(
        &mut *connection,
        &submission,
        PREVIOUS_ATTEMPTS_SHOWN + 1,
    )
    .await?;
    let earlier_attempts_hidden = previous_attempts.len() > usize::from(PREVIOUS_ATTEMPTS_SHOWN);
    previous_attempts.truncate(usize::from(PREVIOUS_ATTEMPTS_SHOWN));
    previous_attempts.reverse();
    let makeup = match submission.kind() {
        SubmissionKind::Makeup => {
            active_makeup_pool(
                &mut *connection,
                context.course.id(),
                student.id(),
                Utc::now(),
            )
            .await?
        }
        SubmissionKind::Regular => None,
    };
    Ok(ReviewQueueItem {
        submission,
        assignment: context.assignment,
        subject: context.subject,
        student,
        group_code: group.code().as_str().to_owned(),
        previous_attempts,
        earlier_attempts_hidden,
        makeup,
    })
}
