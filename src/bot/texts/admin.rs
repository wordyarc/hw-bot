use crate::domain::EducationLevel;

pub const BUTTON_ADMIN: &str = "🛠 Админ";
pub const ADMIN_HELP: &str = "Раздел «🛠 Админ» открывает управление студентами, преподавателями и учебной структурой. Очередь и «Мои предметы» показывают только предметы, где вы назначены преподавателем.";
pub const ADMIN_HOME: &str = "🛠 Раздел администратора\n\nЗдесь можно найти студента, завести преподавателя и изменить учебную структуру.";
pub const BUTTON_ADMIN_STUDENTS: &str = "🎓 Найти студента";
pub const BUTTON_ADMIN_TEACHERS: &str = "🧑‍🏫 Преподаватели";
pub const BUTTON_ADMIN_GROUPS: &str = "👥 Группы";
pub const BUTTON_ADMIN_SUBJECTS: &str = "📚 Справочник предметов";
pub const BUTTON_ADMIN_COURSES: &str = "🗂 Предметы групп";
pub const BUTTON_ADMIN_HOME: &str = "⬅️ В раздел";
pub const BUTTON_ADMIN_BACK: &str = "⬅️ Назад";
pub const BUTTON_ADMIN_NEW_SEARCH: &str = "🔎 Новый поиск";
pub const BUTTON_ADMIN_RENAME: &str = "✏️ Переименовать";
pub const BUTTON_ADMIN_FIX_NAME: &str = "✏️ Исправить имя";
pub const BUTTON_ADMIN_CHANGE_GROUP: &str = "🔁 Сменить группу";
pub const BUTTON_ADMIN_CONFIRM_REMOVE: &str = "➖ Снять с предмета";
pub const BUTTON_ADMIN_NEW_TEACHER: &str = "➕ Завести преподавателя";
pub const BUTTON_ADMIN_TEACHER_COURSES: &str = "📘 Назначить на предметы";
pub const BUTTON_ADMIN_PICK_USER: &str = "👤 Выбрать пользователя";
pub const BUTTON_ADMIN_NEW_GROUP: &str = "➕ Новая группа";
pub const BUTTON_ADMIN_STUDY_YEAR: &str = "📅 Учебный год";
pub const BUTTON_ADMIN_NEW_SUBJECT: &str = "➕ Новый предмет";
pub const BUTTON_ADMIN_NEW_COURSE: &str = "➕ Создать предмет для групп";
pub const BUTTON_ADMIN_CLOSED_COURSES: &str = "🏁 Завершённые";
pub const BUTTON_ADMIN_ADD_GROUP: &str = "➕ Добавить группу";
pub const BUTTON_ADMIN_ASSIGN_SELF: &str = "🙋 Назначить себя";
pub const BUTTON_ADMIN_OPEN_COURSE: &str = "📘 Открыть предмет";
pub const BUTTON_ADMIN_CLOSE_COURSE: &str = "🏁 Завершить предмет";
pub const BUTTON_ADMIN_CONFIRM_CLOSE: &str = "🏁 Да, завершить";
pub const BUTTON_ADMIN_REOPEN_COURSE: &str = "↩️ Вернуть предмет";
pub const BUTTON_CLOSED_COURSES: &str = "🏁 Завершённые предметы";

pub const ADMIN_STUDENT_QUERY_PROMPT: &str =
    "Введите часть имени или фамилии студента, не меньше двух символов.";
pub const ADMIN_STUDENTS_FOUND: &str = "🎓 Результаты поиска:";
pub const ADMIN_STUDENTS_NOT_FOUND: &str = "🎓 Никого не нашлось. Попробуйте другую часть имени.";
pub const ADMIN_SEARCH_DONE: &str = "Поиск выполнен.";
pub const ADMIN_STUDENT_NAME_PROMPT: &str = "Введите исправленное имя студента.";
pub const ADMIN_CHOOSE_NEW_GROUP: &str = "🔁 Выберите новую группу студента:";
pub const ADMIN_TEXT_REQUIRED: &str = "Отправьте ответ текстом или /cancel для отмены.";
pub const ADMIN_CHOOSE_AT_LEAST_ONE: &str = "Отметьте хотя бы один вариант.";
pub const ADMIN_TELL_STUDENT: &str = "Бот студенту ничего не отправил, сообщите ему об этом сами.";
pub const ADMIN_TELL_TEACHER: &str =
    "Бот преподавателю ничего не отправил, сообщите ему об этом сами.";
