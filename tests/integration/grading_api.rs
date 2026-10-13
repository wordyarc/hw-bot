use axum::http::StatusCode;
use chrono::{Duration, Utc};
use hw_bot::app::{GradebookScore, StudentGrades};
use hw_bot::db;
use hw_bot::domain::{Assignment, DeductedFrom, Grading, MakeupTerms, StudentGroupId};
use hw_bot::pagination::PageRequest;
use serde_json::{Value, json};
use uuid::Uuid;

use crate::harness::{
    TestApp, assignment_id, insert_assignment, out_of, submission_id, wall_clock_assignment,
};

async fn scored_submission(app: &TestApp, max_score: u16) -> uuid::Uuid {
    let assignment = app.add_scored_assignment(max_score).await;
    let body = app
        .submit(assignment.id().as_uuid(), app.student(), "pull/1")
        .await;

    submission_id(&body.created())
}

#[tokio::test]
async fn accepting_with_a_score_stores_the_whole_arithmetic() {
    let app = TestApp::start().await;
    let submission = scored_submission(&app, 5).await;

    let body = app
        .review_with_score(submission, app.teacher(), "accept", out_of(4, 5), "Хорошо")
        .await
        .ok();

    assert_eq!(body["status"], json!("accepted"));
    assert_eq!(body["raw_score"], json!(4));
    assert_eq!(body["max_score"], json!(5));
    assert_eq!(body["late_periods"], json!(0));
    assert_eq!(body["penalty"], json!(0));
    assert_eq!(body["penalty_deducted_from"], json!(null));
    assert_eq!(body["work_score"], json!(4));
    assert_eq!(body["course_total_penalty"], json!(0));
    assert!(body.get("final_score").is_none());
}

#[tokio::test]
async fn a_penalty_to_the_course_total_reaches_the_submission_response() {
    let app = TestApp::start().await;
    let assignment = app
        .add_late_assignment("Course total", 10, 4, DeductedFrom::CourseTotal)
        .await;
    let submission = submission_id(
        &app.submit(assignment.id().as_uuid(), app.student(), "pull/1")
            .await
            .created(),
    );

    let body = app
        .review_with_score(submission, app.teacher(), "accept", out_of(8, 10), "Ок")
        .await
        .ok();

    assert_eq!(body["raw_score"], json!(8));
    assert_eq!(body["penalty"], json!(4));
    assert_eq!(body["penalty_deducted_from"], json!("course_total"));
    assert_eq!(body["work_score"], json!(8));
    assert_eq!(body["course_total_penalty"], json!(4));
}

#[tokio::test]
async fn where_the_penalty_is_deducted_from_round_trips_through_http() {
    let app = TestApp::start().await;
    let created = app
        .create_assignment_raw(
            app.course(),
            app.teacher(),
            json!({
                "title": "Penalty deducted_from", "body": "Send a pull request.", "max_attempts": 2,
                "max_score": 10, "deadline_at": Utc::now() + Duration::days(7),
                "late_period_days": 7, "late_penalty_per_period": 2, "late_max_periods": 4,
                "late_deducted_from": "course_total",
            }),
        )
        .await
        .created();
    assert_eq!(created["late_deducted_from"], json!("course_total"));

    let listed = app
        .course_assignments(app.course(), app.teacher())
        .await
        .ok();
    let read = listed
        .as_array()
        .unwrap()
        .iter()
        .find(|assignment| assignment["id"] == created["id"])
        .expect("the created assignment is listed");
    assert_eq!(read["late_deducted_from"], json!("course_total"));
}

#[tokio::test]
async fn a_submission_awaiting_review_has_no_grade() {
    let app = TestApp::start().await;
    let assignment = app.add_scored_assignment(5).await;

    let body = app
        .submit(assignment.id().as_uuid(), app.student(), "pull/1")
        .await
        .created();

    assert_eq!(body["raw_score"], json!(null));
    assert_eq!(body["max_score"], json!(null));
    assert_eq!(body["work_score"], json!(null));
}

