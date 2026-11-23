use std::collections::BTreeSet;

use chrono::{DateTime, Utc};
use sqlx::{PgConnection, PgPool};
use uuid::Uuid;

use crate::app::access::load_admin;
use crate::app::{Actor, AppError, AppResult, Required};
use crate::db;
use crate::domain::{
    DomainError, DomainResult, EducationLevel, StaffRole, StudentGroup, StudentGroupId,
    StudyProgram, Subject, SubjectId, TeachingCourse, TeachingCourseId, TelegramUserId, User,
    UserId,
};
use crate::pagination::{Page, PageRequest};
use crate::read_models::{
    OfferedCourse, StoredSubject, StudentEnrollment, StudentListItem, TeachingCourseDetails,
};

pub const MIN_STUDENT_SEARCH_LEN: usize = 2;

#[derive(Clone, Debug)]
pub struct StudentCard {
    pub student: User,
    pub group: StudentGroup,
    pub enrollments: Vec<StudentEnrollment>,
    pub blocking_submissions: u32,
}

#[derive(Clone, Debug)]
pub struct GroupChangeOffer {
    pub student: User,
    pub group: StudentGroup,
    pub courses: Vec<OfferedCourse>,
    pub enrolled: BTreeSet<TeachingCourseId>,
}

#[derive(Clone, Debug)]
pub struct ChangeStudentGroup {
    pub actor: Actor,
    pub student_id: UserId,
    pub group_id: StudentGroupId,
    pub courses: BTreeSet<TeachingCourseId>,
}

#[derive(Clone, Debug)]
pub struct RegisterTeacher {
    pub actor: Actor,
    pub telegram_user_id: TelegramUserId,
    pub display_name: String,
    pub registered_at: DateTime<Utc>,
}

#[derive(Clone, Debug)]
pub struct TeacherCard {
    pub teacher: User,
    pub courses: Page<TeachingCourseDetails>,
}

#[derive(Clone, Debug)]
pub struct CreateStudentGroup {
    pub actor: Actor,
    pub code: String,
    pub education_level: EducationLevel,
    pub study_year: u8,
}

#[derive(Clone, Debug)]
pub struct CreateCourse {
    pub actor: Actor,
    pub subject_id: SubjectId,
    pub group_ids: BTreeSet<StudentGroupId>,
    pub teacher_ids: BTreeSet<UserId>,
    pub created_at: DateTime<Utc>,
}

#[derive(Clone, Debug)]
pub struct AdminCourseCard {
    pub details: TeachingCourseDetails,
    pub sandbox: bool,
    pub teachers: Vec<User>,
    pub actor_teaches: bool,
}

#[derive(Clone, Debug)]
pub struct ClosePreview {
    pub details: TeachingCourseDetails,
    pub published: u32,
    pub pending: u32,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ClosedCourse {
    pub archived: u32,
    pub pending: u32,
}

#[derive(Clone, Debug)]
pub struct AdminService {
    pool: PgPool,
}

impl AdminService {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    pub async fn ensure_admin(&self, actor: Actor) -> AppResult<User> {
        load_admin(&self.pool, actor, "open admin section").await
    }

    pub async fn search_students(
        &self,
        actor: Actor,
        query: &str,
        page: PageRequest,
    ) -> AppResult<Page<StudentListItem>> {
        load_admin(&self.pool, actor, "search students").await?;
        let pattern = search_pattern(query)?;

        Ok(Page::from_rows(
            db::users::search_students(&self.pool, &pattern, page).await?,
            page,
        ))
    }

    pub async fn student_card(&self, actor: Actor, student_id: UserId) -> AppResult<StudentCard> {
        let mut connection = self.pool.acquire().await?;
        load_admin(&mut *connection, actor, "read student card").await?;
        let (student, group_id) = find_student(&mut connection, student_id).await?;
        let group = db::academic::find_student_group_by_id(&mut *connection, group_id)
            .await?
            .required("student group")?;
        let enrollments = db::enrollments::list_for_student(&mut *connection, student_id).await?;
        let blocking_submissions =
            db::users::count_blocking_submissions(&mut *connection, student_id, None).await?;

        Ok(StudentCard {
            student,
            group,
            enrollments,
            blocking_submissions,
        })
    }

