use chrono::{DateTime, Utc};

use crate::domain::{
    AssignmentStatus, DeductedFrom, Grade, LatePenalty, MAX_LATE_PERIOD_DAYS, MAX_LATE_PERIODS,
    MAX_LATENESS_DAYS, MAX_PENALTY_PER_PERIOD, ReviewDecision, Role, SubmissionStatus,
};

pub const BUTTON_ASSIGNMENTS: &str = "📚 Мои задания";
pub const BUTTON_ARCHIVE: &str = "🗄 Архив";
pub const BUTTON_QUEUE: &str = "⏳ Очередь проверки";
pub const BUTTON_REVIEW: &str = "✅ Проверить работы";
pub const BUTTON_COURSES: &str = "📘 Мои предметы";
pub const BUTTON_SANDBOX: &str = "🧪 Песочница";
pub const BUTTON_PROFILE: &str = "👤 Профиль";
pub const BUTTON_HELP: &str = "❓ Помощь";
pub const MENU_PLACEHOLDER: &str = "Выберите пункт меню";

pub const BUTTON_STUDENT_CONTACT: &str = "👤 Контакт студента";
pub const BUTTON_OPEN_TELEGRAM: &str = "↗ Открыть в Telegram";
pub const BUTTON_REFRESH_CONTACT: &str = "🔄 Обновить контакт";
pub const BUTTON_RETRY_CONTACT: &str = "Повторить";
pub const CONTACT_NO_USERNAME: &str = "Публичный username не указан.";
pub const CONTACT_RESTRICTED: &str =
    "У студента нет публичного username, а ссылка на аккаунт ограничена настройками приватности.";
pub const CONTACT_UNAVAILABLE: &str = "Контакт сейчас недоступен.";
pub const CONTACT_LOOKUP_FAILED: &str = "Не удалось получить контакт. Попробуйте ещё раз.";
pub const CONTACT_PERSONAL_ACCOUNT: &str =
    "Откройте Telegram, чтобы написать со своего личного аккаунта.";

pub fn student_contact_header(name: &str, group: &str) -> String {
    format!("👤 {name}\nГруппа: {group}")
}

pub fn contact_username(username: &str) -> String {
    format!("Telegram: @{username}")
}

pub const GUEST_NEEDS_INVITE: &str = "Чтобы открыть задания, сначала зарегистрируйтесь. Сейчас у вас нет регистрации.\n\n\
    Учитесь в группе? Перейдите по её ссылке-приглашению. Ссылку можно получить у преподавателя.\n\
    Если вы преподаватель, попросите администратора добавить вас в бот.";
pub const GUEST_START_DESCRIPTION: &str = "Как получить доступ";
pub const GUEST_HELP_DESCRIPTION: &str = "Помощь с подключением";
pub const USE_THE_MENU: &str =
    "Меню находится под полем ввода. Выберите пункт или вызовите меню командой /menu.";
pub const SUBMISSION_PROMPT: &str =
    "Пришлите ссылку на работу или её текст. Всё должно быть в одном сообщении.";
pub const STUDENT_QUEUE_EMPTY: &str = "⏳ Сейчас у вас нет работ на проверке.";
pub const QUEUE_NO_TEACHER: &str =
    "У этого предмета пока нет преподавателя. Попросите администратора бота назначить его.";
pub const QUEUE_FIRST: &str = "Ваша работа первая в очереди.";
pub const REVIEW_QUEUE_EMPTY: &str = "🎉 Работ на проверку пока нет.";
pub const REVIEW_COMMENT_PROMPT: &str = "Объясните студенту своё решение. Комментарий обязателен и для принятой, и для отклонённой работы:";
pub const REVIEW_DECISION_RESET: &str = "Решение не сохранено. Выберите его заново кнопками ниже.";
pub const REVIEW_GRADING_CHANGED: &str = "Решение не удалось сохранить: у задания теперь другая шкала оценивания. Ниже обновлённая карточка. Проверьте работу по ней ещё раз.";
pub const FILE_DOWNLOAD_FAILED: &str = "Файл не скачался. Повторите отправку.";
pub const ASSIGNMENT_FILES_SEND_FAILED: &str = "Не удалось отправить все вложения. Откройте задание ещё раз, чтобы повторить. Действия с заданием доступны выше.";
pub const BUTTON_REOPEN_ASSIGNMENT: &str = "🔄 Открыть задание снова";
pub const STALE_BUTTON: &str = "Действие не выполнено: эта кнопка больше не действует. Через /menu откройте нужный экран заново.";

