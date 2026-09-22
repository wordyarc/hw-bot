use chrono::{DateTime, Utc};

use crate::domain::{AssignmentStatus, Grade, Role, SubmissionStatus};

pub const BUTTON_ASSIGNMENTS: &str = "📚 Мои задания";
pub const BUTTON_ARCHIVE: &str = "🗄 Архив";
pub const BUTTON_QUEUE: &str = "⏳ Очередь проверки";
pub const BUTTON_REVIEW: &str = "✅ Проверить работы";
pub const BUTTON_COURSES: &str = "📘 Мои курсы";
pub const BUTTON_PROFILE: &str = "👤 Профиль";
pub const BUTTON_HELP: &str = "❓ Помощь";
pub const MENU_PLACEHOLDER: &str = "Выберите пункт меню";

pub const GUEST_NEEDS_INVITE: &str = "Вы ещё не зарегистрированы, поэтому задания пока недоступны.\n\n\
    Если вы студент, попросите у преподавателя ссылку для вашей группы и перейдите по ней.\n\
    Если вы преподаватель, попросите администратора бота добавить вас.";
pub const GUEST_START_DESCRIPTION: &str = "Как получить доступ";
pub const GUEST_HELP_DESCRIPTION: &str = "Помощь с подключением";
pub const USE_THE_MENU: &str = "Выберите пункт в меню под полем ввода или отправьте /menu.";
pub const SUBMISSION_PROMPT: &str = "Отправьте работу одним сообщением: ссылкой или текстом.";
pub const STUDENT_QUEUE_EMPTY: &str = "⏳ Сейчас у вас нет работ на проверке.";
pub const QUEUE_NO_TEACHER: &str =
    "На курс ещё не назначен преподаватель. Обратитесь к администратору бота.";
pub const QUEUE_FIRST: &str = "Ваша работа первая в очереди.";
pub const REVIEW_QUEUE_EMPTY: &str = "🎉 Работ на проверку пока нет.";
pub const REVIEW_COMMENT_PROMPT: &str =
    "Напишите студенту комментарий. Он нужен и при принятии, и при отклонении работы:";
pub const REVIEW_GRADING_CHANGED: &str = "У задания изменилась шкала оценивания, поэтому решение не сохранилось. Проверьте работу ещё раз по обновлённой карточке.";
pub const FILE_DOWNLOAD_FAILED: &str = "Не удалось скачать файл. Попробуйте ещё раз.";
pub const STALE_BUTTON: &str =
    "Эта кнопка уже неактуальна, действие не выполнено. Откройте /menu и попробуйте снова.";

pub const FILE_DELETE_CONFIRM: &str = "Удалить вложение из уже выданного задания?";
pub const LATE_POLICY_QUESTION: &str = "Разрешить сдавать работы с опозданием?";
pub const SUBMISSIONS_CLOSE_LABEL: &str = "Приём работ:";
pub const LATE_PENALTY_LABEL: &str = "Штраф, если сдать сейчас:";
pub const DEADLINE_QUESTION: &str = "До какого числа можно сдать работу? Позже бот её не примет.";
pub const NO_PENALTY_QUESTION: &str = "До какого числа можно сдать без штрафа?";
pub const DEADLINE_IN_FUTURE: &str = "Этот срок уже прошёл. Выберите будущую дату или время.";
pub const DEADLINE_PASSED: &str = "срок прошёл";
pub const GRADING_QUESTION: &str = "Как будете оценивать работу?";
pub const ATTEMPTS_QUESTION: &str = "Сколько попыток будет у каждого студента?";
pub const ASSIGNMENT_TITLE_PROMPT: &str = "Напишите короткое название задания:";
pub const ASSIGNMENT_BODY_PROMPT: &str = "Пришлите текст задания одним сообщением:";
pub const PERIOD_PROMPT: &str =
    "Сколько дней в одном периоде опоздания? Напишите целое число от 1 до 365:";