pub const ADMIN_TEACHERS: &str = "🧑‍🏫 Преподаватели и администраторы:";
pub const ADMIN_TEACHER_USER_PROMPT: &str =
    "Выберите пользователя кнопкой внизу или отправьте его числовой Telegram ID. Отмена: /cancel.";
pub const ADMIN_TEACHER_ID_INVALID: &str =
    "Telegram ID должен быть положительным числом. Попробуйте ещё раз или отправьте /cancel.";
pub const ADMIN_TEACHER_NAME_PROMPT: &str = "Введите имя преподавателя, как его увидят студенты.";
pub const ADMIN_SUGGESTED_NAME: &str = "Можно оставить имя из Telegram:";
pub const ADMIN_USER_ALREADY_REGISTERED: &str =
    "Этот пользователь уже зарегистрирован в боте. Его учётная запись не изменена.";
pub const ADMIN_TEACHER_REGISTERED: &str =
    "✅ Преподаватель заведён. Попросите его открыть бота и нажать /start.";
pub const ADMIN_TEACHER_COURSES: &str = "📘 Отметьте предметы преподавателя. Изменения сохраняются сразу. Бот преподавателю ничего не отправит, сообщите ему об этом сами.";
pub const ADMIN_GROUPS: &str = "👥 Группы:";
pub const ADMIN_NO_GROUPS: &str = "👥 Групп пока нет.";
pub const ADMIN_GROUP_CODE_PROMPT: &str = "Введите код новой группы.";
pub const ADMIN_GROUP_RENAME_PROMPT: &str = "Введите новый код группы.";
pub const ADMIN_CHOOSE_LEVEL: &str = "Выберите уровень образования:";
pub const ADMIN_CHOOSE_YEAR: &str = "Выберите курс обучения:";
pub const ADMIN_GROUP_CREATED: &str = "✅ Группа создана.";
pub const ADMIN_GROUP_RENAMED: &str = "✅ Код группы изменён.";
pub const ADMIN_GROUP_YEAR_CHANGED: &str = "✅ Учебный год изменён.";
pub const ADMIN_SUBJECTS: &str = "📚 Справочник предметов:";
pub const ADMIN_NO_SUBJECTS: &str = "📚 Предметов пока нет.";
pub const ADMIN_SUBJECT_NAME_PROMPT: &str = "Введите название нового предмета.";
pub const ADMIN_SUBJECT_RENAME_PROMPT: &str = "Введите новое название предмета.";
pub const ADMIN_SUBJECT_CREATED: &str = "✅ Предмет создан.";
pub const ADMIN_SUBJECT_RENAMED: &str = "✅ Название предмета изменено.";
pub const ADMIN_COURSES: &str = "🗂 Предметы групп:";
pub const ADMIN_NO_COURSES: &str = "🗂 Предметов для групп пока нет.";
pub const ADMIN_CLOSED_COURSES: &str = "🏁 Завершённые предметы:";
pub const ADMIN_NO_CLOSED_COURSES: &str = "🏁 Завершённых предметов нет.";
pub const ADMIN_CHOOSE_COURSE_SUBJECT: &str = "Выберите предмет из справочника:";
pub const ADMIN_CHOOSE_COURSE_GROUPS: &str =
    "Отметьте группы. Группы, уже проходящие этот предмет, не показаны.";
pub const ADMIN_CHOOSE_COURSE_TEACHERS: &str = "Отметьте преподавателей предмета:";
pub const ADMIN_COURSE_CREATED: &str = "✅ Предмет для групп создан.";
pub const ADMIN_CHOOSE_GROUP_TO_ADD: &str =
    "Выберите группу. Прежние задания ей не выдаются, студенты записываются сами.";
pub const ADMIN_NO_FREE_GROUPS: &str = "Свободных групп для этого предмета нет.";
pub const ADMIN_GROUP_ADDED: &str = "✅ Группа добавлена.";
pub const ADMIN_SELF_ASSIGNED: &str = "✅ Вы назначены преподавателем, работы появятся в очереди.";
pub const ADMIN_COURSE_REOPENED: &str =
    "↩️ Предмет возвращён. Ссылка новая, задания остаются в архиве и публикуются заново.";