    pub async fn rename_student(
        &self,
        actor: Actor,
        student_id: UserId,
        display_name: String,
    ) -> AppResult<User> {
        let mut tx = self.pool.begin().await?;
        let admin = load_admin(&mut *tx, actor, "rename student").await?;
        let student = lock_student(&mut tx, student_id).await?.0;
        let renamed = student.renamed(display_name)?;
        db::users::update_display_name(&mut *tx, student_id, renamed.display_name().as_str())
            .await?;
        tx.commit().await?;

        log_change("student_renamed", &admin, student_id.as_uuid());
        Ok(renamed)
    }

    pub async fn group_change_offer(
        &self,
        actor: Actor,
        student_id: UserId,
        group_id: StudentGroupId,
    ) -> AppResult<GroupChangeOffer> {
        let mut connection = self.pool.acquire().await?;
        load_admin(&mut *connection, actor, "prepare group change").await?;
        let (student, _) = find_student(&mut connection, student_id).await?;
        ensure_no_submissions(&mut connection, student_id, None).await?;
        let group = db::academic::find_student_group_by_id(&mut *connection, group_id)
            .await?
            .required("student group")?;
        let courses = db::enrollments::offered_courses(&mut *connection, group_id, None).await?;
        let enrolled = db::enrollments::list_for_student(&mut *connection, student_id)
            .await?
            .into_iter()
            .map(|enrollment| enrollment.course_id)
            .filter(|id| courses.iter().any(|course| course.course_id == *id))
            .collect();

        Ok(GroupChangeOffer {
            student,
            group,
            courses,
            enrolled,
        })
    }

    pub async fn change_student_group(&self, command: ChangeStudentGroup) -> AppResult<()> {
        let mut tx = self.pool.begin().await?;
        let admin = load_admin(&mut *tx, command.actor, "change student group").await?;
        lock_student(&mut tx, command.student_id).await?;
        ensure_no_submissions(&mut tx, command.student_id, None).await?;
        let group = db::academic::find_student_group_by_id(&mut *tx, command.group_id)
            .await?
            .required("student group")?;
        let offered = db::enrollments::offered_courses(&mut *tx, group.id(), None).await?;
        if !command
            .courses
            .iter()
            .all(|id| offered.iter().any(|course| course.course_id == *id))
        {
            return Err(DomainError::CourseNotOfferedToStudentGroup.into());
        }

        let courses: Vec<_> = command.courses.iter().copied().collect();
        db::users::update_student_group(&mut *tx, command.student_id, group.id()).await?;
        db::enrollments::retain_only(&mut *tx, command.student_id, &courses).await?;
        db::enrollments::insert(&mut *tx, command.student_id, &courses).await?;
        tx.commit().await?;

        log_change(
            "student_group_changed",
            &admin,
            command.student_id.as_uuid(),
        );
        Ok(())
    }

    pub async fn unenroll_student(
        &self,
        actor: Actor,
        student_id: UserId,
        course_id: TeachingCourseId,
    ) -> AppResult<()> {
        let mut tx = self.pool.begin().await?;
        let admin = load_admin(&mut *tx, actor, "remove student from course").await?;
        lock_student(&mut tx, student_id).await?;
        ensure_no_submissions(&mut tx, student_id, Some(course_id)).await?;
        if !db::enrollments::remove(&mut *tx, student_id, course_id).await? {
            return Err(AppError::NotFound {
                resource: "enrollment",
            });
        }
        tx.commit().await?;

        log_change("student_unenrolled", &admin, student_id.as_uuid());
        Ok(())
    }

    pub async fn list_staff(&self, actor: Actor, page: PageRequest) -> AppResult<Page<User>> {
        load_admin(&self.pool, actor, "list teachers").await?;

        Ok(Page::from_rows(
            db::users::list_staff(&self.pool, page).await?,
            page,
        ))
    }