pub const TERMS_CONFIRM: &str = "Всё верно? Проверьте условия перед сохранением:";
pub const SCHEDULE_HEADER: &str = "📅 Баллы с учётом опоздания:";
pub const TERMS_SAVED: &str = "Новые условия сохранены.";
pub const DRAFT_CREATED: &str = "📝 Черновик сохранён. Перед публикацией проверьте условия:";
pub const CHOOSE_BUTTON: &str = "Нажмите нужную кнопку выше. Чтобы отменить, отправьте /cancel.";
pub const CHOOSE_REVIEW_DECISION: &str = "Нажмите «Принять» или «Отклонить» под работой.";
pub const CHOOSE_ASSIGNMENT_ACTION: &str = "Нажмите нужную кнопку под заданием.";
pub const CHOOSE_ACTION_ABOVE: &str = "Нажмите нужную кнопку выше.";
pub const CHOOSE_FILE_ACTION: &str = "Нажмите нужную кнопку под списком вложений.";
pub const CONFIRM_FILE_DELETION: &str = "Чтобы удалить файл, нажмите кнопку подтверждения выше.";
pub const CHOOSE_STUDENT: &str = "Нажмите на имя студента в списке выше.";
pub const STRICT_AFTER_DEADLINE: &str = "Позже сдать работу нельзя.";
pub const BUTTON_STRICT: &str = "Нет, закрыть приём";
pub const BUTTON_LATE: &str = "Да, со штрафом";
pub const BUTTON_EDIT_DATES: &str = "⏰ Сроки и штраф";
pub const BUTTON_SAVE_TERMS: &str = "Сохранить условия";
pub const BUTTON_CANCEL_TERMS: &str = "Отменить изменения";
pub const SUBMIT_UNTIL: &str = "Срок:";
pub const NO_PENALTY_UNTIL: &str = "Срок без штрафа:";
pub const PENALTY_LABEL: &str = "Штраф:";
pub const PERIOD_LABEL: &str = "Период:";
pub const LATENESS_LABEL: &str = "Опоздание:";
pub const CLOSED_ON: &str = "Приём работ закрыт";
pub const WHOLE_DAY_HINT: &str = "Если указать только дату, работу можно сдать до конца этого дня. Дату можно написать сообщением.";
pub const MAX_SCORE_LABEL: &str = "🏅 Максимум:";
pub const PASS_FAIL_LABEL: &str = "🏅 Оценивание: зачёт/незачёт";
pub const BUTTON_OTHER_NUMBER: &str = "Другое число";
pub const BUTTON_SAVE_DRAFT: &str = "💾 Сохранить черновик";
pub const BUTTON_CANCEL_CREATION: &str = "✖️ Отменить создание";
pub const BUTTON_BACK: &str = "⬅️ Назад";
pub const BUTTON_DATE_OPTIONS: &str = "⬅️ К способам выбора";
pub const BUTTON_STAY: &str = "Продолжить настройку";
pub const BUTTON_EXIT: &str = "Выйти без сохранения";
pub const TERMS_EXIT_QUESTION: &str =
    "Выйти без сохранения? Всё, что вы ввели в этой настройке, будет потеряно.";
pub const DRAFT_DISCARDED: &str =
    "Создание отменено. Введённые данные не сохранены, задание не создано.";
pub const TERMS_DISCARDED: &str = "Правка отменена. У задания остались прежние условия.";
pub const TERMS_CONFLICT: &str = "Пока вы меняли условия, задание изменил другой преподаватель. Правка не сохранена, ниже актуальные условия.";
pub const DATE_FORMAT: &str = "Пришлите дату в формате ДД.ММ.ГГГГ или ДД.ММ.ГГГГ ЧЧ:ММ.";
pub const TIMEZONE_LABEL: &str = "🕒 Часовой пояс:";
pub const INTEGER_REQUIRED: &str = "Напишите целое число, без дробной части.";
pub const INVALID_TERMS: &str =
    "С такими условиями продолжить нельзя. Проверьте срок и настройки опоздания.";
pub const HORIZON_OUT_OF_RANGE: &str = "При таких настройках дата закрытия приёма выходит за допустимый диапазон. Выберите другой срок или сократите опоздание.";
pub const DEADLINE_AFTER_CREATION: &str = "Срок сдачи должен быть позже даты создания задания.";
pub const INVALID_DURATION_DATE: &str =
    "Нужна дата позже срока сдачи, но в пределах разрешённого опоздания.";
pub const DURATION_QUESTION: &str = "Как долго принимать работы после срока?";
pub const DURATION_DATE_QUESTION: &str = "До какой даты принимать работы с опозданием? Бот округлит опоздание вверх до целого числа периодов и предложит подтвердить итоговую дату.";
pub const PAST_PENALTY_WARNING: &str = "⚠️ Этот срок уже прошёл. Штраф начнёт действовать сразу.";
pub const PAST_STRICT_WARNING: &str =
    "⚠️ Этот срок уже прошёл. Приём закроется сразу после сохранения.";