pub const FILE_DELETE_CONFIRM: &str = "Задание уже выдано студентам. Удалить из него это вложение?";
pub const LATE_POLICY_QUESTION: &str = "Разрешить сдавать работы с опозданием?";
pub const DEDUCTED_FROM_HINT: &str = "За опоздание назначается штраф. Его можно учитывать двумя способами:\n\
    • в балле работы: опоздавший теряет баллы за эту работу, и для того же балла её нужно сделать лучше;\n\
    • в итоге по предмету: балл работы отражает только качество, а штрафы копятся отдельно и вычитаются из суммы баллов по предмету.";
pub const SUBMISSIONS_CLOSE_LABEL: &str = "Приём работ:";
pub const LATE_PENALTY_LABEL: &str = "Штраф, если сдать сейчас:";
pub const DEADLINE_QUESTION: &str =
    "До какого числа принимать работы? После этого срока сдать уже нельзя.";
pub const NO_PENALTY_QUESTION: &str = "До какого числа можно сдать без штрафа?";
pub const DEADLINE_IN_FUTURE: &str = "Нужен срок в будущем. Указанные дата или время уже прошли.";
pub const DEADLINE_PASSED: &str = "срок прошёл";
pub const GRADING_QUESTION: &str = "Как будете оценивать работу?";
pub const ATTEMPTS_QUESTION: &str = "Сколько попыток будет у каждого студента?";
pub const ASSIGNMENT_TITLE_PROMPT: &str = "Напишите короткое название задания:";
pub const ASSIGNMENT_BODY_PROMPT: &str = "Пришлите текст задания одним сообщением:";
pub const TERMS_CONFIRM: &str = "Проверьте условия и сохраните, если всё верно:";
pub const SCHEDULE_HEADER: &str = "📅 Баллы с учётом опоздания:";
pub const SCHEDULE_TOTAL_HEADER: &str = "📅 Штраф в итог по предмету с учётом опоздания:";
pub const TERMS_SAVED: &str = "Новые условия сохранены.";
pub const DRAFT_CREATED: &str =
    "📝 Задание сохранено как черновик. Проверьте условия, прежде чем публиковать:";
pub const CHOOSE_BUTTON: &str = "Выберите ответ кнопкой выше или отправьте /cancel для отмены.";
pub const CHOOSE_REVIEW_DECISION: &str = "Нажмите «Принять» или «Отклонить» под работой.";
pub const CHOOSE_ASSIGNMENT_ACTION: &str = "Нажмите нужную кнопку под заданием.";
pub const CHOOSE_ACTION_ABOVE: &str = "Нажмите нужную кнопку выше.";
pub const CHOOSE_FILE_ACTION: &str = "Нажмите нужную кнопку под списком вложений.";
pub const CONFIRM_FILE_DELETION: &str = "Удаление нужно подтвердить кнопкой выше.";
pub const CHOOSE_STUDENT: &str = "Нажмите на имя студента в списке выше.";
pub const STRICT_AFTER_DEADLINE: &str = "Позже сдать работу нельзя.";
pub const BUTTON_STRICT: &str = "Нет, закрыть приём";
pub const BUTTON_LATE_WORK_SCORE: &str = "Да, штраф снижает балл работы";
pub const BUTTON_LATE_COURSE_TOTAL: &str = "Да, штраф вычитается из итога";
pub const BUTTON_EDIT_DATES: &str = "⏰ Сроки и штраф";
pub const BUTTON_SAVE_TERMS: &str = "Сохранить условия";
pub const BUTTON_CANCEL_TERMS: &str = "Отменить изменения";
pub const SUBMIT_UNTIL: &str = "Срок:";
pub const NO_PENALTY_UNTIL: &str = "Срок без штрафа:";
pub const PENALTY_LABEL: &str = "Штраф:";
pub const DEDUCTED_FROM_LABEL: &str = "Учёт штрафа:";
pub const PERIOD_LABEL: &str = "Период:";
pub const LATENESS_LABEL: &str = "Опоздание:";
pub const CLOSED_ON: &str = "Приём работ закрыт";
pub const WHOLE_DAY_HINT: &str =
    "Можно написать дату сообщением. Без указания времени она означает приём до конца этого дня.";
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
    "Всё введённое в этой форме будет потеряно. Выйти без сохранения?";
