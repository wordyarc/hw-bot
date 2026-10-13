use chrono::{Duration, Utc};
use hw_bot::app::{
    Actor, AppError, ChangeTerms, ConfigureMakeup, ErrorCode, MakeupDecision, MakeupPool,
    MakeupSettings, ReviewSubmission, SubmitHomework, SubmittedHomework,
};
use hw_bot::db;
use hw_bot::domain::{
    Assignment, AssignmentId, CourseMakeup, DeductedFrom, Grading, MakeupOffer, MakeupRequest,
    MakeupTerms, ReviewDecision, SubmissionId, SubmissionKind, User,
};
use hw_bot::pagination::PageRequest;
use hw_bot::test_support::{assignment as assignment_of, submission as submission_of};
use uuid::Uuid;

use crate::harness::{
    TestApp, insert_assignment, out_of, submission_id, wait_for_blocked_connection,
};

fn ten() -> Grading {
    Grading::scored(10).unwrap()
}

fn credit(value: u16) -> MakeupTerms {
    MakeupTerms::scored(value, ten()).unwrap()
}

fn for_days(days: i64, attempts: u16) -> MakeupSettings {
    MakeupSettings {
        until: Utc::now() + Duration::days(days),
        attempts,
    }
}

async fn configure(
    app: &TestApp,
    actor: Actor,
    expected: Option<CourseMakeup>,
    settings: Option<MakeupSettings>,
    decisions: Vec<MakeupDecision>,
) -> Result<Option<CourseMakeup>, AppError> {
    app.application
        .courses
        .configure_makeup(ConfigureMakeup {
            course_id: app.fixture.course.id(),
            actor,
            expected,
            settings,
            decisions,
            now: Utc::now(),
        })
        .await
}

async fn current_settings(app: &TestApp) -> Option<CourseMakeup> {
    app.application
        .courses
        .active_makeup(app.teacher_actor(), app.fixture.course.id())
        .await
        .unwrap()
}

async fn student_pool(app: &TestApp) -> Option<MakeupPool> {
    app.application
        .courses
        .student_makeup(
            app.teacher_actor(),
            app.fixture.course.id(),
            app.fixture.student.id(),
        )
        .await
        .unwrap()
}

async fn submit(
    app: &TestApp,
    assignment: &Assignment,
    allow_makeup: bool,
) -> Result<SubmittedHomework, AppError> {
    app.application
        .submissions
        .submit_homework(SubmitHomework {
            assignment_id: assignment.id(),
            actor: app.student_actor(),
            content: "pull/makeup".to_owned(),
            submitted_at: Utc::now(),
            allow_makeup,
        })
        .await
}

async fn makeup_terms(app: &TestApp, assignment: AssignmentId) -> (Option<MakeupTerms>, i64) {
    let stored = db::assignments::find_versioned_by_id(&app.pool, assignment)
        .await
        .unwrap()
        .unwrap();
    (stored.assignment.makeup(), stored.revision)
}

async fn count_submissions(app: &TestApp, assignment: &Assignment) -> i64 {
    sqlx::query_scalar("select count(*) from submissions where assignment_id = $1")
        .bind(assignment.id().as_uuid())
        .fetch_one(&app.pool)
        .await
        .unwrap()
}

#[tokio::test]
async fn a_teacher_of_another_course_cannot_configure_its_makeup() {
    let app = TestApp::start().await;
    let outsider = app.add_outsider_teacher("Outsider").await;

    let error = configure(
        &app,
        Actor::new(outsider.id()),
        None,
        Some(for_days(30, 3)),
        Vec::new(),
    )
    .await
    .unwrap_err();

    assert_eq!(error.code(), ErrorCode::PermissionDenied);
    assert_eq!(current_settings(&app).await, None);
}