#[tokio::test]
async fn rejecting_a_scored_submission_stores_no_grade() {
    let app = TestApp::start().await;
    let submission = scored_submission(&app, 5).await;

    let body = app
        .review(submission, app.teacher(), "reject", "Нет тестов")
        .await
        .ok();

    assert_eq!(body["status"], json!("rejected"));
    assert_eq!(body["raw_score"], json!(null));
    assert_eq!(body["max_score"], json!(null));
    assert_eq!(body["work_score"], json!(null));
}

#[tokio::test]
async fn the_notification_carries_the_score_of_the_review() {
    let app = TestApp::start().await;
    let submission = scored_submission(&app, 5).await;

    app.review_with_score(submission, app.teacher(), "accept", out_of(4, 5), "Хорошо")
        .await
        .ok();

    let grade = claim_notification(&app)
        .await
        .grade
        .expect("a scored review");

    assert_eq!((grade.work_score(), grade.max_score()), (4, 5));
}

async fn claim_notification(app: &TestApp) -> db::notifications::ClaimedReviewNotification {
    let now = Utc::now();
    db::notifications::claim_next(&app.pool, now, Uuid::new_v4(), now + Duration::seconds(10))
        .await
        .expect("claim notification")
        .expect("notification is enqueued")
}

#[tokio::test]
async fn a_review_freezes_the_scale_after_the_assignment_becomes_pass_fail() {
    let app = TestApp::start().await;
    let assignment = app.add_scored_assignment(5).await;
    let submission = submission_id(
        &app.submit(assignment.id().as_uuid(), app.student(), "pull/1")
            .await
            .created(),
    );
    app.review_with_score(submission, app.teacher(), "accept", out_of(4, 5), "Хорошо")
        .await
        .ok();

    app.change_grading(assignment.id().as_uuid(), app.teacher(), None)
        .await
        .ok();

    let details = app
        .student_assignment(assignment.id().as_uuid(), app.student())
        .await
        .ok();
    assert_eq!(details["latest_submission"]["max_score"], json!(5));

    let gradebook = gradebook(&app, app.teacher()).await.ok();
    assert_eq!(gradebook["students"][0]["grades"][0]["max_score"], json!(5));
    assert_eq!(gradebook["students"][0]["max_total"], json!(5));
}

#[tokio::test]
async fn a_pass_fail_notification_carries_no_score() {
    let app = TestApp::start().await;
    let submission = app.queued_submission(app.student(), "pull/1").await;

    app.review(submission, app.teacher(), "accept", "Зачтено")
        .await
        .ok();

    assert_eq!(claim_notification(&app).await.grade, None);
}

#[tokio::test]
async fn an_assignment_without_a_scale_stays_pass_fail() {
    let app = TestApp::start().await;

    let body = app
        .student_assignment(app.assignment(), app.student())
        .await;

    assert_eq!(body.ok()["assignment"]["max_score"], json!(null));
}

#[tokio::test]
async fn a_scale_can_be_set_after_creation() {
    let app = TestApp::start().await;
    let draft = app.drafted_assignment(app.teacher()).await;

    let body = app
        .change_grading(draft, app.teacher(), Some(10))
        .await
        .ok();

    assert_eq!(body["max_score"], json!(10));
}

#[tokio::test]
async fn a_scale_change_advances_the_revision() {
    let app = TestApp::start().await;
    let draft = app.drafted_assignment(app.teacher()).await;

    app.change_grading(draft, app.teacher(), Some(10))
        .await
        .ok();

    let revision: i64 = sqlx::query_scalar("select revision from assignments where id = $1")
        .bind(draft)
        .fetch_one(&app.pool)
        .await
        .expect("read revision");
    assert_eq!(revision, 1);
}

#[tokio::test]
async fn a_scale_change_on_a_stale_revision_loses() {
    let app = TestApp::start().await;
    let mut assignment = app.add_draft_assignment().await;
    app.change_deadline(
        assignment.id().as_uuid(),
        app.teacher(),
        Utc::now() + Duration::days(14),
    )
    .await
    .ok();

    assignment
        .change_grading(
            &app.fixture.course,
            &app.fixture.teacher,
            Grading::scored(10).unwrap(),
            None,
        )
        .expect("teacher may change the scale");

    let result = db::assignments::update(&app.pool, &assignment, 0).await;

    assert!(
        matches!(result, Err(db::DbError::Conflict { .. })),
        "a stale revision must not overwrite the winner, got {result:?}"
    );
}