pub const DRAFT_DISCARDED: &str =
    "Вы отменили создание. Задание не создано, введённые данные не сохранены.";
pub const TERMS_DISCARDED: &str = "У задания остались прежние условия: изменения отменены.";
pub const TERMS_CONFLICT: &str = "Сохранение отменено: другой преподаватель успел изменить задание. Ваша правка не записана. Сверьтесь с текущими условиями ниже.";
pub const DATE_FORMAT: &str = "Пришлите дату в формате ДД.ММ.ГГГГ или ДД.ММ.ГГГГ ЧЧ:ММ.";
pub const TIMEZONE_LABEL: &str = "🕒 Часовой пояс:";
pub const INTEGER_REQUIRED: &str = "Напишите целое число, без дробной части.";
pub const INVALID_TERMS: &str =
    "Продолжить не удалось. Проверьте, согласованы ли срок и условия опоздания.";
pub const HORIZON_OUT_OF_RANGE: &str = "С такими условиями дата окончания приёма выходит за допустимые пределы. Попробуйте другой срок или разрешите меньше дней опоздания.";
pub const DEADLINE_AFTER_CREATION: &str = "Срок сдачи должен быть позже даты создания задания.";
pub const INVALID_DURATION_DATE: &str =
    "Выберите дату после срока сдачи, в пределах разрешённого опоздания.";
pub const DURATION_QUESTION: &str = "Как долго принимать работы после срока?";
pub const DURATION_DATE_QUESTION: &str = "Укажите, до какой даты разрешить сдачу с опозданием. Этот промежуток бот округлит вверх до целого числа периодов. Рассчитанную дату он покажет для подтверждения.";
pub const PAST_PENALTY_WARNING: &str =
    "⚠️ Штраф начнёт действовать сразу: выбранный срок уже в прошлом.";
pub const PAST_STRICT_WARNING: &str =
    "⚠️ Сохранение сразу закроет приём работ, потому что выбранный срок уже прошёл.";
pub const NAME_REQUIRED: &str = "Вы не указали имя. Как вас зовут?";
pub const TITLE_REQUIRED: &str = "У задания нет названия. Как его назвать?";
pub const BODY_REQUIRED: &str = "Опишите, что нужно сделать студентам: текст задания пока пуст.";
pub const TEXT_REQUIRED: &str = "Отправьте ответ текстовым сообщением.";
pub const REGISTRATION_COMPLETE: &str = "🎉 Вы зарегистрированы! Теперь можно открыть задания.";
pub const FILE_SIZE_MISMATCH: &str = "Telegram сообщил другой размер файла, чем получилось скачать. Попробуйте отправить файл снова.";
pub const INVITE_REUSE_HINT: &str =
    "Перешлите ссылку группе. Каждый студент сможет зарегистрироваться по ней.";
pub const INVITE_ROTATION_CONFIRM: &str = "После перевыпуска старая ссылка сразу перестанет работать во всех предметах группы. На доступ уже зарегистрированных студентов это не повлияет. Создать новую ссылку?";
pub const REVIEW_SAVED: &str =
    "✅ Проверка сохранена. Студент получит от бота результат вместе с вашим комментарием.";

pub fn period_prompt() -> String {
    format!("Укажите длину периода опоздания в днях: целое число от 1 до {MAX_LATE_PERIOD_DAYS}.")
}

pub fn timezone_note(timezone: &str) -> String {
    format!(
        "{TIMEZONE_LABEL} {timezone}. Указывайте время в этом поясе. В нём же показаны все сроки."
    )
}

pub fn registration_prompt(group: &str) -> String {
    format!("Группа для регистрации: {group}.\n\nНапишите, как вас зовут.")
}

pub fn home_greeting(name: &str) -> String {
    format!("👋 Здравствуйте, {name}!\nВыберите действие в меню под полем ввода.")
}

pub fn text_too_long(field: &str, max_len: usize) -> String {
    format!("{field}: текст длиннее {max_len} символов. Пришлите сокращённый вариант.")
}