pub const NAME_REQUIRED: &str = "Имя пустое. Напишите, как вас зовут.";
pub const TITLE_REQUIRED: &str = "Название пустое. Напишите, как называется задание.";
pub const BODY_REQUIRED: &str = "Текст задания пустой. Напишите, что нужно сделать студентам.";
pub const TEXT_REQUIRED: &str = "На этом шаге нужен текст. Напишите ответ сообщением.";
pub const REGISTRATION_COMPLETE: &str = "🎉 Вы зарегистрированы! Теперь можно открыть задания.";
pub const FILE_SIZE_MISMATCH: &str =
    "Размер скачанного файла не совпал с тем, который сообщил Telegram. Пришлите файл ещё раз.";
pub const INVITE_REUSE_HINT: &str =
    "Отправьте эту ссылку студентам группы. По ней может зарегистрироваться каждый из них.";
pub const INVITE_ROTATION_CONFIRM: &str = "Заменить ссылку на новую? Старая сразу перестанет работать во всех курсах этой группы. У тех, кто уже зарегистрировался, доступ сохранится.";
pub const REVIEW_SAVED: &str =
    "✅ Решение сохранено. Бот отправит студенту результат и ваш комментарий.";

pub fn timezone_note(timezone: &str) -> String {
    format!("{TIMEZONE_LABEL} {timezone}. Все сроки показываются и вводятся в нём.")
}

pub fn registration_prompt(group: &str) -> String {
    format!("Вы регистрируетесь в группе {group}.\n\nКак вас зовут?")
}

pub fn home_greeting(name: &str) -> String {
    format!("👋 Здравствуйте, {name}!\nВыберите, что хотите сделать, в меню под полем ввода.")
}

pub fn text_too_long(field: &str, max_len: usize) -> String {
    format!("{field}: не больше {max_len} символов. Сократите текст и отправьте ещё раз.")
}

pub fn review_score_question(max_score: u16) -> String {
    format!("Сколько баллов поставить за работу? Максимум — {max_score}. Штраф бот вычтет сам.")
}

pub fn horizon_too_long(max_days: u32) -> String {
    format!(
        "Можно разрешить не больше {max_days} дней опоздания. Сократите период или уменьшите число периодов."
    )
}

pub fn lateness_duration(days: u16, periods: u16) -> String {
    if days == 7 {
        format!("{periods} нед.")
    } else {
        format!(
            "{periods} {} по {days} дн.",
            plural(u32::from(periods), "период", "периода", "периодов")
        )
    }
}

