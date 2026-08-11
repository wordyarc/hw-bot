use sqlx::PgPool;

use crate::app::{AssignmentService, CourseService, ReviewService, SubmissionService, UserService};

#[derive(Clone, Debug)]
pub struct ApplicationContainer {
    pub users: UserService,
    pub assignments: AssignmentService,
    pub submissions: SubmissionService,
    pub courses: CourseService,
    pub reviews: ReviewService,
}

impl ApplicationContainer {
    pub fn new(pool: PgPool) -> Self {
        Self {
            users: UserService::new(pool.clone()),
            assignments: AssignmentService::new(pool.clone()),
            submissions: SubmissionService::new(pool.clone()),
            courses: CourseService::new(pool.clone()),
            reviews: ReviewService::new(pool),
        }
    }
}