pub fn review_score_question(max_score: u16, late: Option<LatePenalty>) -> String {
    let question = format!("Сначала поставьте балл от 0 до {max_score}.");
    match late {
        None => question,
        Some(late) if late.deducted_from() == DeductedFrom::WorkScore => format!(
            "{question} Ставьте балл без учёта штрафа: бот сам вычтет штраф −{} из балла работы.",
            late.points()
        ),
        Some(late) => format!(
            "{question} Штраф −{} балл работы не снижает: бот вычтет его из итога по предмету.",
            late.points()
        ),
    }
}

pub fn horizon_too_long(max_days: u32) -> String {
    format!(
        "Сократите длину периода или число периодов: разрешить опоздание можно максимум на {max_days} дней."
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
        "{DURATION_QUESTION} В одном периоде {days} дн. Сколько периодов разрешить? Введите число от 1 до {MAX_LATE_PERIODS}. Вместе они должны укладываться в {MAX_LATENESS_DAYS} дней."
    )
}

pub fn schedule_free_row(until: &str, max_score: u16) -> String {
    format!("• до {until} — без штрафа, {max_score} из {max_score}")
}

pub fn schedule_penalty_row(until: &str, penalty: u16, score: u16, max_score: u16) -> String {
    format!("• до {until} — штраф −{penalty}, {score} из {max_score}")
}

pub fn schedule_free_total_row(until: &str) -> String {
    format!("• до {until} — без штрафа")
}

pub fn schedule_total_row(until: &str, penalty: u16) -> String {
    format!("• до {until} — штраф −{penalty} в итог")
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
        format!("Какой штраф в баллах назначить за каждую начавшуюся {period} опоздания?")
    } else {
        format!("Какой штраф в баллах назначить за каждый начавшийся {period} опоздания?")
    }
}

pub fn penalty_rule(period_days: u16, penalty_per_period: u16) -> String {
    let period = if period_days == 7 {
        "каждую начавшуюся неделю".to_owned()
    } else {
        format!("каждый начавшийся период в {period_days} дн.")
    };
    format!(
        "{PENALTY_LABEL} −{} за {period} опоздания после срока.",
        points(penalty_per_period)
    )
}

pub fn deducted_from_line(deducted_from: DeductedFrom) -> String {
    let description = match deducted_from {
        DeductedFrom::WorkScore => "снижает балл работы.",
        DeductedFrom::CourseTotal => "вычитается из итога по предмету, балл работы не снижает.",
    };
    format!("{DEDUCTED_FROM_LABEL} {description}")
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
        "✅ Ваша работа в очереди на проверку. После проверки бот сообщит результат отдельным сообщением. Сколько работ перед вашей, можно посмотреть в «{BUTTON_QUEUE}»."
    )
}

pub fn student_help() -> String {
    format!(
        "❓ Как сдавать работы\n\n\
        Начните с раздела «{BUTTON_ASSIGNMENTS}». Откройте нужное задание и посмотрите сроки, условия и оставшиеся попытки. Кнопка «{BUTTON_SUBMIT}» открывает сдачу: отправьте ссылку или текст работы одним сообщением.\n\n\
        Пока работа ждёт проверки, в «{BUTTON_QUEUE}» можно посмотреть, сколько работ стоят перед ней. Определить время проверки по этому числу нельзя.\n\n\
        Решение преподавателя и его комментарий придут от бота после проверки. Если работу отклонили, повторная сдача доступна до закрытия приёма, пока есть попытки. Условия штрафа за опоздание смотрите в задании.\n\n\
        Закрытое задание вместе с историей сдач можно найти в «{BUTTON_ARCHIVE}»."
    )
}

