use super::views::{
    BUTTON_ARCHIVE, BUTTON_ASSIGNMENTS, BUTTON_COURSES, BUTTON_QUEUE, BUTTON_REVIEW,
};

pub const GUEST_NEEDS_INVITE: &str = "Вы ещё не зарегистрированы. Доступ к заданиям не открыт.\n\n\
    Студентам: попросите у преподавателя ссылку-приглашение вашей группы и откройте её.\n\
    Преподавателям: обратитесь к администратору бота, чтобы он добавил вас.";
pub const GUEST_START_DESCRIPTION: &str = "Как получить доступ";
pub const GUEST_HELP_DESCRIPTION: &str = "Помощь с подключением";
pub const USE_THE_MENU: &str = "Используйте меню под полем ввода или команду /menu.";
pub const SUBMISSION_PROMPT: &str = "Пришлите ссылку или текст сдачи одним сообщением:";
pub const STUDENT_QUEUE_EMPTY: &str = "⏳ У вас нет работ, ожидающих проверки.";
pub const QUEUE_NO_TEACHER: &str =
    "На курс ещё не назначен преподаватель. Обратитесь к администратору бота.";
pub const QUEUE_FIRST: &str = "Перед вашей работой нет ожидающих работ.";
pub const REVIEW_QUEUE_EMPTY: &str = "🎉 Очередь проверки пуста.";
pub const REVIEW_COMMENT_PROMPT: &str = "Напишите обязательный комментарий к решению:";
pub const REVIEW_GRADING_CHANGED: &str = "Шкала задания изменилась. Прежнее решение не сохранено. Повторите проверку по обновлённой карточке.";
pub const FILE_DOWNLOAD_FAILED: &str = "Не удалось скачать файл. Попробуйте ещё раз.";
pub const STALE_BUTTON: &str =
    "Эта кнопка из старого сообщения, действие не выполнено. Откройте /menu.";

pub const FILE_DELETE_CONFIRM: &str = "Удалить вложение из уже выданного задания?";
pub const LATE_POLICY_QUESTION: &str = "После срока принимать работы?";
pub const SUBMISSIONS_CLOSE_LABEL: &str = "Приём работ:";
pub const LATE_PENALTY_LABEL: &str = "При отправке сейчас штраф:";
pub const DEADLINE_QUESTION: &str =
    "До какого числа нужно сдать работу? После этого приём закрывается.";
pub const NO_PENALTY_QUESTION: &str = "До какого числа сдать без штрафа?";
pub const DEADLINE_IN_FUTURE: &str = "Выберите срок сдачи позже текущего времени.";
pub const GRADING_QUESTION: &str = "Как оцениваете задание?";
pub const ATTEMPTS_QUESTION: &str = "Сколько попыток даёте студенту?";
pub const ASSIGNMENT_TITLE_PROMPT: &str = "Напишите короткое название задания:";
pub const ASSIGNMENT_BODY_PROMPT: &str = "Пришлите текст задания одним сообщением:";
pub const PERIOD_PROMPT: &str =
    "Период штрафа в днях. Пришлите целое число от 1 до 365 сообщением:";
pub const TERMS_CONFIRM: &str = "Проверьте условия приёма перед сохранением:";
pub const SCHEDULE_HEADER: &str = "📅 Как будет снижаться балл:";
pub const TERMS_SAVED: &str = "Условия приёма сохранены.";
pub const DRAFT_CREATED: &str = "📝 Черновик создан. Проверьте условия перед публикацией:";
pub const CHOOSE_BUTTON: &str = "Выберите действие кнопкой выше или отправьте /cancel для отмены.";
pub const CHOOSE_REVIEW_DECISION: &str = "Выберите решение кнопкой под сдачей.";
pub const CHOOSE_ASSIGNMENT_ACTION: &str = "Выберите действие кнопкой под заданием.";
pub const CHOOSE_ACTION_ABOVE: &str = "Выберите действие кнопкой выше.";
pub const CHOOSE_FILE_ACTION: &str = "Выберите действие кнопкой под списком вложений.";
pub const CONFIRM_FILE_DELETION: &str = "Подтвердите удаление кнопкой выше.";
pub const CHOOSE_STUDENT: &str = "Выберите студента кнопкой выше.";
pub const STRICT_AFTER_DEADLINE: &str = "После этого приём закрывается.";
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
pub const WHOLE_DAY_HINT: &str =
    "Дата без времени означает конец этого дня. Можно прислать дату сообщением.";
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
    "Выйти из настройки задания? Введённые изменения не сохранятся.";