#[tokio::test]
async fn a_scale_given_at_creation_reaches_the_response() {
    let app = TestApp::start().await;

    let body = app
        .create_assignment_raw(
            app.course(),
            app.teacher(),
            json!({
                "title": "Rust lifetimes",
                "body": "Send a pull request link.",
                "max_attempts": 2,
                "max_score": 10,
                "deadline_at": Utc::now() + Duration::days(7),
            }),
        )
        .await
        .created();

    assert_eq!(body["max_score"], json!(10));
}

#[tokio::test]
async fn a_submission_past_the_horizon_is_closed() {
    let app = TestApp::start().await;
    let assignment = app.add_overdue_assignment(29, Some((7, 1, 4))).await;

    app.submit(assignment.id().as_uuid(), app.student(), "pull/1")
        .await
        .error(StatusCode::CONFLICT, "submission_closed");
}

#[tokio::test]
async fn the_horizon_reaches_the_assignment_response() {
    let app = TestApp::start().await;
    let assignment = app.add_overdue_assignment(1, Some((7, 1, 4))).await;

    let body = app
        .student_assignment(assignment.id().as_uuid(), app.student())
        .await
        .ok();

    let card = &body["assignment"];
    assert_eq!(card["late_period_days"], json!(7));
    assert_eq!(card["late_penalty_per_period"], json!(1));
    assert!(card.get("late_closes_at").is_none());
    assert_eq!(card["late_max_periods"], json!(4));
    let closes_at: chrono::DateTime<Utc> =
        serde_json::from_value(card["closes_at"].clone()).unwrap();
    assert_eq!(
        closes_at.timestamp_micros(),
        (assignment.deadline_at() + Duration::days(28)).timestamp_micros()
    );
}

#[tokio::test]
async fn a_nonweekly_policy_round_trips_through_http() {
    let app = TestApp::start().await;
    let deadline = Utc::now() + Duration::days(7);
    let closing = deadline + Duration::days(10);
    let body = app
        .create_assignment_raw(
            app.course(),
            app.teacher(),
            json!({
                "title": "Periodic closing", "body": "Send a pull request.", "max_attempts": 2,
                "max_score": 5, "deadline_at": deadline, "late_period_days": 5,
                "late_penalty_per_period": 1, "late_max_periods": 2, "late_deducted_from": "work_score",
            }),
        )
        .await
        .created();
    let id = hw_bot::domain::AssignmentId::from_uuid(body["id"].as_str().unwrap().parse().unwrap());
    let saved = db::assignments::find_by_id(&app.pool, id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        saved.closes_at().timestamp_micros(),
        closing.timestamp_micros()
    );
    assert_eq!(body["late_max_periods"], json!(2));
    assert!(body.get("late_closes_at").is_none());
}

#[tokio::test]
async fn a_deadline_can_move_past_the_previous_computed_closing() {
    let app = TestApp::start().await;
    let assignment = app.add_overdue_assignment(1, Some((7, 1, 4))).await;
    let deadline = assignment.closes_at() + Duration::days(1);
    let closing = deadline + Duration::days(28);
    let body = app
        .post_as(
            &format!("/assignments/{}/deadline", assignment.id().as_uuid()),
            app.teacher(),
            json!({ "deadline_at": deadline }),
        )
        .await
        .ok();
    let saved = db::assignments::find_by_id(&app.pool, assignment.id())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        saved.deadline_at().timestamp_micros(),
        deadline.timestamp_micros()
    );
    assert_eq!(
        saved.closes_at().timestamp_micros(),
        closing.timestamp_micros()
    );
    assert_eq!(body["late_max_periods"], json!(4));
}