pub fn teacher_help() -> String {
    format!(
        "❓ Как вести предмет\n\n\
        Новое задание создаётся внутри предмета в «{BUTTON_COURSES}». Бот спросит текст, число попыток, шкалу оценивания, срок и возможность сдать с опозданием. Штраф за опоздание снижает балл работы либо вычитается из итога по предмету: способ выбирается вместе с допуском опозданий. Для штрафа укажите, сколько баллов снимать, за какой период и сколько таких периодов разрешить. Суммарное опоздание не может превышать {MAX_LATENESS_DAYS} дней.\n\n\
        После проверки условий сохраните черновик и при необходимости прикрепите файлы. На этом этапе задание доступно только преподавателям предмета. Студенты увидят его, когда вы нажмёте «{BUTTON_PUBLISH}».\n\n\
        Для проверки нажмите «{BUTTON_REVIEW}»: бот откроет следующую работу из очереди. Оставьте комментарий независимо от того, принимаете работу или отклоняете. Бот передаст студенту результат.\n\n\
        Работы по предмету-песочнице в очередь не попадают: их список открывает «{BUTTON_SANDBOX}». Работу можно проверить по одной или ответить всем сразу кнопкой «{BUTTON_ACCEPT_ALL}» или «{BUTTON_REJECT_ALL}» с одним комментарием. Такое решение касается работ, отправленных до открытия списка. При принятии оценочные задания получают максимальный балл с учётом штрафа за опоздание.\n\n\
        Кнопка «{BUTTON_STUDENT_CONTACT}» есть под работой и в «{BUTTON_COURSES} → Предмет → {BUTTON_GRADEBOOK} → Студент». Она сохраняет текущий шаг. Нажмите «{BUTTON_OPEN_TELEGRAM}», чтобы написать со своего личного аккаунта; доступность перехода зависит от Telegram. Если адрес изменился, нажмите «{BUTTON_REFRESH_CONTACT}».\n\n\
        Чтобы изменить условия, воспользуйтесь кнопкой «{BUTTON_EDIT_DATES}» в задании. Перенос срока также сдвигает конец приёма, поэтому перед сохранением сверьте новую дату. Команда /cancel во время настройки возвращает к предыдущему шагу. На первом шаге она завершает настройку."
    )
}

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

pub fn student_pending_line(pending: u32) -> String {
    format!("⏳ Ваших работ на проверке: {pending}")
}

pub fn review_queue_line(pending: u32) -> String {
    format!("⏳ Работ в очереди на проверку: {pending}")
}

pub fn home_sandbox_line(pending: u32) -> String {
    format!("🧪 Работ в песочнице: {pending}")
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
    format!("📘 Предметов: {count}")
}

pub fn profile_review_queue(count: u32) -> String {
    format!("⏳ Ждут проверки: {count}")
}

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

pub const SUBMISSION_HEADER: &str = "📤 Работа студента:";
pub const PREVIOUS_ATTEMPTS_INCOMPLETE: &str =
    "Не удалось показать все прошлые попытки. Откройте проверку ещё раз, чтобы повторить.";
pub const BUTTON_ACCEPT: &str = "✅ Принять";
pub const BUTTON_REJECT: &str = "❌ Отклонить";
pub const BUTTON_CHANGE_DECISION: &str = "↩️ Изменить решение";

pub fn review_button(pending: u32) -> String {
    format!("{BUTTON_REVIEW} ({pending})")
}

pub fn review_summary(subject: &str, title: &str, student: &str, attempt: u16) -> String {
    format!("📘 {subject} · {title}\n👤 Студент: {student}\n🔁 Попытка {attempt}")
}

pub fn late_submission_line(late: LatePenalty, max_score: u16) -> String {
    let points = late.points();
    match late.deducted_from() {
        DeductedFrom::WorkScore => format!(
            "⏳ Сдана с опозданием: штраф −{points} снижает балл работы, максимум {} из {max_score}",
            Grade::new(max_score, max_score, Some(late)).work_score()
        ),
        DeductedFrom::CourseTotal => format!(
            "⏳ Сдана с опозданием: штраф −{points} вычитается из итога по предмету, балл работы не снижает"
        ),
    }
}