#[tokio::test]
async fn enabling_again_starts_a_new_pool_while_a_changed_size_keeps_the_spent_attempts() {
    let app = TestApp::start().await;
    let work = app.add_closed_assignment("Closed", 10, 6).await;
    app.enable_makeup(1).await;
    submit(&app, &work, true).await.unwrap();
    assert_eq!(student_pool(&app).await.unwrap().balance.remaining(), 0);

    let opened = current_settings(&app).await.unwrap();
    configure(
        &app,
        app.teacher_actor(),
        Some(opened),
        Some(MakeupSettings {
            until: opened.until(),
            attempts: 2,
        }),
        Vec::new(),
    )
    .await
    .unwrap();
    let changed = student_pool(&app).await.unwrap();
    assert_eq!(changed.course.opened_at(), opened.opened_at());
    assert_eq!((changed.balance.allowed(), changed.balance.used()), (2, 1));

    configure(
        &app,
        app.teacher_actor(),
        current_settings(&app).await,
        None,
        Vec::new(),
    )
    .await
    .unwrap();
    assert_eq!(student_pool(&app).await, None);
    app.enable_makeup(1).await;

    let reopened = student_pool(&app).await.unwrap();
    assert!(reopened.course.opened_at() > opened.opened_at());
    assert_eq!(
        (reopened.balance.allowed(), reopened.balance.used()),
        (1, 0)
    );
}

#[tokio::test]
async fn a_save_opened_before_a_colleagues_save_changes_nothing() {
    let app = TestApp::start().await;
    let colleague = app.add_course_teacher("Colleague").await;
    let undecided = app.add_scored_assignment(10).await;
    let opened = current_settings(&app).await;

    configure(
        &app,
        Actor::new(colleague.id()),
        opened,
        Some(for_days(30, 3)),
        Vec::new(),
    )
    .await
    .unwrap();
    let error = configure(
        &app,
        app.teacher_actor(),
        opened,
        Some(for_days(20, 5)),
        vec![MakeupDecision {
            grading: ten(),
            assignment_ids: vec![undecided.id()],
            terms: credit(6),
        }],
    )
    .await
    .unwrap_err();

    assert_eq!(error.code(), ErrorCode::ConcurrentModification);
    assert_eq!(current_settings(&app).await.unwrap().attempts(), 3);
    assert_eq!(makeup_terms(&app, undecided.id()).await, (None, 0));
}

#[tokio::test]
async fn a_scale_decision_skips_changed_archived_and_other_scale_assignments() {
    let app = TestApp::start().await;
    let colleague = app.add_course_teacher("Colleague").await;
    let first = app.add_scored_assignment(10).await;
    let second = app.add_scored_assignment(10).await;
    let declined = app.add_scored_assignment(10).await;
    let archived = app.add_scored_assignment(10).await;
    let five = app.add_scored_assignment(5).await;
    app.application
        .assignments
        .change_terms(ChangeTerms {
            assignment_id: declined.id(),
            actor: Actor::new(colleague.id()),
            deadline_at: declined.deadline_at(),
            late_policy: None,
            makeup: Some(MakeupTerms::NotOffered),
            expected_revision: 0,
        })
        .await
        .unwrap();
    app.application
        .assignments
        .archive(archived.id(), app.teacher_actor())
        .await
        .unwrap();

    configure(
        &app,
        app.teacher_actor(),
        None,
        Some(for_days(30, 3)),
        vec![MakeupDecision {
            grading: ten(),
            assignment_ids: [&first, &second, &declined, &archived, &five]
                .map(Assignment::id)
                .to_vec(),
            terms: credit(6),
        }],
    )
    .await
    .unwrap();

    assert_eq!(makeup_terms(&app, first.id()).await, (Some(credit(6)), 1));
    assert_eq!(makeup_terms(&app, second.id()).await, (Some(credit(6)), 1));
    assert_eq!(
        makeup_terms(&app, declined.id()).await,
        (Some(MakeupTerms::NotOffered), 1)
    );
    assert_eq!(makeup_terms(&app, archived.id()).await, (None, 1));
    assert_eq!(makeup_terms(&app, five.id()).await, (None, 0));
}