#[tokio::test]
async fn moving_a_deadline_back_does_not_truncate_the_stored_lateness_counter() {
    let app = TestApp::start().await;
    let now = Utc::now();
    let work = hw_bot::test_support::assignment(&app.fixture.course, &app.fixture.teacher)
        .created_at(now - Duration::days(70_001))
        .deadline_at(now + Duration::days(1))
        .scored(5)
        .late_policy(1, 1, 1, DeductedFrom::WorkScore)
        .published()
        .build();
    insert_assignment(&app.pool, &work).await;
    let id = submission_id(
        &app.submit(work.id().as_uuid(), app.student(), "pull/1")
            .await
            .created(),
    );
    app.change_deadline(
        work.id().as_uuid(),
        app.teacher(),
        now - Duration::days(70_000),
    )
    .await
    .ok();
    let body = app
        .review_with_score(id, app.teacher(), "accept", out_of(5, 5), "Ок")
        .await
        .ok();
    assert!(body["late_periods"].as_u64().unwrap() > u64::from(u16::MAX));
    let saved = db::submissions::find_by_id(&app.pool, hw_bot::domain::SubmissionId::from_uuid(id))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        u64::from(saved.grade().unwrap().late_periods()),
        body["late_periods"].as_u64().unwrap()
    );
    assert_eq!(saved.grade().unwrap().work_score(), 0);
}

async fn gradebook(app: &TestApp, telegram_id: i64) -> crate::harness::TestResponse {
    group_gradebook(app, app.fixture.group.id(), telegram_id).await
}

async fn group_gradebook(
    app: &TestApp,
    group_id: StudentGroupId,
    telegram_id: i64,
) -> crate::harness::TestResponse {
    app.get_as(
        &format!(
            "/me/courses/{}/groups/{}/gradebook",
            app.course(),
            group_id.as_uuid()
        ),
        telegram_id,
    )
    .await
}

#[tokio::test]
async fn the_gradebook_of_a_group_holds_only_its_students_and_the_assignments_issued_to_it() {
    let app = TestApp::start().await;
    let second = app.add_group_to_course(&app.fixture.course).await;
    let course = app.reload_course(&app.fixture.course).await;
    let other = app
        .add_student_to_group("Other group student", &second)
        .await;
    let shared = wall_clock_assignment(&course, &app.fixture.teacher)
        .title("Shared")
        .scored(5)
        .published()
        .build();
    let only_second = wall_clock_assignment(&course, &app.fixture.teacher)
        .title("Second only")
        .scored(10)
        .groups([&second])
        .published()
        .build();
    insert_assignment(&app.pool, &shared).await;
    insert_assignment(&app.pool, &only_second).await;

    let first = gradebook(&app, app.teacher()).await.ok();
    let second_book = group_gradebook(&app, second.id(), app.teacher()).await.ok();

    let names = |book: &Value| -> Vec<String> {
        book["students"]
            .as_array()
            .unwrap()
            .iter()
            .map(|student| student["display_name"].as_str().unwrap().to_owned())
            .collect()
    };
    let titles = |book: &Value| -> Vec<String> {
        book["students"][0]["grades"]
            .as_array()
            .unwrap()
            .iter()
            .map(|grade| grade["title"].as_str().unwrap().to_owned())
            .collect()
    };
    assert_eq!(
        names(&first),
        vec![app.fixture.student.display_name().as_str()]
    );
    assert_eq!(titles(&first), vec!["Shared"]);
    assert_eq!(first["students"][0]["max_total"], json!(5));
    assert_eq!(names(&second_book), vec![other.display_name().as_str()]);
    assert_eq!(titles(&second_book), vec!["Shared", "Second only"]);
    assert_eq!(second_book["students"][0]["max_total"], json!(15));
}

#[tokio::test]
async fn the_gradebook_of_a_group_outside_the_course_is_not_found() {
    let app = TestApp::start().await;
    let foreign = app.add_foreign_course().await;

    group_gradebook(&app, foreign.group.id(), app.teacher())
        .await
        .error(StatusCode::NOT_FOUND, "not_found");
}

#[tokio::test]
async fn a_student_without_submissions_is_still_in_the_gradebook() {
    let app = TestApp::start().await;
    app.add_scored_assignment(5).await;

    let body = gradebook(&app, app.teacher()).await.ok();

    let students = body["students"].as_array().unwrap();
    assert_eq!(students.len(), 1);
    assert_eq!(students[0]["total"], json!(0));
    assert_eq!(students[0]["max_total"], json!(5));
    assert_eq!(students[0]["grades"][0]["work_score"], json!(null));
}