pub const DRAFT_DISCARDED: &str = "Несохранённый черновик отброшен. Задание не создано.";
pub const TERMS_DISCARDED: &str = "Изменения условий отменены. Задание осталось прежним.";
pub const DATE_FORMAT: &str = "Пришлите дату в формате ДД.ММ.ГГГГ или ДД.ММ.ГГГГ ЧЧ:ММ.";
pub const INTEGER_REQUIRED: &str = "Нужно целое число.";
pub const INVALID_TERMS: &str = "Проверьте условия сдачи.";
pub const HORIZON_OUT_OF_RANGE: &str =
    "Вычисленная дата приёма выходит за поддерживаемый диапазон дат.";
pub const DEADLINE_AFTER_CREATION: &str = "Срок должен быть позже создания задания.";
pub const INVALID_DURATION_DATE: &str =
    "Выберите дату после срока в пределах допустимого опоздания.";
pub const DURATION_QUESTION: &str = "Какое опоздание разрешить?";
pub const DURATION_DATE_QUESTION: &str = "Выберите дату, до которой хотите принимать работы. Опоздание округлится вверх до целого числа периодов; пересчёт нужно подтвердить.";
pub const PAST_PENALTY_WARNING: &str = "⚠️ Срок уже прошёл: штраф применяется сразу.";
pub const PAST_STRICT_WARNING: &str = "⚠️ Срок уже прошёл: приём будет закрыт сразу.";

pub fn horizon_too_long(max_days: u32) -> String {
    format!("Опоздание не может превышать {max_days} дней. Уменьшите период или число периодов.")
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
        "{DURATION_QUESTION} Пришлите число периодов по {days} дн. от 1 до 100. Общее опоздание — не больше 366 дней."
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
        _ => format!("Работ перед вашей: {}", position - 1),
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
        format!("На сколько снижать оценку за каждую начавшуюся {period} опоздания?")
    } else {
        format!("На сколько снижать оценку за каждый начавшийся {period} опоздания?")
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
        "✅ Работа отправлена на проверку. В «{BUTTON_QUEUE}» можно посмотреть, сколько работ перед вашей. Результат придёт отдельным сообщением."
    )
}

pub fn student_help() -> String {
    format!(
        "❓ Как сдавать работы\n\n\
        1. «{BUTTON_ASSIGNMENTS}» — задания, условия приёма и оставшиеся попытки.\n\
        2. Откройте задание, нажмите «📤 Сдать работу» и пришлите ссылку или текст одним сообщением.\n\
        3. «{BUTTON_QUEUE}» — сколько работ перед вашей в очереди преподавателей. Это текущее положение, а не срок проверки.\n\
        4. Результат придёт отдельным сообщением с комментарием преподавателя. Отклонённую работу можно сдать заново, пока есть попытки и приём открыт. После срока без штрафа балл может снижаться — условия указаны в задании.\n\
        5. «{BUTTON_ARCHIVE}» — закрытые задания и история сдач."
    )
}

pub fn teacher_help() -> String {
    format!(
        "❓ Как вести курс\n\n\
        1. «{BUTTON_COURSES}» — ваши курсы и задания.\n\
        2. При создании задания укажите текст, число попыток и оценивание. Выберите приём после срока, затем срок. Для штрафа задайте размер, период и число периодов опоздания (не больше 366 дней).\n\
        3. Добавьте вложения и проверьте условия перед публикацией. Черновик доступен преподавателям курса; студенты увидят его после «🚀 Опубликовать».\n\
        4. «{BUTTON_REVIEW}» — следующая работа из очереди. Примите её или верните на доработку, обязательно оставив комментарий. Студент получит результат сообщением.\n\
        5. «Сроки и штраф» — правка условий; перенос срока сдвигает последний день приёма. В итоге проверьте вычисленную дату и сохраните черновик. /cancel внутри настройки возвращает шаг назад, на первом шаге — выходит."
    )
}