fn plural(value: u32, one: &'static str, few: &'static str, many: &'static str) -> &'static str {
    match (value % 100, value % 10) {
        (11..=14, _) => many,
        (_, 1) => one,
        (_, 2..=4) => few,
        _ => many,
    }
}

pub fn periods_question(days: u16) -> String {
    format!(
        "{DURATION_QUESTION} Один период — {days} дн. Напишите, сколько их разрешить: от 1 до 100, но суммарно не больше 366 дней."
    )
}

pub fn schedule_free_row(until: &str, max_score: u16) -> String {
    format!("• до {until} — без штрафа, {max_score} из {max_score}")
}

pub fn schedule_penalty_row(until: &str, penalty: u16, score: u16, max_score: u16) -> String {
    format!("• до {until} — штраф −{penalty}, {score} из {max_score}")
}

pub fn schedule_skipped(periods: u32) -> String {
    format!(
        "• … ещё {periods} {}",
        plural(periods, "период", "периода", "периодов")
    )
}

pub fn schedule_closed(since: &str) -> String {
    format!("• с {since} — приём закрыт")
}

pub fn closing_change(before: &str, after: &str) -> String {
    format!("{SUBMISSIONS_CLOSE_LABEL} было {before} → стало {after}.")
}

pub fn queue_ahead(position: u32) -> String {
    match position {
        0 => QUEUE_NO_TEACHER.to_owned(),
        1 => QUEUE_FIRST.to_owned(),
        _ => format!("Перед вашей работой в очереди: {}", position - 1),
    }
}

pub fn number_prompt(max: u16) -> String {
    format!("Введите целое число от 1 до {max}:")
}

pub fn penalty_question(period_days: u16) -> String {
    let period = if period_days == 7 {
        "неделю".to_owned()
    } else {
        format!("период в {period_days} дн.")
    };
    if period_days == 7 {
        format!("Сколько баллов снимать за каждую начавшуюся {period} опоздания?")
    } else {
        format!("Сколько баллов снимать за каждый начавшийся {period} опоздания?")
    }
}

pub fn penalty_rule(period_days: u16, penalty: u16) -> String {
    let period = if period_days == 7 {
        "каждую начавшуюся неделю".to_owned()
    } else {
        format!("каждый начавшийся период в {period_days} дн.")
    };
    format!(
        "{PENALTY_LABEL} −{} за {period} опоздания после срока.",
        points(penalty)
    )
}

pub fn points(value: u16) -> String {
    format!(
        "{value} {}",
        plural(u32::from(value), "балл", "балла", "баллов")
    )
}

const CANCEL_HINT: &str = "/cancel — отменить";

pub fn input_prompt(prompt: &str) -> String {
    format!("{prompt}\n\n{CANCEL_HINT}")
}

pub fn submission_queued() -> String {
    format!(
        "✅ Работа отправлена. Когда преподаватель её проверит, бот пришлёт результат отдельным сообщением. Посмотреть, сколько работ перед вашей, можно в «{BUTTON_QUEUE}»."
    )
}

pub fn student_help() -> String {
    format!(
        "❓ Как сдавать работы\n\n\
        Откройте «{BUTTON_ASSIGNMENTS}» и выберите задание. В карточке есть сроки, условия и оставшиеся попытки. Нажмите «📤 Сдать работу» и отправьте ссылку или текст одним сообщением.\n\n\
        В «{BUTTON_QUEUE}» видно, сколько работ перед вашей. По этому числу нельзя узнать, когда преподаватель закончит проверку.\n\n\
        Результат и комментарий преподавателя придут отдельным сообщением. Если работу отклонили, её можно отправить снова — при условии, что остались попытки и приём ещё открыт. За опоздание могут сниматься баллы; точные условия смотрите в задании.\n\n\
        Закрытые задания и историю сдач ищите в «{BUTTON_ARCHIVE}»."
    )
}

pub fn teacher_help() -> String {
    format!(
        "❓ Как вести курс\n\n\
        Откройте «{BUTTON_COURSES}», выберите курс и создайте задание. Бот попросит текст, число попыток и шкалу оценивания. Затем настройте приём с опозданием и срок сдачи. Для штрафа нужны размер, длина периода и число периодов. Всего можно разрешить до 366 дней опоздания.\n\n\
        Проверьте итоговые условия и сохраните черновик. При необходимости добавьте вложения. Черновик видят только преподаватели курса; студентам задание станет доступно после нажатия «🚀 Опубликовать».\n\n\
        «{BUTTON_REVIEW}» открывает следующую работу из очереди. Примите или отклоните её и обязательно напишите комментарий. Бот отправит студенту результат.\n\n\
        Чтобы поменять условия, откройте «Сроки и штраф» в задании. Перенос срока сдвигает и конец приёма — проверьте итоговую дату перед сохранением. При настройке /cancel возвращает на предыдущий шаг, а на первом шаге выходит."
    )
}

// Общие
pub const CHOOSE_ACTION: &str = "Выберите действие:";
pub const ACTION_CANCELLED: &str = "Текущее действие отменено.";
pub const PRIVATE_CHATS_ONLY: &str = "Бот работает только в личных сообщениях.";
pub const ASSIGNMENT_BODY_HEADER: &str = "📄 Текст задания:";
pub const FIELD_NAME: &str = "Имя";
pub const FIELD_TITLE: &str = "Название";
pub const FIELD_BODY: &str = "Текст задания";
pub const FIELD_COMMENT: &str = "Комментарий";
pub const FIELD_SUBMISSION: &str = "Текст работы";

pub fn number_error(max: u16) -> String {
    format!("Введите целое число от 1 до {max}.")
}

// Главный экран и профиль
pub fn student_pending_line(pending: u32) -> String {
    format!("⏳ Ваших работ на проверке: {pending}")
}

pub fn review_queue_line(pending: u32) -> String {
    format!("⏳ Работ в очереди на проверку: {pending}")
}

pub fn profile_header(name: &str, role: &str) -> String {
    format!("👤 Профиль\n\nИмя: {name}\nРоль: {role}")
}

pub fn profile_group(code: &str) -> String {
    format!("Группа: {code}")
}

pub fn profile_active_assignments(count: u32) -> String {
    format!("📚 Активных заданий: {count}")
}

pub fn profile_pending(count: u32) -> String {
    format!("⏳ Работ на проверке: {count}")
}

pub fn profile_courses(count: u32) -> String {
    format!("📘 Курсов: {count}")
}

pub fn profile_review_queue(count: u32) -> String {
    format!("⏳ Ждут проверки: {count}")
}

// Студент
pub const STUDENT_QUEUE_HEADER: &str = "⏳ Ваши работы на проверке:";
pub const ASSIGNMENTS_HEADER: &str = "📚 Ваши задания:";
pub const NO_ASSIGNMENTS: &str = "📚 Опубликованных заданий пока нет.";
pub const ARCHIVE_HEADER: &str = "🗄 Архив заданий:";
pub const ARCHIVE_EMPTY: &str = "🗄 Архив заданий пока пуст.";
pub const PROGRESS_ACCEPTED: &str = "✅ принято";
pub const PROGRESS_PENDING: &str = "⏳ на проверке";
pub const PROGRESS_NOT_SUBMITTED: &str = "✍️ не сдано";
pub const FILES_ATTACHED_HEADER: &str = "📎 Вложения задания:";
pub const REVIEW_COMMENT_HEADER: &str = "💬 Комментарий преподавателя:";
pub const BUTTON_HISTORY: &str = "🕘 История сдач";
pub const BUTTON_SUBMIT: &str = "📤 Сдать работу";
pub const BUTTON_BACK_TO_ARCHIVE: &str = "⬅️ К архиву";
pub const BUTTON_BACK_TO_ASSIGNMENTS: &str = "⬅️ К списку заданий";
pub const BUTTON_BACK_TO_ASSIGNMENT: &str = "⬅️ К заданию";

pub fn queue_entry(subject: &str, title: &str, sent_at: &str, ahead: &str) -> String {
    format!("📘 {subject} · {title}\nотправлено {sent_at} · {ahead}")
}

pub fn subject_header(subject: &str) -> String {
    format!("📘 {subject}")
}

pub fn assignment_entry(
    title: &str,
    status: &str,
    until: &str,
    time_left: &str,
    closing: Option<&str>,
    remaining: u16,
) -> String {
    let closing = closing
        .map(|closing| format!("\n{closing}"))
        .unwrap_or_default();
    format!("{title}\n{status} · {until} ({time_left}){closing}\nосталось попыток: {remaining}")
}

pub fn closing_line(closes_at: &str) -> String {
    format!("{SUBMISSIONS_CLOSE_LABEL} {closes_at}")
}

pub fn archive_entry(title: &str, used: u16) -> String {
    format!("{title}\nиспользовано попыток: {used}")
}

pub fn assignment_card(
    subject: &str,
    title: &str,
    terms: &str,
    used: u16,
    allowed: u16,
    remaining: u16,
) -> String {
    format!(
        "📘 {subject}\n{title}\n{terms}\n🔁 Попытки: использовано {used}, доступно {allowed}, осталось {remaining}"
    )
}

pub fn pending_place(ahead: &str) -> String {
    format!("⏳ Работа на проверке.\n{ahead}")
}

pub fn attempt_line(number: u16, status: &str, grade: Option<&str>) -> String {
    let grade = grade.map(|grade| format!("\n{grade}")).unwrap_or_default();
    format!("Попытка {number}: {status}{grade}")
}

pub fn history_header(subject: &str, title: &str) -> String {
    format!("📘 {subject} · {title}\n🕘 История сдач: от новых к старым")
}

pub fn assignment_button(subject: &str, title: &str) -> String {
    format!("{subject} · {title}")
}

// Проверка
pub const SUBMISSION_HEADER: &str = "📤 Работа студента:";
pub const BUTTON_ACCEPT: &str = "✅ Принять";
pub const BUTTON_REJECT: &str = "❌ Отклонить";

pub fn review_button(pending: u32) -> String {
    format!("{BUTTON_REVIEW} ({pending})")
}

pub fn review_summary(subject: &str, title: &str, student: &str, attempt: u16) -> String {
    format!("📘 {subject} · {title}\n👤 Студент: {student}\n🔁 Попытка {attempt}")
}

pub fn late_submission_line(penalty: u16) -> String {
    format!("⏳ Сдана с опозданием, штраф −{penalty}")
}

pub fn late_penalty_notice(penalty: u16) -> String {
    format!("⏳ Работа сдана с опозданием, штраф — {penalty}.")
}

pub fn previous_attempt(number: u16, status: &str, content: &str, comment: Option<&str>) -> String {
    let comment = comment
        .map(|comment| format!("\n💬 {comment}"))
        .unwrap_or_default();
    format!("🕘 Попытка {number}: {status}\n{content}{comment}")
}

pub fn score_prompt(max_score: u16) -> String {
    format!("Введите целое число от 0 до {max_score}:")
}

pub fn score_error(max_score: u16) -> String {
    format!("Введите целое число от 0 до {max_score}.")
}

pub fn score_with_penalty(score: u16, penalty: u16, max_score: u16) -> String {
    format!(
        "🏅 Итог: {score} − {penalty} = {} из {max_score}.",
        score.saturating_sub(penalty)
    )
}

pub fn grade_saved(grade: Grade) -> String {
    format!(
        "🏅 Сохранено: {} − {} = {} из {}.",
        grade.raw_score(),
        grade.penalty(),
        grade.final_score(),
        grade.max_score(),
    )
}

// Курсы и задания преподавателя
pub const NO_COURSES: &str = "📘 У вас пока нет учебных курсов.";
pub const CHOOSE_COURSE: &str = "📘 Выберите курс:";
pub const NO_COURSE_ASSIGNMENTS: &str = "В этом курсе пока нет заданий.";
pub const COURSE_ASSIGNMENTS_HEADER: &str = "Задания курса:";
pub const CHOOSE_ASSIGNMENT: &str = "Выберите задание или создайте новое:";
pub const ASSIGNMENT_PUBLISHED: &str = "🚀 Задание опубликовано.";
pub const ASSIGNMENT_ARCHIVED: &str = "📦 Задание архивировано.";
pub const BUTTON_CREATE_ASSIGNMENT: &str = "➕ Создать задание";
pub const BUTTON_GRADEBOOK: &str = "📊 Оценки";
pub const BUTTON_GROUP_INVITE: &str = "🔗 Ссылка для группы";
pub const BUTTON_BACK_TO_COURSES: &str = "⬅️ К списку курсов";
pub const BUTTON_BACK_TO_COURSE: &str = "⬅️ К курсу";
pub const BUTTON_BACK_TO_COURSE_ASSIGNMENTS: &str = "⬅️ К заданиям курса";
pub const BUTTON_PUBLISH: &str = "🚀 Опубликовать";
pub const BUTTON_PUBLISH_AGAIN: &str = "🚀 Опубликовать снова";
pub const BUTTON_ARCHIVE_ASSIGNMENT: &str = "📦 Архивировать";
pub const BUTTON_FILES: &str = "📎 Вложения";
pub const BUTTON_GRANT_ATTEMPTS: &str = "🎁 Выдать попытки";

pub fn course_button(subject: &str, group: &str) -> String {
    format!("📘 {subject} · {group}")
}

pub fn course_assignment_entry(title: &str, status: &str, deadline: &str) -> String {
    format!("{title} · {status}\n⏰ {deadline}")
}

pub fn course_assignment_button(status_icon: &str, title: &str) -> String {
    format!("{status_icon} {title}")
}

pub fn published_line(at: &str) -> String {
    format!("🚀 Опубликовано: {at}")
}

pub fn teacher_assignment_card(
    title: &str,
    status: &str,
    terms: &str,
    attempts: u16,
    published: Option<&str>,
    body: &str,
) -> String {
    let published = published
        .map(|line| format!("\n{line}"))
        .unwrap_or_default();
    format!(
        "{title}\n\nСтатус: {status}{terms}\n🔁 Попыток: {attempts}{published}\n\n{ASSIGNMENT_BODY_HEADER}\n{body}"
    )
}

// Создание задания и условия
pub const BUTTON_OTHER_AMOUNT: &str = "✍️ Другое число";
pub const BUTTON_OTHER_MAX_SCORE: &str = "✍️ Другой максимум";
pub const BUTTON_PASS_FAIL: &str = "⏭ Без баллов, зачёт/незачёт";
pub const BUTTON_CALENDAR: &str = "🗓 Выбрать дату";
pub const BUTTON_MANUAL: &str = "✍️ Ввести вручную";
pub const BUTTON_PICK_DATE: &str = "Выбрать дату";
pub const BUTTON_OTHER_PERIODS: &str = "Другое число периодов";
pub const BUTTON_APPLY_PERIODS: &str = "Применить число периодов";
pub const BUTTON_POLICY_PENALTY: &str = "После срока: со штрафом";
pub const BUTTON_POLICY_STRICT: &str = "После срока: не принимать";
pub const PRESET_WEEK: &str = "Через неделю";
pub const PRESET_TWO_WEEKS: &str = "Через 2 недели";
pub const PRESET_MONTH: &str = "Через месяц";
pub const PRESET_WEEK_AFTER: &str = "Неделя после срока";
pub const PRESET_TWO_WEEKS_AFTER: &str = "2 недели после срока";
pub const PRESET_MONTH_AFTER: &str = "Месяц после срока";
pub const MONTHS: [&str; 12] = [
    "Январь",
    "Февраль",
    "Март",
    "Апрель",
    "Май",
    "Июнь",
    "Июль",
    "Август",
    "Сентябрь",
    "Октябрь",
    "Ноябрь",
    "Декабрь",
];
pub const WEEKDAYS: [&str; 7] = ["Пн", "Вт", "Ср", "Чт", "Пт", "Сб", "Вс"];

pub fn attempts_prompt(max: u16) -> String {
    format!("Введите количество попыток от 1 до {max}:")
}

pub fn max_score_prompt(max: u16) -> String {
    format!("Введите максимальный балл от 1 до {max}:")
}

pub fn scale_button(max_score: u16) -> String {
    format!("{max_score}-балльная")
}

pub fn penalty_button(points: u16) -> String {
    format!("На {}", self::points(points))
}

pub fn period_button(period_days: u16) -> String {
    format!("Изменить период: {period_days} дн.")
}

pub fn period_summary(period_days: u16) -> String {
    format!("{PERIOD_LABEL} {period_days} дн.")
}

pub fn until_short(date: &str) -> String {
    format!("до {date}")
}

pub fn date_preset(label: &str, date: &str) -> String {
    format!("📅 {label} · {date}")
}

pub fn periods_option(duration: &str, until: &str) -> String {
    format!("{duration} · до {until}")
}

pub fn penalty_step(question: &str, period_days: u16) -> String {
    format!(
        "{question}\nШтраф начисляется за каждые начавшиеся {period_days} дн. Напишите целое число баллов от 1 до 100:"
    )
}

pub fn duration_confirmation(selected: &str, duration: &str, closing: &str) -> String {
    format!(
        "Вы выбрали приём {selected}. После округления опоздание составит {duration}.\n{SUBMISSIONS_CLOSE_LABEL} {closing}. Использовать эту дату?"
    )
}

// Вложения
pub const FILE_PROMPT: &str = "Пришлите файл или фотографию.";
pub const FILE_ADDED: &str = "Вложение добавлено.";
pub const FILE_DELETED: &str = "Вложение удалено.";
pub const NO_FILES: &str = "Вложений нет.";
pub const FILES_HEADER: &str = "Вложения задания:";
pub const BUTTON_ADD_FILE: &str = "📎 Добавить вложение";
pub const BUTTON_FINISH_CREATION: &str = "✅ Завершить создание";
pub const BUTTON_DELETE: &str = "🗑 Удалить";
pub const BUTTON_CANCEL: &str = "↩️ Отмена";

pub fn file_entry(position: u64, filename: &str, size: &str) -> String {
    format!("{position}. {filename} · {size}")
}

pub fn file_button(filename: &str) -> String {
    format!("📄 {filename}")
}

pub fn files_used(used: usize, max: usize) -> String {
    format!("Использовано: {used} из {max}.")
}

// Студенты, попытки и оценки
pub const NO_STUDENTS: &str = "В группе пока нет студентов.";
pub const STUDENTS_HEADER: &str = "👥 Студенты группы:";
pub const STUDENT_PENDING_MARK: &str = " · ⏳ сдача на проверке";
pub const STUDENT_ACCEPTED_MARK: &str = " · ✅ работа принята";
pub const CHOOSE_STUDENT_PROMPT: &str = "Выберите студента:";
pub const EXTRA_ATTEMPTS_QUESTION: &str = "Сколько дополнительных попыток выдать?";
pub const GRADEBOOK_CHOOSE_STUDENT: &str = "📊 Оценки курса. Выберите студента:";
pub const NO_SCORED_ASSIGNMENTS: &str = "В курсе пока нет оценочных заданий.";
pub const BUTTON_BACK_TO_STUDENTS: &str = "⬅️ К студентам курса";

pub fn student_entry(name: &str, used: u16, extra: u16, marks: &str) -> String {
    format!("{name} · использовано {used} · дополнительных {extra}{marks}")
}

pub fn student_button(name: &str) -> String {
    format!("👤 {name}")
}

pub fn attempts_granted(amount: u16, total: u16) -> String {
    format!("🎁 Добавлено попыток: {amount}. Всего дополнительных: {total}.")
}

pub fn grades_header(name: &str, total: u32, max_total: u32) -> String {
    format!("👤 {name}\n🏅 Итого: {total} из {max_total}")
}

pub fn grade_entry(title: &str, score: &str, max_score: u16) -> String {
    format!("{title} · {score} из {max_score}")
}

// Ссылка группы
pub const INVITE_ROTATED: &str = "♻️ Прежняя ссылка больше не работает.";
pub const BUTTON_ROTATE_INVITE: &str = "♻️ Перевыпустить ссылку";
pub const BUTTON_CONFIRM_ROTATE: &str = "♻️ Перевыпустить";

pub fn invite_header(group: &str) -> String {
    format!("🔗 Ссылка для регистрации в группу {group}:")
}

// Постраничная навигация
pub const BUTTON_PREVIOUS_PAGE: &str = "◀️ Назад";
pub const BUTTON_NEXT_PAGE: &str = "Вперёд ▶️";

// Статусы и величины

pub fn submission_status(status: SubmissionStatus) -> &'static str {
    match status {
        SubmissionStatus::PendingReview => "⏳ на проверке",
        SubmissionStatus::Accepted => "✅ принята",
        SubmissionStatus::Rejected => "❌ отклонена",
    }
}

