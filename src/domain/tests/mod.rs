use chrono::{DateTime, Duration, TimeZone, Utc};

use crate::domain::{Assignment, DeductedFrom, ReviewScore, StudentGroup, TeachingCourse, User};
use crate::test_support::{
    GROUP_CODE, assignment, course_for, student, student_group, t0, teacher,
};

mod attempts;
mod lateness;
mod lifecycle;
mod makeup;
mod review;
mod staff;
mod submission;
mod terms;

struct Fixture {
    teacher: User,
    group: StudentGroup,
    course: TeachingCourse,
    student: User,
}

fn fixture() -> Fixture {
    let teacher = teacher(1);
    let group = student_group(GROUP_CODE);
    let course = course_for(&group, &teacher);
    let student = student(2, &group);

    Fixture {
        teacher,
        group,
        course,
        student,
    }
}

fn reviewed_at() -> DateTime<Utc> {
    t0() + Duration::hours(2)
}

fn out_of(value: u16, max_score: u16) -> Option<ReviewScore> {
    Some(ReviewScore { value, max_score })
}

fn weekly_penalty_assignment(
    course: &TeachingCourse,
    teacher: &User,
) -> (Assignment, DateTime<Utc>) {
    let deadline_at = Utc.with_ymd_and_hms(2026, 8, 24, 23, 59, 0).unwrap();
    let assignment = assignment(course, teacher)
        .created_at(Utc.with_ymd_and_hms(2026, 8, 17, 13, 30, 0).unwrap())
        .deadline_at(deadline_at)
        .scored(5)
        .late_policy(7, 1, 4, DeductedFrom::WorkScore)
        .published()
        .build();

    (assignment, deadline_at)
}
