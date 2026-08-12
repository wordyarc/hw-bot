use sqlx::PgPool;

use crate::app::{
    AssignmentService, CourseService, FilePolicy, FileService, ReviewService, SubmissionService,
    UserService,
};

#[derive(Clone, Debug)]
pub struct ApplicationContainer {
    pub users: UserService,
    pub assignments: AssignmentService,
    pub files: FileService,
    pub submissions: SubmissionService,
    pub courses: CourseService,
    pub reviews: ReviewService,
}

impl ApplicationContainer {
    pub fn new(pool: PgPool, file_policy: FilePolicy) -> Self {
        Self {
            users: UserService::new(pool.clone()),
            assignments: AssignmentService::new(pool.clone()),
            files: FileService::new(pool.clone(), file_policy),
            submissions: SubmissionService::new(pool.clone()),
            courses: CourseService::new(pool.clone()),
            reviews: ReviewService::new(pool),
        }
    }
}