async fn reviewed(app: &TestApp, assignment: &Assignment, score: u16) {
    let submission = submission_id(
        &app.submit(assignment.id().as_uuid(), app.student(), "pull/1")
            .await
            .created(),
    );
    let max_score = assignment
        .grading()
        .max_score()
        .expect("a scored assignment");
    app.review_with_score(
        submission,
        app.teacher(),
        "accept",
        out_of(score, max_score),
        "Ок",
    )
    .await
    .ok();
}

async fn student_grades(app: &TestApp) -> StudentGrades {
    app.application
        .courses
        .student_grades(
            app.teacher_actor(),
            app.fixture.course.id(),
            app.fixture.student.id(),
            PageRequest::default(),
        )
        .await
        .expect("read student grades")
}

#[tokio::test]
async fn the_gradebook_sums_work_scores_and_course_penalties_of_both_deductions() {
    let app = TestApp::start().await;
    let to_total = app
        .add_late_assignment("To total", 10, 4, DeductedFrom::CourseTotal)
        .await;
    let to_work = app
        .add_late_assignment("To work", 10, 2, DeductedFrom::WorkScore)
        .await;
    reviewed(&app, &to_total, 8).await;
    reviewed(&app, &to_work, 6).await;

    let grades = student_grades(&app).await;

    assert_eq!(grades.work_total(), 12);
    assert_eq!(grades.penalty_total(), 4);
    assert_eq!(grades.total(), 8);
    assert_eq!(grades.max_total(), 20);
    let scores: Vec<_> = grades.grades.iter().map(|grade| grade.score).collect();
    assert!(scores.contains(&Some(GradebookScore {
        work_score: 8,
        course_total_penalty: 4,
        makeup: false,
    })));
    assert!(scores.contains(&Some(GradebookScore {
        work_score: 4,
        course_total_penalty: 0,
        makeup: false,
    })));
}

#[tokio::test]
async fn the_gradebook_total_goes_below_zero_instead_of_losing_a_penalty() {
    let app = TestApp::start().await;
    let to_total = app
        .add_late_assignment("To total", 10, 4, DeductedFrom::CourseTotal)
        .await;
    reviewed(&app, &to_total, 0).await;

    let grades = student_grades(&app).await;

    assert_eq!(grades.work_total(), 0);
    assert_eq!(grades.penalty_total(), 4);
    assert_eq!(grades.total(), -4);
}

#[tokio::test]
async fn a_submission_awaiting_review_has_no_gradebook_score_unlike_a_given_zero() {
    let app = TestApp::start().await;
    let pending = app.add_scored_assignment(5).await;
    let zero = app
        .add_late_assignment("Zero", 10, 4, DeductedFrom::WorkScore)
        .await;
    app.submit(pending.id().as_uuid(), app.student(), "pull/pending")
        .await
        .created();
    reviewed(&app, &zero, 2).await;

    let grades = student_grades(&app).await;

    let score = |assignment: &Assignment| {
        grades
            .grades
            .iter()
            .find(|grade| grade.assignment_id == assignment.id())
            .expect("the scored assignment is in the gradebook")
            .score
    };
    assert_eq!(score(&pending), None);
    assert_eq!(
        score(&zero),
        Some(GradebookScore {
            work_score: 0,
            course_total_penalty: 0,
            makeup: false,
        })
    );
}

#[tokio::test]
async fn the_gradebook_contract_reports_penalties_to_the_total_and_null_before_review() {
    let app = TestApp::start().await;
    let to_total = app
        .add_late_assignment("To total", 10, 4, DeductedFrom::CourseTotal)
        .await;
    let pending = app.add_scored_assignment(5).await;
    reviewed(&app, &to_total, 8).await;
    app.submit(pending.id().as_uuid(), app.student(), "pull/pending")
        .await
        .created();

    let body = gradebook(&app, app.teacher()).await.ok();

    let student = &body["students"][0];
    assert_eq!(student["work_total"], json!(8));
    assert_eq!(student["penalty_total"], json!(4));
    assert_eq!(student["total"], json!(4));
    assert_eq!(student["max_total"], json!(15));
    let grade = |assignment: &Assignment| {
        student["grades"]
            .as_array()
            .unwrap()
            .iter()
            .find(|grade| grade["assignment_id"] == json!(assignment.id().as_uuid()))
            .cloned()
            .expect("the assignment is in the gradebook")
    };
    let penalized = grade(&to_total);
    assert_eq!(penalized["work_score"], json!(8));
    assert_eq!(penalized["course_total_penalty"], json!(4));
    assert!(penalized.get("final_score").is_none());
    let awaiting = grade(&pending);
    assert_eq!(awaiting["work_score"], json!(null));
    assert_eq!(awaiting["course_total_penalty"], json!(null));
}

