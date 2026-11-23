use std::collections::BTreeSet;

use teloxide::types::{
    ButtonRequest, InlineKeyboardMarkup, KeyboardButton, KeyboardButtonRequestUsers,
    KeyboardMarkup, RequestId,
};

use super::keyboards::{Row, button, course_label, row, subjects_keyboard};
use super::render::button_label;
use crate::app::{
    AdminCourseCard, OfferedCourse, StoredSubject, StudentCard, StudentListItem,
    TeachingCourseDetails,
};
use crate::bot::callbacks::{
    AdminNav, AdminScreen, AdminStep, FlowAction, PageTarget, ReplayableAction, ScreenAction,
};
use crate::bot::texts;
use crate::domain::{
    EducationLevel, Role, StudentGroup, StudentGroupId, SubjectId, TeachingCourseId, User, UserId,
};
use crate::pagination::Page;

pub(crate) const TEACHER_REQUEST_ID: i32 = 1;

const EDUCATION_LEVELS: [EducationLevel; 4] = [
    EducationLevel::Bachelor,
    EducationLevel::Master,
    EducationLevel::Specialist,
    EducationLevel::Postgraduate,
];

fn nav(action: AdminNav) -> String {
    ReplayableAction::Admin(action).callback()
}

fn step(action: AdminStep) -> String {
    FlowAction::Admin(action).callback()
}

fn on_screen(action: AdminScreen) -> String {
    ScreenAction::Admin(action).callback()
}

fn with_pager<T>(rows: &mut Vec<Row>, page: &Page<T>, to: impl Fn(u32) -> String) {
    let previous = page
        .page
        .checked_sub(1)
        .map(|previous| button(texts::BUTTON_PREVIOUS_PAGE, to(previous)));
    let next = page
        .next_page
        .map(|next| button(texts::BUTTON_NEXT_PAGE, to(next)));
    let pager: Row = previous.into_iter().chain(next).collect();
    if !pager.is_empty() {
        rows.push(pager);
    }
}

fn back_home() -> Row {
    row(texts::BUTTON_ADMIN_HOME, nav(AdminNav::Home))
}

pub(crate) fn admin_home_keyboard() -> InlineKeyboardMarkup {
    InlineKeyboardMarkup::new([
        row(texts::BUTTON_ADMIN_STUDENTS, nav(AdminNav::SearchStudents)),
        row(texts::BUTTON_ADMIN_TEACHERS, nav(AdminNav::Teachers(0))),
        row(texts::BUTTON_ADMIN_GROUPS, nav(AdminNav::Groups(0))),
        row(texts::BUTTON_ADMIN_SUBJECTS, nav(AdminNav::Subjects(0))),
        row(texts::BUTTON_ADMIN_COURSES, nav(AdminNav::Courses(0))),
    ])
}

pub(crate) fn admin_search_keyboard(found: &Page<StudentListItem>) -> InlineKeyboardMarkup {
    let mut rows: Vec<Row> = found
        .items
        .iter()
        .map(|student| {
            row(
                button_label(&texts::admin_student_button(
                    &student.display_name,
                    &student.group_code,
                )),
                nav(AdminNav::Student(student.student_id)),
            )
        })
        .collect();
    with_pager(&mut rows, found, |page| {
        on_screen(AdminScreen::SearchPage(page))
    });
    rows.push(row(
        texts::BUTTON_ADMIN_NEW_SEARCH,
        nav(AdminNav::SearchStudents),
    ));
    rows.push(back_home());
    InlineKeyboardMarkup::new(rows)
}

pub(crate) fn admin_student_keyboard(card: &StudentCard) -> InlineKeyboardMarkup {
    let student_id = card.student.id();
    let mut rows = vec![row(
        texts::BUTTON_ADMIN_FIX_NAME,
        nav(AdminNav::RenameStudent(student_id)),
    )];
    if card.blocking_submissions == 0 {
        rows.push(row(
            texts::BUTTON_ADMIN_CHANGE_GROUP,
            nav(AdminNav::ChangeGroup(student_id, 0)),
        ));
    }
    rows.extend(
        card.enrollments
            .iter()
            .filter(|enrollment| enrollment.blocking_submissions == 0)
            .map(|enrollment| {
                row(
                    button_label(&texts::admin_remove_from_subject_button(
                        &enrollment.subject_name,
                    )),
                    nav(AdminNav::Unenroll(student_id, enrollment.course_id)),
                )
            }),
    );
    rows.push(row(
        texts::BUTTON_ADMIN_NEW_SEARCH,
        nav(AdminNav::SearchStudents),
    ));
    rows.push(back_home());
    InlineKeyboardMarkup::new(rows)
}