    pub async fn teacher_card(
        &self,
        actor: Actor,
        teacher_id: UserId,
        page: PageRequest,
    ) -> AppResult<TeacherCard> {
        let mut connection = self.pool.acquire().await?;
        load_admin(&mut *connection, actor, "read teacher card").await?;
        let teacher = find_staff(&mut connection, teacher_id).await?;
        let courses = Page::from_rows(
            db::academic::list_every_course_of_teacher(&mut *connection, teacher_id, page).await?,
            page,
        );

        Ok(TeacherCard { teacher, courses })
    }

    pub async fn register_teacher(&self, command: RegisterTeacher) -> AppResult<User> {
        let mut tx = self.pool.begin().await?;
        let admin = load_admin(&mut *tx, command.actor, "register teacher").await?;
        let teacher = User::new_staff(
            UserId::new(),
            command.telegram_user_id,
            StaffRole::Teacher,
            command.display_name,
            &admin,
            command.registered_at,
        )?;
        db::users::insert(&mut *tx, &teacher).await?;
        tx.commit().await?;

        log_change("teacher_registered", &admin, teacher.id().as_uuid());
        Ok(teacher)
    }

    pub async fn assign_teacher(
        &self,
        actor: Actor,
        teacher_id: UserId,
        course_id: TeachingCourseId,
    ) -> AppResult<()> {
        let mut tx = self.pool.begin().await?;
        let admin = load_admin(&mut *tx, actor, "assign teacher").await?;
        db::academic::find_teaching_course_by_id(&mut *tx, course_id)
            .await?
            .required("teaching course")?;
        let teacher = db::users::find_by_id(&mut *tx, teacher_id)
            .await?
            .required("teacher")?;
        if !teacher.role().can_manage_assignments() {
            return Err(DomainError::CourseTeacherMustBeStaff.into());
        }
        db::academic::add_course_teacher(&mut *tx, course_id, teacher_id).await?;
        tx.commit().await?;

        log_change("teacher_assigned", &admin, teacher_id.as_uuid());
        Ok(())
    }

    pub async fn unassign_teacher(
        &self,
        actor: Actor,
        teacher_id: UserId,
        course_id: TeachingCourseId,
    ) -> AppResult<()> {
        let mut tx = self.pool.begin().await?;
        let admin = load_admin(&mut *tx, actor, "remove teacher from course").await?;
        db::academic::lock_course_teachers(&mut *tx, course_id).await?;
        let course = db::academic::find_teaching_course_by_id(&mut *tx, course_id)
            .await?
            .required("teaching course")?;
        if !course.teacher_ids().contains(&teacher_id) {
            return Ok(());
        }
        if course.teacher_ids().len() == 1 {
            return Err(DomainError::LastCourseTeacher.into());
        }
        db::academic::remove_course_teacher(&mut *tx, course_id, teacher_id).await?;
        tx.commit().await?;

        log_change("teacher_unassigned", &admin, teacher_id.as_uuid());
        Ok(())
    }

    pub async fn list_groups(
        &self,
        actor: Actor,
        page: PageRequest,
    ) -> AppResult<Page<StudentGroup>> {
        load_admin(&self.pool, actor, "list student groups").await?;

        Ok(Page::from_rows(
            db::academic::list_student_groups(&self.pool, page).await?,
            page,
        ))
    }

    pub async fn find_group(
        &self,
        actor: Actor,
        group_id: StudentGroupId,
    ) -> AppResult<StudentGroup> {
        load_admin(&self.pool, actor, "read student group").await?;

        db::academic::find_student_group_by_id(&self.pool, group_id)
            .await?
            .required("student group")
    }

    pub async fn create_group(&self, command: CreateStudentGroup) -> AppResult<StudentGroup> {
        let admin = load_admin(&self.pool, command.actor, "create student group").await?;
        let group = StudentGroup::new(
            StudentGroupId::new(),
            command.code,
            StudyProgram::new(command.education_level, command.study_year)?,
        )?;
        db::academic::insert_student_group(&self.pool, &group).await?;

        log_change("group_created", &admin, group.id().as_uuid());
        Ok(group)
    }