#[tokio::test]
async fn the_gradebook_sums_only_reviewed_scores() {
    let app = TestApp::start().await;
    let first = app.add_scored_assignment(5).await;
    let second = app.add_scored_assignment(10).await;

    let reviewed = submission_id(
        &app.submit(first.id().as_uuid(), app.student(), "pull/1")
            .await
            .created(),
    );
    app.review_with_score(reviewed, app.teacher(), "accept", out_of(4, 5), "Ок")
        .await
        .ok();
    app.submit(second.id().as_uuid(), app.student(), "pull/2")
        .await
        .created();

    let body = gradebook(&app, app.teacher()).await.ok();

    let student = &body["students"][0];
    assert_eq!(student["total"], json!(4), "сдача на проверке не считается");
    assert_eq!(student["max_total"], json!(15));
}

#[tokio::test]
async fn pass_fail_assignments_stay_out_of_the_gradebook() {
    let app = TestApp::start().await;
    let submission = app.queued_submission(app.student(), "pull/1").await;
    app.review(submission, app.teacher(), "accept", "Зачтено")
        .await
        .ok();

    let body = gradebook(&app, app.teacher()).await.ok();

    let student = &body["students"][0];
    assert_eq!(student["grades"].as_array().unwrap().len(), 0);
    assert_eq!(student["max_total"], json!(0));
}

#[tokio::test]
async fn a_pass_fail_acceptance_stays_out_of_the_gradebook_after_the_scale_appears() {
    let app = TestApp::start().await;
    let submission = app.queued_submission(app.student(), "pull/1").await;
    app.review(submission, app.teacher(), "accept", "Зачтено")
        .await
        .ok();

    app.change_grading(app.assignment(), app.teacher(), Some(10))
        .await
        .ok();

    let body = gradebook(&app, app.teacher()).await.ok();

    let student = &body["students"][0];
    assert_eq!(student["grades"].as_array().unwrap().len(), 0);
    assert_eq!(student["max_total"], json!(0));
}

#[tokio::test]
async fn a_gradebook_of_a_foreign_course_is_denied() {
    let app = TestApp::start().await;
    let outsider = app.add_outsider_teacher("Outsider teacher").await;

    gradebook(&app, outsider.telegram_user_id().as_i64())
        .await
        .error(StatusCode::FORBIDDEN, "permission_denied");
}

#[tokio::test]
async fn a_draft_assignment_is_not_in_the_gradebook_yet() {
    let app = TestApp::start().await;
    app.create_assignment_raw(
        app.course(),
        app.teacher(),
        json!({
            "title": "Draft with a scale",
            "body": "Send a pull request link.",
            "max_attempts": 2,
            "max_score": 5,
            "deadline_at": Utc::now() + Duration::days(7),
        }),
    )
    .await
    .created();

    let body = gradebook(&app, app.teacher()).await.ok();

    assert_eq!(body["students"][0]["grades"].as_array().unwrap().len(), 0);
}

#[tokio::test]
async fn archiving_an_unpublished_draft_does_not_increase_the_gradebook_maximum() {
    let app = TestApp::start().await;
    let created = app
        .create_assignment_raw(
            app.course(),
            app.teacher(),
            json!({
                "title": "Unused draft", "body": "Not assigned", "max_attempts": 2,
                "max_score": 10, "deadline_at": Utc::now() + Duration::days(7),
            }),
        )
        .await
        .created();
    let id = created["id"].as_str().unwrap();
    app.post_empty_as(&format!("/assignments/{id}/archive"), app.teacher())
        .await
        .ok();
    let body = gradebook(&app, app.teacher()).await.ok();
    assert_eq!(body["students"][0]["max_total"], json!(0));
}