pub(crate) fn admin_student_groups_keyboard(
    student_id: UserId,
    groups: &Page<StudentGroup>,
    current: StudentGroupId,
) -> InlineKeyboardMarkup {
    let mut rows: Vec<Row> = groups
        .items
        .iter()
        .filter(|group| group.id() != current)
        .map(|group| {
            row(
                button_label(group.code().as_str()),
                step(AdminStep::StudentGroup(group.id())),
            )
        })
        .collect();
    with_pager(&mut rows, groups, |page| {
        nav(AdminNav::ChangeGroup(student_id, page))
    });
    rows.push(row(
        texts::BUTTON_ADMIN_BACK,
        nav(AdminNav::Student(student_id)),
    ));
    InlineKeyboardMarkup::new(rows)
}

pub(crate) fn admin_student_courses_keyboard(
    courses: &[OfferedCourse],
    selected: &BTreeSet<TeachingCourseId>,
) -> InlineKeyboardMarkup {
    subjects_keyboard(
        courses,
        selected,
        |id| FlowAction::Admin(AdminStep::StudentCourse(id)),
        FlowAction::Admin(AdminStep::StudentCoursesDone),
    )
}

pub(crate) fn admin_unenroll_keyboard(student_id: UserId) -> InlineKeyboardMarkup {
    InlineKeyboardMarkup::new([
        row(
            texts::BUTTON_ADMIN_CONFIRM_REMOVE,
            step(AdminStep::ConfirmUnenroll),
        ),
        row(texts::BUTTON_ADMIN_BACK, nav(AdminNav::Student(student_id))),
    ])
}

pub(crate) fn admin_teachers_keyboard(staff: &Page<User>) -> InlineKeyboardMarkup {
    let mut rows: Vec<Row> = staff
        .items
        .iter()
        .map(|user| {
            row(
                button_label(&texts::admin_teacher_button(
                    user.display_name().as_str(),
                    matches!(user.role(), Role::Admin),
                )),
                nav(AdminNav::Teacher(user.id())),
            )
        })
        .collect();
    with_pager(&mut rows, staff, |page| nav(AdminNav::Teachers(page)));
    rows.push(row(
        texts::BUTTON_ADMIN_NEW_TEACHER,
        nav(AdminNav::NewTeacher),
    ));
    rows.push(back_home());
    InlineKeyboardMarkup::new(rows)
}

pub(crate) fn admin_teacher_keyboard(teacher_id: UserId) -> InlineKeyboardMarkup {
    InlineKeyboardMarkup::new([
        row(
            texts::BUTTON_ADMIN_TEACHER_COURSES,
            nav(AdminNav::TeacherCourses(teacher_id, 0)),
        ),
        row(texts::BUTTON_ADMIN_BACK, nav(AdminNav::Teachers(0))),
    ])
}

pub(crate) fn admin_teacher_pick_keyboard() -> KeyboardMarkup {
    let request = KeyboardButtonRequestUsers {
        user_is_bot: Some(false),
        request_name: true,
        ..KeyboardButtonRequestUsers::new(RequestId(TEACHER_REQUEST_ID))
    };
    KeyboardMarkup::new([[KeyboardButton::new(texts::BUTTON_ADMIN_PICK_USER)
        .request(ButtonRequest::RequestUsers(request))]])
    .resize_keyboard()
}

pub(crate) fn admin_keep_name_keyboard(name: &str) -> InlineKeyboardMarkup {
    InlineKeyboardMarkup::new([row(
        button_label(&texts::admin_keep_name_button(name)),
        step(AdminStep::KeepSuggestedName),
    )])
}

pub(crate) fn admin_teacher_courses_keyboard(
    teacher_id: UserId,
    courses: &Page<TeachingCourseDetails>,
) -> InlineKeyboardMarkup {
    let mut rows: Vec<Row> = courses
        .items
        .iter()
        .map(|course| {
            row(
                button_label(&texts::admin_check_button(
                    &course_label(course),
                    course.course.teacher_ids().contains(&teacher_id),
                )),
                on_screen(AdminScreen::ToggleTeacherCourse(course.course.id())),
            )
        })
        .collect();
    with_pager(&mut rows, courses, |page| {
        nav(AdminNav::TeacherCourses(teacher_id, page))
    });
    rows.push(row(
        texts::BUTTON_ADMIN_BACK,
        nav(AdminNav::Teacher(teacher_id)),
    ));
    InlineKeyboardMarkup::new(rows)
}