#[tokio::test]
async fn a_makeup_attempt_is_granted_only_on_a_course_with_makeup() {
    let app = TestApp::start().await;

    let error = app
        .application
        .courses
        .grant_makeup_attempt(
            app.teacher_actor(),
            app.fixture.course.id(),
            app.fixture.student.id(),
        )
        .await
        .unwrap_err();

    assert_eq!(error.code(), ErrorCode::InvalidInput);
}

async fn spend_an_ended_makeup(app: &TestApp) -> CourseMakeup {
    let opened_at = Utc::now() - Duration::days(40);
    let work = app
        .add_makeup_assignment("Old", ten(), credit(6), opened_at - Duration::days(1))
        .await;
    let ended = app.open_makeup(&app.fixture.course, 1, opened_at).await;
    app.application
        .submissions
        .submit_homework(SubmitHomework {
            assignment_id: work.id(),
            actor: app.student_actor(),
            content: "pull/old".to_owned(),
            submitted_at: opened_at + Duration::days(1),
            allow_makeup: true,
        })
        .await
        .unwrap();
    sqlx::query("update student_course_progress set extra_makeup_attempts = 5")
        .execute(&app.pool)
        .await
        .unwrap();
    assert!(ended.until() < Utc::now());

    ended
}

#[tokio::test]
async fn an_ended_makeup_counts_as_disabled() {
    let app = TestApp::start().await;
    spend_an_ended_makeup(&app).await;
    let courses = &app.application.courses;

    assert_eq!(current_settings(&app).await, None);
    assert_eq!(student_pool(&app).await, None);
    let question = courses
        .makeup_question(app.teacher_actor(), app.fixture.course.id(), ten(), None)
        .await
        .unwrap();
    assert!(!question.enabled);
    let error = courses
        .grant_makeup_attempt(
            app.teacher_actor(),
            app.fixture.course.id(),
            app.fixture.student.id(),
        )
        .await
        .unwrap_err();
    assert_eq!(error.code(), ErrorCode::InvalidInput);
}

#[tokio::test]
async fn saving_after_the_end_starts_a_new_pool() {
    let app = TestApp::start().await;
    let ended = spend_an_ended_makeup(&app).await;

    let reopened = configure(
        &app,
        app.teacher_actor(),
        Some(ended),
        Some(for_days(30, 1)),
        Vec::new(),
    )
    .await
    .unwrap()
    .unwrap();

    assert!(reopened.opened_at() > ended.opened_at());
    let pool = student_pool(&app).await.unwrap();
    assert_eq!((pool.balance.allowed(), pool.balance.used()), (1, 0));
}

#[tokio::test]
async fn a_makeup_attempt_is_granted_only_to_a_student_of_the_course_group() {
    let app = TestApp::start().await;
    app.enable_makeup(3).await;
    let outsider = app.add_outsider_student().await;

    let error = app
        .application
        .courses
        .grant_makeup_attempt(app.teacher_actor(), app.fixture.course.id(), outsider.id())
        .await
        .unwrap_err();

    assert_eq!(error.code(), ErrorCode::NotFound);
}

#[tokio::test]
async fn a_submission_in_the_makeup_window_is_a_makeup_spending_the_pool() {
    let app = TestApp::start().await;
    let work = app.add_closed_assignment("Closed", 10, 6).await;
    app.enable_makeup(3).await;

    let submitted = submit(&app, &work, true).await.unwrap();

    assert_eq!(submitted.submission.kind(), SubmissionKind::Makeup);
    assert_eq!(
        submitted.makeup_balance.map(|balance| balance.remaining()),
        Some(2)
    );
    let stored = db::submissions::find_by_id(&app.pool, submitted.submission.id())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(stored.kind(), SubmissionKind::Makeup);
}