#[tokio::test]
async fn moving_the_deadline_does_not_change_a_grade_that_was_already_given() {
    let app = TestApp::start().await;
    let assignment = app.add_overdue_assignment(1, Some((7, 1, 4))).await;
    let submission = submission_id(
        &app.submit(assignment.id().as_uuid(), app.student(), "pull/1")
            .await
            .created(),
    );
    let reviewed = app
        .review_with_score(submission, app.teacher(), "accept", out_of(5, 5), "Ок")
        .await
        .ok();
    assert_eq!(reviewed["work_score"], json!(4));

    app.change_deadline(
        assignment.id().as_uuid(),
        app.teacher(),
        Utc::now() + Duration::days(7),
    )
    .await
    .ok();

    let body = app
        .student_assignment(assignment.id().as_uuid(), app.student())
        .await
        .ok();

    let stored = &body["latest_submission"];
    assert_eq!(stored["late_periods"], json!(1));
    assert_eq!(stored["penalty"], json!(1));
    assert_eq!(stored["work_score"], json!(4));
}

#[tokio::test]
async fn the_assignment_contract_reports_each_makeup_state() {
    let app = TestApp::start().await;
    let ten = Grading::scored(10).unwrap();
    let deadline = Utc::now() + Duration::days(7);
    let created = assignment_id(
        &app.create_assignment(app.course(), app.teacher())
            .await
            .created(),
    );
    let declined = app
        .add_makeup_assignment("Declined", ten, MakeupTerms::NotOffered, deadline)
        .await;
    let credited = app
        .add_makeup_assignment(
            "Credited",
            ten,
            MakeupTerms::scored(6, ten).unwrap(),
            deadline,
        )
        .await;
    let pass_fail = app
        .add_makeup_assignment(
            "Pass/fail",
            Grading::PassFail,
            MakeupTerms::PassFail,
            deadline,
        )
        .await;

    let listed = app
        .course_assignments(app.course(), app.teacher())
        .await
        .ok();
    let terms = |id: Uuid| {
        let item = listed
            .as_array()
            .unwrap()
            .iter()
            .find(|item| item["id"] == json!(id))
            .expect("the assignment is listed");
        (
            item["makeup_offered"].clone(),
            item["makeup_credit"].clone(),
        )
    };

    assert_eq!(terms(created), (json!(null), json!(null)));
    assert_eq!(terms(declined.id().as_uuid()), (json!(false), json!(null)));
    assert_eq!(terms(credited.id().as_uuid()), (json!(true), json!(6)));
    assert_eq!(terms(pass_fail.id().as_uuid()), (json!(true), json!(null)));
}

#[tokio::test]
async fn the_gradebook_contract_marks_a_grade_given_by_a_makeup() {
    let app = TestApp::start().await;
    let regular = app.add_scored_assignment(5).await;
    let closed = app.add_closed_assignment("Closed", 10, 6).await;
    let pending = app.add_scored_assignment(10).await;
    reviewed(&app, &regular, 4).await;
    app.enable_makeup(3).await;
    let makeup = submission_id(
        &app.submit(closed.id().as_uuid(), app.student(), "pull/makeup")
            .await
            .created(),
    );
    app.review(makeup, app.teacher(), "accept", "Досдано")
        .await
        .ok();
    app.submit(pending.id().as_uuid(), app.student(), "pull/pending")
        .await
        .created();

    let body = gradebook(&app, app.teacher()).await.ok();

    let grade = |assignment: &Assignment| {
        body["students"][0]["grades"]
            .as_array()
            .unwrap()
            .iter()
            .find(|grade| grade["assignment_id"] == json!(assignment.id().as_uuid()))
            .cloned()
            .expect("the assignment is in the gradebook")
    };
    assert_eq!(grade(&closed)["work_score"], json!(6));
    assert_eq!(grade(&closed)["makeup"], json!(true));
    assert_eq!(grade(&regular)["makeup"], json!(false));
    assert_eq!(grade(&pending)["makeup"], json!(null));
}
