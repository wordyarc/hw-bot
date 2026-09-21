use super::views::{
    BUTTON_ARCHIVE, BUTTON_ASSIGNMENTS, BUTTON_COURSES, BUTTON_QUEUE, BUTTON_REVIEW,
};

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

pub fn timezone_note(timezone: &str) -> String {
    format!("{TIMEZONE_LABEL} {timezone}. Все сроки показываются и вводятся в нём.")
}
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