#[tokio::test]
async fn two_simultaneous_makeups_on_the_last_attempt_create_one_submission() {
    let app = TestApp::start().await;
    let first = app.add_closed_assignment("First", 10, 6).await;
    let second = app.add_closed_assignment("Second", 10, 6).await;
    let makeup = app.enable_makeup(1).await;
    let mut concurrent = app.pool.begin().await.expect("begin a concurrent makeup");
    let blocker: i32 = sqlx::query_scalar("select pg_backend_pid()")
        .fetch_one(&mut *concurrent)
        .await
        .expect("read backend pid");
    db::makeup::lock_pool(
        &mut concurrent,
        app.fixture.course.id(),
        app.fixture.student.id(),
    )
    .await
    .expect("lock the makeup pool");
    let spent = submission_of(&first, &app.fixture.student)
        .submitted_at(Utc::now())
        .makeup(MakeupRequest {
            course: Some(makeup),
            consent: true,
            balance: makeup.balance(0, 0),
        })
        .build();
    db::submissions::insert(&mut *concurrent, &spent)
        .await
        .expect("write the concurrent makeup");

    let (submitted, ()) = tokio::join!(submit(&app, &second, true), async {
        wait_for_blocked_connection(&app.pool, blocker).await;
        concurrent
            .commit()
            .await
            .expect("commit the concurrent makeup");
    });

    assert_eq!(
        submitted.unwrap_err().code(),
        ErrorCode::MakeupAttemptsExhausted
    );
    assert_eq!(spent.kind(), SubmissionKind::Makeup);
    assert_eq!(count_submissions(&app, &second).await, 0);
}

#[tokio::test]
async fn a_work_sent_before_makeup_was_enabled_keeps_the_closing_code() {
    let app = TestApp::start().await;
    let work = app.add_closed_assignment("Closed", 10, 6).await;
    let sent_at = Utc::now();
    app.open_makeup(&app.fixture.course, 3, sent_at + Duration::seconds(1))
        .await;

    let error = app
        .application
        .submissions
        .submit_homework(SubmitHomework {
            assignment_id: work.id(),
            actor: app.student_actor(),
            content: "pull/early".to_owned(),
            submitted_at: sent_at,
            allow_makeup: true,
        })
        .await
        .unwrap_err();

    assert_eq!(error.code(), ErrorCode::DeadlineExceeded);
    assert_eq!(count_submissions(&app, &work).await, 0);
}

#[tokio::test]
async fn a_rejected_makeup_spends_the_pool_but_not_the_assignment_attempts() {
    let app = TestApp::start().await;
    let work = app.add_closed_assignment("Closed", 10, 6).await;
    app.enable_makeup(3).await;
    let makeup = submit(&app, &work, true).await.unwrap().submission;

    app.application
        .reviews
        .review_submission(ReviewSubmission {
            submission_id: makeup.id(),
            actor: app.teacher_actor(),
            decision: ReviewDecision::Reject,
            score: None,
            comment: "Не то".to_owned(),
            reviewed_at: Utc::now(),
        })
        .await
        .unwrap();

    let details = app
        .application
        .assignments
        .find_for_student(app.student_actor(), work.id())
        .await
        .unwrap();
    assert_eq!(details.assignment.history.used_attempts(), 0);
    assert_eq!(
        details.assignment.history.remaining_attempts_for(&work),
        work.max_attempts()
    );
    let students = app
        .application
        .assignments
        .list_students_for_assignment(app.teacher_actor(), work.id(), PageRequest::default())
        .await
        .unwrap()
        .items;
    assert_eq!(students[0].history.used_attempts(), 0);
    assert_eq!(student_pool(&app).await.unwrap().balance.remaining(), 2);
}

#[tokio::test]
async fn a_makeup_without_consent_creates_nothing_and_keeps_the_pool() {
    let app = TestApp::start().await;
    let work = app.add_closed_assignment("Closed", 10, 6).await;
    app.enable_makeup(3).await;

    let error = submit(&app, &work, false).await.unwrap_err();

    assert_eq!(error.code(), ErrorCode::DeadlineExceeded);
    assert_eq!(count_submissions(&app, &work).await, 0);
    let pools: i64 = sqlx::query_scalar("select count(*) from student_course_progress")
        .fetch_one(&app.pool)
        .await
        .unwrap();
    assert_eq!(pools, 0);
    assert_eq!(student_pool(&app).await.unwrap().balance.remaining(), 3);
}

