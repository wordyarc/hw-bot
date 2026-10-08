use chrono::{Duration, Utc};
use hw_bot::db;
use hw_bot::domain::DeductedFrom;
use serde_json::json;
use uuid::Uuid;

use crate::harness::{TestApp, submission_id};

#[tokio::test]
async fn expired_notification_lease_can_be_reclaimed() {
    let app = TestApp::start().await;
    let submission = app.queued_submission(app.student(), "pull/1").await;
    app.review(submission, app.teacher(), "accept", "good")
        .await
        .ok();
    let now = Utc::now();
    let first_token = Uuid::new_v4();

    let first =
        db::notifications::claim_next(&app.pool, now, first_token, now + Duration::seconds(10))
            .await
            .expect("claim notification")
            .expect("notification exists");

    assert!(
        db::notifications::claim_next(
            &app.pool,
            now + Duration::seconds(5),
            Uuid::new_v4(),
            now + Duration::seconds(15),
        )
        .await
        .expect("try claim leased notification")
        .is_none()
    );

    let second_token = Uuid::new_v4();
    let reclaimed = db::notifications::claim_next(
        &app.pool,
        now + Duration::seconds(11),
        second_token,
        now + Duration::seconds(21),
    )
    .await
    .expect("reclaim notification")
    .expect("expired notification exists");

    assert_eq!(reclaimed.id, first.id);
    assert_eq!(reclaimed.attempt_count, 2);
    assert!(matches!(
        db::notifications::mark_delivered(&app.pool, first.id, first_token, Utc::now()).await,
        Err(db::DbError::Conflict { .. })
    ));
    db::notifications::mark_delivered(&app.pool, first.id, second_token, Utc::now())
        .await
        .expect("current acknowledgement succeeds");
}

#[tokio::test]
async fn concurrent_workers_claim_different_notifications() {
    let app = TestApp::start().await;
    let second_student = app.add_student("Second student").await;
    let first_submission = app.queued_submission(app.student(), "pull/1").await;
    let second_submission = app
        .queued_submission(second_student.telegram_user_id().as_i64(), "pull/2")
        .await;
    app.review(first_submission, app.teacher(), "accept", "good")
        .await
        .ok();
    app.review(second_submission, app.teacher(), "reject", "fix it")
        .await
        .ok();
    let now = Utc::now();

    let (first, second) = tokio::join!(
        db::notifications::claim_next(&app.pool, now, Uuid::new_v4(), now + Duration::seconds(10),),
        db::notifications::claim_next(&app.pool, now, Uuid::new_v4(), now + Duration::seconds(10),),
    );

    let first = first.expect("first claim").expect("first notification");
    let second = second.expect("second claim").expect("second notification");
    assert_ne!(first.id, second.id);
}

#[tokio::test]
async fn a_claimed_notification_names_the_scale_and_deduction_in_force_at_review() {
    let app = TestApp::start().await;
    let assignment = app
        .add_late_assignment("To total", 10, 4, DeductedFrom::CourseTotal)
        .await;
    let submission = submission_id(
        &app.submit(assignment.id().as_uuid(), app.student(), "pull/1")
            .await
            .created(),
    );
    app.review_with_score(submission, app.teacher(), "accept", 8, "Ок")
        .await
        .ok();
    app.post_as(
        &format!("/assignments/{}/grading", assignment.id().as_uuid()),
        app.teacher(),
        json!({
            "max_score": 20, "late_period_days": 7, "late_penalty_per_period": 1, "late_max_periods": 4,
            "late_deducted_from": "work_score",
        }),
    )
    .await
    .ok();
    let now = Utc::now();

    let notification =
        db::notifications::claim_next(&app.pool, now, Uuid::new_v4(), now + Duration::seconds(10))
            .await
            .expect("claim notification")
            .expect("notification exists");

    let grade = notification
        .grade
        .expect("a scored review is notified with its grade");
    assert_eq!(grade.max_score(), 10);
    assert_eq!(grade.deducted_from(), Some(DeductedFrom::CourseTotal));
    assert_eq!((grade.work_score(), grade.course_total_penalty()), (8, 4));
}