pub(crate) fn admin_groups_keyboard(groups: &Page<StudentGroup>) -> InlineKeyboardMarkup {
    let mut rows: Vec<Row> = groups
        .items
        .iter()
        .map(|group| {
            row(
                button_label(group.code().as_str()),
                nav(AdminNav::Group(group.id())),
            )
        })
        .collect();
    with_pager(&mut rows, groups, |page| nav(AdminNav::Groups(page)));
    rows.push(row(texts::BUTTON_ADMIN_NEW_GROUP, nav(AdminNav::NewGroup)));
    rows.push(back_home());
    InlineKeyboardMarkup::new(rows)
}

pub(crate) fn admin_group_keyboard(group_id: StudentGroupId) -> InlineKeyboardMarkup {
    InlineKeyboardMarkup::new([
        row(
            texts::BUTTON_ADMIN_RENAME,
            nav(AdminNav::RenameGroup(group_id)),
        ),
        row(
            texts::BUTTON_ADMIN_STUDY_YEAR,
            nav(AdminNav::GroupYear(group_id)),
        ),
        row(texts::BUTTON_ADMIN_BACK, nav(AdminNav::Groups(0))),
    ])
}

pub(crate) fn admin_levels_keyboard() -> InlineKeyboardMarkup {
    let mut rows: Vec<Row> = EDUCATION_LEVELS
        .iter()
        .map(|&level| {
            row(
                texts::education_level_name(level),
                step(AdminStep::Level(level)),
            )
        })
        .collect();
    rows.push(row(texts::BUTTON_ADMIN_BACK, nav(AdminNav::Groups(0))));
    InlineKeyboardMarkup::new(rows)
}

pub(crate) fn admin_years_keyboard(level: EducationLevel, back: AdminNav) -> InlineKeyboardMarkup {
    let years: Row = (1..=level.max_study_year())
        .map(|year| button(texts::admin_year_button(year), step(AdminStep::Year(year))))
        .collect();
    let mut rows: Vec<Row> = years.chunks(3).map(<[_]>::to_vec).collect();
    rows.push(row(texts::BUTTON_ADMIN_BACK, nav(back)));
    InlineKeyboardMarkup::new(rows)
}

pub(crate) fn admin_subjects_keyboard(subjects: &Page<StoredSubject>) -> InlineKeyboardMarkup {
    let mut rows: Vec<Row> = subjects
        .items
        .iter()
        .map(|stored| {
            row(
                button_label(stored.subject.name().as_str()),
                nav(AdminNav::Subject(stored.subject.id())),
            )
        })
        .collect();
    with_pager(&mut rows, subjects, |page| nav(AdminNav::Subjects(page)));
    rows.push(row(
        texts::BUTTON_ADMIN_NEW_SUBJECT,
        nav(AdminNav::NewSubject),
    ));
    rows.push(back_home());
    InlineKeyboardMarkup::new(rows)
}

pub(crate) fn admin_subject_keyboard(subject_id: SubjectId) -> InlineKeyboardMarkup {
    InlineKeyboardMarkup::new([
        row(
            texts::BUTTON_ADMIN_RENAME,
            nav(AdminNav::RenameSubject(subject_id)),
        ),
        row(texts::BUTTON_ADMIN_BACK, nav(AdminNav::Subjects(0))),
    ])
}

pub(crate) fn admin_courses_keyboard(
    courses: &Page<TeachingCourseDetails>,
    closed: bool,
) -> InlineKeyboardMarkup {
    let mut rows: Vec<Row> = courses
        .items
        .iter()
        .map(|course| {
            row(
                button_label(&course_label(course)),
                nav(AdminNav::Course(course.course.id())),
            )
        })
        .collect();
    if closed {
        with_pager(&mut rows, courses, |page| {
            nav(AdminNav::ClosedCourses(page))
        });
        rows.push(row(texts::BUTTON_ADMIN_BACK, nav(AdminNav::Courses(0))));
    } else {
        with_pager(&mut rows, courses, |page| nav(AdminNav::Courses(page)));
        rows.push(row(
            texts::BUTTON_ADMIN_NEW_COURSE,
            nav(AdminNav::NewCourse(0)),
        ));
        rows.push(row(
            texts::BUTTON_ADMIN_CLOSED_COURSES,
            nav(AdminNav::ClosedCourses(0)),
        ));
        rows.push(back_home());
    }
    InlineKeyboardMarkup::new(rows)
}