#[tokio::test]
async fn a_granted_attempt_lets_a_student_with_an_empty_pool_send_another_makeup() {
    let app = TestApp::start().await;
    let first = app.add_closed_assignment("First", 10, 6).await;
    let second = app.add_closed_assignment("Second", 10, 6).await;
    app.enable_makeup(1).await;
    submit(&app, &first, true).await.unwrap();
    assert_eq!(
        submit(&app, &second, true).await.unwrap_err().code(),
        ErrorCode::MakeupAttemptsExhausted
    );

    app.application
        .courses
        .grant_makeup_attempt(
            app.teacher_actor(),
            app.fixture.course.id(),
            app.fixture.student.id(),
        )
        .await
        .unwrap();

    assert_eq!(
        submit(&app, &second, true).await.unwrap().submission.kind(),
        SubmissionKind::Makeup
    );
}

fn offered(
    items: &[hw_bot::app::StudentAssignment],
    assignment: &Assignment,
) -> Option<MakeupOffer> {
    items
        .iter()
        .find(|item| item.assignment.id() == assignment.id())
        .expect("the assignment is listed")
        .makeup
}

#[tokio::test]
async fn the_student_list_and_card_offer_a_makeup_only_where_it_can_be_sent() {
    let app = TestApp::start().await;
    let open = app.add_closed_assignment("Open makeup", 10, 6).await;
    let spent = app.add_closed_assignment("Spent", 10, 6).await;
    let minimum = app
        .add_makeup_assignment(
            "At minimum",
            ten(),
            credit(6),
            Utc::now() + Duration::hours(1),
        )
        .await;
    let accepted = submission_id(
        &app.submit(minimum.id().as_uuid(), app.student(), "pull/minimum")
            .await
            .created(),
    );
    app.review_with_score(accepted, app.teacher(), "accept", out_of(6, 10), "Минимум")
        .await
        .ok();
    app.change_deadline(
        minimum.id().as_uuid(),
        app.teacher(),
        Utc::now() - Duration::minutes(30),
    )
    .await
    .ok();
    let makeup = app.enable_makeup(1).await;

    let listed = app
        .application
        .assignments
        .list_for_student(app.student_actor(), PageRequest::default())
        .await
        .unwrap()
        .items;
    let offer = offered(&listed, &open).expect("a makeup is offered");
    assert_eq!(offer.until, makeup.until());
    assert_eq!(offer.credit.map(|credit| credit.get()), Some(6));
    assert_eq!(offer.balance.remaining(), 1);
    assert_eq!(offered(&listed, &minimum), None);

    submit(&app, &spent, true).await.unwrap();
    let card = app
        .application
        .assignments
        .find_for_student(app.student_actor(), open.id())
        .await
        .unwrap();
    let exhausted = card.assignment.makeup.expect("the empty pool is shown");
    assert_eq!(exhausted.balance.remaining(), 0);
    let spent_card = app
        .application
        .assignments
        .find_for_student(app.student_actor(), spent.id())
        .await
        .unwrap();
    assert_eq!(spent_card.assignment.makeup, None);
}

async fn makeup_of(app: &TestApp, assignment: &Assignment, student: &User) -> Uuid {
    submission_id(
        &app.submit(
            assignment.id().as_uuid(),
            student.telegram_user_id().as_i64(),
            "pull/makeup",
        )
        .await
        .created(),
    )
}