    pub async fn rename_group(
        &self,
        actor: Actor,
        group_id: StudentGroupId,
        code: String,
    ) -> AppResult<StudentGroup> {
        let admin = load_admin(&self.pool, actor, "rename student group").await?;
        let group = db::academic::find_student_group_by_id(&self.pool, group_id)
            .await?
            .required("student group")?;
        let renamed = StudentGroup::new(group.id(), code, group.program())?;
        db::academic::rename_student_group(&self.pool, &renamed).await?;

        log_change("group_renamed", &admin, group_id.as_uuid());
        Ok(renamed)
    }

    pub async fn change_group_year(
        &self,
        actor: Actor,
        group_id: StudentGroupId,
        study_year: u8,
    ) -> AppResult<StudentGroup> {
        let admin = load_admin(&self.pool, actor, "change group study year").await?;
        let group = db::academic::find_student_group_by_id(&self.pool, group_id)
            .await?
            .required("student group")?;
        let program = StudyProgram::new(group.program().education_level(), study_year)?;
        let changed = StudentGroup::new(group.id(), group.code().as_str(), program)?;
        db::academic::update_student_group_year(&self.pool, &changed).await?;

        log_change("group_year_changed", &admin, group_id.as_uuid());
        Ok(changed)
    }

    pub async fn list_subjects(
        &self,
        actor: Actor,
        page: PageRequest,
    ) -> AppResult<Page<StoredSubject>> {
        load_admin(&self.pool, actor, "list subjects").await?;

        Ok(Page::from_rows(
            db::academic::list_subjects(&self.pool, page).await?,
            page,
        ))
    }

    pub async fn find_subject(
        &self,
        actor: Actor,
        subject_id: SubjectId,
    ) -> AppResult<StoredSubject> {
        load_admin(&self.pool, actor, "read subject").await?;

        db::academic::find_subject(&self.pool, subject_id)
            .await?
            .required("subject")
    }

    pub async fn create_subject(&self, actor: Actor, name: String) -> AppResult<Subject> {
        let admin = load_admin(&self.pool, actor, "create subject").await?;
        let subject = Subject::new(SubjectId::new(), name)?;
        db::academic::insert_subject(&self.pool, &subject, false).await?;

        log_change("subject_created", &admin, subject.id().as_uuid());
        Ok(subject)
    }

    pub async fn rename_subject(
        &self,
        actor: Actor,
        subject_id: SubjectId,
        name: String,
    ) -> AppResult<Subject> {
        let admin = load_admin(&self.pool, actor, "rename subject").await?;
        db::academic::find_subject(&self.pool, subject_id)
            .await?
            .required("subject")?;
        let renamed = Subject::new(subject_id, name)?;
        db::academic::rename_subject(&self.pool, &renamed).await?;

        log_change("subject_renamed", &admin, subject_id.as_uuid());
        Ok(renamed)
    }

    pub async fn list_courses(
        &self,
        actor: Actor,
        closed: bool,
        page: PageRequest,
    ) -> AppResult<Page<TeachingCourseDetails>> {
        load_admin(&self.pool, actor, "list courses").await?;

        Ok(Page::from_rows(
            db::academic::list_all_courses(&self.pool, closed, page).await?,
            page,
        ))
    }

    pub async fn list_every_course(
        &self,
        actor: Actor,
        page: PageRequest,
    ) -> AppResult<Page<TeachingCourseDetails>> {
        load_admin(&self.pool, actor, "list courses").await?;

        Ok(Page::from_rows(
            db::academic::list_every_course(&self.pool, page).await?,
            page,
        ))
    }

    pub async fn course_card(
        &self,
        actor: Actor,
        course_id: TeachingCourseId,
    ) -> AppResult<AdminCourseCard> {
        let mut connection = self.pool.acquire().await?;
        let admin = load_admin(&mut *connection, actor, "read course card").await?;
        let details = db::academic::find_teaching_course_details(&mut *connection, course_id)
            .await?
            .required("teaching course")?;
        let sandbox = db::academic::find_subject(&mut *connection, details.subject.id())
            .await?
            .required("subject")?
            .sandbox;
        let teachers = db::users::list_course_teachers(&mut *connection, course_id).await?;
        let actor_teaches = details.course.teacher_ids().contains(&admin.id());

        Ok(AdminCourseCard {
            details,
            sandbox,
            teachers,
            actor_teaches,
        })
    }