pub(crate) fn admin_course_keyboard(card: &AdminCourseCard) -> InlineKeyboardMarkup {
    let course_id = card.details.course.id();
    let closed = card.details.course.is_closed();
    let mut rows = vec![row(
        texts::BUTTON_ADMIN_OPEN_COURSE,
        ReplayableAction::Page(PageTarget::Course(course_id), 0).callback(),
    )];
    if !card.actor_teaches {
        rows.push(row(
            texts::BUTTON_ADMIN_ASSIGN_SELF,
            on_screen(AdminScreen::AssignSelf),
        ));
    }
    if closed {
        rows.push(row(
            texts::BUTTON_ADMIN_REOPEN_COURSE,
            on_screen(AdminScreen::Reopen),
        ));
    } else {
        rows.push(row(
            texts::BUTTON_ADMIN_ADD_GROUP,
            nav(AdminNav::AddCourseGroup(course_id, 0)),
        ));
        if !card.sandbox {
            rows.push(row(
                texts::BUTTON_ADMIN_CLOSE_COURSE,
                nav(AdminNav::CloseCourse(course_id)),
            ));
        }
    }
    let back = if closed {
        AdminNav::ClosedCourses(0)
    } else {
        AdminNav::Courses(0)
    };
    rows.push(row(texts::BUTTON_ADMIN_BACK, nav(back)));
    InlineKeyboardMarkup::new(rows)
}

pub(crate) fn admin_course_subjects_keyboard(
    subjects: &Page<StoredSubject>,
) -> InlineKeyboardMarkup {
    let mut rows: Vec<Row> = subjects
        .items
        .iter()
        .map(|stored| {
            row(
                button_label(stored.subject.name().as_str()),
                step(AdminStep::CourseSubject(stored.subject.id())),
            )
        })
        .collect();
    with_pager(&mut rows, subjects, |page| nav(AdminNav::NewCourse(page)));
    rows.push(row(texts::BUTTON_ADMIN_BACK, nav(AdminNav::Courses(0))));
    InlineKeyboardMarkup::new(rows)
}

pub(crate) fn admin_course_groups_keyboard(
    groups: &Page<StudentGroup>,
    selected: &BTreeSet<StudentGroupId>,
) -> InlineKeyboardMarkup {
    let mut rows: Vec<Row> = groups
        .items
        .iter()
        .map(|group| {
            row(
                button_label(&texts::admin_check_button(
                    group.code().as_str(),
                    selected.contains(&group.id()),
                )),
                step(AdminStep::CourseGroup(group.id())),
            )
        })
        .collect();
    with_pager(&mut rows, groups, |page| step(AdminStep::ListPage(page)));
    rows.push(row(
        texts::BUTTON_GROUPS_DONE,
        step(AdminStep::CourseGroupsDone),
    ));
    rows.push(row(texts::BUTTON_ADMIN_BACK, nav(AdminNav::Courses(0))));
    InlineKeyboardMarkup::new(rows)
}

pub(crate) fn admin_course_teachers_keyboard(
    staff: &Page<User>,
    selected: &BTreeSet<UserId>,
) -> InlineKeyboardMarkup {
    let mut rows: Vec<Row> = staff
        .items
        .iter()
        .map(|user| {
            row(
                button_label(&texts::admin_check_button(
                    user.display_name().as_str(),
                    selected.contains(&user.id()),
                )),
                step(AdminStep::CourseTeacher(user.id())),
            )
        })
        .collect();
    with_pager(&mut rows, staff, |page| step(AdminStep::ListPage(page)));
    rows.push(row(
        texts::BUTTON_GROUPS_DONE,
        step(AdminStep::CourseTeachersDone),
    ));
    rows.push(row(texts::BUTTON_ADMIN_BACK, nav(AdminNav::Courses(0))));
    InlineKeyboardMarkup::new(rows)
}

pub(crate) fn admin_add_group_keyboard(
    course_id: TeachingCourseId,
    groups: &Page<StudentGroup>,
) -> InlineKeyboardMarkup {
    let mut rows: Vec<Row> = groups
        .items
        .iter()
        .map(|group| {
            row(
                button_label(group.code().as_str()),
                step(AdminStep::AddGroup(group.id())),
            )
        })
        .collect();
    with_pager(&mut rows, groups, |page| {
        nav(AdminNav::AddCourseGroup(course_id, page))
    });
    rows.push(row(
        texts::BUTTON_ADMIN_BACK,
        nav(AdminNav::Course(course_id)),
    ));
    InlineKeyboardMarkup::new(rows)
}

pub(crate) fn admin_close_keyboard(course_id: TeachingCourseId) -> InlineKeyboardMarkup {
    InlineKeyboardMarkup::new([
        row(
            texts::BUTTON_ADMIN_CONFIRM_CLOSE,
            step(AdminStep::ConfirmClose),
        ),
        row(texts::BUTTON_ADMIN_BACK, nav(AdminNav::Course(course_id))),
    ])
}