#[tokio::test]
async fn the_main_queue_and_its_counter_skip_an_earlier_makeup() {
    let app = TestApp::start().await;
    let closed = app.add_closed_assignment("Closed", 10, 6).await;
    app.enable_makeup(3).await;
    makeup_of(&app, &closed, &app.fixture.student).await;
    let regular = app.queued_submission(app.student(), "pull/regular").await;

    assert_eq!(
        app.application
            .reviews
            .queue_size(app.teacher_actor())
            .await
            .unwrap(),
        1
    );
    let next = app
        .application
        .reviews
        .take_next_from_queue(app.teacher_actor())
        .await
        .unwrap()
        .expect("the regular submission is in the queue");
    assert_eq!(next.submission.id().as_uuid(), regular);
}

#[tokio::test]
async fn the_makeup_queue_hands_makeups_out_in_order_and_counts_what_it_hands_out() {
    let app = TestApp::start().await;
    let closed = app.add_closed_assignment("Closed", 10, 6).await;
    let second_student = app.add_student("Second").await;
    app.enable_makeup(3).await;
    app.queued_submission(app.student(), "pull/regular").await;
    let first = makeup_of(&app, &closed, &app.fixture.student).await;
    let second = makeup_of(&app, &closed, &second_student).await;
    let reviews = &app.application.reviews;

    assert_eq!(reviews.makeup_size(app.teacher_actor()).await.unwrap(), 2);
    let next = reviews
        .take_next_makeup(app.teacher_actor())
        .await
        .unwrap()
        .expect("a makeup is waiting");
    assert_eq!(next.submission.id().as_uuid(), first);
    assert_eq!(next.makeup.map(|pool| pool.balance.remaining()), Some(2));
    app.review(first, app.teacher(), "reject", "Не то")
        .await
        .ok();

    assert_eq!(reviews.makeup_size(app.teacher_actor()).await.unwrap(), 1);
    let next = reviews
        .take_next_makeup(app.teacher_actor())
        .await
        .unwrap()
        .expect("the second makeup is waiting");
    assert_eq!(next.submission.id().as_uuid(), second);
    app.review(second, app.teacher(), "reject", "Не то")
        .await
        .ok();
    assert_eq!(reviews.makeup_size(app.teacher_actor()).await.unwrap(), 0);
    assert!(
        reviews
            .take_next_makeup(app.teacher_actor())
            .await
            .unwrap()
            .is_none()
    );
}

#[tokio::test]
async fn a_sandbox_makeup_stays_in_the_sandbox_list() {
    let app = TestApp::start().await;
    let sandbox = app.add_sandbox_course().await;
    let trial = app
        .add_makeup_assignment_on(
            &sandbox,
            "Trial",
            ten(),
            credit(6),
            Utc::now() - Duration::days(1),
        )
        .await;
    app.enable_makeup_on(&sandbox, 3).await;
    let makeup = makeup_of(&app, &trial, &app.fixture.student).await;
    let reviews = &app.application.reviews;

    assert_eq!(reviews.makeup_size(app.teacher_actor()).await.unwrap(), 0);
    assert!(
        reviews
            .take_next_makeup(app.teacher_actor())
            .await
            .unwrap()
            .is_none()
    );
    let listed = reviews
        .sandbox_page(app.teacher_actor(), PageRequest::default())
        .await
        .unwrap()
        .items;
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].submission_id, SubmissionId::from_uuid(makeup));
    assert_eq!(listed[0].kind, SubmissionKind::Makeup);
}

#[tokio::test]
async fn the_place_of_a_makeup_counts_only_makeups() {
    let app = TestApp::start().await;
    let closed = app.add_closed_assignment("Closed", 10, 6).await;
    let other = app.add_student("Other").await;
    app.enable_makeup(3).await;
    app.queued_submission(other.telegram_user_id().as_i64(), "pull/regular")
        .await;
    makeup_of(&app, &closed, &app.fixture.student).await;
    app.queued_submission(app.student(), "pull/regular").await;

    let place = |assignment: AssignmentId| {
        app.application
            .submissions
            .queue_place(app.student_actor(), assignment)
    };
    let makeup_place = place(closed.id()).await.unwrap().unwrap();
    assert_eq!((makeup_place.position, makeup_place.total), (1, 1));
    let regular_place = place(app.fixture.assignment.id()).await.unwrap().unwrap();
    assert_eq!((regular_place.position, regular_place.total), (2, 2));
}