pub const ADMIN_CLOSE_NUMBERS_CHANGED: &str =
    "Числа успели измениться. Проверьте их и подтвердите ещё раз.";

pub fn education_level_name(level: EducationLevel) -> &'static str {
    match level {
        EducationLevel::Bachelor => "Бакалавриат",
        EducationLevel::Master => "Магистратура",
        EducationLevel::Specialist => "Специалитет",
        EducationLevel::Postgraduate => "Аспирантура",
    }
}

pub fn admin_year_button(year: u8) -> String {
    format!("{year} курс")
}

pub fn admin_check_button(label: &str, checked: bool) -> String {
    let mark = if checked { "✅" } else { "⬜️" };
    format!("{mark} {label}")
}

pub fn admin_student_button(name: &str, group: &str) -> String {
    format!("{name} · {group}")
}

pub fn admin_remove_from_subject_button(subject: &str) -> String {
    format!("➖ {subject}")
}

pub fn admin_student_card(
    name: &str,
    group: &str,
    subjects: &[&str],
    blocking_submissions: u32,
) -> String {
    let subjects = if subjects.is_empty() {
        "Предметы: не записан".to_owned()
    } else {
        format!("Предметы: {}", subjects.join(", "))
    };
    let reason = if blocking_submissions > 0 {
        format!(
            "\n\nСдач: {blocking_submissions}. Группу сменить нельзя, исправление только средствами БД."
        )
    } else {
        String::new()
    };
    format!("🎓 {name}\nГруппа: {group}\n{subjects}{reason}")
}

pub fn admin_student_renamed() -> String {
    format!("✅ Имя исправлено. {ADMIN_TELL_STUDENT}")
}

pub fn admin_student_moved() -> String {
    format!("✅ Группа изменена. {ADMIN_TELL_STUDENT}")
}

pub fn admin_student_unenrolled() -> String {
    format!("✅ Студент снят с предмета. {ADMIN_TELL_STUDENT}")
}

pub fn admin_choose_student_courses(name: &str, group: &str) -> String {
    format!("{name} переходит в группу {group}. Отметьте предметы этой группы:")
}

pub fn admin_confirm_unenroll(name: &str, subject: &str) -> String {
    format!("Снять {name} с предмета «{subject}»? Студент сможет записаться снова по ссылке.")
}

pub fn admin_teacher_button(name: &str, is_admin: bool) -> String {
    if is_admin {
        format!("{name} · админ")
    } else {
        name.to_owned()
    }
}

pub fn admin_keep_name_button(name: &str) -> String {
    format!("✅ {name}")
}

pub fn admin_teacher_card(name: &str, role: &str, courses: &[String]) -> String {
    let courses = if courses.is_empty() {
        "Предметов нет".to_owned()
    } else {
        format!("Предметы:\n{}", courses.join("\n"))
    };
    format!("🧑‍🏫 {name}\nРоль: {role}\n{courses}")
}

pub fn admin_group_card(code: &str, level: EducationLevel, year: u8) -> String {
    format!("👥 {code}\n{}, {year} курс", education_level_name(level))
}

pub fn admin_subject_card(name: &str, sandbox: bool) -> String {
    if sandbox {
        format!("📚 {name}\nПесочница")
    } else {
        format!("📚 {name}")
    }
}

pub fn admin_course_card(
    subject: &str,
    groups: &[&str],
    teachers: &[&str],
    closed: bool,
    sandbox: bool,
) -> String {
    let status = match (closed, sandbox) {
        (true, _) => "\nСтатус: завершён",
        (false, true) => "\nПесочница",
        (false, false) => "",
    };
    format!(
        "🗂 {subject}\nГруппы: {}\nПреподаватели: {}{status}",
        groups.join(", "),
        teachers.join(", ")
    )
}

pub fn admin_close_confirmation(subject: &str, published: u32, pending: u32) -> String {
    format!(
        "🏁 Завершить предмет «{subject}»?\n\nОпубликованных заданий уйдёт в архив: {published}.\nНепроверенных работ останется в очереди: {pending}.\n\nСсылка перестанет действовать. Предмет можно будет вернуть, но задания придётся публиковать заново."
    )
}

pub fn admin_course_closed(archived: u32, pending: u32) -> String {
    format!(
        "🏁 Предмет завершён. В архиве заданий: {archived}. Работ на проверке: {pending}, они остаются в очереди."
    )
}