    pub async fn groups_free_for_subject(
        &self,
        actor: Actor,
        subject_id: SubjectId,
        page: PageRequest,
    ) -> AppResult<Page<StudentGroup>> {
        load_admin(&self.pool, actor, "list groups for a course").await?;

        Ok(Page::from_rows(
            db::academic::list_groups_free_for_subject(&self.pool, subject_id, page).await?,
            page,
        ))
    }

    pub async fn create_course(&self, command: CreateCourse) -> AppResult<TeachingCourse> {
        let mut tx = self.pool.begin().await?;
        let admin = load_admin(&mut *tx, command.actor, "create course").await?;
        if command.group_ids.is_empty() || command.teacher_ids.is_empty() {
            return Err(DomainError::CourseCompositionEmpty.into());
        }
        let subject = db::academic::find_subject(&mut *tx, command.subject_id)
            .await?
            .required("subject")?
            .subject;
        let mut groups = Vec::with_capacity(command.group_ids.len());
        for &id in &command.group_ids {
            groups.push(
                db::academic::find_student_group_by_id(&mut *tx, id)
                    .await?
                    .required("student group")?,
            );
        }
        let mut teachers = Vec::with_capacity(command.teacher_ids.len());
        for &id in &command.teacher_ids {
            teachers.push(
                db::users::find_by_id(&mut *tx, id)
                    .await?
                    .required("teacher")?,
            );
        }
        let course = TeachingCourse::new(
            TeachingCourseId::new(),
            &subject,
            &groups.iter().collect::<Vec<_>>(),
            &teachers.iter().collect::<Vec<_>>(),
            command.created_at,
        )?;
        db::academic::insert_teaching_course(&mut tx, &course).await?;
        tx.commit().await?;

        log_change("course_created", &admin, course.id().as_uuid());
        Ok(course)
    }

    pub async fn add_course_group(
        &self,
        actor: Actor,
        course_id: TeachingCourseId,
        group_id: StudentGroupId,
    ) -> AppResult<()> {
        let mut tx = self.pool.begin().await?;
        let admin = load_admin(&mut *tx, actor, "add group to course").await?;
        db::academic::lock_course_for_share(&mut *tx, course_id).await?;
        let course = db::academic::find_teaching_course_by_id(&mut *tx, course_id)
            .await?
            .required("teaching course")?;
        course.ensure_open()?;
        db::academic::find_student_group_by_id(&mut *tx, group_id)
            .await?
            .required("student group")?;
        db::academic::add_teaching_course_group(&mut *tx, course_id, group_id).await?;
        tx.commit().await?;

        log_change("course_group_added", &admin, course_id.as_uuid());
        Ok(())
    }

    pub async fn close_preview(
        &self,
        actor: Actor,
        course_id: TeachingCourseId,
    ) -> AppResult<ClosePreview> {
        let mut connection = self.pool.acquire().await?;
        load_admin(&mut *connection, actor, "preview course closing").await?;
        let details = load_closable(&mut connection, course_id).await?;
        let published =
            db::assignments::count_published_for_course(&mut *connection, course_id).await?;
        let pending =
            db::submissions::count_pending_for_course(&mut *connection, course_id).await?;

        Ok(ClosePreview {
            details,
            published,
            pending,
        })
    }

    pub async fn close_course(
        &self,
        actor: Actor,
        course_id: TeachingCourseId,
        now: DateTime<Utc>,
    ) -> AppResult<ClosedCourse> {
        let mut tx = self.pool.begin().await?;
        let admin = load_admin(&mut *tx, actor, "close course").await?;
        let details = load_closable(&mut tx, course_id).await?;

        if !db::academic::mark_course_closed(&mut *tx, course_id, now).await? {
            return Err(DomainError::CourseClosed.into());
        }
        let generation = db::course_invites::lock_generation(&mut *tx, course_id)
            .await?
            .required("course invite")?;
        db::course_invites::rotate(&mut *tx, course_id, generation).await?;

        let published = db::assignments::lock_published_for_course(&mut *tx, course_id).await?;
        let archived = db::checked_num_cast("admin.archived_assignments", published.len())?;
        for mut stored in published {
            stored.assignment.archive(&details.course, &admin)?;
            db::assignments::update(&mut *tx, &stored.assignment, stored.revision).await?;
        }
        let pending = db::submissions::count_pending_for_course(&mut *tx, course_id).await?;
        tx.commit().await?;

        log_change("course_closed", &admin, course_id.as_uuid());
        Ok(ClosedCourse { archived, pending })
    }