pub fn assignment_status(status: AssignmentStatus) -> &'static str {
    match status {
        AssignmentStatus::Draft => "⚪️ черновик",
        AssignmentStatus::Published => "🟢 опубликовано",
        AssignmentStatus::Archived => "📦 архив",
    }
}

pub fn assignment_status_icon(status: AssignmentStatus) -> &'static str {
    match status {
        AssignmentStatus::Draft => "⚪️",
        AssignmentStatus::Published => "🟢",
        AssignmentStatus::Archived => "📦",
    }
}

pub fn role_name(role: Role) -> &'static str {
    match role {
        Role::Student { .. } => "🎓 студент",
        Role::Teacher => "👨‍🏫 преподаватель",
        Role::Admin => "🛠 администратор",
    }
}

pub fn grade_line(grade: Grade) -> String {
    if grade.penalty() == 0 {
        return format!("🏅 Балл: {} из {}", grade.final_score(), grade.max_score());
    }
    format!(
        "🏅 Балл: {} − {} = {} из {} (штраф за опоздание)",
        grade.raw_score(),
        grade.penalty(),
        grade.final_score(),
        grade.max_score(),
    )
}

pub fn deadline_left(deadline: DateTime<Utc>, now: DateTime<Utc>) -> String {
    let left = deadline - now;
    if left.num_seconds() <= 0 {
        return DEADLINE_PASSED.to_owned();
    }
    let days = left.num_days();
    let hours = left.num_hours() % 24;
    if days > 0 {
        format!("осталось {days} д {hours} ч")
    } else if left.num_hours() > 0 {
        format!("осталось {} ч", left.num_hours())
    } else {
        format!("осталось {} мин", left.num_minutes().max(1))
    }
}

pub fn format_file_size(size_bytes: u64) -> String {
    if size_bytes < 1024 {
        format!("{size_bytes} Б")
    } else if size_bytes < 1024 * 1024 {
        format!("{:.1} КиБ", size_bytes as f64 / 1024.0)
    } else {
        format!("{:.1} МиБ", size_bytes as f64 / (1024.0 * 1024.0))
    }
}