async fn closed_with_accepted_work(
    app: &TestApp,
    penalty_to_total: Option<u16>,
    score: u16,
) -> Assignment {
    let now = Utc::now();
    let mut builder = assignment_of(&app.fixture.course, &app.fixture.teacher)
        .created_at(now - Duration::days(60))
        .deadline_at(now - Duration::days(40))
        .scored(10)
        .makeup_credit(6)
        .published();
    let mut submitted_at = now - Duration::days(41);
    if let Some(penalty) = penalty_to_total {
        builder = builder.late_policy(7, penalty, 4, DeductedFrom::CourseTotal);
        submitted_at = now - Duration::days(39);
    }
    let work = builder.build();
    insert_assignment(&app.pool, &work).await;
    let accepted = submission_of(&work, &app.fixture.student)
        .submitted_at(submitted_at)
        .build();
    db::submissions::insert(&app.pool, &accepted).await.unwrap();
    app.review_with_score(
        accepted.id().as_uuid(),
        app.teacher(),
        "accept",
        out_of(score, 10),
        "Принято",
    )
    .await
    .ok();

    work
}

async fn gradebook_score(
    app: &TestApp,
    assignment: &Assignment,
) -> Option<hw_bot::app::GradebookScore> {
    app.application
        .courses
        .student_grades(
            app.teacher_actor(),
            app.fixture.course.id(),
            app.fixture.student.id(),
            PageRequest::default(),
        )
        .await
        .unwrap()
        .grades
        .into_iter()
        .find(|grade| grade.assignment_id == assignment.id())
        .expect("the assignment is in the gradebook")
        .score
}

#[tokio::test]
async fn a_credited_makeup_replaces_a_work_with_a_penalty_to_the_total() {
    let app = TestApp::start().await;
    let work = closed_with_accepted_work(&app, Some(3), 4).await;
    app.enable_makeup(3).await;
    let makeup = makeup_of(&app, &work, &app.fixture.student).await;
    app.review(makeup, app.teacher(), "accept", "Досдано")
        .await
        .ok();

    let score = gradebook_score(&app, &work).await.unwrap();

    assert_eq!(
        (score.work_score, score.course_total_penalty, score.makeup),
        (6, 0, true)
    );
}

#[tokio::test]
async fn a_makeup_credited_below_the_earlier_work_does_not_lower_the_gradebook() {
    let app = TestApp::start().await;
    let work = closed_with_accepted_work(&app, None, 5).await;
    app.enable_makeup(3).await;
    let makeup = makeup_of(&app, &work, &app.fixture.student).await;
    let stored = db::assignments::find_versioned_by_id(&app.pool, work.id())
        .await
        .unwrap()
        .unwrap();
    app.application
        .assignments
        .change_terms(ChangeTerms {
            assignment_id: work.id(),
            actor: app.teacher_actor(),
            deadline_at: work.deadline_at(),
            late_policy: work.late_policy(),
            makeup: Some(credit(4)),
            expected_revision: stored.revision,
        })
        .await
        .unwrap();
    app.review(makeup, app.teacher(), "accept", "Досдано")
        .await
        .ok();

    let score = gradebook_score(&app, &work).await.unwrap();

    assert_eq!(
        (score.work_score, score.course_total_penalty, score.makeup),
        (5, 0, false)
    );
}

#[tokio::test]
async fn a_makeup_awaiting_review_leaves_the_gradebook_as_it_was() {
    let app = TestApp::start().await;
    let work = closed_with_accepted_work(&app, Some(3), 4).await;
    app.enable_makeup(3).await;
    makeup_of(&app, &work, &app.fixture.student).await;

    let score = gradebook_score(&app, &work).await.unwrap();

    assert_eq!(
        (score.work_score, score.course_total_penalty, score.makeup),
        (4, 3, false)
    );
}