    pub async fn reopen_course(&self, actor: Actor, course_id: TeachingCourseId) -> AppResult<()> {
        let mut connection = self.pool.acquire().await?;
        let admin = load_admin(&mut *connection, actor, "reopen course").await?;
        db::academic::find_teaching_course_by_id(&mut *connection, course_id)
            .await?
            .required("teaching course")?;
        if !db::academic::mark_course_open(&mut *connection, course_id).await? {
            return Err(DomainError::CourseNotClosed.into());
        }

        log_change("course_reopened", &admin, course_id.as_uuid());
        Ok(())
    }
}

fn log_change(action: &'static str, admin: &User, target: Uuid) {
    tracing::info!(
        action,
        actor_id = %admin.id().as_uuid(),
        target_id = %target,
        "admin change"
    );
}

fn search_pattern(query: &str) -> DomainResult<String> {
    let query = query.split_whitespace().collect::<Vec<_>>().join(" ");
    if query.chars().count() < MIN_STUDENT_SEARCH_LEN {
        return Err(DomainError::SearchQueryTooShort {
            min: MIN_STUDENT_SEARCH_LEN,
        });
    }
    let escaped = query
        .to_lowercase()
        .replace('ё', "е")
        .replace('\\', "\\\\")
        .replace('%', "\\%")
        .replace('_', "\\_");

    Ok(format!("%{escaped}%"))
}

async fn find_student(
    connection: &mut PgConnection,
    student_id: UserId,
) -> AppResult<(User, StudentGroupId)> {
    db::users::find_by_id(connection, student_id)
        .await?
        .and_then(|user| user.student_group_id().map(|group_id| (user, group_id)))
        .required("student")
}

async fn lock_student(
    connection: &mut PgConnection,
    student_id: UserId,
) -> AppResult<(User, StudentGroupId)> {
    db::users::find_for_update(connection, student_id)
        .await?
        .and_then(|user| user.student_group_id().map(|group_id| (user, group_id)))
        .required("student")
}

async fn find_staff(connection: &mut PgConnection, teacher_id: UserId) -> AppResult<User> {
    db::users::find_by_id(connection, teacher_id)
        .await?
        .filter(|user| user.role().can_manage_assignments())
        .required("teacher")
}

async fn ensure_no_submissions(
    connection: &mut PgConnection,
    student_id: UserId,
    course_id: Option<TeachingCourseId>,
) -> AppResult<()> {
    if db::users::count_blocking_submissions(connection, student_id, course_id).await? > 0 {
        return Err(DomainError::StudentHasSubmissions.into());
    }

    Ok(())
}

async fn load_closable(
    connection: &mut PgConnection,
    course_id: TeachingCourseId,
) -> AppResult<TeachingCourseDetails> {
    let details = db::academic::find_teaching_course_details(&mut *connection, course_id)
        .await?
        .required("teaching course")?;
    details.course.ensure_open()?;
    let subject = db::academic::find_subject(connection, details.subject.id())
        .await?
        .required("subject")?;
    if subject.sandbox {
        return Err(DomainError::SandboxCourseCannotBeClosed.into());
    }

    Ok(details)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_query_shorter_than_two_characters_is_rejected() {
        for query in ["", " ", "а", " а "] {
            assert_eq!(
                search_pattern(query),
                Err(DomainError::SearchQueryTooShort { min: 2 })
            );
        }
    }

    #[test]
    fn a_query_is_folded_like_the_stored_name() {
        assert_eq!(search_pattern(" СЕМЁНОВ ").unwrap(), "%семенов%");
    }

    #[test]
    fn pattern_characters_are_escaped() {
        assert_eq!(search_pattern("%_\\").unwrap(), "%\\%\\_\\\\%");
    }
}