pub fn review_decision(decision: ReviewDecision, grade: Option<Grade>) -> String {
    let decision = match decision {
        ReviewDecision::Accept => "✅ Решение: принять работу.",
        ReviewDecision::Reject => "❌ Решение: отклонить работу.",
    };
    match grade {
        Some(grade) => format!("{decision}\n{}", grade_line(grade)),
        None => decision.to_owned(),
    }
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

pub const SANDBOX_EMPTY: &str = "🧪 В песочнице нет работ на проверке.";
pub const SANDBOX_CHOOSE: &str = "Откройте работу или примите решение сразу по всем:";
pub const SANDBOX_NOTHING_TO_DECIDE: &str =
    "В песочнице не осталось работ, отправленных до открытия этого списка: их уже проверили.";
pub const BUTTON_ACCEPT_ALL: &str = "✅ Принять все";
pub const BUTTON_REJECT_ALL: &str = "❌ Отклонить все";

pub fn sandbox_header(total: u32) -> String {
    format!("{BUTTON_SANDBOX}\nРабот на проверке: {total}")
}

pub fn sandbox_entry(student: &str, title: &str, sent_at: &str) -> String {
    format!("👤 {student} · {title}\nотправлено {sent_at}")
}

pub fn sandbox_button(student: &str, title: &str) -> String {
    format!("👤 {student} · {title}")
}

pub fn bulk_review_prompt(decision: ReviewDecision, count: u32) -> String {
    let question = match decision {
        ReviewDecision::Accept => format!(
            "Принять все работы в песочнице, отправленные до открытия этого списка: {count}? Оценочные задания получат максимальный балл с учётом штрафа за опоздание, а принятую работу студент пересдать не сможет."
        ),
        ReviewDecision::Reject => format!(
            "Отклонить все работы в песочнице, отправленные до открытия этого списка: {count}? Каждое отклонение тратит одну попытку студента."
        ),
    };
    format!("{question}\n\nНапишите один комментарий для всех студентов:")
}

pub fn bulk_review_saved(decision: ReviewDecision, count: usize) -> String {
    let outcome = match decision {
        ReviewDecision::Accept => "✅ Принято",
        ReviewDecision::Reject => "❌ Отклонено",
    };
    format!(
        "{outcome} работ в песочнице: {count}. Студенты получат результат вместе с вашим комментарием."
    )
}

pub const NO_COURSES: &str = "📘 У вас пока нет предметов.";
pub const CHOOSE_COURSE: &str = "📘 Выберите предмет:";
pub const NO_COURSE_ASSIGNMENTS: &str = "По этому предмету пока нет заданий.";
pub const COURSE_ASSIGNMENTS_HEADER: &str = "Задания по предмету:";
pub const CHOOSE_ASSIGNMENT: &str = "Выберите задание или создайте новое:";
pub const ASSIGNMENT_PUBLISHED: &str = "🚀 Задание опубликовано.";
pub const ASSIGNMENT_ARCHIVED: &str = "📦 Задание архивировано.";
pub const BUTTON_CREATE_ASSIGNMENT: &str = "➕ Создать задание";
pub const BUTTON_GRADEBOOK: &str = "📊 Оценки";
pub const BUTTON_GROUP_INVITE: &str = "🔗 Ссылка для группы";
pub const BUTTON_BACK_TO_COURSES: &str = "⬅️ К списку предметов";
pub const BUTTON_BACK_TO_COURSE: &str = "⬅️ К предмету";
pub const BUTTON_BACK_TO_COURSE_ASSIGNMENTS: &str = "⬅️ К заданиям предмета";
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

pub const BUTTON_OTHER_AMOUNT: &str = "✍️ Другое число";
pub const BUTTON_OTHER_MAX_SCORE: &str = "✍️ Другой максимум";
pub const BUTTON_PASS_FAIL: &str = "⏭ Без баллов, зачёт/незачёт";
pub const BUTTON_CALENDAR: &str = "🗓 Выбрать дату";
pub const BUTTON_MANUAL: &str = "✍️ Ввести вручную";
pub const BUTTON_PICK_DATE: &str = "Выбрать дату";
pub const BUTTON_OTHER_PERIODS: &str = "Другое число периодов";
pub const BUTTON_APPLY_PERIODS: &str = "Применить число периодов";
pub const BUTTON_POLICY_WORK_SCORE: &str = "После срока: штраф в балл работы";
pub const BUTTON_POLICY_COURSE_TOTAL: &str = "После срока: штраф в итог";
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

pub fn until(moment: &str) -> String {
    format!("до {moment}")
}

pub fn since(moment: &str) -> String {
    format!("с {moment}")
}

pub fn end_of_day(date: &str) -> String {
    format!("конца дня {date}")
}

pub fn date_preset(label: &str, date: &str) -> String {
    format!("📅 {label} · {date}")
}

pub fn periods_option(duration: &str, until: &str) -> String {
    format!("{duration} · до {until}")
}

pub fn penalty_step(question: &str, period_days: u16) -> String {
    format!(
        "{question}\nНовый штраф начисляется с началом каждого периода в {period_days} дн. Введите его размер целым числом от 1 до {MAX_PENALTY_PER_PERIOD} баллов:"
    )
}

pub fn duration_confirmation(selected: &str, duration: &str, closing: &str) -> String {
    format!(
        "Запрошен приём {selected}. После округления вверх до целых периодов длительность опоздания будет {duration}.\n{SUBMISSIONS_CLOSE_LABEL} {closing}. Применить рассчитанную дату?"
    )
}

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

pub const NO_STUDENTS: &str = "В группе пока нет студентов.";
pub const STUDENTS_HEADER: &str = "👥 Студенты группы:";
pub const STUDENT_PENDING_MARK: &str = " · ⏳ сдача на проверке";
pub const STUDENT_ACCEPTED_MARK: &str = " · ✅ работа принята";
pub const CHOOSE_STUDENT_PROMPT: &str = "Выберите студента:";
pub const EXTRA_ATTEMPTS_QUESTION: &str = "Сколько дополнительных попыток выдать?";
pub const GRADEBOOK_CHOOSE_STUDENT: &str = "📊 Оценки по предмету. Выберите студента:";
pub const NO_SCORED_ASSIGNMENTS: &str = "По этому предмету пока нет оценочных заданий.";
pub const BUTTON_BACK_TO_STUDENTS: &str = "⬅️ К студентам группы";

pub fn student_entry(name: &str, used: u16, extra: u16, marks: &str) -> String {
    format!("{name} · использовано {used} · дополнительных {extra}{marks}")
}

pub fn student_button(name: &str) -> String {
    format!("👤 {name}")
}

pub fn attempts_granted(amount: u16, total: u16) -> String {
    format!("🎁 Добавлено попыток: {amount}. Всего дополнительных: {total}.")
}

pub fn grades_header(
    name: &str,
    work_total: u32,
    penalty_total: u32,
    total: i64,
    max_total: u32,
) -> String {
    if penalty_total == 0 {
        return format!("👤 {name}\n🏅 Итого: {total} из {max_total}");
    }
    format!(
        "👤 {name}\n📚 Сумма работ: {work_total}\n⏳ Штрафы за опоздание: −{penalty_total}\n🏅 Итого: {total} из {max_total}"
    )
}

pub fn grade_entry(title: &str, score: &str, max_score: u16, course_total_penalty: u16) -> String {
    let penalty = if course_total_penalty > 0 {
        format!(" · штраф −{course_total_penalty} в итог")
    } else {
        String::new()
    };
    format!("{title} · {score} из {max_score}{penalty}")
}

pub const INVITE_ROTATED: &str = "♻️ Прежняя ссылка больше не работает.";
pub const BUTTON_ROTATE_INVITE: &str = "♻️ Перевыпустить ссылку";
pub const BUTTON_CONFIRM_ROTATE: &str = "♻️ Перевыпустить";

pub fn invite_header(group: &str) -> String {
    format!("🔗 Ссылка для регистрации в группу {group}:")
}

pub const BUTTON_PREVIOUS_PAGE: &str = "◀️ Назад";
pub const BUTTON_NEXT_PAGE: &str = "Вперёд ▶️";

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

pub const REVIEW_ACCEPTED: &str = "✅ Работа принята.";
pub const REVIEW_REJECTED: &str = "❌ Работа отклонена.";

pub fn review_notification(
    verdict: &str,
    subject: &str,
    title: &str,
    grade: Option<Grade>,
    comment: &str,
) -> String {
    let grade = grade
        .map(|grade| format!("\n{}", grade_line(grade)))
        .unwrap_or_default();
    format!(
        "{verdict}\n📘 Предмет: {subject}\n📚 Задание: {title}{grade}\n\n{REVIEW_COMMENT_HEADER}\n{comment}"
    )
}

pub fn grade_line(grade: Grade) -> String {
    let max_score = grade.max_score();
    match grade.late() {
        None => format!("🏅 Балл: {} из {max_score}", grade.work_score()),
        Some(late) if late.deducted_from() == DeductedFrom::WorkScore => format!(
            "🏅 Балл: {} − {} = {} из {max_score} (штраф за опоздание)",
            grade.raw_score(),
            late.points(),
            grade.work_score(),
        ),
        Some(late) => format!(
            "🏅 Балл: {} из {max_score} (штраф за опоздание −{} вычитается из итога по предмету)",
            grade.work_score(),
            late.points(),
        ),
    }
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::{LatePenalty, LatePolicy};

    fn late_by_one_period(deducted_from: DeductedFrom) -> Option<LatePenalty> {
        LatePolicy::new(7, 4, 4, deducted_from)
            .unwrap()
            .late_penalty(1)
    }

    fn late(penalty_per_period: u16, periods: u32, deducted_from: DeductedFrom) -> LatePenalty {
        LatePolicy::new(7, penalty_per_period, 4, deducted_from)
            .unwrap()
            .late_penalty(periods)
            .unwrap()
    }

    #[test]
    fn a_late_card_shows_the_maximum_left_after_a_penalty_to_the_work_score() {
        let line = late_submission_line(late(4, 1, DeductedFrom::WorkScore), 10);

        assert!(line.contains("−4"), "{line}");
        assert!(line.contains("6 из 10"), "{line}");
    }

    #[test]
    fn a_penalty_above_the_scale_leaves_a_maximum_of_zero() {
        let line = late_submission_line(late(6, 2, DeductedFrom::WorkScore), 10);

        assert!(line.contains("−12"), "{line}");
        assert!(line.contains("0 из 10"), "{line}");
    }

    #[test]
    fn a_late_card_with_a_penalty_to_the_course_total_does_not_cap_the_score() {
        let line = late_submission_line(late(4, 1, DeductedFrom::CourseTotal), 10);

        assert!(line.contains("−4"), "{line}");
        assert!(!line.contains("из 10"), "{line}");
        assert_ne!(
            line,
            late_submission_line(late(4, 1, DeductedFrom::WorkScore), 10)
        );
    }

    #[test]
    fn the_score_question_explains_where_a_late_penalty_goes() {
        let on_time = review_score_question(10, None);
        let to_work = review_score_question(10, Some(late(4, 1, DeductedFrom::WorkScore)));
        let to_total = review_score_question(10, Some(late(4, 1, DeductedFrom::CourseTotal)));

        assert!(!on_time.contains('−'), "{on_time}");
        for question in [&to_work, &to_total] {
            assert!(question.starts_with(&on_time), "{question}");
            assert!(question.contains("−4"), "{question}");
        }
        assert_ne!(to_work, to_total);
    }

    #[test]
    fn the_decision_message_previews_the_grade_only_for_a_scored_acceptance() {
        let grade = Grade::new(8, 10, Some(late(4, 1, DeductedFrom::CourseTotal)));

        let accepted = review_decision(ReviewDecision::Accept, Some(grade));

        assert!(accepted.ends_with(&grade_line(grade)), "{accepted}");
        assert!(!review_decision(ReviewDecision::Reject, None).contains(&grade_line(grade)));
        assert_ne!(
            review_decision(ReviewDecision::Accept, None),
            review_decision(ReviewDecision::Reject, None)
        );
    }

    #[test]
    fn an_on_time_grade_names_only_the_work_score_and_the_scale() {
        let line = grade_line(Grade::new(8, 10, None));

        assert!(line.contains("8 из 10"), "{line}");
        assert!(!line.contains('−'), "{line}");
    }

    #[test]
    fn a_penalty_to_the_work_score_shows_the_raw_score_the_penalty_and_the_rest() {
        let line = grade_line(Grade::new(
            8,
            10,
            late_by_one_period(DeductedFrom::WorkScore),
        ));

        assert!(line.contains('8'), "{line}");
        assert!(line.contains("− 4"), "{line}");
        assert!(line.contains("4 из 10"), "{line}");
        assert!(!line.contains("8 из 10"), "{line}");
    }

    #[test]
    fn a_penalty_to_the_course_total_keeps_the_work_score_and_names_the_penalty() {
        let line = grade_line(Grade::new(
            8,
            10,
            late_by_one_period(DeductedFrom::CourseTotal),
        ));
        let to_work = grade_line(Grade::new(
            8,
            10,
            late_by_one_period(DeductedFrom::WorkScore),
        ));

        assert!(line.contains("8 из 10"), "{line}");
        assert!(line.contains("−4"), "{line}");
        assert_ne!(line, to_work);
    }
}
